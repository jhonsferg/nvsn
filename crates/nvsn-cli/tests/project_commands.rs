//! End-to-end tests of `local` and `path`. No network: the store and HOME are temporary.

use assert_cmd::Command;
use predicates::prelude::*;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// `nvsn` command isolated in `dir`: no active version, no network notice.
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

/// Creates the installation of `tag` (only the directory and its `bin`) and returns the path of `bin`.
fn install_dir(dir: &Path, tag: &str) -> PathBuf {
    let bin = dir.join("store").join("versions").join(tag).join("bin");
    std::fs::create_dir_all(&bin).expect("bin dir");
    bin
}

fn read_nvmrc(dir: &Path) -> String {
    std::fs::read_to_string(dir.join(".nvmrc")).expect("nvmrc written")
}

#[test]
fn local_writes_the_plain_version_without_the_v_prefix() {
    let dir = sandbox();
    install_dir(dir.path(), "v20.11.1");

    nvsn(dir.path())
        .args(["local", "v20.11.1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("20.11.1"));

    assert_eq!(read_nvmrc(dir.path()), "20.11.1\n");
}

#[test]
fn local_latest_writes_the_literal_word() {
    let dir = sandbox();

    nvsn(dir.path())
        .args(["local", "latest"])
        .assert()
        .success();

    assert_eq!(read_nvmrc(dir.path()), "latest\n");
}

#[test]
fn local_warns_but_still_writes_when_the_version_is_not_installed() {
    let dir = sandbox();

    nvsn(dir.path())
        .args(["local", "18.20.0"])
        .assert()
        .success()
        .stderr(predicate::str::contains("warning"))
        .stderr(predicate::str::contains("nvsn install 18.20.0"));

    assert_eq!(read_nvmrc(dir.path()), "18.20.0\n");
}

#[test]
fn local_does_not_warn_for_an_installed_version() {
    let dir = sandbox();
    install_dir(dir.path(), "v20.11.1");

    nvsn(dir.path())
        .args(["local", "20.11.1"])
        .assert()
        .success()
        .stderr(predicate::str::contains("warning").not());
}

#[test]
fn local_without_a_version_is_a_usage_error() {
    let dir = sandbox();

    nvsn(dir.path()).arg("local").assert().code(2);

    assert!(!dir.path().join(".nvmrc").exists());
}

#[test]
fn local_refuses_specs_that_cannot_be_pinned() {
    let dir = sandbox();

    nvsn(dir.path())
        .args(["local", "system"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("cannot be pinned"));

    assert!(!dir.path().join(".nvmrc").exists());
}

#[test]
fn path_prints_the_bin_directory_of_the_default_version() {
    let dir = sandbox();
    let bin = install_dir(dir.path(), "v20.11.1");
    std::fs::write(dir.path().join("store").join("default"), "v20.11.1").expect("default");

    let assert = nvsn(dir.path()).arg("path").assert().success();
    let out = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();

    assert_eq!(out.trim(), bin.display().to_string());
}

#[test]
fn path_with_a_version_prints_its_bin_directory() {
    let dir = sandbox();
    let bin = install_dir(dir.path(), "v20.11.1");

    let assert = nvsn(dir.path()).args(["path", "20"]).assert().success();
    let out = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();

    assert_eq!(out.trim(), bin.display().to_string());
}

#[test]
fn path_without_any_active_version_exits_6() {
    let dir = sandbox();
    install_dir(dir.path(), "v20.11.1");

    nvsn(dir.path())
        .arg("path")
        .assert()
        .code(6)
        .stderr(predicate::str::contains("no version is active"));
}

#[test]
fn path_for_a_version_that_is_not_installed_exits_6() {
    let dir = sandbox();
    install_dir(dir.path(), "v20.11.1");

    nvsn(dir.path()).args(["path", "18"]).assert().code(6);
}

#[test]
fn path_uses_the_session_version_over_the_default() {
    let dir = sandbox();
    install_dir(dir.path(), "v18.20.0");
    let session = install_dir(dir.path(), "v20.11.1");
    std::fs::write(dir.path().join("store").join("default"), "v18.20.0").expect("default");

    let assert = nvsn(dir.path())
        .env("NVSN_VERSION", "v20.11.1")
        .arg("path")
        .assert()
        .success();
    let out = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();

    assert_eq!(out.trim(), session.display().to_string());
}

#[test]
fn path_json_reports_the_version_and_the_path() {
    let dir = sandbox();
    install_dir(dir.path(), "v20.11.1");

    nvsn(dir.path())
        .args(["--json", "path", "20.11.1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"command\":\"path\""))
        .stdout(predicate::str::contains("\"version\":\"v20.11.1\""));
}
