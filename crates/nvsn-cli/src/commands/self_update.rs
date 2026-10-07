//! `self-update`: replaces the nvsn binary with the latest GitHub release.
//!
//! Flow: queries the index of the latest release, compares versions (semver),
//! downloads the platform archive and `checksums.txt`, verifies the SHA-256
//! (mandatory, ADR-033) and only then extracts and replaces the binary. With
//! `--check` it only queries the release index.
//!
//! Environment variables: `NVSN_REPO` (default `jhonsferg/nvsn`),
//! `NVSN_TEST_API_BASE` and `NVSN_TEST_DL_BASE` for tests with a local
//! server. If they are `http://`, `--allow-http` is required.

use crate::ctx::Ctx;
use crate::exit::{CliError, ErrorKind};
use crate::progress::ByteBar;
use crate::prompt::confirm;
use nvsn_core::{parse_shasums, unpack, verify_sha256, TempDir};
use nvsn_net::download::fetch_with_progress;
use nvsn_net::{ensure_https, fetch, HttpClient, HttpOptions};
use nvsn_platform::env::{EnvSource, ProcessEnv};
use nvsn_platform::{Arch, Os, Platform};
use semver::Version;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Repository of nvsn releases.
pub const DEFAULT_REPO: &str = "jhonsferg/nvsn";
const DEFAULT_API_BASE: &str = "https://api.github.com";
const DEFAULT_DL_BASE: &str = "https://github.com";
const CHECKSUMS_FILE: &str = "checksums.txt";
const UNIX_BINARY: &str = "nvsn";
const WINDOWS_BINARY: &str = "nvsn.exe";
/// Maximum depth when searching for the binary inside the extracted archive.
const MAX_SEARCH_DEPTH: u8 = 3;

/// Runs `self-update` (alias `upgrade`). With `check` it only reports whether a
/// new version exists. With `force` it reinstalls even when there is no new version.
/// `retries` sets the network retries, with exponential back-off (1, 2, 4 s...).
///
/// # Errors
///
/// `usage` (2) if the URL is not https without `--allow-http`, `no-artifact` (4) if
/// the platform has no binary, `confirmation-required` (9) without a TTY and without
/// `--yes`, `download-failed` (7) if the network fails, `checksum-mismatch` (7) if
/// the SHA-256 does not match or is not published. On any verification error the
/// installed binary is not touched.
pub fn run(ctx: &Ctx, check: bool, force: bool, retries: u8) -> Result<(), CliError> {
    if ctx.offline {
        return Err(CliError::new(
            ErrorKind::OfflineNoCache,
            "self-update needs network access, but --offline is set",
        )
        .with_hint("run `nvsn self-update` without --offline"));
    }

    let current = current_version()?;
    let repo = repo()?;
    let http = HttpClient::with_options(HttpOptions {
        verbose: ctx.verbose,
        retries,
        allow_http: ctx.allow_http,
    })
    .map_err(|err| CliError::general(format!("cannot create the HTTP client: {err:#}")))?;
    let work = TempDir::new().map_err(|err| {
        CliError::general(format!("cannot create a temporary directory: {err:#}"))
    })?;

    ctx.out.note("Checking for updates...");
    let (tag, latest) = latest_release(ctx, &http, &repo, work.path())?;
    let newer = latest > current;

    if check || (!newer && !force) {
        let human = if newer {
            format!(
                "nvsn {current} -> {latest} is available. Run `nvsn self-update` to install it."
            )
        } else {
            format!("nvsn {current} is already up to date.")
        };
        ctx.out.result(
            "self-update",
            &human,
            json!({
                "current": current.to_string(),
                "latest": latest.to_string(),
                "update_available": newer,
                "updated": false,
            }),
        );
        return Ok(());
    }

    let platform = ctx.platform()?;
    let archive = artifact_name(platform)?;
    let exe = std::env::current_exe()
        .map_err(|err| CliError::general(format!("cannot locate the nvsn binary: {err}")))?;

    let question = if newer {
        format!("Update nvsn {current} to {latest} at {}?", exe.display())
    } else {
        format!("Reinstall nvsn {latest} at {}?", exe.display())
    };
    if !confirm(ctx, &question)? {
        return Err(CliError::general("update cancelled; nvsn was not changed"));
    }

    let base = format!("{}/{repo}/releases/download/{tag}", download_base());
    let sums_path = work.path().join(CHECKSUMS_FILE);
    fetch(&http, &format!("{base}/{CHECKSUMS_FILE}"), &sums_path).map_err(|err| {
        CliError::classify(err)
            .with_hint("the release must publish checksums.txt; nothing was changed")
    })?;
    let sums = std::fs::read_to_string(&sums_path)
        .map_err(|err| CliError::general(format!("cannot read {CHECKSUMS_FILE}: {err}")))?;
    let expected = parse_shasums(&sums, archive).ok_or_else(|| {
        CliError::new(
            ErrorKind::ChecksumMismatch,
            format!("{CHECKSUMS_FILE} has no SHA-256 for {archive}"),
        )
        .with_hint("refusing to update without a published checksum; nothing was changed")
    })?;

    let archive_path = work.path().join(archive);
    let bar = ByteBar::new(ctx.show_progress, format!("Downloading nvsn {tag}"));
    let outcome = fetch_with_progress(&http, &format!("{base}/{archive}"), &archive_path, &bar);
    bar.finish();
    outcome.map_err(CliError::classify)?;

    verify_sha256(&archive_path, &expected).map_err(|err| {
        CliError::new(ErrorKind::ChecksumMismatch, format!("{err:#}"))
            .with_hint("the download does not match the published checksum; nothing was changed")
    })?;

    let extract_dir = work.path().join("extract");
    std::fs::create_dir_all(&extract_dir).map_err(|err| {
        CliError::general(format!("cannot create {}: {err}", extract_dir.display()))
    })?;
    unpack(&archive_path, &extract_dir)
        .map_err(|err| CliError::general(format!("cannot extract {archive}: {err:#}")))?;

    let binary_name = if platform.os == Os::Windows {
        WINDOWS_BINARY
    } else {
        UNIX_BINARY
    };
    let new_binary = find_binary(&extract_dir, binary_name, MAX_SEARCH_DEPTH)
        .ok_or_else(|| CliError::general(format!("{archive} does not contain {binary_name}")))?;

    replace_binary_at(&new_binary, &exe)?;

    let human = format!("nvsn updated from {current} to {latest}.");
    ctx.out.result(
        "self-update",
        &human,
        json!({
            "current": current.to_string(),
            "latest": latest.to_string(),
            "update_available": true,
            "updated": true,
            "path": exe.display().to_string(),
        }),
    );
    Ok(())
}

