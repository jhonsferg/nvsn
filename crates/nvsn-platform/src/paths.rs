//! nvsn root directory (`NVSN_DIR`) and its default values.
//!
//! Resolution order (ARCHITECTURE.md §8.1): `--dir` > `NVSN_DIR` variable >
//! `NVSN_DIR` variable > platform default (ADR-014, ADR-015). `NVM_DIR` is never used (see below).
//!
//! Default values:
//! - Windows: `%LOCALAPPDATA%\nvsn`.
//! - Linux and macOS: `$XDG_DATA_HOME/nvsn` if the variable is absolute;
//!   otherwise `~/.local/share/nvsn`.
//! - Android (Termux): `$HOME/.nvsn`. If `HOME` is missing, it is derived from
//!   `$PREFIX` (`.../files/usr` -> `.../files/home`). Never under shared
//!   storage (`/sdcard`, `/storage/emulated`).
//!
//! `dirs::home_dir` is used only as a last resort for the home directory. The other
//! paths are built from environment variables, so they can be tested.

use crate::arch::Os;
use crate::env::EnvSource;
use anyhow::{anyhow, bail, Result};
use std::path::{Path, PathBuf};

/// nvsn's directory name on Windows, Linux and macOS.
const APP_DIR: &str = "nvsn";

/// Android shared-storage prefixes where nvsn is never installed.
const SHARED_STORAGE: [&str; 2] = ["/sdcard", "/storage/emulated"];

/// Resolves nvsn's root directory.
///
/// `cli_dir` is the value of `--dir`, if given. Order of precedence:
/// `--dir`, then `NVSN_DIR`, then the platform default. `NVM_DIR` is ignored: it belongs to nvm-sh.
///
/// # Errors
///
/// Returns an error if no usable value exists (e.g. Windows without
/// `LOCALAPPDATA` or a home directory), or if the default value would fall
/// in Android shared storage.
pub fn resolve_nvsn_dir(cli_dir: Option<&Path>, os: Os, env: &dyn EnvSource) -> Result<PathBuf> {
    if let Some(dir) = cli_dir {
        return Ok(dir.to_path_buf());
    }
    if let Some(dir) = env.var("NVSN_DIR") {
        return Ok(PathBuf::from(dir));
    }
    default_nvsn_dir(os, env)
}

/// Default root directory for `os`, ignoring `--dir` and `NVSN_DIR`.
///
/// # Errors
///
/// See [`resolve_nvsn_dir`].
pub fn default_nvsn_dir(os: Os, env: &dyn EnvSource) -> Result<PathBuf> {
    match os {
        Os::Windows => {
            let base = env
                .var("LOCALAPPDATA")
                .map(PathBuf::from)
                .or_else(dirs::data_local_dir)
                .ok_or_else(|| anyhow!("could not determine %LOCALAPPDATA%; set NVSN_DIR"))?;
            Ok(base.join(APP_DIR))
        }
        Os::Linux | Os::MacOs => {
            // The XDG specification requires absolute paths: a relative one is ignored.
            if let Some(xdg) = env
                .var("XDG_DATA_HOME")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
            {
                return Ok(xdg.join(APP_DIR));
            }
            Ok(home_dir(os, env)?
                .join(".local")
                .join("share")
                .join(APP_DIR))
        }
        Os::Android => {
            let dir = home_dir(os, env)?.join(".nvsn");
            reject_shared_storage(&dir)?;
            Ok(dir)
        }
    }
}

/// The user's home directory. On Termux, `HOME` is usually set; otherwise it is
/// derived from `PREFIX`, which points to `.../files/usr`.
fn home_dir(os: Os, env: &dyn EnvSource) -> Result<PathBuf> {
    if let Some(home) = env.var("HOME") {
        return Ok(PathBuf::from(home));
    }
    if os == Os::Android {
        if let Some(prefix) = env.var("PREFIX") {
            if let Some(files) = Path::new(&prefix).parent() {
                return Ok(files.join("home"));
            }
        }
    }
    dirs::home_dir()
        .ok_or_else(|| anyhow!("could not determine the home directory; set HOME or NVSN_DIR"))
}

