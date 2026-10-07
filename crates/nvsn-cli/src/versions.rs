//! Local state (versions, aliases, default) and specification resolution.
//!
//! Local resolution does not touch the network: it uses the cached index if it exists to
//! recognize `lts/*` and codenames, and otherwise only the installed version numbers.
//! Writing `default` and aliases uses the store lock.

use crate::ctx::Ctx;
use crate::exit::{CliError, ErrorKind};
use nvsn_core::lock::FileLock;
use nvsn_core::nvmrc::{find_version_file, VersionFile};
use nvsn_core::remote::{LtsField, RemoteRelease};
use nvsn_core::spec::{parse_spec, resolve, VersionSpec};
use nvsn_core::Store;
use semver::Version;
use std::path::{Path, PathBuf};

/// Names that cannot be used as aliases (they are reserved words or specifications).
const RESERVED_ALIASES: [&str; 10] = [
    "default", "node", "latest", "newest", "lts", "stable", "unstable", "system", "current", "iojs",
];

/// Content of the `default` file that means "the newest installed version". It is resolved
/// on every read, so installing or removing versions moves the default along.
pub const DYNAMIC_DEFAULT: &str = "node";

/// Version installed in the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// Tag with the `v` prefix, e.g. `v20.11.1`. It is also the directory name.
    pub tag: String,
    /// Semver version.
    pub version: Version,
    /// Directory of the installation.
    pub dir: PathBuf,
}

/// Parses the specification written by the user.
///
/// # Errors
///
/// Returns `usage` (code 2) if the specification is not valid.
pub fn parse_input(input: &str) -> Result<VersionSpec, CliError> {
    // `newest` is a synonym of `latest` (nvm-windows vocabulary).
    let text = if input.trim().eq_ignore_ascii_case("newest") {
        "latest"
    } else {
        input
    };
    parse_spec(text).map_err(|e| {
        CliError::usage(format!("{e:#}"))
            .with_hint("examples: 20, 20.11.1, lts/*, lts/iron, latest")
    })
}

/// Lists the installed versions, sorted from lowest to highest.
///
/// # Errors
///
/// Returns a general error if `versions/` cannot be read.
pub fn list_installed(store: &Store) -> Result<Vec<Installed>, CliError> {
    let dir = store.versions_dir();
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            // On Windows, NotFound also appears when an intermediate path is a file.
            if store.root().is_file() {
                return Err(CliError::general(format!(
                    "{} is a file, not a directory",
                    store.root().display()
                ))
                .with_hint("set NVSN_DIR (or --dir) to a directory"));
            }
            return Ok(Vec::new());
        }
        Err(err) => {
            return Err(CliError::general(format!(
                "cannot read {}: {err}",
                dir.display()
            )))
        }
    };

    let mut out = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| CliError::general(format!("cannot read entry: {err}")))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        if name.starts_with('.') || !path.is_dir() {
            continue;
        }
        if let Some(version) = parse_tag(&name) {
            out.push(Installed {
                tag: name,
                version,
                dir: path,
            });
        }
    }
    out.sort_by(|a, b| a.version.cmp(&b.version));
    Ok(out)
}

/// Converts a `vX.Y.Z` tag into a semver version.
pub fn parse_tag(tag: &str) -> Option<Version> {
    tag.strip_prefix('v').and_then(|v| Version::parse(v).ok())
}

/// Directory with the `node` executable: `bin/` on Unix, the root on Windows.
pub fn bin_dir(version_dir: &Path) -> PathBuf {
    let bin = version_dir.join("bin");
    if bin.is_dir() {
        bin
    } else {
        version_dir.to_path_buf()
    }
}

/// Path of the `node` executable of an installation, if it exists.
pub fn node_path(version_dir: &Path) -> Option<PathBuf> {
    [
        version_dir.join("bin").join("node"),
        version_dir.join("node.exe"),
        version_dir.join("node"),
    ]
    .into_iter()
    .find(|candidate| candidate.is_file())
}

