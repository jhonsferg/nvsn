//! Tests for `nvsn self-uninstall`. They work on a copy of the binary in a
//! temporary directory, never on the cargo binary.

mod common;

use common::{copy_binary, nvsn};
use predicates::prelude::*;
use std::path::Path;

/// Creates a store with realistic content (versions, aliases, cache).
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
fn without_yes_and_without_a_terminal_it_exits_9_and_deletes_nothing() {
    let _lock = common::exec_lock();
    let sandbox = tempfile::tempdir().expect("sandbox");
    let store = sandbox.path().join("store");
    populate_store(&store);
    let bin = copy_binary(sandbox.path());

    nvsn(&bin, sandbox.path())
        .arg("self-uninstall")
        .assert()
        .code(9)
        .stderr(predicate::str::contains("--yes"));

    assert!(store.join("versions/v20.11.1/bin/node").is_file());
    assert!(store.join("aliases/default").is_file());
    assert!(bin.is_file());
}

#[test]
fn with_yes_removes_the_data_directory_and_the_binary_only() {
    let _lock = common::exec_lock();
    let sandbox = tempfile::tempdir().expect("sandbox");
    let store = sandbox.path().join("store");
    populate_store(&store);
    let bin = copy_binary(sandbox.path());
    let sibling = sandbox.path().join("bin").join("other-tool.txt");
    std::fs::write(&sibling, b"not nvsn").expect("sibling");
    let outside = sandbox.path().join("keep.txt");
    std::fs::write(&outside, b"user file").expect("outside");

    nvsn(&bin, sandbox.path())
        .args(["--yes", "--json", "self-uninstall"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"removed\""));

    assert!(!store.exists(), "data directory must be removed");
    assert!(
        !bin.exists(),
        "binary must be removed (or moved aside on Windows)"
    );
    assert!(sibling.is_file(), "files next to the binary are not ours");
    assert_eq!(std::fs::read(&outside).expect("outside"), b"user file");
}

#[test]
fn refuses_a_data_directory_that_contains_home() {
    let _lock = common::exec_lock();
    let sandbox = tempfile::tempdir().expect("sandbox");
    let home = sandbox.path().join("home");
    std::fs::create_dir_all(home.join("documents")).expect("home");
    std::fs::write(home.join("documents").join("notes.txt"), b"precious").expect("notes");
    let bin = copy_binary(sandbox.path());

    nvsn(&bin, sandbox.path())
        .env("NVSN_DIR", sandbox.path())
        .args(["--yes", "self-uninstall"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("home directory"));

    assert!(home.join("documents").join("notes.txt").is_file());
    assert!(bin.is_file());
}