/// Rejects paths inside Android shared storage. There are no execute permissions
/// or links there, and the content is visible to other apps.
fn reject_shared_storage(dir: &Path) -> Result<()> {
    for shared in SHARED_STORAGE {
        if dir.starts_with(shared) {
            bail!(
                "the default directory {} is in shared storage; set NVSN_DIR",
                dir.display()
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::MapEnv;

    fn absolute_path() -> String {
        std::env::temp_dir().display().to_string()
    }

    #[test]
    fn cli_dir_wins_over_env_and_default() {
        let env = MapEnv::new(&[("NVSN_DIR", "/from/env"), ("HOME", "/home/u")]);
        let cli = Path::new("/from/cli");
        assert_eq!(
            resolve_nvsn_dir(Some(cli), Os::Linux, &env).unwrap(),
            PathBuf::from("/from/cli")
        );
    }

    #[test]
    fn nvsn_dir_env_wins_over_default() {
        let env = MapEnv::new(&[("NVSN_DIR", "/from/env"), ("HOME", "/home/u")]);
        assert_eq!(
            resolve_nvsn_dir(None, Os::Linux, &env).unwrap(),
            PathBuf::from("/from/env")
        );
    }

    #[test]
    fn windows_default_uses_localappdata() {
        let env = MapEnv::new(&[("LOCALAPPDATA", "/local")]);
        assert_eq!(
            default_nvsn_dir(Os::Windows, &env).unwrap(),
            Path::new("/local").join("nvsn")
        );
    }

    #[test]
    fn nvm_dir_is_never_used_as_the_nvsn_root() {
        // nvm-sh keeps its own tree in NVM_DIR. nvsn must never adopt it: prune and
        // implode delete the root, so a shared root would destroy the nvm install.
        let env = MapEnv::new(&[("NVM_DIR", "/home/u/.nvm"), ("HOME", "/home/u")]);
        assert_ne!(
            resolve_nvsn_dir(None, Os::Linux, &env).unwrap(),
            PathBuf::from("/home/u/.nvm")
        );
        assert_eq!(
            resolve_nvsn_dir(None, Os::Linux, &env).unwrap(),
            default_nvsn_dir(Os::Linux, &env).unwrap()
        );
    }
    #[test]
    fn cli_dir_wins_over_nvm_dir() {
        let env = MapEnv::new(&[("NVM_DIR", "/home/u/.nvm")]);
        assert_eq!(
            resolve_nvsn_dir(Some(Path::new("/from/cli")), Os::Linux, &env).unwrap(),
            PathBuf::from("/from/cli")
        );
    }

    #[test]
    fn linux_default_uses_xdg_when_absolute() {
        let xdg = absolute_path();
        let env = MapEnv::new(&[("XDG_DATA_HOME", &xdg), ("HOME", "/home/u")]);
        assert_eq!(
            default_nvsn_dir(Os::Linux, &env).unwrap(),
            Path::new(&xdg).join("nvsn")
        );
    }

    #[test]
    fn relative_xdg_is_ignored() {
        let env = MapEnv::new(&[("XDG_DATA_HOME", "relative/data"), ("HOME", "/home/u")]);
        assert_eq!(
            default_nvsn_dir(Os::Linux, &env).unwrap(),
            Path::new("/home/u")
                .join(".local")
                .join("share")
                .join("nvsn")
        );
    }

    #[test]
    fn linux_and_macos_default_to_local_share() {
        let env = MapEnv::new(&[("HOME", "/home/u")]);
        let expected = Path::new("/home/u")
            .join(".local")
            .join("share")
            .join("nvsn");
        assert_eq!(default_nvsn_dir(Os::Linux, &env).unwrap(), expected);
        assert_eq!(default_nvsn_dir(Os::MacOs, &env).unwrap(), expected);
    }

    #[test]
    fn termux_default_is_home_dot_nvsn() {
        let env = MapEnv::new(&[("HOME", "/data/data/com.termux/files/home")]);
        assert_eq!(
            default_nvsn_dir(Os::Android, &env).unwrap(),
            Path::new("/data/data/com.termux/files/home").join(".nvsn")
        );
    }

    #[test]
    fn termux_home_is_derived_from_prefix_when_missing() {
        let env = MapEnv::new(&[("PREFIX", "/data/data/com.termux/files/usr")]);
        assert_eq!(
            default_nvsn_dir(Os::Android, &env).unwrap(),
            Path::new("/data/data/com.termux/files/home").join(".nvsn")
        );
    }

    #[test]
    fn termux_default_under_shared_storage_is_rejected() {
        let env = MapEnv::new(&[("HOME", "/sdcard/Download")]);
        let err = default_nvsn_dir(Os::Android, &env).unwrap_err();
        assert!(err.to_string().contains("NVSN_DIR"));
    }

    #[test]
    fn explicit_dir_under_shared_storage_is_allowed() {
        let env = MapEnv::new(&[]);
        let cli = Path::new("/sdcard/nvsn-test");
        assert_eq!(
            resolve_nvsn_dir(Some(cli), Os::Android, &env).unwrap(),
            PathBuf::from("/sdcard/nvsn-test")
        );
    }
}
