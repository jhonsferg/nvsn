//! Installing a Node version from a mirror (ARCHITECTURE 4.1).
//!
//! Flow, in order:
//! 1. Per-version lock (`locks/<tag>.lock`), with a timeout.
//! 2. If `versions/<tag>` already exists, it is returned without touching anything.
//! 3. Download `SHASUMS256.txt` and the archive to `cache/downloads/`.
//! 4. Mandatory SHA-256 verification (ADR-033). If it fails, the file is deleted.
//! 5. Extraction into a staging area `versions/.tmp-<id>/` (archive::unpack).
//! 6. Validation of the binary and atomic `rename` to `versions/<tag>` (ADR-020).
//!
//! Nothing is visible in `versions/` until the `rename`. Temporary files are
//! removed through RAII even if any step fails.

#[cfg(test)]
pub(crate) mod test_server;

use crate::archive::extract::unpack;
use crate::artifact::artifact_for_release;
use crate::checksum::{parse_shasums, verify_sha256};
use crate::lock::FileLock;
use crate::remote::cache::unique_id;
use crate::remote::RemoteRelease;
use crate::store::Store;
use crate::tempdir::TempDir;
use anyhow::{bail, Context, Result};
use nvsn_net::download::{fetch, fetch_with_progress, NoProgress, ProgressSink};
use nvsn_net::{ensure_https, HttpClient};
use nvsn_platform::Platform;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

/// Default official mirror (ADR-027).
pub const DEFAULT_MIRROR: &str = "https://nodejs.org/dist";

/// Maximum time to wait for a version's lock (`NVSN_LOCK_TIMEOUT`).
pub const DEFAULT_LOCK_TIMEOUT: Duration = Duration::from_secs(600);

/// Install options decided by the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallOptions {
    /// Mirror base without a trailing slash. It must serve the nodejs.org layout.
    pub mirror: String,
    /// Maximum wait for the version lock.
    pub lock_timeout: Duration,
    /// Allows an `http://` mirror (`--allow-http`). Defaults to `false`.
    ///
    /// It must match the `allow_http` the `HttpClient` passed to [`install_version`]
    /// was built with.
    pub allow_http: bool,
}

impl Default for InstallOptions {
    fn default() -> Self {
        Self {
            mirror: DEFAULT_MIRROR.to_owned(),
            lock_timeout: DEFAULT_LOCK_TIMEOUT,
            allow_http: false,
        }
    }
}

/// Result of [`install_version`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallReport {
    /// Version label with the `v` prefix, e.g. `v20.11.1`.
    pub version: String,
    /// Final directory of the version inside the store.
    pub path: PathBuf,
    /// `true` if the version was already installed and nothing was done.
    pub already_installed: bool,
}

/// Installs `release` for `platform` into `store`, downloading from `options.mirror`.
///
/// It is idempotent: if the version is already in the store, it returns
/// `already_installed = true` without downloading anything.
///
/// # Errors
///
/// Returns an error if:
/// - The version lock is not acquired before `options.lock_timeout`.
/// - The platform has no artifact in that release.
/// - The download fails, or the mirror does not publish `SHASUMS256.txt` with the file.
/// - The SHA-256 does not match (ADR-033). The downloaded file is deleted.
/// - The archive cannot be extracted within the limits.
/// - The expected binary does not exist after extraction.
/// - The mirror is `http://` and `options.allow_http` is `false`.
pub fn install_version(
    client: &HttpClient,
    store: &Store,
    release: &RemoteRelease,
    platform: Platform,
    options: &InstallOptions,
) -> Result<InstallReport> {
    install_version_with_progress(client, store, release, platform, options, &NoProgress)
}

