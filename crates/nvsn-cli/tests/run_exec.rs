//! End-to-end tests of `nvsn run` and `nvsn exec`.
//!
//! The `node` of the fake version differs by platform:
//! - Unix: an `sh` script at `versions/vX/bin/node` that prints its arguments
//!   (`arg:<value>`) and exits with the code given by the first argument if it is
//!   numeric.
//! - Windows: a copy of `cmd.exe` renamed to `node.exe` in the root of the
//!   version. `cmd /d /c ...` serves as node: it prints and exits with the requested code.
//!   This way nothing needs to be compiled or downloaded.

mod common;

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const TAG: &str = "v20.11.1";

/// `nvsn` command isolated in `dir`, as in `cli.rs`.
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

/// Creates a fake installation of `TAG` and returns its directory.
fn install_fake_node(root: &Path) -> PathBuf {
    let version_dir = root.join("store").join("versions").join(TAG);
    if cfg!(windows) {
        fs::create_dir_all(&version_dir).expect("version dir");
        let cmd_exe = std::env::var_os("COMSPEC")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows\System32\cmd.exe"));
        fs::copy(cmd_exe, version_dir.join("node.exe")).expect("copy cmd.exe as node.exe");
    } else {
        let bin = version_dir.join("bin");
        fs::create_dir_all(&bin).expect("bin dir");
        let node = bin.join("node");
        fs::write(
            &node,
            "#!/bin/sh\n\
             for arg in \"$@\"; do echo \"arg:$arg\"; done\n\
             case \"$1\" in\n  [0-9]*) exit \"$1\" ;;\nesac\n\
             exit 0\n",
        )
        .expect("fake node");
        set_executable(&node);
    }
    version_dir
}

#[cfg(unix)]
fn set_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod");
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) {}

/// Arguments for `cmd` (which stands in for node) to exit with `code`.
fn cmd_exit_args(code: u8) -> Vec<String> {
    if cfg!(windows) {
        vec!["/d".into(), "/c".into(), "exit".into(), code.to_string()]
    } else {
        vec![code.to_string()]
    }
}

#[test]
fn run_propagates_the_exit_code_of_node() {
    let _lock = common::exec_lock();
    let dir = TempDir::new().expect("tempdir");
    install_fake_node(dir.path());
    nvsn(dir.path())
        .arg("run")
        .arg("20")
        .args(cmd_exit_args(7))
        .assert()
        .code(7);
}

#[test]
fn run_forwards_the_arguments_to_node() {
    let _lock = common::exec_lock();
    let dir = TempDir::new().expect("tempdir");
    install_fake_node(dir.path());
    let mut cmd = nvsn(dir.path());
    cmd.arg("run").arg("20");
    if cfg!(windows) {
        cmd.args(["/d", "/c", "echo", "hello", "world"])
            .assert()
            .success()
            .stdout(predicate::str::contains("hello world"));
    } else {
        cmd.args(["hello", "world"])
            .assert()
            .success()
            .stdout("arg:hello\narg:world\n");
    }
}

#[test]
fn run_accepts_double_dash_before_node_arguments() {
    let _lock = common::exec_lock();
    let dir = TempDir::new().expect("tempdir");
    install_fake_node(dir.path());
    nvsn(dir.path())
        .args(["run", "20", "--"])
        .args(cmd_exit_args(5))
        .assert()
        .code(5);
}

#[test]
fn run_without_installed_version_fails_with_code_6() {
    let _lock = common::exec_lock();
    let dir = TempDir::new().expect("tempdir");
    nvsn(dir.path())
        .args(["run", "20"])
        .assert()
        .code(6)
        .stderr(predicate::str::contains("nvsn install 20"));
}

/// Regression: `--help`, `-h`, `--version` and `-V` after the version must
/// reach the child verbatim, not be swallowed by clap as nvsn's own flags
/// (reported bug: `nvsn run 26.10.0 --help` printed nvsn's help instead of
/// node's). With no version installed, a passthrough flag that clap ate
/// would print nvsn's own usage and exit 0; forwarding it instead fails with
/// version-not-found (6), since nvsn still tries to resolve "26.10.0" first.
#[test]
fn run_forwards_help_and_version_flags_instead_of_eating_them() {
    let _lock = common::exec_lock();
    for flag in ["--help", "-h", "--version", "-V"] {
        let dir = TempDir::new().expect("tempdir");
        nvsn(dir.path())
            .args(["run", "26.10.0", flag])
            .assert()
            .code(6)
            .stdout(predicate::str::contains("Node.js version manager").not())
            .stderr(predicate::str::contains("Node.js version manager").not());
    }
}

