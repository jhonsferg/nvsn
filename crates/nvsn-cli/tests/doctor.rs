//! End-to-end tests of `doctor`: exit code 1 with problems and check of the
//! `.nvmrc` of the current directory. No network, with a temporary store.

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
        .env_remove("NVSN_NODEJS_ORG_MIRROR")
        .env("NVSN_NO_UPDATE_CHECK", "1");
    cmd
}

fn sandbox() -> TempDir {
    tempfile::tempdir().expect("temporary directory")
}

#[test]
fn exits_1_when_no_default_version_is_set() {
    let dir = sandbox();

    nvsn(dir.path())
        .arg("doctor")
        .assert()
        .code(1)
        .stdout(predicate::str::contains("no default version"));
}

#[test]
fn exits_1_when_the_default_is_not_installed() {
    let dir = sandbox();
    std::fs::create_dir_all(dir.path().join("store")).expect("store");
    std::fs::write(dir.path().join("store").join("default"), "v20.11.1").expect("default");

    nvsn(dir.path())
        .arg("doctor")
        .assert()
        .code(1)
        .stdout(predicate::str::contains(
            "v20.11.1 is set as default but is not installed",
        ));
}

#[test]
fn reports_an_uninstalled_nvmrc_in_the_current_directory() {
    let dir = sandbox();
    std::fs::write(dir.path().join(".nvmrc"), "18.20.0\n").expect("nvmrc");

    nvsn(dir.path())
        .arg("doctor")
        .assert()
        .code(1)
        .stdout(predicate::str::contains(".nvmrc = 18.20.0"))
        .stdout(predicate::str::contains("nvsn install 18.20.0"));
}

#[test]
fn an_installed_nvmrc_in_the_current_directory_is_ok() {
    let dir = sandbox();
    let version = dir.path().join("store").join("versions").join("v20.11.1");
    std::fs::create_dir_all(&version).expect("version dir");
    std::fs::write(dir.path().join(".nvmrc"), "20.11.1\n").expect("nvmrc");

    nvsn(dir.path())
        .arg("doctor")
        .assert()
        .stdout(predicate::str::contains("(installed as v20.11.1)"));
}

#[test]
fn an_invalid_nvmrc_is_a_failure() {
    let dir = sandbox();
    std::fs::write(dir.path().join(".nvmrc"), "node=20\n").expect("nvmrc");

    nvsn(dir.path())
        .arg("doctor")
        .assert()
        .code(1)
        .stdout(predicate::str::contains(".nvmrc is invalid"));
}
