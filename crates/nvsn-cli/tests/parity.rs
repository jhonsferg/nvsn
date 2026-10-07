//! End-to-end tests of the nvm-compatible surface: aliases, `--lts`, `default node`,
//! `use <arch>`, `--silent`, `cache`, the settings and the commands that wait for the
//! core API. No network: every test that reaches the index uses `--offline` without a cache.

use assert_cmd::Command;
use predicates::prelude::*;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// `nvsn` command isolated in `dir`: no integration, no active version, no proxy, no mirror.
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
        .env_remove("HTTPS_PROXY")
        .env_remove("https_proxy")
        .env_remove("HTTP_PROXY")
        .env_remove("http_proxy")
        .env("NVSN_NO_UPDATE_CHECK", "1");
    cmd
}

fn sandbox() -> TempDir {
    tempfile::tempdir().expect("temporary directory")
}

/// Creates a fake installed version with a `node` file at its root (enough to resolve and run it).
fn fake_install(dir: &Path, tag: &str) -> PathBuf {
    let version_dir = dir.join("store").join("versions").join(tag);
    std::fs::create_dir_all(&version_dir).expect("create fake install");
    std::fs::write(version_dir.join("node"), b"").expect("write fake node");
    version_dir
}

#[test]
fn un_is_an_alias_of_uninstall() {
    let dir = sandbox();
    nvsn(dir.path()).args(["un", "20"]).assert().code(6);
}