#[test]
fn exec_forwards_help_and_version_flags_instead_of_eating_them() {
    let _lock = common::exec_lock();
    for flag in ["--help", "-h", "--version", "-V"] {
        let dir = TempDir::new().expect("tempdir");
        nvsn(dir.path())
            .args(["exec", "26.10.0", "node", flag])
            .assert()
            .code(6)
            .stdout(predicate::str::contains("Node.js version manager").not())
            .stderr(predicate::str::contains("Node.js version manager").not());
    }
}

/// Negative case: with no version and no `--lts`, there is no child to forward
/// to, so a bare flag means the user wants nvsn's own help or version for the
/// subcommand, not a usage error.
#[test]
fn run_and_exec_show_their_own_help_or_version_without_a_target() {
    let _lock = common::exec_lock();
    for (subcommand, flag, needle) in [
        ("run", "--help", "Run the node binary"),
        ("run", "-h", "Run the node binary"),
        ("run", "--version", "nvsn "),
        ("run", "-V", "nvsn "),
        ("exec", "--help", "Run a command with the bin directory"),
        ("exec", "-h", "Run a command with the bin directory"),
        ("exec", "--version", "nvsn "),
        ("exec", "-V", "nvsn "),
    ] {
        let dir = TempDir::new().expect("tempdir");
        nvsn(dir.path())
            .args([subcommand, flag])
            .assert()
            .success()
            .stdout(predicate::str::contains(needle));
    }
}

/// With `--lts`, every word (including a leading `--help`) belongs to the
/// child: there is no separate version argument to treat as a sentinel.
#[test]
fn run_with_lts_forwards_a_leading_help_flag_to_the_child_instead_of_showing_its_own() {
    let _lock = common::exec_lock();
    let dir = TempDir::new().expect("tempdir");
    nvsn(dir.path())
        .args(["run", "--lts", "--help"])
        .assert()
        .code(6)
        .stdout(predicate::str::contains("Run the node binary").not())
        .stderr(predicate::str::contains("Run the node binary").not());
}

/// Positive case: with a version actually installed, the flag must land in
/// the child's own argv, not be interpreted by nvsn at all.
#[test]
fn run_passes_a_bare_help_flag_to_node_when_installed() {
    let _lock = common::exec_lock();
    let dir = TempDir::new().expect("tempdir");
    install_fake_node(dir.path());
    if cfg!(windows) {
        // The fake node.exe is a real cmd.exe copy; feed it from an empty
        // stdin so a bare, unrecognized switch can't block on interactive
        // input, and assert nvsn's own clap parser never touched the flag
        // (no usage error, no nvsn help banner).
        nvsn(dir.path())
            .arg("run")
            .arg("20")
            .arg("--help")
            .timeout(std::time::Duration::from_secs(10))
            .assert()
            .stdout(predicate::str::contains("Node.js version manager").not())
            .stderr(predicate::str::contains("unexpected argument").not());
    } else {
        nvsn(dir.path())
            .args(["run", "20", "--help"])
            .assert()
            .success()
            .stdout("arg:--help\n");
    }
}

#[test]
fn exec_propagates_the_exit_code_of_the_command() {
    let _lock = common::exec_lock();
    let dir = TempDir::new().expect("tempdir");
    install_fake_node(dir.path());
    let mut cmd = nvsn(dir.path());
    cmd.args(["exec", "20"]);
    if cfg!(windows) {
        cmd.args(["cmd", "/d", "/c", "exit", "3"]);
    } else {
        cmd.args(["sh", "-c", "exit 3"]);
    }
    cmd.assert().code(3);
}

#[test]
fn exec_puts_the_version_directory_first_in_path() {
    let _lock = common::exec_lock();
    let dir = TempDir::new().expect("tempdir");
    let version_dir = install_fake_node(dir.path());
    let mut cmd = nvsn(dir.path());
    cmd.args(["exec", "20"]);
    if cfg!(windows) {
        cmd.args(["cmd", "/d", "/c", "echo", "%PATH%"])
            .assert()
            .success()
            .stdout(predicate::str::starts_with(
                version_dir.display().to_string(),
            ));
    } else {
        let bin = version_dir.join("bin");
        cmd.args(["sh", "-c", "echo $PATH"])
            .assert()
            .success()
            .stdout(predicate::str::starts_with(bin.display().to_string()));
    }
}