/// Removes the `nvsn.exe.old` left by a previous update on Windows.
///
/// It is called when each command starts. On other platforms it does nothing.
pub fn remove_stale_old() {
    #[cfg(windows)]
    {
        if let Ok(exe) = std::env::current_exe() {
            let _ = std::fs::remove_file(old_path(&exe));
        }
    }
}

/// Path of the previous binary left behind when replacing an executable in use.
pub fn old_path(exe: &Path) -> PathBuf {
    let name = exe
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(WINDOWS_BINARY);
    exe.with_file_name(format!("{name}.old"))
}

/// Release archive name for a platform (nvsn publishing matrix).
///
/// # Errors
///
/// `no-artifact` (4) if the platform has no published binary, for example Termux.
pub fn artifact_name(platform: Platform) -> Result<&'static str, CliError> {
    let name = match (platform.os, platform.arch) {
        (Os::Linux, Arch::X64) => "nvsn_linux_x86_64.tar.gz",
        (Os::Linux, Arch::Arm64) => "nvsn_linux_aarch64.tar.gz",
        (Os::Linux, Arch::Armv7l) => "nvsn_linux_armv7.tar.gz",
        (Os::MacOs, Arch::X64) => "nvsn_darwin_x86_64.tar.gz",
        (Os::MacOs, Arch::Arm64) => "nvsn_darwin_aarch64.tar.gz",
        (Os::Windows, Arch::X64) => "nvsn_windows_x86_64.zip",
        (Os::Windows, Arch::Arm64) => "nvsn_windows_arm64.zip",
        _ => {
            let hint = if platform.os == Os::Android {
                "nvsn publishes no Android binary; build it from source with cargo"
            } else {
                "build nvsn from source, or download a binary from the GitHub releases page"
            };
            return Err(CliError::new(
                ErrorKind::NoArtifact,
                format!(
                    "no nvsn binary is published for {}-{}",
                    platform.os.dist_name(),
                    platform.arch.dist_name()
                ),
            )
            .with_hint(hint));
        }
    };
    Ok(name)
}

/// Version of the running binary.
fn current_version() -> Result<Version, CliError> {
    Version::parse(env!("CARGO_PKG_VERSION"))
        .map_err(|err| CliError::general(format!("invalid package version: {err}")))
}

/// Repository `owner/name` from `NVSN_REPO`, or the official repository.
fn repo() -> Result<String, CliError> {
    let repo = ProcessEnv
        .var("NVSN_REPO")
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_REPO.to_owned());
    validate_repo(&repo)?;
    Ok(repo)
}

