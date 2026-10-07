//! `nvsn default` keeps `<NVSN_DIR>/current` pointing to the global version
//! (ADR-034). These tests use the real binary on a temporary store: the
//! link is a junction on Windows and a symlink on Unix, with no special privileges.

use assert_cmd::Command;
use std::path::{Path, PathBuf};

/// `nvsn` command isolated in `dir`, with the store in `dir/store`.
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

/// Creates a fake installation with `bin/node`, which is what `node_path` looks for.
fn fake_install(store: &Path, tag: &str) -> PathBuf {
    let dir = store.join("versions").join(tag);
    std::fs::create_dir_all(dir.join("bin")).expect("fake install");
    std::fs::write(dir.join("bin").join("node"), b"").expect("fake node");
    dir
}

/// Checks that `link` resolves to the same directory as `target`.
fn assert_points_to(link: &Path, target: &Path) {
    let resolved = std::fs::canonicalize(link).expect("current resolves to a directory");
    let expected = std::fs::canonicalize(target).expect("target exists");
    assert_eq!(resolved, expected);
}

#[test]
fn default_creates_current_pointing_at_the_version() {
    let dir = tempfile::tempdir().expect("sandbox");
    let store = dir.path().join("store");
    let v20 = fake_install(&store, "v20.11.1");

    nvsn(dir.path()).args(["default", "20"]).assert().success();

    assert_points_to(&store.join("current"), &v20);
    assert!(store.join("current").join("bin").join("node").is_file());
    assert_eq!(
        std::fs::read_to_string(store.join("default")).expect("default file"),
        "v20.11.1"
    );
}

#[test]
fn default_moves_current_to_the_new_version() {
    let dir = tempfile::tempdir().expect("sandbox");
    let store = dir.path().join("store");
    fake_install(&store, "v20.11.1");
    let v22 = fake_install(&store, "v22.1.0");

    nvsn(dir.path()).args(["default", "20"]).assert().success();
    nvsn(dir.path()).args(["default", "22"]).assert().success();

    assert_points_to(&store.join("current"), &v22);
}

#[test]
fn without_a_default_there_is_no_current_link() {
    let dir = tempfile::tempdir().expect("sandbox");
    let store = dir.path().join("store");
    fake_install(&store, "v20.11.1");

    nvsn(dir.path()).arg("current").assert().success();

    assert!(
        std::fs::symlink_metadata(store.join("current")).is_err(),
        "there is no global version, so the link must not exist"
    );
}

#[test]
fn a_failed_default_keeps_the_previous_link_and_value() {
    let dir = tempfile::tempdir().expect("sandbox");
    let store = dir.path().join("store");
    let v20 = fake_install(&store, "v20.11.1");
    nvsn(dir.path()).args(["default", "20"]).assert().success();

    nvsn(dir.path()).args(["default", "99"]).assert().code(6);

    assert_points_to(&store.join("current"), &v20);
    assert_eq!(
        std::fs::read_to_string(store.join("default")).expect("default file"),
        "v20.11.1"
    );
}
