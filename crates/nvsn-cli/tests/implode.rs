//! Tests for `nvsn implode`. They work on a copy of the binary in a
//! temporary directory, never on the cargo binary.

mod common;

use common::{copy_binary, nvsn};
use predicates::prelude::*;
use std::path::Path;

/// Block that `init --apply` writes into the bash profile.
const BASH_BLOCK: &str = "# nvsn init\neval \"$(nvsn env --shell bash)\"\n\n# nvsn wrapper\nnvsn() { command nvsn \"$@\"; }\n";

/// Creates a store with versions, aliases and cache.
fn populate_store(store: &Path) {
    let version = store.join("versions").join("v20.11.1").join("bin");
    std::fs::create_dir_all(&version).expect("version dir");
    std::fs::write(version.join("node"), b"node binary").expect("node");
    std::fs::create_dir_all(store.join("aliases")).expect("aliases");
    std::fs::write(store.join("aliases").join("default"), b"20").expect("alias");
    std::fs::create_dir_all(store.join("cache")).expect("cache");
    std::fs::write(store.join("cache").join("index.json"), b"[]").expect("index");
}

#[test]
fn without_force_and_without_a_terminal_it_exits_9_and_deletes_nothing() {
    let _lock = common::exec_lock();
    let sandbox = tempfile::tempdir().expect("sandbox");
    let store = sandbox.path().join("store");
    populate_store(&store);
    let bin = copy_binary(sandbox.path());

    nvsn(&bin, sandbox.path())
        .arg("implode")
        .assert()
        .code(9)
        .stderr(predicate::str::contains("--yes"));

    assert!(store.join("versions/v20.11.1/bin/node").is_file());
    assert!(bin.is_file());
}

#[test]
fn force_removes_the_store_the_profile_blocks_and_the_binary() {
    let _lock = common::exec_lock();
    let sandbox = tempfile::tempdir().expect("sandbox");
    let store = sandbox.path().join("store");
    populate_store(&store);
    let home = sandbox.path().join("home");
    std::fs::create_dir_all(&home).expect("home");
    let bashrc = home.join(".bashrc");
    std::fs::write(&bashrc, format!("keep me\n\n{BASH_BLOCK}")).expect("bashrc");
    let bin = copy_binary(sandbox.path());

    nvsn(&bin, sandbox.path())
        .args(["--json", "implode", "--force"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"removed\""));

    assert!(!store.exists(), "data directory must be removed");
    assert!(
        !bin.exists(),
        "binary must be removed (or moved aside on Windows)"
    );
    let profile = std::fs::read_to_string(&bashrc).expect("bashrc still exists");
    assert!(profile.contains("keep me"), "user lines stay");
    assert!(!profile.contains("# nvsn init"), "nvsn block is removed");
}

#[test]
fn refuses_a_data_directory_that_contains_home() {
    let _lock = common::exec_lock();
    let sandbox = tempfile::tempdir().expect("sandbox");
    let home = sandbox.path().join("home");
    std::fs::create_dir_all(&home).expect("home");
    populate_store(&home.join("store"));
    let bin = copy_binary(sandbox.path());

    nvsn(&bin, sandbox.path())
        .env("NVSN_DIR", &home)
        .args(["implode", "--force"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("home directory"));

    assert!(home.join("store/versions/v20.11.1/bin/node").is_file());
    assert!(bin.is_file());
}

#[test]
fn files_next_to_the_binary_are_not_ours() {
    let _lock = common::exec_lock();
    let sandbox = tempfile::tempdir().expect("sandbox");
    populate_store(&sandbox.path().join("store"));
    let bin = copy_binary(sandbox.path());
    let sibling = sandbox.path().join("bin").join("other-tool.txt");
    std::fs::write(&sibling, b"not nvsn").expect("sibling");

    nvsn(&bin, sandbox.path())
        .args(["--yes", "implode"])
        .assert()
        .success();

    assert!(sibling.is_file(), "files next to the binary are not ours");
}