/// Readable label of the LTS line (`Iron`, `LTS` or `None`).
pub fn lts_label(lts: &LtsField) -> Option<String> {
    match lts {
        LtsField::Codename(name) => Some(name.clone()),
        LtsField::Flag(true) => Some("LTS".to_owned()),
        LtsField::Flag(false) => None,
    }
}

/// Default version stored in `NVSN_DIR/default`, if it exists. When the file holds
/// [`DYNAMIC_DEFAULT`], the answer is the newest installed stable version (none if there is none).
pub fn read_default(store: &Store) -> Option<String> {
    let text = std::fs::read_to_string(store.default_file())
        .ok()
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())?;
    if text == DYNAMIC_DEFAULT {
        return latest_installed(store).map(|inst| inst.tag);
    }
    Some(text)
}

/// Indicates whether the default follows the newest installed version (`default node`).
pub fn default_is_dynamic(store: &Store) -> bool {
    std::fs::read_to_string(store.default_file()).is_ok_and(|text| text.trim() == DYNAMIC_DEFAULT)
}

/// Newest installed stable version, or `None` if no stable version is installed.
pub fn latest_installed(store: &Store) -> Option<Installed> {
    list_installed(store)
        .ok()?
        .into_iter()
        .rev()
        .find(|inst| inst.version.pre.is_empty())
}

/// Saves the default version. Atomic write under the store lock.
///
/// # Errors
///
/// Returns an error if the directory cannot be created, the lock cannot be taken or the write fails.
pub fn write_default(ctx: &Ctx, tag: &str) -> Result<(), CliError> {
    ctx.ensure_store()?;
    let _lock = FileLock::lock(&ctx.store.lock_file()).map_err(CliError::classify)?;
    write_atomic(&ctx.store.default_file(), tag)
}

/// Checks that `name` is a valid alias.
///
/// # Errors
///
/// Returns `usage` if the name is empty, reserved or contains characters that are not allowed.
pub fn validate_alias_name(name: &str) -> Result<(), CliError> {
    let first_ok = name.chars().next().is_some_and(|c| c.is_ascii_alphabetic());
    let chars_ok = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.');
    let lower = name.to_ascii_lowercase();
    let looks_like_tag =
        lower.starts_with('v') && lower[1..].starts_with(|c: char| c.is_ascii_digit());

    if name.len() > 64 || !first_ok || !chars_ok || looks_like_tag {
        return Err(CliError::usage(format!("invalid alias name '{name}'"))
            .with_hint("use 1 to 64 letters, digits, '-', '_' or '.', starting with a letter"));
    }
    if RESERVED_ALIASES.contains(&lower.as_str()) {
        let hint = if lower == "default" {
            "use `nvsn default <version>` to set the default version".to_owned()
        } else {
            format!("'{name}' is reserved; choose another alias name")
        };
        return Err(CliError::usage(format!("'{name}' is a reserved name")).with_hint(hint));
    }
    Ok(())
}

/// Tag that an alias points to, if it exists.
pub fn read_alias(store: &Store, name: &str) -> Option<String> {
    std::fs::read_to_string(store.aliases_dir().join(name))
        .ok()
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
}

/// Creates or replaces an alias.
///
/// # Errors
///
/// Returns an error if the alias directory cannot be written.
pub fn write_alias(ctx: &Ctx, name: &str, tag: &str) -> Result<(), CliError> {
    ctx.ensure_store()?;
    let _lock = FileLock::lock(&ctx.store.lock_file()).map_err(CliError::classify)?;
    write_atomic(&ctx.store.aliases_dir().join(name), tag)
}

