//! End-to-end tests of the settings and npm commands against loopback servers.
//!
//! The mirror serves `index.json`, the checksums and a release archive from `TestServer`.
//! The release archive holds a fake `node`: on Unix it is a shell script that appends its
//! arguments to `calls.log` in the version directory, so `npm` steps can be observed.
//! Windows cannot run a shell script as `node.exe`: the npm success paths are Unix-only.

mod common;

use common::{Route, TestServer};
use predicates::prelude::*;
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const VERSION: &str = "20.11.1";
const TAG: &str = "v20.11.1";

/// Isolated directory: `store` is `NVSN_DIR`, `home` the HOME and `work` the cwd.
struct Sandbox {
    tmp: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            tmp: TempDir::new().expect("tempdir"),
        }
    }

    fn root(&self) -> &Path {
        self.tmp.path()
    }

    fn store(&self) -> PathBuf {
        self.root().join("store")
    }

    fn version_dir(&self, tag: &str) -> PathBuf {
        self.store().join("versions").join(tag)
    }

    fn config(&self) -> String {
        std::fs::read_to_string(self.store().join("config.toml")).unwrap_or_default()
    }

    /// `nvsn` with no proxy variables from the user's environment.
    fn cmd(&self) -> assert_cmd::Command {
        let binary = PathBuf::from(env!("CARGO_BIN_EXE_nvsn"));
        let mut cmd = common::nvsn(&binary, self.root());
        for name in ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy"] {
            cmd.env_remove(name);
        }
        cmd
    }
}

/// Free loopback port that is no longer listening: connection refused.
fn closed_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);
    port
}

/// Node distribution names for the test host (same layout as `faults.rs`).
struct Dist {
    /// Index token, e.g. `linux-x64`.
    token: String,
    /// File published by the mirror.
    file: String,
    /// Root directory inside the archive.
    root: String,
    /// Zip (Windows) or tar.gz.
    is_zip: bool,
}

fn host_dist() -> Dist {
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x64"
    };
    if cfg!(windows) {
        let dist = format!("win-{arch}");
        Dist {
            token: format!("{dist}-zip"),
            file: format!("node-v{VERSION}-{dist}.zip"),
            root: format!("node-v{VERSION}-{dist}"),
            is_zip: true,
        }
    } else if cfg!(target_os = "macos") {
        let dist = format!("darwin-{arch}");
        Dist {
            token: format!("osx-{arch}-tar"),
            file: format!("node-v{VERSION}-{dist}.tar.gz"),
            root: format!("node-v{VERSION}-{dist}"),
            is_zip: false,
        }
    } else {
        let dist = format!("linux-{arch}");
        Dist {
            token: dist.clone(),
            file: format!("node-v{VERSION}-{dist}.tar.gz"),
            root: format!("node-v{VERSION}-{dist}"),
            is_zip: false,
        }
    }
}

/// `index.json` with one release that publishes the host token only.
fn index_json(d: &Dist) -> String {
    format!(
        r#"[{{"version":"v{VERSION}","date":"2024-02-14","files":["{}"],"lts":"Iron"}}]"#,
        d.token
    )
}

/// Schedules index, checksums and archive for `TAG` on `server`.
fn serve(server: &TestServer, d: &Dist, archive: &[u8]) {
    server.set_route("/index.json", Route::ok(index_json(d).into_bytes()));
    let sums = format!("{}  {}\n", common::sha256_hex(archive), d.file);
    server.set_route(
        &format!("/{TAG}/SHASUMS256.txt"),
        Route::ok(sums.into_bytes()),
    );
    server.set_route(&format!("/{TAG}/{}", d.file), Route::ok(archive.to_vec()));
}

/// Archive with the given entries (paths relative to the archive, bytes).
fn archive(d: &Dist, entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    if d.is_zip {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, content) in entries {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .expect("zip entry");
            zip.write_all(content).expect("zip write");
        }
        zip.finish().expect("zip finish").into_inner()
    } else {
        let enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut ar = tar::Builder::new(enc);
        for (name, content) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            ar.append_data(&mut header, name, content.as_slice())
                .expect("tar entry");
        }
        ar.into_inner()
            .expect("tar finish")
            .finish()
            .expect("gz finish")
    }
}

/// Release archive whose `node` is a plain executable file (no npm).
fn plain_release(d: &Dist) -> Vec<u8> {
    let node = if d.is_zip { "node.exe" } else { "bin/node" };
    archive(d, &[(format!("{}/{node}", d.root), b"fake node".to_vec())])
}

/// Shell script that logs its arguments to `calls.log` next to `bin/`.
#[cfg(unix)]
const LOGGING_NODE: &[u8] = b"#!/bin/sh\necho \"$@\" >> \"$(dirname \"$0\")/../calls.log\"\n";