/// Checks that `repo` has the form `owner/name` with characters that are safe in a URL.
fn validate_repo(repo: &str) -> Result<(), CliError> {
    let parts: Vec<&str> = repo.split('/').collect();
    let safe_part = |part: &str| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    };
    if parts.len() == 2 && parts.iter().all(|part| safe_part(part)) {
        Ok(())
    } else {
        Err(CliError::usage(format!(
            "NVSN_REPO must look like owner/name, got '{repo}'"
        )))
    }
}

/// GitHub API base, without a trailing slash. `NVSN_TEST_API_BASE` overrides it.
fn api_base() -> String {
    ProcessEnv
        .var("NVSN_TEST_API_BASE")
        .map(|value| value.trim_end_matches('/').to_owned())
        .unwrap_or_else(|| DEFAULT_API_BASE.to_owned())
}

/// Release download base, without a trailing slash. `NVSN_TEST_DL_BASE` overrides it.
fn download_base() -> String {
    ProcessEnv
        .var("NVSN_TEST_DL_BASE")
        .map(|value| value.trim_end_matches('/').to_owned())
        .unwrap_or_else(|| DEFAULT_DL_BASE.to_owned())
}

/// Queries the latest release and returns its tag and version.
fn latest_release(
    ctx: &Ctx,
    http: &HttpClient,
    repo: &str,
    work: &Path,
) -> Result<(String, Version), CliError> {
    let url = format!("{}/repos/{repo}/releases/latest", api_base());
    ensure_https(&url, ctx.allow_http).map_err(|err| {
        CliError::usage(format!("{err:#}")).with_hint(
            "the release index must use https; --allow-http is only for local test servers",
        )
    })?;

    let index_path = work.join("latest.json");
    fetch(http, &url, &index_path).map_err(|err| {
        CliError::classify(err).with_hint(format!(
            "check your connection; {repo} must have a published release"
        ))
    })?;
    let text = std::fs::read_to_string(&index_path)
        .map_err(|err| CliError::general(format!("cannot read the release index: {err}")))?;
    let body: Value = serde_json::from_str(&text)
        .map_err(|err| CliError::general(format!("invalid release index from GitHub: {err}")))?;
    let tag = body
        .get("tag_name")
        .and_then(Value::as_str)
        .ok_or_else(|| CliError::general("the release index has no tag_name"))?;
    let version = Version::parse(tag.trim_start_matches('v'))
        .map_err(|err| CliError::general(format!("cannot parse release tag '{tag}': {err}")))?;
    Ok((tag.to_owned(), version))
}

/// Replaces `exe` with `new_binary` without leaving it half-done.
///
/// The new binary is first copied next to the current one (same volume). On Unix
/// a `rename` puts it in place atomically. On Windows the executable in use cannot
/// be overwritten: it is renamed to `<name>.old`, the new one is placed and
/// deletion of the previous one is attempted; if it is still locked, the next run deletes it.
pub(crate) fn replace_binary_at(new_binary: &Path, exe: &Path) -> Result<(), CliError> {
    let file_name = exe
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| CliError::general(format!("invalid binary path {}", exe.display())))?;
    let staged = exe.with_file_name(format!(".{file_name}.new-{}", std::process::id()));
    let install_hint = "check write permissions for the install directory";

    std::fs::copy(new_binary, &staged).map_err(|err| {
        CliError::general(format!(
            "cannot stage the new binary next to {}: {err}",
            exe.display()
        ))
        .with_hint(install_hint)
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        if let Err(err) = std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))
        {
            let _ = std::fs::remove_file(&staged);
            return Err(CliError::general(format!(
                "cannot set permissions on the new binary: {err}"
            )));
        }
        if let Err(err) = std::fs::rename(&staged, exe) {
            let _ = std::fs::remove_file(&staged);
            return Err(
                CliError::general(format!("cannot replace {}: {err}", exe.display()))
                    .with_hint(install_hint),
            );
        }
    }

    #[cfg(windows)]
    {
        let old = old_path(exe);
        let _ = std::fs::remove_file(&old);
        if let Err(err) = std::fs::rename(exe, &old) {
            let _ = std::fs::remove_file(&staged);
            return Err(CliError::general(format!(
                "cannot move {} out of the way: {err}",
                exe.display()
            ))
            .with_hint(install_hint));
        }
        if let Err(err) = std::fs::rename(&staged, exe) {
            let _ = std::fs::rename(&old, exe);
            let _ = std::fs::remove_file(&staged);
            return Err(CliError::general(format!(
                "cannot place the new binary, previous version restored: {err}"
            ))
            .with_hint(install_hint));
        }
        // The process still uses the previous one: if it cannot be deleted, the next run deletes it.
        let _ = std::fs::remove_file(&old);
    }

    Ok(())
}