/// Removes an alias. Returns `false` if it did not exist.
///
/// # Errors
///
/// Returns an error if the file exists and cannot be deleted.
pub fn remove_alias(ctx: &Ctx, name: &str) -> Result<bool, CliError> {
    let path = ctx.store.aliases_dir().join(name);
    if !path.is_file() {
        return Ok(false);
    }
    let _lock = FileLock::lock(&ctx.store.lock_file()).map_err(CliError::classify)?;
    std::fs::remove_file(&path)
        .map_err(|err| CliError::general(format!("cannot remove {}: {err}", path.display())))?;
    Ok(true)
}

/// List of aliases (name, tag), sorted by name.
///
/// # Errors
///
/// Returns a general error if the alias directory cannot be read.
pub fn list_aliases(store: &Store) -> Result<Vec<(String, String)>, CliError> {
    let dir = store.aliases_dir();
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            // On Windows, NotFound also appears when an intermediate path is a file.
            if store.root().is_file() {
                return Err(CliError::general(format!(
                    "{} is a file, not a directory",
                    store.root().display()
                ))
                .with_hint("set NVSN_DIR (or --dir) to a directory"));
            }
            return Ok(Vec::new());
        }
        Err(err) => {
            return Err(CliError::general(format!(
                "cannot read {}: {err}",
                dir.display()
            )))
        }
    };
    let mut out: Vec<(String, String)> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_file())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            read_alias(store, &name).map(|tag| (name, tag))
        })
        .collect();
    out.sort();
    Ok(out)
}

/// Resolves a specification against the installed versions, without network.
///
/// # Errors
///
/// Returns `version-not-found` (code 6) if no installed version matches,
/// or `unsupported` for `system`.
pub fn resolve_installed(ctx: &Ctx, input: &str) -> Result<Installed, CliError> {
    let spec = parse_input(input)?;
    resolve_installed_spec(ctx, &spec, input)
}

/// Same as [`resolve_installed`] with an already parsed specification.
///
/// # Errors
///
/// See [`resolve_installed`].
pub fn resolve_installed_spec(
    ctx: &Ctx,
    spec: &VersionSpec,
    input: &str,
) -> Result<Installed, CliError> {
    let installed = list_installed(&ctx.store)?;
    match spec {
        VersionSpec::Alias(name) => {
            let target = if name == "default" {
                read_default(&ctx.store)
            } else {
                read_alias(&ctx.store, name)
            };
            let Some(tag) = target else {
                let hint = if name == "default" {
                    "run `nvsn default <version>` to set it".to_owned()
                } else {
                    format!("run `nvsn alias {name} <version>` to define it")
                };
                return Err(
                    CliError::not_found(format!("alias '{name}' is not defined")).with_hint(hint),
                );
            };
            installed_by_tag(&installed, &tag)
        }
        VersionSpec::Current => {
            let Some((tag, _)) = effective_version(ctx) else {
                return Err(
                    CliError::not_found("no version is active and no default is set").with_hint(
                        "pass a version, or set a default with `nvsn default <version>`",
                    ),
                );
            };
            installed_by_tag(&installed, &tag)
        }
        VersionSpec::System => Err(CliError::unsupported(
            "'system' is not supported: nvsn only manages versions it installed",
        )
        .with_hint("pass an installed version such as 20")),
        _ => {
            let index = ctx.cached_index().unwrap_or_default();
            let releases: Vec<RemoteRelease> = installed
                .iter()
                .map(|inst| {
                    index
                        .iter()
                        .find(|release| release.version == inst.version)
                        .cloned()
                        .unwrap_or_else(|| local_release(&inst.version))
                })
                .collect();
            let found = resolve(spec, &releases).map_err(|e| {
                // If the index does have the version, the problem is that it is not installed.
                match resolve(spec, &index) {
                    Ok(remote) => CliError::not_found(format!(
                        "{} matches '{input}' but is not installed",
                        remote.tag()
                    )),
                    // Without an index in the cache we do not know whether it exists remotely: we say so.
                    Err(_) if index.is_empty() => {
                        CliError::not_found(format!("'{input}' is not installed"))
                    }
                    Err(_) => CliError::not_found(format!("{e:#}")),
                }
                .with_hint(format!("run `nvsn install {input}` to install it"))
            })?;
            installed_by_tag(&installed, &found.tag())
        }
    }
}

