//! End-to-end tests of the `nvsn` binary. No network: the root directory is temporary
//! and `HOME` points to the same directory, so the `.nvmrc` search does not leave it.

use assert_cmd::Command;
use predicates::prelude::*;
use std::path::Path;
use tempfile::TempDir;

/// `nvsn` command isolated in `dir`: no integration, no active version and no mirror.
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
        // No update notice: no test command touches the network.
        .env("NVSN_NO_UPDATE_CHECK", "1");
    cmd
}

fn sandbox() -> TempDir {
    tempfile::tempdir().expect("temporary directory")
}

#[test]
fn version_flag_prints_package_version() {
    let dir = sandbox();
    nvsn(dir.path())
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("nvsn"))
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn help_lists_the_commands() {
    let dir = sandbox();
    nvsn(dir.path())
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("install"))
        .stdout(predicate::str::contains("list-remote"))
        .stdout(predicate::str::contains("completions"))
        .stdout(predicate::str::contains("doctor"));
}

#[test]
fn install_without_nvmrc_explains_how_to_fix_it() {
    let dir = sandbox();
    nvsn(dir.path())
        .arg("install")
        .assert()
        .code(3)
        .stderr(predicate::str::contains("no .nvmrc"))
        .stderr(predicate::str::contains("nvsn install 20"));
}

#[test]
fn list_on_empty_store_says_so() {
    let dir = sandbox();
    nvsn(dir.path())
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("No Node.js versions installed"));
}

#[test]
fn current_without_active_version_prints_none() {
    let dir = sandbox();
    nvsn(dir.path())
        .arg("current")
        .assert()
        .success()
        .stdout("none\n");
}

#[test]
fn env_for_bash_has_minimal_snapshot() {
    let dir = sandbox();
    let assert = nvsn(dir.path())
        .args(["env", "--shell", "bash"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
    let first_line = stdout.lines().next().unwrap_or_default();
    assert!(
        first_line.starts_with("export NVSN_DIR='"),
        "first line was: {first_line}"
    );
    assert!(stdout.contains("_nvsn_hook"), "hook missing: {stdout}");
    assert!(
        !stdout.contains("export NVSN_VERSION"),
        "no version is active, so none must be exported"
    );
}

#[test]
fn json_list_is_versioned_and_valid() {
    let dir = sandbox();
    let assert = nvsn(dir.path()).args(["--json", "list"]).assert().success();
    let value: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("stdout is JSON");
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["ok"], true);
    assert_eq!(value["command"], "list");
    assert_eq!(value["data"]["versions"], serde_json::json!([]));
}

#[test]
fn json_error_keeps_stdout_as_json() {
    let dir = sandbox();
    let assert = nvsn(dir.path())
        .args(["--json", "install"])
        .assert()
        .code(3);
    let value: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("stdout is JSON");
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["ok"], false);
    assert_eq!(value["error"]["code"], "version-file-missing");
    assert!(value["error"]["hint"].is_string());
}

#[test]
fn offline_without_cache_exits_with_five() {
    let dir = sandbox();
    nvsn(dir.path())
        .args(["--offline", "list-remote"])
        .assert()
        .code(5)
        .stderr(predicate::str::contains("offline"));
}

#[test]
fn uninstall_unknown_version_exits_with_six() {
    let dir = sandbox();
    nvsn(dir.path())
        .args(["uninstall", "20"])
        .assert()
        .code(6)
        .stderr(predicate::str::contains("nvsn install 20"));
}

#[test]
fn default_requires_an_installed_version() {
    let dir = sandbox();
    nvsn(dir.path()).args(["default", "20"]).assert().code(6);
}

#[test]
fn unknown_subcommand_is_a_usage_error() {
    let dir = sandbox();
    nvsn(dir.path()).arg("frobnicate").assert().code(2);
}