/// Same as [`install_version`], but reports the progress of the main file's
/// download to `progress` (byte bar in the CLI).
///
/// # Errors
///
/// The same as [`install_version`].
pub fn install_version_with_progress(
    client: &HttpClient,
    store: &Store,
    release: &RemoteRelease,
    platform: Platform,
    options: &InstallOptions,
    progress: &dyn ProgressSink,
) -> Result<InstallReport> {
    ensure_https(&options.mirror, options.allow_http)?;
    store.create_dirs()?;
    let tag = release.tag();
    let version_dir = store.version_dir(&tag);

    let _lock = acquire_version_lock(store, &tag, options.lock_timeout)?;
    if version_dir.is_dir() {
        return Ok(InstallReport {
            version: tag,
            path: version_dir,
            already_installed: true,
        });
    }

    let artifact = artifact_for_release(release, platform)?;
    let base = options.mirror.trim_end_matches('/');

    let downloads = store.cache_dir().join("downloads");
    std::fs::create_dir_all(&downloads)
        .with_context(|| format!("Failed to create {}", downloads.display()))?;
    let archive_path = downloads.join(&artifact.file_name);
    let shasums_path = downloads.join(format!("SHASUMS256-{tag}.txt"));
    // Deleted on exit, also on error. An unverified file is never left behind.
    let _cleanup = Cleanup(vec![archive_path.clone(), shasums_path.clone()]);

    let shasums_url = format!("{base}/{tag}/SHASUMS256.txt");
    fetch(client, &shasums_url, &shasums_path)
        .with_context(|| format!("Failed to download {shasums_url}"))?;
    let shasums = std::fs::read_to_string(&shasums_path)
        .with_context(|| format!("Failed to read {}", shasums_path.display()))?;
    let expected = parse_shasums(&shasums, &artifact.file_name).with_context(|| {
        format!(
            "SHASUMS256.txt for {tag} does not list {}; the mirror must publish checksums (ADR-033)",
            artifact.file_name
        )
    })?;

    let archive_url = format!("{base}/{tag}/{}", artifact.file_name);
    fetch_with_progress(client, &archive_url, &archive_path, progress)
        .with_context(|| format!("Failed to download {archive_url}"))?;
    verify_sha256(&archive_path, &expected)?;

    let staging = TempDir::new_in(store.versions_dir(), format!(".tmp-{}-", unique_id()))?;
    unpack(&archive_path, staging.path())?;

    let extracted_root = staging.path().join(&artifact.root_dir);
    let binary = extracted_root.join(artifact.bin_path);
    if !binary.is_file() {
        bail!(
            "archive {} does not contain {} under {}",
            artifact.file_name,
            artifact.bin_path,
            artifact.root_dir
        );
    }

    std::fs::rename(&extracted_root, &version_dir).with_context(|| {
        format!(
            "Failed to move the extracted {tag} into {}; if an antivirus or a terminal holds it, retry",
            version_dir.display()
        )
    })?;

    Ok(InstallReport {
        version: tag,
        path: version_dir,
        already_installed: false,
    })
}

/// Acquires the lock for `tag`, retrying until `timeout`.
fn acquire_version_lock(store: &Store, tag: &str, timeout: Duration) -> Result<FileLock> {
    let path: PathBuf = store.locks_dir().join(format!("{tag}.lock"));
    let deadline = Instant::now() + timeout;
    loop {
        match FileLock::try_lock(&path) {
            Ok(Some(lock)) => return Ok(lock),
            Ok(None) => {}
            Err(err) if is_contention(&err) => {}
            Err(err) => return Err(err),
        }
        if Instant::now() >= deadline {
            bail!(
                "timed out after {} s waiting for the lock on {tag}; \
                 another nvsn process is installing it (NVSN_LOCK_TIMEOUT)",
                timeout.as_secs()
            );
        }
        thread::sleep(Duration::from_millis(100));
    }
}

/// Indicates whether a `try_lock` failure means another process holds the lock.
///
/// On Windows, `fs2` returns ERROR_LOCK_VIOLATION (33) instead of `WouldBlock`
/// when the lock is taken, so it is recognized by code.
fn is_contention(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause.downcast_ref::<std::io::Error>().is_some_and(|io| {
            io.kind() == std::io::ErrorKind::WouldBlock
                || (cfg!(windows) && io.raw_os_error() == Some(33))
        })
    })
}

/// Deletes the given files on drop.
struct Cleanup(Vec<PathBuf>);

