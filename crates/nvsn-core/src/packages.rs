//! Global npm packages and npm upgrades, run through an installed Node version (nvm parity).
//!
//! Every external process started here is the `node` binary of the target
//! version, running that version's npm. The system `node` and `npm` are never
//! used. The npm global prefix is set to the version directory, so the
//! operations write only inside it and never need elevated privileges.

use crate::store::Store;
use anyhow::{bail, Context, Result};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Global packages that ship with every Node installation. They are never reinstalled.
const BUNDLED: [&str; 2] = ["npm", "corepack"];

/// Lists the global packages installed for version `tag`.
///
/// The list is read from the version's global `node_modules` directory, so no
/// process is started. Scoped packages keep their scope (`@scope/name`).
/// `npm` and `corepack` are excluded. The result is sorted.
///
/// # Errors
///
/// Returns an error if `tag` is not an installed version or the directory cannot be read.
pub fn list_global_packages(store: &Store, tag: &str) -> Result<Vec<String>> {
    let dir = installed_dir(store, tag)?;
    collect_packages(&modules_dir(&dir))
}

/// Reinstalls the global packages of `from_tag` into `to_tag`.
///
/// Runs `npm install -g <packages>` with the npm of `to_tag`, and with the
/// global prefix set to `to_tag`'s directory. Packages are installed by name,
/// so each one gets its latest release. Returns the names that were installed.
/// If `from_tag` has no global packages, nothing is run.
///
/// # Errors
///
/// Returns an error if either version is not installed, if `to_tag` has no npm,
/// or if npm fails. The npm output tail is included in the error.
pub fn reinstall_packages(store: &Store, from_tag: &str, to_tag: &str) -> Result<Vec<String>> {
    let from_dir = installed_dir(store, from_tag)?;
    let to_dir = installed_dir(store, to_tag)?;
    require_npm(&to_dir, to_tag)?;

    let packages = collect_packages(&modules_dir(&from_dir))?;
    if packages.is_empty() {
        return Ok(packages);
    }

    let mut args = vec!["install", "-g"];
    args.extend(packages.iter().map(String::as_str));
    run_npm(&to_dir, to_tag, &args)?;
    Ok(packages)
}

/// Upgrades the npm bundled with version `tag` to the latest release.
///
/// Runs `npm install -g npm@latest` with the node and npm of `tag`.
///
/// # Errors
///
/// Returns an error if `tag` is not installed, has no npm, or npm fails.
pub fn install_latest_npm(store: &Store, tag: &str) -> Result<()> {
    let dir = installed_dir(store, tag)?;
    require_npm(&dir, tag)?;
    run_npm(&dir, tag, &["install", "-g", "npm@latest"])
}

/// Returns the directory of an installed version, after checking the tag is well formed.
fn installed_dir(store: &Store, tag: &str) -> Result<PathBuf> {
    if !is_plain_tag(tag) {
        bail!("invalid Node version '{tag}'; expected a tag such as v20.11.1");
    }
    let dir = store.version_dir(tag);
    if !dir.is_dir() {
        bail!("Node {tag} is not installed; install it first");
    }
    Ok(dir)
}

/// `v` followed by digits and dots. Rejects anything that could escape `versions/`.
fn is_plain_tag(tag: &str) -> bool {
    tag.strip_prefix('v').is_some_and(|rest| {
        !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit() || c == '.')
    })
}

/// Node executable of an installed version.
fn node_exe(dir: &Path) -> PathBuf {
    if cfg!(windows) {
        dir.join("node.exe")
    } else {
        dir.join("bin").join("node")
    }
}

/// Directory added to `PATH` for a version, so its scripts find its own node.
fn bin_dir(dir: &Path) -> PathBuf {
    if cfg!(windows) {
        dir.to_path_buf()
    } else {
        dir.join("bin")
    }
}

/// Global `node_modules` of a version. It is the global prefix of its npm.
fn modules_dir(dir: &Path) -> PathBuf {
    if cfg!(windows) {
        dir.join("node_modules")
    } else {
        dir.join("lib").join("node_modules")
    }
}

/// npm's entry script, run with the version's own node.
fn npm_cli(dir: &Path) -> PathBuf {
    modules_dir(dir).join("npm").join("bin").join("npm-cli.js")
}

/// Fails with a clear message when the version has no npm.
fn require_npm(dir: &Path, tag: &str) -> Result<()> {
    let cli = npm_cli(dir);
    if !cli.is_file() {
        bail!(
            "npm is missing from Node {tag} ({}); reinstall that version and try again",
            cli.display()
        );
    }
    Ok(())
}