/// Resolves a specification against the remote index.
///
/// # Errors
///
/// Returns `version-not-found` (code 6) if no release matches.
pub fn resolve_remote<'a>(
    releases: &'a [RemoteRelease],
    spec: &VersionSpec,
    input: &str,
) -> Result<&'a RemoteRelease, CliError> {
    resolve(spec, releases).map_err(|e| {
        CliError::not_found(format!("{e:#}")).with_hint(format!(
            "run `nvsn list-remote` to see the versions for '{input}'"
        ))
    })
}

/// Searches for `.nvmrc` or `.node-version` from the current directory upwards.
///
/// # Errors
///
/// `version-file-missing` if there is none (code 3) and `version-file-invalid`
/// if the file found is not valid.
pub fn read_nvmrc(ctx: &Ctx) -> Result<VersionFile, CliError> {
    let cwd = std::env::current_dir()
        .map_err(|err| CliError::general(format!("cannot read the current directory: {err}")))?;
    let home = ctx.home();
    let found = find_version_file(&cwd, home.as_deref()).map_err(|e| {
        CliError::new(ErrorKind::VersionFileInvalid, format!("{e:#}"))
            .with_hint("keep exactly one version per .nvmrc, e.g. `20`")
    })?;
    found.ok_or_else(|| {
        CliError::new(
            ErrorKind::VersionFileMissing,
            format!(
                "no .nvmrc or .node-version found from {} upwards",
                cwd.display()
            ),
        )
        .with_hint(
            "run `nvsn install 20`, or create a .nvmrc with a version, e.g. `echo 20 > .nvmrc`",
        )
    })
}

/// Writes a file atomically (temporary file and `rename`).
fn write_atomic(path: &Path, content: &str) -> Result<(), CliError> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = path.with_file_name(format!("{name}.tmp"));
    std::fs::write(&tmp, content)
        .map_err(|err| CliError::general(format!("cannot write {}: {err}", tmp.display())))?;
    std::fs::rename(&tmp, path)
        .map_err(|err| CliError::general(format!("cannot update {}: {err}", path.display())))
}

fn installed_by_tag(installed: &[Installed], tag: &str) -> Result<Installed, CliError> {
    installed
        .iter()
        .find(|inst| inst.tag == tag)
        .cloned()
        .ok_or_else(|| {
            CliError::not_found(format!("{tag} is not installed")).with_hint(format!(
                "run `nvsn install {}` to install it",
                tag.trim_start_matches('v')
            ))
        })
}

/// Minimal release for an installed version without an entry in the cache.
fn local_release(version: &Version) -> RemoteRelease {
    RemoteRelease {
        version: version.clone(),
        date: String::new(),
        files: Vec::new(),
        npm: None,
        lts: LtsField::Flag(false),
        security: false,
    }
}