/// Release archive whose node logs its calls, with npm's entry script in place.
#[cfg(unix)]
fn logging_release(d: &Dist) -> Vec<u8> {
    archive(
        d,
        &[
            (format!("{}/bin/node", d.root), LOGGING_NODE.to_vec()),
            (
                format!("{}/lib/node_modules/npm/bin/npm-cli.js", d.root),
                b"// npm\n".to_vec(),
            ),
        ],
    )
}

/// Installed fake version with logging node, npm, and optionally global packages.
#[cfg(unix)]
fn fake_version(sb: &Sandbox, tag: &str, packages: &[&str]) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let dir = sb.version_dir(tag);
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).expect("bin dir");
    let node = bin.join("node");
    std::fs::write(&node, LOGGING_NODE).expect("node script");
    std::fs::set_permissions(&node, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let npm = dir.join("lib/node_modules/npm/bin");
    std::fs::create_dir_all(&npm).expect("npm dir");
    std::fs::write(npm.join("npm-cli.js"), "// npm\n").expect("npm cli");
    for package in packages {
        std::fs::create_dir_all(dir.join("lib/node_modules").join(package)).expect("package");
    }
    dir
}

/// Lines appended to `calls.log` by the fake node of `tag`.
#[cfg(unix)]
fn calls(sb: &Sandbox, tag: &str) -> String {
    std::fs::read_to_string(sb.version_dir(tag).join("calls.log")).unwrap_or_default()
}

/// Installed fake version without npm (no `lib/node_modules/npm`).
fn bare_version(sb: &Sandbox, tag: &str) -> PathBuf {
    let dir = sb.version_dir(tag);
    std::fs::create_dir_all(&dir).expect("version dir");
    std::fs::write(dir.join("node"), b"").expect("node file");
    dir
}

/// `install` arguments against a loopback mirror.
fn install_args(base: &str, extra: &[&str]) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "install".into(),
        VERSION.into(),
        "--allow-http".into(),
        "--mirror".into(),
        base.into(),
    ];
    args.extend(extra.iter().map(|s| (*s).to_owned()));
    args
}

#[test]
fn node_mirror_stored_in_config_is_used_by_list_remote() {
    let server = TestServer::start();
    serve(&server, &host_dist(), &plain_release(&host_dist()));
    let sb = Sandbox::new();

    sb.cmd()
        .args(["node-mirror", &server.base()])
        .assert()
        .success();
    assert!(sb.config().contains("node_mirror"), "{}", sb.config());
    sb.cmd()
        .args(["list-remote", "--allow-http"])
        .assert()
        .success()
        .stdout(predicate::str::contains(TAG));
    sb.cmd()
        .arg("node-mirror")
        .assert()
        .success()
        .stdout(predicate::str::contains(server.base()));
}

#[test]
fn environment_mirror_takes_precedence_over_the_stored_one() {
    let server = TestServer::start();
    serve(&server, &host_dist(), &plain_release(&host_dist()));
    let sb = Sandbox::new();
    // The stored mirror is dead; the variable points at the live server.
    sb.cmd()
        .args([
            "node-mirror",
            &format!("http://127.0.0.1:{}", closed_port()),
        ])
        .assert()
        .success();

    sb.cmd()
        .env("NVSN_NODEJS_ORG_MIRROR", server.base())
        .args(["list-remote", "--allow-http"])
        .assert()
        .success()
        .stdout(predicate::str::contains(TAG));
}

#[test]
fn stored_proxy_is_used_by_the_http_client() {
    let server = TestServer::start();
    serve(&server, &host_dist(), &plain_release(&host_dist()));
    let sb = Sandbox::new();
    let dead_proxy = format!("http://127.0.0.1:{}", closed_port());

    sb.cmd().args(["proxy", &dead_proxy]).assert().success();
    // The request goes to the dead proxy: download failure (7), not the mirror.
    sb.cmd()
        .args(["list-remote", "--allow-http", "--mirror", &server.base()])
        .assert()
        .code(7);

    sb.cmd().args(["proxy", "none"]).assert().success();
    sb.cmd()
        .args(["list-remote", "--allow-http", "--mirror", &server.base()])
        .assert()
        .success()
        .stdout(predicate::str::contains(TAG));
}

#[test]
fn proxy_credentials_are_hidden_in_output() {
    let sb = Sandbox::new();
    sb.cmd()
        .args(["proxy", "http://user:hunter2@127.0.0.1:3128"])
        .assert()
        .success()
        .stdout(predicate::str::contains("hunter2").not())
        .stdout(predicate::str::contains("***@127.0.0.1:3128"));
    sb.cmd()
        .arg("proxy")
        .assert()
        .success()
        .stdout("http://***@127.0.0.1:3128\n");
}

