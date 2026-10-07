//! Tests for `nvsn self-update` against a local server. No real network.

mod common;

use common::{
    copy_binary, host_archive_name, nvsn, release_archive, sha256_hex, Route, TestServer,
};
use predicates::prelude::*;
use std::path::Path;

const API_PATH: &str = "/repos/jhonsferg/nvsn/releases/latest";

fn release_path(tag: &str, file: &str) -> String {
    format!("/jhonsferg/nvsn/releases/download/{tag}/{file}")
}

/// Schedules the index of release `tag` as the latest published one.
fn serve_latest(server: &TestServer, tag: &str) {
    let body = format!(r#"{{"tag_name":"{tag}"}}"#);
    server.set_route(API_PATH, Route::ok(body.into_bytes()));
}

/// Schedules the platform archive and `checksums.txt` with `sums` as content.
fn serve_artifacts(server: &TestServer, tag: &str, archive: Vec<u8>, sums: &str) {
    server.set_route(&release_path(tag, host_archive_name()), Route::ok(archive));
    server.set_route(
        &release_path(tag, "checksums.txt"),
        Route::ok(sums.as_bytes().to_vec()),
    );
}

/// Runs the nvsn copy against the local server (requires `--allow-http`).
fn run_update(
    bin: &Path,
    dir: &Path,
    server: &TestServer,
    args: &[&str],
) -> assert_cmd::assert::Assert {
    nvsn(bin, dir)
        .env("NVSN_TEST_API_BASE", server.base())
        .env("NVSN_TEST_DL_BASE", server.base())
        .arg("--allow-http")
        .args(args)
        .assert()
}

#[test]
fn check_reports_a_newer_release_and_downloads_nothing_else() {
    let _lock = common::exec_lock();
    let sandbox = tempfile::tempdir().expect("sandbox");
    let server = TestServer::start();
    serve_latest(&server, "v999.0.0");
    let bin = copy_binary(sandbox.path());

    let assert = run_update(
        &bin,
        sandbox.path(),
        &server,
        &["--json", "self-update", "--check"],
    );
    let output = assert.success().get_output().stdout.clone();
    let envelope: serde_json::Value = serde_json::from_slice(&output).expect("json envelope");

    assert_eq!(envelope["data"]["update_available"], true);
    assert_eq!(envelope["data"]["latest"], "999.0.0");
    assert_eq!(server.request_count(API_PATH), 1);
    assert_eq!(
        server.request_count(&release_path("v999.0.0", host_archive_name())),
        0
    );
    assert_eq!(
        server.request_count(&release_path("v999.0.0", "checksums.txt")),
        0
    );
}

#[test]
fn check_reports_up_to_date_for_an_older_release() {
    let _lock = common::exec_lock();
    let sandbox = tempfile::tempdir().expect("sandbox");
    let server = TestServer::start();
    serve_latest(&server, "v0.0.1");
    let bin = copy_binary(sandbox.path());

    run_update(
        &bin,
        sandbox.path(),
        &server,
        &["--json", "self-update", "--check"],
    )
    .success()
    .stdout(predicate::str::contains("\"update_available\":false"));
}

#[test]
fn plain_http_index_is_refused_without_allow_http() {
    let _lock = common::exec_lock();
    let sandbox = tempfile::tempdir().expect("sandbox");
    let server = TestServer::start();
    serve_latest(&server, "v999.0.0");
    let bin = copy_binary(sandbox.path());

    nvsn(&bin, sandbox.path())
        .env("NVSN_TEST_API_BASE", server.base())
        .args(["self-update", "--check"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--allow-http"));
    assert_eq!(server.request_count(API_PATH), 0);
}

#[test]
fn without_a_terminal_the_update_needs_yes_and_changes_nothing() {
    let _lock = common::exec_lock();
    let sandbox = tempfile::tempdir().expect("sandbox");
    let server = TestServer::start();
    let archive = release_archive(host_archive_name(), b"new nvsn");
    serve_latest(&server, "v999.0.0");
    serve_artifacts(
        &server,
        "v999.0.0",
        archive.clone(),
        &format!("{}  {}\n", sha256_hex(&archive), host_archive_name()),
    );
    let bin = copy_binary(sandbox.path());
    let before = std::fs::read(&bin).expect("read binary");

    run_update(&bin, sandbox.path(), &server, &["self-update"])
        .code(9)
        .stderr(predicate::str::contains("--yes"));

    assert_eq!(std::fs::read(&bin).expect("read binary"), before);
    assert_eq!(
        server.request_count(&release_path("v999.0.0", host_archive_name())),
        0
    );
}

#[test]
fn checksum_mismatch_aborts_and_keeps_the_installed_binary() {
    let _lock = common::exec_lock();
    let sandbox = tempfile::tempdir().expect("sandbox");
    let server = TestServer::start();
    let archive = release_archive(host_archive_name(), b"tampered nvsn");
    serve_latest(&server, "v999.0.0");
    let wrong = "0".repeat(64);
    serve_artifacts(
        &server,
        "v999.0.0",
        archive,
        &format!("{wrong}  {}\n", host_archive_name()),
    );
    let bin = copy_binary(sandbox.path());
    let before = std::fs::read(&bin).expect("read binary");

    run_update(&bin, sandbox.path(), &server, &["--yes", "self-update"])
        .code(7)
        .stderr(predicate::str::contains("Checksum mismatch"));

    assert_eq!(std::fs::read(&bin).expect("read binary"), before);
    let bin_dir_entries = std::fs::read_dir(bin.parent().expect("bin dir"))
        .expect("list bin dir")
        .count();
    assert_eq!(bin_dir_entries, 1, "no staged or temporary file may remain");
}

#[test]
fn missing_checksum_for_the_artifact_refuses_the_update() {
    let _lock = common::exec_lock();
    let sandbox = tempfile::tempdir().expect("sandbox");
    let server = TestServer::start();
    let archive = release_archive(host_archive_name(), b"unsigned nvsn");
    serve_latest(&server, "v999.0.0");
    serve_artifacts(
        &server,
        "v999.0.0",
        archive.clone(),
        &format!("{}  other-file.zip\n", sha256_hex(&archive)),
    );
    let bin = copy_binary(sandbox.path());
    let before = std::fs::read(&bin).expect("read binary");

    run_update(&bin, sandbox.path(), &server, &["--yes", "self-update"])
        .code(7)
        .stderr(predicate::str::contains("no SHA-256"));

    assert_eq!(std::fs::read(&bin).expect("read binary"), before);
}

#[test]
fn verified_release_replaces_the_binary() {
    let _lock = common::exec_lock();
    let sandbox = tempfile::tempdir().expect("sandbox");
    let server = TestServer::start();
    let content = b"brand new nvsn build";
    let archive = release_archive(host_archive_name(), content);
    serve_latest(&server, "v999.0.0");
    serve_artifacts(
        &server,
        "v999.0.0",
        archive.clone(),
        &format!("{}  {}\n", sha256_hex(&archive), host_archive_name()),
    );
    let bin = copy_binary(sandbox.path());

    run_update(
        &bin,
        sandbox.path(),
        &server,
        &["--yes", "--json", "self-update"],
    )
    .success()
    .stdout(predicate::str::contains("\"updated\":true"));

    assert_eq!(std::fs::read(&bin).expect("read binary"), content);
}

#[test]
fn upgrade_is_an_alias_of_self_update() {
    let _lock = common::exec_lock();
    let sandbox = tempfile::tempdir().expect("sandbox");
    let server = TestServer::start();
    serve_latest(&server, "v999.0.0");
    let bin = copy_binary(sandbox.path());

    run_update(
        &bin,
        sandbox.path(),
        &server,
        &["--json", "upgrade", "--check"],
    )
    .success()
    .stdout(predicate::str::contains("\"update_available\":true"));
}

#[test]
fn force_reinstalls_the_latest_release_even_when_up_to_date() {
    let _lock = common::exec_lock();
    let sandbox = tempfile::tempdir().expect("sandbox");
    let server = TestServer::start();
    let content = b"reinstalled nvsn build";
    let archive = release_archive(host_archive_name(), content);
    serve_latest(&server, "v0.0.1");
    serve_artifacts(
        &server,
        "v0.0.1",
        archive.clone(),
        &format!("{}  {}\n", sha256_hex(&archive), host_archive_name()),
    );
    let bin = copy_binary(sandbox.path());

    run_update(
        &bin,
        sandbox.path(),
        &server,
        &["--yes", "--json", "self-update", "--force"],
    )
    .success()
    .stdout(predicate::str::contains("\"updated\":true"));

    assert_eq!(std::fs::read(&bin).expect("read binary"), content);
}

#[test]
fn without_force_an_up_to_date_nvsn_changes_nothing() {
    let _lock = common::exec_lock();
    let sandbox = tempfile::tempdir().expect("sandbox");
    let server = TestServer::start();
    serve_latest(&server, "v0.0.1");
    let bin = copy_binary(sandbox.path());
    let before = std::fs::read(&bin).expect("read binary");

    run_update(
        &bin,
        sandbox.path(),
        &server,
        &["--yes", "--json", "self-update"],
    )
    .success()
    .stdout(predicate::str::contains("\"updated\":false"));

    assert_eq!(std::fs::read(&bin).expect("read binary"), before);
}

#[test]
fn retries_out_of_range_is_a_usage_error() {
    let _lock = common::exec_lock();
    let sandbox = tempfile::tempdir().expect("sandbox");
    let server = TestServer::start();
    let bin = copy_binary(sandbox.path());

    run_update(
        &bin,
        sandbox.path(),
        &server,
        &["self-update", "--retries", "11"],
    )
    .code(2);
    assert_eq!(server.request_count(API_PATH), 0);
}