/// Effective version of the user: the one of the session (`NVSN_VERSION`) or, if there is none,
/// the default one. Returns the tag and the origin (`session` or `default`).
pub fn effective_version(ctx: &Ctx) -> Option<(String, &'static str)> {
    ctx.active_version()
        .map(|tag| (tag, "session"))
        .or_else(|| read_default(&ctx.store).map(|tag| (tag, "default")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn parses_tags() {
        assert_eq!(parse_tag("v20.11.1"), Version::parse("20.11.1").ok());
        assert_eq!(parse_tag("20.11.1"), None);
        assert_eq!(parse_tag("vfoo"), None);
    }

    #[test]
    fn alias_names_are_validated() {
        assert!(validate_alias_name("work").is_ok());
        assert!(validate_alias_name("my-node.18").is_ok());
        assert!(validate_alias_name("").is_err());
        assert!(validate_alias_name("18").is_err());
        assert!(validate_alias_name("v20").is_err());
        assert!(validate_alias_name("default").is_err());
        assert!(validate_alias_name("LTS").is_err());
        assert!(validate_alias_name("a/b").is_err());
    }

    #[test]
    fn lists_installed_sorted_and_skips_staging() {
        let dir = tempdir().expect("tempdir");
        let store = Store::new(dir.path());
        for name in ["v20.11.1", "v9.0.0", ".tmp-x", "v18.0.0"] {
            std::fs::create_dir_all(store.version_dir(name)).expect("mkdir");
        }
        let tags: Vec<String> = list_installed(&store)
            .expect("list")
            .into_iter()
            .map(|i| i.tag)
            .collect();
        assert_eq!(tags, ["v9.0.0", "v18.0.0", "v20.11.1"]);
    }

    #[test]
    fn missing_versions_dir_is_empty() {
        let dir = tempdir().expect("tempdir");
        let store = Store::new(dir.path().join("absent"));
        assert!(list_installed(&store).expect("list").is_empty());
    }

    #[test]
    fn default_round_trip_trims_whitespace() {
        let dir = tempdir().expect("tempdir");
        let store = Store::new(dir.path());
        store.create_dirs().expect("dirs");
        std::fs::write(store.default_file(), "v20.11.1\n").expect("write");
        assert_eq!(read_default(&store).as_deref(), Some("v20.11.1"));
    }

    #[test]
    fn newest_is_a_synonym_of_latest() {
        assert_eq!(parse_input("newest").ok(), Some(VersionSpec::Latest));
        assert_eq!(parse_input(" NEWEST ").ok(), Some(VersionSpec::Latest));
        assert_eq!(parse_input("latest").ok(), Some(VersionSpec::Latest));
        assert!(validate_alias_name("newest").is_err());
    }

    #[test]
    fn dynamic_default_follows_the_newest_stable_version() {
        let dir = tempdir().expect("tempdir");
        let store = Store::new(dir.path());
        for name in ["v18.20.0", "v20.11.1", "v21.0.0-rc.1"] {
            std::fs::create_dir_all(store.version_dir(name)).expect("mkdir");
        }
        store.create_dirs().expect("dirs");
        std::fs::write(store.default_file(), DYNAMIC_DEFAULT).expect("write");

        assert!(default_is_dynamic(&store));
        // The prerelease is not picked: the newest stable one is.
        assert_eq!(read_default(&store).as_deref(), Some("v20.11.1"));

        std::fs::create_dir_all(store.version_dir("v22.1.0")).expect("mkdir");
        assert_eq!(read_default(&store).as_deref(), Some("v22.1.0"));
    }

    #[test]
    fn dynamic_default_without_versions_is_none() {
        let dir = tempdir().expect("tempdir");
        let store = Store::new(dir.path());
        store.create_dirs().expect("dirs");
        std::fs::write(store.default_file(), DYNAMIC_DEFAULT).expect("write");
        assert_eq!(read_default(&store), None);
    }

    #[test]
    fn fixed_default_is_not_dynamic() {
        let dir = tempdir().expect("tempdir");
        let store = Store::new(dir.path());
        store.create_dirs().expect("dirs");
        std::fs::write(store.default_file(), "v20.11.1").expect("write");
        assert!(!default_is_dynamic(&store));
        assert_eq!(read_default(&store).as_deref(), Some("v20.11.1"));
    }

    #[test]
    fn lts_labels() {
        assert_eq!(
            lts_label(&LtsField::Codename("Iron".into())).as_deref(),
            Some("Iron")
        );
        assert_eq!(lts_label(&LtsField::Flag(true)).as_deref(), Some("LTS"));
        assert_eq!(lts_label(&LtsField::Flag(false)), None);
    }
}
