//! `nvsn init` without an explicit shell argument: it must auto-detect the
//! shell the same way `env`/`deactivate`/`on` already do, instead of requiring
//! the caller to pass one. This is what lets the installers run
//! `nvsn init --apply --yes` right after placing the binary, without having to
//! reimplement shell detection themselves in `sh`/PowerShell.

use assert_cmd::Command;
use predicates::prelude::*;
use std::path::Path;
use tempfile::TempDir;

fn nvsn(dir: &Path) -> Command {
    let mut cmd = Command::cargo_bin("nvsn").expect("nvsn binary is built");
    cmd.current_dir(dir)
        .env("NVSN_DIR", dir.join("store"))
        .env("HOME", dir)
        .env("USERPROFILE", dir)
        .env("NO_COLOR", "1")
        .env_remove("NVSN_VERSION")
        .env_remove("NVSN_INTEGRATION")
        .env("NVSN_NO_UPDATE_CHECK", "1");
    cmd
}

#[test]
fn init_without_a_shell_argument_detects_it_from_nvsn_shell() {
    let dir = TempDir::new().expect("tempdir");
    nvsn(dir.path())
        .arg("init")
        .env("NVSN_SHELL", "bash")
        .env_remove("SHELL")
        .env_remove("PSModulePath")
        .assert()
        .success()
        .stdout(predicate::str::contains("nvsn init bash --apply"));
}

#[test]
fn init_without_a_shell_argument_detects_it_from_shell_env_var() {
    let dir = TempDir::new().expect("tempdir");
    nvsn(dir.path())
        .arg("init")
        .env_remove("NVSN_SHELL")
        .env("SHELL", "/usr/bin/zsh")
        .env_remove("PSModulePath")
        .assert()
        .success()
        .stdout(predicate::str::contains("nvsn init zsh --apply"));
}

#[test]
fn init_with_an_explicit_shell_argument_ignores_detection() {
    let dir = TempDir::new().expect("tempdir");
    nvsn(dir.path())
        .args(["init", "fish"])
        .env("NVSN_SHELL", "bash")
        .assert()
        .success()
        .stdout(predicate::str::contains("nvsn init fish --apply"));
}