#[test]
fn exec_forwards_the_arguments_to_the_command() {
    let _lock = common::exec_lock();
    let dir = TempDir::new().expect("tempdir");
    install_fake_node(dir.path());
    let mut cmd = nvsn(dir.path());
    cmd.args(["exec", "20"]);
    if cfg!(windows) {
        cmd.args(["cmd", "/d", "/c", "echo", "forwarded"])
            .assert()
            .success()
            .stdout(predicate::str::contains("forwarded"));
    } else {
        cmd.args(["sh", "-c", "echo \"$0 $1\"", "forwarded", "value"])
            .assert()
            .success()
            .stdout("forwarded value\n");
    }
}

#[test]
fn exec_accepts_double_dash_before_the_command() {
    let _lock = common::exec_lock();
    let dir = TempDir::new().expect("tempdir");
    install_fake_node(dir.path());
    let mut cmd = nvsn(dir.path());
    cmd.args(["exec", "20", "--"]);
    if cfg!(windows) {
        cmd.args(["cmd", "/d", "/c", "exit", "4"]).assert().code(4);
    } else {
        cmd.args(["sh", "-c", "exit 4"]).assert().code(4);
    }
}

#[test]
fn exec_missing_command_fails_with_code_1() {
    let _lock = common::exec_lock();
    let dir = TempDir::new().expect("tempdir");
    install_fake_node(dir.path());
    nvsn(dir.path())
        .args(["exec", "20", "nvsn-no-such-command-xyz"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains(
            "'nvsn-no-such-command-xyz' not found",
        ));
}

#[test]
fn exec_without_installed_version_fails_with_code_6() {
    let _lock = common::exec_lock();
    let dir = TempDir::new().expect("tempdir");
    nvsn(dir.path())
        .args(["exec", "20", "node"])
        .assert()
        .code(6)
        .stderr(predicate::str::contains("nvsn install 20"));
}

#[cfg(windows)]
#[test]
fn exec_runs_a_cmd_shim_found_through_pathext() {
    let _lock = common::exec_lock();
    let dir = TempDir::new().expect("tempdir");
    let version_dir = install_fake_node(dir.path());
    fs::write(
        version_dir.join("npm.cmd"),
        "@echo off\r\necho shim:%1 %2\r\n",
    )
    .expect("npm.cmd shim");
    nvsn(dir.path())
        .args(["exec", "20", "npm", "hello", "world"])
        .assert()
        .success()
        .stdout(predicate::str::contains("shim:hello world"));
}

#[cfg(windows)]
#[test]
fn exec_passes_special_characters_to_a_cmd_shim_literally() {
    let _lock = common::exec_lock();
    let dir = TempDir::new().expect("tempdir");
    let version_dir = install_fake_node(dir.path());
    fs::write(version_dir.join("npm.cmd"), "@echo off\r\necho shim:%1\r\n").expect("npm.cmd shim");
    nvsn(dir.path())
        .args(["exec", "20", "npm", "a&b|c"])
        .assert()
        .success()
        .stdout(predicate::str::contains("a&b|c"));
}

#[cfg(windows)]
#[test]
fn exec_rejects_double_quotes_for_a_cmd_shim() {
    let _lock = common::exec_lock();
    let dir = TempDir::new().expect("tempdir");
    let version_dir = install_fake_node(dir.path());
    fs::write(version_dir.join("npm.cmd"), "@echo off\r\n").expect("npm.cmd shim");
    nvsn(dir.path())
        .args(["exec", "20", "npm", "a\"b"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("cannot be passed"));
}

#[cfg(not(windows))]
#[test]
fn exec_does_not_apply_pathext_on_unix() {
    let _lock = common::exec_lock();
    let dir = TempDir::new().expect("tempdir");
    let version_dir = install_fake_node(dir.path());
    let bin = version_dir.join("bin");
    fs::write(bin.join("nvsn-pathext-probe.cmd"), "#!/bin/sh\nexit 0\n").expect("probe");
    nvsn(dir.path())
        .args(["exec", "20", "nvsn-pathext-probe"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("not found"));
}