#[test]
fn npm_mirror_is_stored_and_shown() {
    let sb = Sandbox::new();
    sb.cmd()
        .args(["npm-mirror", "https://registry.example.test/"])
        .assert()
        .success();
    sb.cmd()
        .arg("npm-mirror")
        .assert()
        .success()
        .stdout("https://registry.example.test\n");
}

#[cfg(target_arch = "x86_64")]
#[test]
fn stored_arch_changes_which_artifact_install_picks() {
    let d = host_dist();
    let archive_bytes = plain_release(&d);
    let server = TestServer::start();
    serve(&server, &d, &archive_bytes);
    let sb = Sandbox::new();

    // The host is x64; the stored override asks for arm64, which the index does not list.
    sb.cmd().args(["arch", "arm64"]).assert().success();
    sb.cmd()
        .args(install_args(&server.base(), &[]))
        .assert()
        .code(4);

    sb.cmd().args(["arch", "none"]).assert().success();
    sb.cmd()
        .args(install_args(&server.base(), &[]))
        .assert()
        .success();
}

#[test]
fn install_makes_the_first_version_the_default_unless_no_use() {
    let d = host_dist();
    let server = TestServer::start();
    serve(&server, &d, &plain_release(&d));

    let kept = Sandbox::new();
    kept.cmd()
        .args(install_args(&server.base(), &["--no-use"]))
        .assert()
        .success();
    kept.cmd()
        .arg("current")
        .assert()
        .success()
        .stdout("none\n");

    let default = Sandbox::new();
    default
        .cmd()
        .args(install_args(&server.base(), &[]))
        .assert()
        .success();
    default
        .cmd()
        .arg("current")
        .assert()
        .success()
        .stdout(format!("{TAG}\n"));
}

#[cfg(unix)]
#[test]
fn install_latest_npm_runs_npm_after_the_install() {
    let d = host_dist();
    let server = TestServer::start();
    serve(&server, &d, &logging_release(&d));
    let sb = Sandbox::new();

    sb.cmd()
        .args(install_args(&server.base(), &["--latest-npm"]))
        .assert()
        .success();
    assert!(
        calls(&sb, TAG).contains("install -g npm@latest"),
        "calls: {:?}",
        calls(&sb, TAG)
    );
}

#[cfg(unix)]
#[test]
fn install_reinstall_packages_from_copies_the_global_packages() {
    let d = host_dist();
    let server = TestServer::start();
    serve(&server, &d, &logging_release(&d));
    let sb = Sandbox::new();
    fake_version(&sb, "v22.0.0", &["semver"]);

    sb.cmd()
        .args(install_args(
            &server.base(),
            &["--reinstall-packages-from", "22"],
        ))
        .assert()
        .success();
    assert!(
        calls(&sb, TAG).contains("install -g semver"),
        "calls: {:?}",
        calls(&sb, TAG)
    );
}

#[cfg(unix)]
#[test]
fn install_latest_npm_command_runs_npm_in_the_version() {
    let sb = Sandbox::new();
    fake_version(&sb, TAG, &[]);

    sb.cmd()
        .args(["install-latest-npm", "20"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Latest npm installed into v20.11.1",
        ));
    assert!(calls(&sb, TAG).contains("install -g npm@latest"));
}

#[test]
fn install_latest_npm_reports_a_version_without_npm() {
    let sb = Sandbox::new();
    bare_version(&sb, TAG);
    sb.cmd()
        .args(["install-latest-npm", "20"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("npm is missing"));
}

#[cfg(unix)]
#[test]
fn reinstall_packages_command_copies_from_the_given_version() {
    let sb = Sandbox::new();
    fake_version(&sb, "v22.0.0", &["semver", "@scope/tool"]);
    fake_version(&sb, TAG, &[]);

    sb.cmd()
        .args(["reinstall-packages", "20", "--from", "22"])
        .assert()
        .success()
        .stdout(predicate::str::contains("semver"))
        .stdout(predicate::str::contains("@scope/tool"));
    let log = calls(&sb, TAG);
    // Package names are sorted by core: `@scope/tool` comes before `semver`.
    assert!(
        log.contains("install -g @scope/tool semver"),
        "calls: {log:?}"
    );
}

#[cfg(unix)]
#[test]
fn reinstall_packages_without_packages_runs_nothing() {
    let sb = Sandbox::new();
    fake_version(&sb, "v22.0.0", &[]);
    fake_version(&sb, TAG, &[]);

    sb.cmd()
        .args(["reinstall-packages", "20", "--from", "22"])
        .assert()
        .success()
        .stdout(predicate::str::contains("no global packages"));
    assert!(calls(&sb, TAG).is_empty());
}
