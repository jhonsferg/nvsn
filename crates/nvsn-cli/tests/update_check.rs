//! Tests of the background update notice. The server is loopback
//! (`NVSN_TEST_API_BASE`); no test touches the network.

mod common;

use assert_cmd::Command;
use common::{Route, TestServer};
use predicates::prelude::*;
use std::path::Path;
use tempfile::TempDir;

const API_PATH: &str = "/repos/jhonsferg/nvsn/releases/latest";

/// `nvsn` command isolated in `dir` with the notice enabled and pointing to `server`.
fn notifying(dir: &Path, server: &TestServer) -> Command {
    let mut cmd = Command::cargo_bin("nvsn").expect("nvsn binary is built");
    cmd.current_dir(dir)
        .env("NVSN_DIR", dir.join("store"))
        .env("HOME", dir)
        .env("USERPROFILE", dir)
        .env("NO_COLOR", "1")
        .env_remove("NVSN_VERSION")
        .env_remove("NVSN_INTEGRATION")
        .env_remove("NVSN_NODEJS_ORG_MIRROR")
        .env_remove("CI")
        .env("NVSN_NO_UPDATE_CHECK", "")
        .env("NVSN_TEST_API_BASE", server.base())
        .arg("--allow-http");
    cmd
}

fn sandbox() -> TempDir {
    tempfile::tempdir().expect("temporary directory")
}

fn serve_tag(server: &TestServer, tag: &str) {
    let body = format!(r#"{{"tag_name":"{tag}"}}"#);
    server.set_route(API_PATH, Route::ok(body.into_bytes()));
}

#[test]
fn prints_a_notice_when_a_newer_release_exists() {
    let dir = sandbox();
    let server = TestServer::start();
    serve_tag(&server, "v999.0.0");

    notifying(dir.path(), &server)
        .arg("list")
        .assert()
        .success()
        .stderr(predicate::str::contains("Update available"))
        .stderr(predicate::str::contains("v999.0.0"))
        .stderr(predicate::str::contains("nvsn upgrade"));
}

#[test]
fn queries_the_api_at_most_once_per_day() {
    let dir = sandbox();
    let server = TestServer::start();
    serve_tag(&server, "v999.0.0");

    notifying(dir.path(), &server)
        .arg("list")
        .assert()
        .success();
    notifying(dir.path(), &server)
        .arg("list")
        .assert()
        .success();

    assert_eq!(server.request_count(API_PATH), 1);
    assert!(dir.path().join("store").join("update-check.json").is_file());
}

#[test]
fn no_notice_when_the_latest_release_is_not_newer() {
    let dir = sandbox();
    let server = TestServer::start();
    serve_tag(&server, "v0.0.1");

    notifying(dir.path(), &server)
        .arg("list")
        .assert()
        .success()
        .stderr(predicate::str::contains("Update available").not());
}

#[test]
fn a_failing_api_never_fails_the_command() {
    let dir = sandbox();
    let server = TestServer::start();

    notifying(dir.path(), &server)
        .arg("list")
        .assert()
        .success()
        .stderr(predicate::str::contains("Update available").not());
}

#[test]
fn the_switch_turns_the_check_off() {
    let dir = sandbox();
    let server = TestServer::start();
    serve_tag(&server, "v999.0.0");

    notifying(dir.path(), &server)
        .env("NVSN_NO_UPDATE_CHECK", "1")
        .arg("list")
        .assert()
        .success()
        .stderr(predicate::str::contains("Update available").not());

    assert_eq!(server.request_count(API_PATH), 0);
}

#[test]
fn ci_turns_the_check_off() {
    let dir = sandbox();
    let server = TestServer::start();
    serve_tag(&server, "v999.0.0");

    notifying(dir.path(), &server)
        .env("CI", "true")
        .arg("list")
        .assert()
        .success();

    assert_eq!(server.request_count(API_PATH), 0);
}

#[test]
fn offline_turns_the_check_off() {
    let dir = sandbox();
    let server = TestServer::start();
    serve_tag(&server, "v999.0.0");

    notifying(dir.path(), &server)
        .arg("--offline")
        .arg("list")
        .assert()
        .success();

    assert_eq!(server.request_count(API_PATH), 0);
}

#[test]
fn excluded_commands_never_query_the_api() {
    let dir = sandbox();
    let server = TestServer::start();
    serve_tag(&server, "v999.0.0");

    notifying(dir.path(), &server).arg("path").assert().code(6);
    notifying(dir.path(), &server)
        .args(["completions", "bash"])
        .assert()
        .success();

    assert_eq!(server.request_count(API_PATH), 0);
}