impl Drop for Cleanup {
    fn drop(&mut self) {
        for path in &self.0 {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install::test_server::{Route, TestServer};
    use crate::remote::parse_index;
    use nvsn_net::HttpOptions;
    use sha2::{Digest, Sha256};
    use std::path::Path;
    use tempfile::tempdir;

    const TAG: &str = "v20.11.1";
    const FILE: &str = "node-v20.11.1-linux-x64.tar.gz";
    const SHASUMS_PATH: &str = "/v20.11.1/SHASUMS256.txt";
    const ARCHIVE_PATH: &str = "/v20.11.1/node-v20.11.1-linux-x64.tar.gz";

    fn linux_x64() -> Platform {
        Platform {
            os: nvsn_platform::Os::Linux,
            arch: nvsn_platform::Arch::X64,
            libc: nvsn_platform::Libc::Glibc,
        }
    }

    fn release() -> RemoteRelease {
        parse_index(
            r#"[{"version":"v20.11.1","date":"2024-02-14","files":["linux-x64"],"lts":"Iron"}]"#,
        )
        .unwrap()
        .remove(0)
    }

    /// Builds a fake `.tar.gz` with `node-v20.11.1-linux-x64/bin/node`.
    fn tarball(with_binary: bool) -> Vec<u8> {
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::default(),
        ));
        let root = "node-v20.11.1-linux-x64";
        append(&mut builder, &format!("{root}/README.md"), b"readme");
        if with_binary {
            append(&mut builder, &format!("{root}/bin/node"), b"fake node");
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    fn append(
        builder: &mut tar::Builder<flate2::write::GzEncoder<Vec<u8>>>,
        name: &str,
        data: &[u8],
    ) {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder.append_data(&mut header, name, data).unwrap();
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    fn shasums_for(archive: &[u8], file: &str) -> String {
        format!("{}  {file}\n", sha256_hex(archive))
    }

    /// Server with consistent SHASUMS and archive for `archive`.
    fn server_with(archive: &[u8], shasums: &str) -> TestServer {
        let server = TestServer::start();
        server.set_route(SHASUMS_PATH, Route::ok(shasums.as_bytes().to_vec()));
        server.set_route(ARCHIVE_PATH, Route::ok(archive.to_vec()));
        server
    }

    fn entries(dir: &Path) -> Vec<String> {
        let Ok(read) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut names: Vec<String> = read
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// Options against the loopback server. `allow_http` is enabled only in
    /// tests: `TestServer` speaks http without TLS and never leaves 127.0.0.1.
    fn options(server: &TestServer) -> InstallOptions {
        InstallOptions {
            mirror: server.base(),
            lock_timeout: Duration::from_secs(5),
            allow_http: true,
        }
    }

    /// Client with an explicit `allow_http`, matching [`options`].
    fn client() -> HttpClient {
        HttpClient::with_options(HttpOptions {
            verbose: false,
            retries: 0,
            allow_http: true,
        })
        .unwrap()
    }

    #[test]
    fn installs_verifies_extracts_and_renames_atomically() {
        let archive = tarball(true);
        let server = server_with(&archive, &shasums_for(&archive, FILE));
        let dir = tempdir().unwrap();
        let store = Store::new(dir.path().join("nvsn"));

        let report = install_version(
            &client(),
            &store,
            &release(),
            linux_x64(),
            &options(&server),
        )
        .unwrap();

        assert_eq!(report.version, TAG);
        assert!(!report.already_installed);
        assert_eq!(report.path, store.version_dir(TAG));
        assert_eq!(
            std::fs::read(report.path.join("bin").join("node")).unwrap(),
            b"fake node"
        );
        assert_eq!(entries(&store.versions_dir()), vec![TAG.to_owned()]);
        assert!(entries(&store.cache_dir().join("downloads")).is_empty());
    }

    #[test]
    fn second_install_is_a_noop() {
        let archive = tarball(true);
        let server = server_with(&archive, &shasums_for(&archive, FILE));
        let dir = tempdir().unwrap();
        let store = Store::new(dir.path().join("nvsn"));
        let opts = options(&server);

        install_version(&client(), &store, &release(), linux_x64(), &opts).unwrap();
        let again = install_version(&client(), &store, &release(), linux_x64(), &opts).unwrap();

        assert!(again.already_installed);
        assert_eq!(server.request_count(ARCHIVE_PATH), 1);
        assert_eq!(server.request_count(SHASUMS_PATH), 1);
    }

    #[test]
    fn checksum_mismatch_aborts_and_leaves_nothing_visible() {
        let archive = tarball(true);
        let wrong = format!("{}  {FILE}\n", "0".repeat(64));
        let server = server_with(&archive, &wrong);
        let dir = tempdir().unwrap();
        let store = Store::new(dir.path().join("nvsn"));

        let err = install_version(
            &client(),
            &store,
            &release(),
            linux_x64(),
            &options(&server),
        )
        .unwrap_err();

        assert!(err.to_string().contains("Checksum mismatch"), "{err:#}");
        assert!(!store.version_dir(TAG).exists());
        assert!(entries(&store.versions_dir()).is_empty());
        assert!(entries(&store.cache_dir().join("downloads")).is_empty());
    }

    #[test]
    fn archive_without_binary_aborts_and_leaves_nothing_visible() {
        let archive = tarball(false);
        let server = server_with(&archive, &shasums_for(&archive, FILE));
        let dir = tempdir().unwrap();
        let store = Store::new(dir.path().join("nvsn"));

        let err = install_version(
            &client(),
            &store,
            &release(),
            linux_x64(),
            &options(&server),
        )
        .unwrap_err();

        assert!(err.to_string().contains("bin/node"), "{err:#}");
        assert!(!store.version_dir(TAG).exists());
        assert!(entries(&store.versions_dir()).is_empty());
    }

    #[test]
    fn missing_checksum_entry_is_an_error() {
        let archive = tarball(true);
        let other = format!("{}  node-v20.11.1-win-x64.zip\n", "a".repeat(64));
        let server = server_with(&archive, &other);
        let dir = tempdir().unwrap();
        let store = Store::new(dir.path().join("nvsn"));

        let err = install_version(
            &client(),
            &store,
            &release(),
            linux_x64(),
            &options(&server),
        )
        .unwrap_err();

        assert!(err.to_string().contains("does not list"), "{err:#}");
        assert!(!store.version_dir(TAG).exists());
        assert_eq!(server.request_count(ARCHIVE_PATH), 0);
    }

    #[test]
    fn mirror_without_shasums_is_an_error() {
        let server = TestServer::start();
        let dir = tempdir().unwrap();
        let store = Store::new(dir.path().join("nvsn"));

        let err = install_version(
            &client(),
            &store,
            &release(),
            linux_x64(),
            &options(&server),
        )
        .unwrap_err();

        assert!(err.to_string().contains("SHASUMS256.txt"), "{err:#}");
        assert!(!store.version_dir(TAG).exists());
    }

    #[test]
    fn install_times_out_when_another_process_holds_the_lock() {
        let dir = tempdir().unwrap();
        let store = Store::new(dir.path().join("nvsn"));
        store.create_dirs().unwrap();
        let held = FileLock::lock(&store.locks_dir().join(format!("{TAG}.lock"))).unwrap();

        let opts = InstallOptions {
            mirror: "http://127.0.0.1:1".to_owned(),
            lock_timeout: Duration::from_millis(200),
            allow_http: true,
        };
        let err = install_version(&client(), &store, &release(), linux_x64(), &opts).unwrap_err();

        assert!(err.to_string().contains("timed out"), "{err:#}");
        drop(held);
    }

    #[test]
    fn http_mirror_is_rejected_without_allow_http() {
        let archive = tarball(true);
        let server = server_with(&archive, &shasums_for(&archive, FILE));
        let dir = tempdir().unwrap();
        let store = Store::new(dir.path().join("nvsn"));
        let opts = InstallOptions {
            allow_http: false,
            ..options(&server)
        };
        let strict = HttpClient::new(false, 0).unwrap();

        let err = install_version(&strict, &store, &release(), linux_x64(), &opts).unwrap_err();

        assert!(err.to_string().contains("insecure URL rejected"), "{err:#}");
        assert_eq!(server.request_count(SHASUMS_PATH), 0);
        assert!(!store.version_dir(TAG).exists());
    }

    #[test]
    fn default_options_use_official_mirror_and_600_seconds() {
        let opts = InstallOptions::default();
        assert_eq!(opts.mirror, "https://nodejs.org/dist");
        assert_eq!(opts.lock_timeout, Duration::from_secs(600));
        assert!(!opts.allow_http);
    }
}