/// Reads the package names from a global `node_modules` directory.
fn collect_packages(modules: &Path) -> Result<Vec<String>> {
    if !modules.is_dir() {
        return Ok(Vec::new());
    }
    let mut names = Vec::new();
    for entry in read_dir(modules)? {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || BUNDLED.contains(&name.as_str()) {
            continue;
        }
        // `file_type` does not follow symlinks: `npm link` targets are local
        // development packages, not registry packages, and are skipped.
        let file_type = entry
            .file_type()
            .with_context(|| format!("failed to inspect {}", entry.path().display()))?;
        if !file_type.is_dir() {
            continue;
        }
        if name.starts_with('@') {
            for scoped in read_dir(&entry.path())? {
                let scoped_name = scoped.file_name().to_string_lossy().into_owned();
                let scoped_type = scoped
                    .file_type()
                    .with_context(|| format!("failed to inspect {}", scoped.path().display()))?;
                if scoped_type.is_dir() && !scoped_name.starts_with('.') {
                    names.push(format!("{name}/{scoped_name}"));
                }
            }
        } else {
            names.push(name);
        }
    }
    names.sort();
    names.dedup();
    Ok(names)
}

/// Lists a directory, turning read failures into errors that name the path.
fn read_dir(dir: &Path) -> Result<Vec<std::fs::DirEntry>> {
    std::fs::read_dir(dir)
        .with_context(|| format!("failed to read {}", dir.display()))?
        .map(|entry| entry.with_context(|| format!("failed to read {}", dir.display())))
        .collect()
}

/// Runs npm of version `tag` with `args`, using that version's node.
///
/// The global prefix is forced to `dir` (overriding any `prefix` in the user's
/// npmrc) and the version's bin directory is first in `PATH`.
fn run_npm(dir: &Path, tag: &str, args: &[&str]) -> Result<()> {
    let node = node_exe(dir);
    if !node.is_file() {
        bail!(
            "node is missing from Node {tag} ({}); reinstall that version and try again",
            node.display()
        );
    }
    let output = Command::new(&node)
        .arg(npm_cli(dir))
        .args(args)
        .env("NPM_CONFIG_PREFIX", dir)
        .env("PATH", prepend_path(&bin_dir(dir))?)
        .output()
        .with_context(|| format!("failed to start {}", node.display()))?;
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut tail: Vec<&str> = stderr.lines().rev().take(15).collect();
    tail.reverse();
    bail!(
        "npm {} failed in Node {tag} ({}): {}",
        args.join(" "),
        output.status,
        tail.join("\n")
    );
}

