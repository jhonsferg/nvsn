//! End-to-end tests of `outdated` and `prune`. No real network: the index comes from a
//! loopback server and the store is temporary.

mod common;

use assert_cmd::Command;
use common::{Route, TestServer};
use predicates::prelude::*;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const INDEX: &str = r#"[
    {"version":"v20.12.0","date":"2024-03-01","files":["linux-x64"],"lts":"Iron","security":false},
    {"version":"v20.11.1","date":"2024-02-01","files":["linux-x64"],"lts":"Iron","security":false},
    {"version":"v20.11.0","date":"2024-01-01","files":["linux-x64"],"lts":"Iron","security":false},
    {"version":"v18.20.0","date":"2024-02-10","files":["linux-x64"],"lts":"Hydrogen","security":false}
]"#;

/// `nvsn` command isolated in `dir`, no network notice.
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

/// Creates the fake installation of `tag` (only its directory) and returns its path.
fn install(dir: &Path, tag: &str) -> PathBuf {
    let version = dir.join("store").join("versions").join(tag);
    std::fs::create_dir_all(&version).expect("version dir");
    version
}

fn serve_index(server: &TestServer) {
    server.set_route("/index.json", Route::ok(INDEX.as_bytes().to_vec()));
}

#[test]
fn outdated_reports_patches_behind_and_missing_lines() {
    let _lock = common::exec_lock();
    let dir = sandbox();
    install(dir.path(), "v20.11.0");
    install(dir.path(), "v20.11.1");
    install(dir.path(), "v18.0.0");
    let server = TestServer::start();
    serve_index(&server);
    let base = server.base();

    nvsn(dir.path())
        .args(["--allow-http", "--mirror", base.as_str(), "outdated"])
        .assert()
        .success()
        .stdout(predicate::str::contains("v20.11.0"))
        .stdout(predicate::str::contains("1 patch behind"))
        .stdout(predicate::str::contains("up to date"))
        .stdout(predicate::str::contains("no data in the index"));

    assert_eq!(server.request_count("/index.json"), 1);
}

#[test]
fn outdated_json_has_one_entry_per_installed_version() {
    let _lock = common::exec_lock();
    let dir = sandbox();
    install(dir.path(), "v20.11.0");
    let server = TestServer::start();
    serve_index(&server);
    let base = server.base();

    nvsn(dir.path())
        .args([
            "--json",
            "--allow-http",
            "--mirror",
            base.as_str(),
            "outdated",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"command\":\"outdated\""))
        .stdout(predicate::str::contains("\"status\":\"behind\""))
        .stdout(predicate::str::contains("\"latest\":\"v20.11.1\""))
        .stdout(predicate::str::contains("\"patches_behind\":1"));
}

#[test]
fn outdated_without_installed_versions_does_not_touch_the_network() {
    let _lock = common::exec_lock();
    let dir = sandbox();
    let server = TestServer::start();
    serve_index(&server);
    let base = server.base();

    nvsn(dir.path())
        .args(["--allow-http", "--mirror", base.as_str(), "outdated"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No Node.js versions installed"));

    assert_eq!(server.request_count("/index.json"), 0);
}

/// Store with `v20.11.1` as the default version and `v18.20.0` with no references.
fn store_with_unreferenced_version(dir: &Path) -> (PathBuf, PathBuf) {
    let default = install(dir, "v20.11.1");
    let stale = install(dir, "v18.20.0");
    std::fs::write(dir.join("store").join("default"), "v20.11.1").expect("default");
    (default, stale)
}

#[test]
fn prune_dry_run_lists_the_version_and_deletes_nothing() {
    let dir = sandbox();
    let (default, stale) = store_with_unreferenced_version(dir.path());

    nvsn(dir.path())
        .args(["prune", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("v18.20.0"))
        .stdout(predicate::str::contains("Dry run"));

    assert!(stale.is_dir());
    assert!(default.is_dir());
}

#[test]
fn prune_without_a_terminal_or_force_exits_9_and_deletes_nothing() {
    let dir = sandbox();
    let (_, stale) = store_with_unreferenced_version(dir.path());

    nvsn(dir.path())
        .arg("prune")
        .assert()
        .code(9)
        .stderr(predicate::str::contains("--yes"));

    assert!(stale.is_dir());
}

#[test]
fn prune_force_removes_only_the_unreferenced_version() {
    let dir = sandbox();
    let (default, stale) = store_with_unreferenced_version(dir.path());

    nvsn(dir.path())
        .args(["prune", "--force"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Pruned 1 version"));

    assert!(!stale.exists());
    assert!(default.is_dir(), "the default version is never pruned");
}

#[test]
fn prune_keeps_a_version_named_by_the_project_file() {
    let dir = sandbox();
    let (_, stale) = store_with_unreferenced_version(dir.path());
    std::fs::write(dir.path().join(".nvmrc"), "18\n").expect("nvmrc");

    nvsn(dir.path())
        .args(["prune", "--force"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Nothing to prune"));

    assert!(stale.is_dir());
}

#[test]
fn prune_scan_dir_keeps_versions_named_by_nested_projects() {
    let dir = sandbox();
    let (_, stale) = store_with_unreferenced_version(dir.path());
    let app = dir.path().join("projects").join("app");
    std::fs::create_dir_all(&app).expect("project");
    std::fs::write(app.join(".nvmrc"), "18.20.0\n").expect("nvmrc");

    nvsn(dir.path())
        .args(["prune", "--force", "--scan-dir"])
        .arg(dir.path().join("projects"))
        .assert()
        .success();

    assert!(stale.is_dir());
}

#[test]
fn prune_never_removes_the_session_version() {
    let dir = sandbox();
    let (_, stale) = store_with_unreferenced_version(dir.path());

    nvsn(dir.path())
        .env("NVSN_VERSION", "v18.20.0")
        .args(["prune", "--force"])
        .assert()
        .success();

    assert!(stale.is_dir());
}

#[test]
fn prune_scan_dir_must_be_a_directory() {
    let dir = sandbox();
    install(dir.path(), "v20.11.1");
    let file = dir.path().join("not-a-dir.txt");
    std::fs::write(&file, "x").expect("file");

    nvsn(dir.path())
        .args(["prune", "--force", "--scan-dir"])
        .arg(&file)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("is not a directory"));
}