#[test]
fn alias_cannot_use_a_reserved_name() {
    let dir = sandbox();
    nvsn(dir.path())
        .args(["alias", "default", "20"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("reserved"));
}

#[test]
fn completions_for_bash_are_generated() {
    let dir = sandbox();
    nvsn(dir.path())
        .args(["completions", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("_nvsn"));
}

#[test]
fn nvmrc_in_the_working_directory_is_used_by_install() {
    let dir = sandbox();
    std::fs::write(dir.path().join(".nvmrc"), "not-a-version\n").expect("write nvmrc");
    nvsn(dir.path())
        .arg("install")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("not-a-version"));
}

/// Creates a fake installed version (only the directory: enough to resolve it).
fn fake_install(dir: &Path, tag: &str) {
    std::fs::create_dir_all(dir.join("store").join("versions").join(tag))
        .expect("create fake install");
}

#[test]
fn hook_without_nvmrc_prints_nothing_and_exits_zero() {
    let dir = sandbox();
    nvsn(dir.path())
        .args(["__hook", "--shell", "bash"])
        .assert()
        .success()
        .stdout("");
}

#[test]
fn hook_with_installed_nvmrc_exports_the_version() {
    let dir = sandbox();
    fake_install(dir.path(), "v20.11.1");
    std::fs::write(dir.path().join(".nvmrc"), "20.11.1\n").expect("write .nvmrc");

    nvsn(dir.path())
        .args(["__hook", "--shell", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("export NVSN_VERSION='v20.11.1'"))
        .stdout(predicate::str::contains("NVSN_SHELL_VERSION").not());
}

#[test]
fn hook_with_uninstalled_nvmrc_prints_nothing() {
    let dir = sandbox();
    std::fs::write(dir.path().join(".nvmrc"), "18.0.0\n").expect("write .nvmrc");

    nvsn(dir.path())
        .args(["__hook", "--shell", "bash"])
        .assert()
        .success()
        .stdout("");
}

#[test]
fn hook_rejects_an_unknown_shell_with_usage_code() {
    let dir = sandbox();
    nvsn(dir.path())
        .args(["__hook", "--shell", "tcsh"])
        .assert()
        .code(2);
}

#[test]
fn hook_is_hidden_from_help() {
    let dir = sandbox();
    nvsn(dir.path())
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("__hook").not());
}

#[test]
fn deactivate_prints_the_cleanup_script_for_bash() {
    let dir = sandbox();
    nvsn(dir.path())
        .args(["deactivate", "--shell", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("unset NVSN_VERSION"))
        .stdout(predicate::str::contains("NVSN_PATH_ENTRY"));
}

#[test]
fn deactivate_prints_the_cleanup_script_for_fish() {
    let dir = sandbox();
    nvsn(dir.path())
        .args(["deactivate", "--shell", "fish"])
        .assert()
        .success()
        .stdout(predicate::str::contains("set -e NVSN_PATH_ENTRY"));
}

#[test]
fn deactivate_without_shell_still_prints_a_cleanup_script() {
    let dir = sandbox();
    nvsn(dir.path())
        .arg("deactivate")
        .assert()
        .success()
        .stdout(predicate::str::contains("NVSN_VERSION"));
}

#[test]
fn deactivate_rejects_an_unknown_shell_with_usage_code() {
    let dir = sandbox();
    nvsn(dir.path())
        .args(["deactivate", "--shell", "tcsh"])
        .assert()
        .code(2);
}

#[test]
fn no_progress_keeps_stderr_free_of_the_install_indicator() {
    // No network: the index is not in the cache, so it fails before downloading.
    let dir = sandbox();
    nvsn(dir.path())
        .args(["--no-progress", "install", "20", "--offline"])
        .assert()
        .code(5)
        .stderr(predicate::str::contains("Installing").not());
}

#[test]
fn non_terminal_stderr_has_no_progress_indicator() {
    // assert_cmd passes stderr as a pipe (not a TTY): no indicator must appear.
    let dir = sandbox();
    nvsn(dir.path())
        .args(["install", "20", "--offline"])
        .assert()
        .code(5)
        .stderr(predicate::str::contains("Installing").not());
}