/// `PATH` with `first` in front of the current entries.
fn prepend_path(first: &Path) -> Result<OsString> {
    let current = std::env::var_os("PATH").unwrap_or_default();
    let mut entries = vec![first.to_path_buf()];
    entries.extend(std::env::split_paths(&current));
    std::env::join_paths(entries).context("PATH contains an entry that cannot be used")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Creates an installed version with an empty global `node_modules`.
    fn version(store: &Store, tag: &str) -> PathBuf {
        let dir = store.version_dir(tag);
        std::fs::create_dir_all(modules_dir(&dir)).unwrap();
        dir
    }

    /// Adds npm's entry script to a version.
    fn add_npm(dir: &Path) {
        let cli = npm_cli(dir);
        std::fs::create_dir_all(cli.parent().unwrap()).unwrap();
        std::fs::write(cli, "// fake npm\n").unwrap();
    }

    /// Adds a global package (possibly scoped, e.g. `@scope/name`) to a version.
    fn add_package(dir: &Path, name: &str) {
        std::fs::create_dir_all(modules_dir(dir).join(name)).unwrap();
    }

    #[test]
    fn lists_packages_sorted_with_scopes_and_without_bundled_ones() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path());
        let dir = version(&store, "v20.11.1");
        add_npm(&dir);
        add_package(&dir, "zeta");
        add_package(&dir, "a");
        add_package(&dir, "@scope/b");
        add_package(&dir, "corepack");
        add_package(&dir, ".cache");
        std::fs::write(modules_dir(&dir).join("notes.txt"), "not a package").unwrap();

        let packages = list_global_packages(&store, "v20.11.1").unwrap();
        assert_eq!(packages, vec!["@scope/b", "a", "zeta"]);
    }

    #[test]
    fn listing_a_version_without_global_packages_is_empty() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path());
        version(&store, "v18.0.0");
        assert!(list_global_packages(&store, "v18.0.0").unwrap().is_empty());
    }

    #[test]
    fn rejects_malformed_tags_and_uninstalled_versions() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path());
        version(&store, "v20.11.1");

        for bad in ["20.11.1", "v", "../v20.11.1", "v20/../x", "latest", ""] {
            assert!(list_global_packages(&store, bad).is_err(), "{bad}");
        }
        let err = list_global_packages(&store, "v22.0.0").unwrap_err();
        assert!(err.to_string().contains("not installed"));
    }

    #[test]
    fn reinstall_requires_npm_in_the_target_version() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path());
        let from = version(&store, "v20.11.1");
        add_package(&from, "typescript");
        version(&store, "v22.1.0");

        let err = reinstall_packages(&store, "v20.11.1", "v22.1.0").unwrap_err();
        assert!(err.to_string().contains("npm is missing from Node v22.1.0"));
    }

    #[test]
    fn reinstall_with_no_packages_runs_nothing() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path());
        version(&store, "v20.11.1");
        let to = version(&store, "v22.1.0");
        add_npm(&to);

        // There is no node binary in the target: it would fail if it were started.
        let installed = reinstall_packages(&store, "v20.11.1", "v22.1.0").unwrap();
        assert!(installed.is_empty());
    }

    #[test]
    fn reinstall_with_packages_fails_clearly_when_node_is_missing() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path());
        let from = version(&store, "v20.11.1");
        add_package(&from, "typescript");
        let to = version(&store, "v22.1.0");
        add_npm(&to);

        let err = reinstall_packages(&store, "v20.11.1", "v22.1.0").unwrap_err();
        assert!(err
            .to_string()
            .contains("node is missing from Node v22.1.0"));
    }

    #[test]
    fn install_latest_npm_requires_an_installed_version_with_npm() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path());
        assert!(install_latest_npm(&store, "v20.11.1").is_err());
        version(&store, "v20.11.1");
        let err = install_latest_npm(&store, "v20.11.1").unwrap_err();
        assert!(err.to_string().contains("npm is missing"));
    }

    /// Creates `bin/node` (unix) as a shell script that records its arguments and
    /// the npm prefix it was given in `out`.
    #[cfg(unix)]
    fn fake_node(dir: &Path, out: &Path) {
        use std::os::unix::fs::PermissionsExt;
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let node = bin.join("node");
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{out}'\nprintf '%s\\n' \"$NPM_CONFIG_PREFIX\" >> '{out}'\n",
            out = out.display()
        );
        std::fs::write(&node, script).unwrap();
        std::fs::set_permissions(&node, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn reinstall_runs_npm_of_the_target_with_its_own_prefix() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path());
        let from = version(&store, "v20.11.1");
        add_package(&from, "typescript");
        add_package(&from, "@scope/tool");
        let to = version(&store, "v22.1.0");
        add_npm(&to);
        let out = root.path().join("args.txt");
        fake_node(&to, &out);

        let installed = reinstall_packages(&store, "v20.11.1", "v22.1.0").unwrap();
        assert_eq!(installed, vec!["@scope/tool", "typescript"]);

        let recorded = std::fs::read_to_string(&out).unwrap();
        let lines: Vec<&str> = recorded.lines().collect();
        assert_eq!(lines[0], npm_cli(&to).display().to_string());
        assert_eq!(&lines[1..5], ["install", "-g", "@scope/tool", "typescript"]);
        assert_eq!(lines[5], to.display().to_string());
    }

    #[cfg(unix)]
    #[test]
    fn install_latest_npm_passes_the_expected_arguments() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path());
        let dir = version(&store, "v20.11.1");
        add_npm(&dir);
        let out = root.path().join("args.txt");
        fake_node(&dir, &out);

        install_latest_npm(&store, "v20.11.1").unwrap();
        let recorded = std::fs::read_to_string(&out).unwrap();
        let lines: Vec<&str> = recorded.lines().collect();
        assert_eq!(&lines[1..4], ["install", "-g", "npm@latest"]);
    }

    #[cfg(unix)]
    #[test]
    fn npm_failure_reports_the_tail_of_its_output() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path());
        let dir = version(&store, "v20.11.1");
        add_npm(&dir);
        let node = dir.join("bin").join("node");
        std::fs::create_dir_all(node.parent().unwrap()).unwrap();
        std::fs::write(
            &node,
            "#!/bin/sh\necho 'EACCES: permission denied' >&2\nexit 3\n",
        )
        .unwrap();
        std::fs::set_permissions(&node, std::fs::Permissions::from_mode(0o755)).unwrap();

        let err = install_latest_npm(&store, "v20.11.1")
            .unwrap_err()
            .to_string();
        assert!(err.contains("EACCES: permission denied"), "{err}");
    }
}