/// Searches for a file named `name` under `dir`, up to `depth` levels. Ignores links.
fn find_binary(dir: &Path, name: &str, depth: u8) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut subdirs = Vec::new();
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_file() && entry.file_name().to_str() == Some(name) {
            return Some(path);
        }
        if kind.is_dir() {
            subdirs.push(path);
        }
    }
    if depth == 0 {
        return None;
    }
    subdirs
        .into_iter()
        .find_map(|sub| find_binary(&sub, name, depth - 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nvsn_platform::Libc;

    fn platform(os: Os, arch: Arch) -> Platform {
        Platform {
            os,
            arch,
            libc: Libc::NotApplicable,
        }
    }

    #[test]
    fn artifact_names_follow_the_release_matrix() {
        assert_eq!(
            artifact_name(platform(Os::Linux, Arch::X64)).ok(),
            Some("nvsn_linux_x86_64.tar.gz")
        );
        assert_eq!(
            artifact_name(platform(Os::Linux, Arch::Arm64)).ok(),
            Some("nvsn_linux_aarch64.tar.gz")
        );
        assert_eq!(
            artifact_name(platform(Os::Linux, Arch::Armv7l)).ok(),
            Some("nvsn_linux_armv7.tar.gz")
        );
        assert_eq!(
            artifact_name(platform(Os::MacOs, Arch::Arm64)).ok(),
            Some("nvsn_darwin_aarch64.tar.gz")
        );
        assert_eq!(
            artifact_name(platform(Os::Windows, Arch::X64)).ok(),
            Some("nvsn_windows_x86_64.zip")
        );
        assert_eq!(
            artifact_name(platform(Os::Windows, Arch::Arm64)).ok(),
            Some("nvsn_windows_arm64.zip")
        );
    }

    #[test]
    fn termux_has_no_artifact_and_exits_with_code_4() {
        let err = artifact_name(platform(Os::Android, Arch::Arm64)).expect_err("no artifact");
        assert_eq!(err.exit_code(), 4);
        assert_eq!(err.code(), "no-artifact");
    }

    #[test]
    fn unpublished_architectures_have_no_artifact() {
        let err = artifact_name(platform(Os::Linux, Arch::Ppc64le)).expect_err("no artifact");
        assert_eq!(err.exit_code(), 4);
        assert!(artifact_name(platform(Os::Windows, Arch::X86)).is_err());
    }

    #[test]
    fn repo_must_be_owner_and_name() {
        assert!(validate_repo("jhonsferg/nvsn").is_ok());
        assert!(validate_repo("a/b/c").is_err());
        assert!(validate_repo("../etc").is_err());
        assert!(validate_repo("owner/na me").is_err());
        assert!(validate_repo("owner/..").is_err());
    }

    #[test]
    fn current_version_is_valid_semver() {
        assert!(current_version().is_ok());
    }

    #[test]
    fn old_path_appends_old_suffix() {
        let exe = Path::new("/opt/bin/nvsn.exe");
        assert_eq!(old_path(exe), Path::new("/opt/bin/nvsn.exe.old"));
    }

    #[test]
    fn replace_swaps_contents_and_leaves_no_staged_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let exe = dir.path().join("nvsn-target");
        std::fs::write(&exe, b"old contents").expect("write old");
        let new_binary = dir.path().join("downloaded");
        std::fs::write(&new_binary, b"new contents").expect("write new");

        replace_binary_at(&new_binary, &exe).expect("replace");

        assert_eq!(std::fs::read(&exe).expect("read"), b"new contents");
        let names: Vec<String> = std::fs::read_dir(dir.path())
            .expect("list")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(names.iter().all(|name| !name.contains(".new-")));
    }

    #[test]
    fn find_binary_searches_nested_directories() {
        let dir = tempfile::tempdir().expect("temp dir");
        let nested = dir.path().join("release").join("bin");
        std::fs::create_dir_all(&nested).expect("mkdir");
        std::fs::write(nested.join("nvsn"), b"x").expect("write");

        assert_eq!(
            find_binary(dir.path(), "nvsn", 3),
            Some(nested.join("nvsn"))
        );
        assert_eq!(find_binary(dir.path(), "nvsn", 1), None);
        assert_eq!(find_binary(dir.path(), "nvsn.exe", 3), None);
    }
}