#[test]
fn off_and_on_print_the_integration_scripts() {
    let dir = sandbox();
    nvsn(dir.path())
        .args(["off", "--shell", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::is_empty().not());
    nvsn(dir.path())
        .args(["on", "--shell", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("export NVSN_DIR='"));
}

#[test]
fn ls_and_list_are_the_same_command() {
    let dir = sandbox();
    for name in ["ls", "list"] {
        nvsn(dir.path())
            .arg(name)
            .assert()
            .success()
            .stdout(predicate::str::contains("No Node.js versions installed"));
    }
}

#[test]
fn list_available_and_ls_remote_go_to_the_remote_index() {
    let dir = sandbox();
    nvsn(dir.path())
        .args(["list", "available", "--offline"])
        .assert()
        .code(5);
    nvsn(dir.path())
        .args(["ls-remote", "--offline"])
        .assert()
        .code(5);
    nvsn(dir.path())
        .args(["list-remote", "--offline"])
        .assert()
        .code(5);
}

#[test]
fn list_filters_need_the_available_scope() {
    let dir = sandbox();
    nvsn(dir.path())
        .args(["list", "--lts"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("list available"));
}

#[test]
fn no_colors_is_accepted_and_ignored_by_the_list_commands() {
    let dir = sandbox();
    nvsn(dir.path())
        .args(["list", "--no-colors"])
        .assert()
        .success();
    nvsn(dir.path())
        .args(["ls-remote", "--no-colors", "--offline"])
        .assert()
        .code(5);
}

#[test]
fn lts_and_an_explicit_version_are_rejected_with_usage_code() {
    let dir = sandbox();
    for args in [
        vec!["install", "20", "--lts"],
        vec!["uninstall", "20", "--lts"],
        vec!["use", "20", "--lts"],
        vec!["which", "20", "--lts"],
        vec!["version-remote", "20", "--lts"],
    ] {
        nvsn(dir.path()).args(&args).assert().code(2);
    }
}

#[test]
fn lts_flags_resolve_against_the_index() {
    let dir = sandbox();
    nvsn(dir.path())
        .args(["install", "--lts", "--offline"])
        .assert()
        .code(5);
    nvsn(dir.path())
        .args(["install", "--lts=iron", "--offline"])
        .assert()
        .code(5);
    nvsn(dir.path())
        .args(["version-remote", "--lts=-1", "--offline"])
        .assert()
        .code(5);
}

#[test]
fn run_by_lts_resolves_only_installed_versions() {
    // run, exec and which work on installed versions: nothing is downloaded.
    let dir = sandbox();
    nvsn(dir.path())
        .args(["run", "--lts", "--", "--version"])
        .assert()
        .code(6)
        .stderr(predicate::str::contains("lts/*"));
}

#[test]
fn uninstall_by_lts_without_installed_lts_is_not_found() {
    let dir = sandbox();
    nvsn(dir.path())
        .args(["uninstall", "--lts"])
        .assert()
        .code(6)
        .stderr(predicate::str::contains("lts/*"));
}

#[test]
fn uninstall_without_version_or_lts_is_a_usage_error() {
    let dir = sandbox();
    nvsn(dir.path())
        .arg("uninstall")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("needs a version or --lts"));
}

#[test]
fn exec_with_lts_needs_a_command() {
    let dir = sandbox();
    nvsn(dir.path())
        .args(["exec", "--lts", "--offline"])
        .assert()
        .code(2);
}

#[test]
fn use_newest_and_node_need_an_installed_version() {
    let dir = sandbox();
    for spec in ["newest", "node", "latest"] {
        nvsn(dir.path()).args(["use", spec]).assert().code(6);
    }
}

#[test]
fn use_silent_prints_nothing() {
    let dir = sandbox();
    fake_install(dir.path(), "v20.11.1");
    nvsn(dir.path())
        .args(["use", "20", "--silent"])
        .assert()
        .success()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::is_empty());
    nvsn(dir.path())
        .args(["use", "20"])
        .assert()
        .success()
        .stdout(predicate::str::contains("v20.11.1 is installed"));
}

#[test]
fn arch_is_stored_in_config_toml_and_removed_with_none() {
    let dir = sandbox();
    nvsn(dir.path()).args(["arch", "arm64"]).assert().success();
    let config = std::fs::read_to_string(dir.path().join("store").join("config.toml"))
        .expect("config.toml written");
    assert!(config.contains("arch = \"arm64\""), "{config}");
    nvsn(dir.path())
        .arg("arch")
        .assert()
        .success()
        .stdout("arm64\n");

    nvsn(dir.path()).args(["arch", "none"]).assert().success();
    let config = std::fs::read_to_string(dir.path().join("store").join("config.toml"))
        .expect("config.toml still there");
    assert!(!config.contains("arch"), "{config}");
}

#[test]
fn use_with_an_architecture_word_stores_it() {
    let dir = sandbox();
    nvsn(dir.path()).args(["use", "x64"]).assert().success();
    let config = std::fs::read_to_string(dir.path().join("store").join("config.toml"))
        .expect("config.toml written");
    assert!(config.contains("arch = \"x64\""), "{config}");
    nvsn(dir.path()).args(["use", "32"]).assert().success();
    nvsn(dir.path())
        .arg("arch")
        .assert()
        .success()
        .stdout("x86\n");
}

#[test]
fn root_cannot_be_stored_and_points_to_nvsn_dir() {
    let dir = sandbox();
    nvsn(dir.path())
        .args(["root", "/tmp/elsewhere"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("NVSN_DIR"));
}

#[test]
fn invalid_setting_values_are_usage_errors() {
    let dir = sandbox();
    for args in [
        vec!["arch", "mips"],
        vec!["proxy", "socks5://proxy:1080"],
        vec!["node-mirror", "ftp://mirror.local"],
        vec!["npm-mirror", "not a url"],
    ] {
        nvsn(dir.path()).args(&args).assert().code(2);
    }
    assert!(!dir.path().join("store").join("config.toml").exists());
}

#[test]
fn arch_without_value_shows_the_host_architecture() {
    let dir = sandbox();
    nvsn(dir.path())
        .arg("arch")
        .assert()
        .success()
        .stdout(predicate::str::is_empty().not());
}

#[test]
fn arch_rejects_unknown_names() {
    let dir = sandbox();
    nvsn(dir.path())
        .args(["arch", "mips"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("unsupported architecture"));
}

#[test]
fn proxy_shows_the_environment_proxy_without_credentials() {
    let dir = sandbox();
    nvsn(dir.path())
        .arg("proxy")
        .assert()
        .success()
        .stdout("none\n");
    nvsn(dir.path())
        .arg("proxy")
        .env("HTTPS_PROXY", "http://user:secret@proxy.local:3128")
        .assert()
        .success()
        .stdout("http://***@proxy.local:3128\n");
}

#[test]
fn mirrors_and_root_show_their_values() {
    let dir = sandbox();
    nvsn(dir.path())
        .arg("node-mirror")
        .assert()
        .success()
        .stdout("https://nodejs.org/dist\n");
    nvsn(dir.path())
        .arg("npm-mirror")
        .assert()
        .success()
        .stdout(predicate::str::contains("registry.npmjs.org"));
    nvsn(dir.path())
        .arg("root")
        .assert()
        .success()
        .stdout(predicate::str::contains("store"));
}

#[test]
fn set_colors_is_unsupported_with_exit_code_ten() {
    let dir = sandbox();
    nvsn(dir.path())
        .arg("set-colors")
        .assert()
        .code(10)
        .stderr(predicate::str::contains("not supported"));
    nvsn(dir.path())
        .args(["set-colors", "bgRGB"])
        .assert()
        .code(10);
}

#[test]
fn default_node_follows_the_newest_installed_version() {
    let dir = sandbox();
    fake_install(dir.path(), "v20.11.1");
    nvsn(dir.path())
        .args(["default", "node"])
        .assert()
        .success()
        .stdout(predicate::str::contains("v20.11.1"));
    nvsn(dir.path())
        .arg("current")
        .assert()
        .success()
        .stdout("v20.11.1\n");

    fake_install(dir.path(), "v22.0.0");
    nvsn(dir.path())
        .arg("current")
        .assert()
        .success()
        .stdout("v22.0.0\n");
}

#[test]
fn alias_default_node_is_the_same_as_default_node() {
    let dir = sandbox();
    fake_install(dir.path(), "v20.11.1");
    nvsn(dir.path())
        .args(["alias", "default", "node"])
        .assert()
        .success();
    nvsn(dir.path())
        .arg("current")
        .assert()
        .success()
        .stdout("v20.11.1\n");
}

#[test]
fn alias_default_with_a_fixed_version_keeps_the_reserved_name_error() {
    let dir = sandbox();
    fake_install(dir.path(), "v20.11.1");
    nvsn(dir.path())
        .args(["alias", "default", "20"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("reserved"));
    nvsn(dir.path()).args(["default", "20"]).assert().success();
}

#[test]
fn default_node_without_versions_is_not_found() {
    let dir = sandbox();
    nvsn(dir.path())
        .args(["default", "node"])
        .assert()
        .code(6)
        .stderr(predicate::str::contains("no Node.js version is installed"));
}

#[test]
fn removing_the_newest_keeps_the_node_default_on_the_next_one() {
    let dir = sandbox();
    fake_install(dir.path(), "v20.11.1");
    fake_install(dir.path(), "v22.0.0");
    nvsn(dir.path())
        .args(["default", "node"])
        .assert()
        .success();
    nvsn(dir.path())
        .args(["uninstall", "22", "--yes"])
        .assert()
        .success();
    nvsn(dir.path())
        .arg("current")
        .assert()
        .success()
        .stdout("v20.11.1\n");
}

#[test]
fn cache_lists_and_clears_the_download_cache() {
    let dir = sandbox();
    let index = dir.path().join("store").join("cache").join("index");
    std::fs::create_dir_all(&index).expect("cache dir");
    std::fs::write(index.join("index.json"), b"0123456789").expect("entry");

    nvsn(dir.path())
        .args(["cache", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("index/index.json"));
    nvsn(dir.path())
        .args(["cache", "clear"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed 1 cache files (10 bytes)"));
    nvsn(dir.path())
        .args(["cache", "list"])
        .assert()
        .success()
        .stdout("The cache is empty.\n");
    assert!(index.is_dir(), "the cache area itself is kept");
}

#[test]
fn cache_dir_prints_the_cache_path() {
    let dir = sandbox();
    nvsn(dir.path())
        .args(["cache", "dir"])
        .assert()
        .success()
        .stdout(predicate::str::contains("cache"));
}

#[test]
fn cache_requires_an_action() {
    let dir = sandbox();
    nvsn(dir.path()).arg("cache").assert().code(2);
}

#[test]
fn reinstall_packages_validates_both_versions_before_the_npm_step() {
    let dir = sandbox();
    nvsn(dir.path())
        .args(["reinstall-packages", "20"])
        .assert()
        .code(6);

    fake_install(dir.path(), "v20.11.1");
    nvsn(dir.path())
        .args(["reinstall-packages", "20", "--from", "22"])
        .assert()
        .code(6);

    // Both installed, but the fake versions have no npm: the npm step reports it.
    fake_install(dir.path(), "v22.0.0");
    nvsn(dir.path())
        .args(["reinstall-packages", "20", "--from", "22"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("npm is missing"));
}

#[test]
fn install_latest_npm_needs_a_version_when_none_is_active() {
    let dir = sandbox();
    nvsn(dir.path())
        .arg("install-latest-npm")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("needs a version"));
}

#[test]
fn install_options_check_their_source_before_any_download() {
    let dir = sandbox();
    nvsn(dir.path())
        .args(["install", "20", "--latest-npm", "--offline"])
        .assert()
        .code(5);
    nvsn(dir.path())
        .args([
            "install",
            "20",
            "--reinstall-packages-from",
            "18",
            "--offline",
        ])
        .assert()
        .code(6);
    nvsn(dir.path())
        .args(["install", "20", "--no-use", "--offline"])
        .assert()
        .code(5);
}

#[test]
fn which_by_lts_without_installed_lts_is_not_found() {
    let dir = sandbox();
    nvsn(dir.path()).args(["which", "--lts"]).assert().code(6);
}
