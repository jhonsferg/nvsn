//! CLI failure scenarios: exit code, store state and message.
//!
//! No real network: everything goes against `TestServer` on loopback or against a closed port.
//! The expected codes are those in the table of `src/exit.rs`.

mod common;

use common::{Route, TestServer};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::thread;
use tempfile::TempDir;

const VERSION: &str = "20.11.1";
const TAG: &str = "v20.11.1";

/// Exit codes documented in `src/exit.rs`.
const EXIT_GENERAL: i32 = 1;
const EXIT_USAGE: i32 = 2;
const EXIT_VERSION_FILE: i32 = 3;
const EXIT_NOT_FOUND: i32 = 6;
const EXIT_DOWNLOAD: i32 = 7;
const EXIT_LOCK_BUSY: i32 = 8;

/// Marker that a malicious tar tries to write outside the store.
const ESCAPE_MARKER: &str = "nvsn-escape-marker.txt";

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

    fn cmd(&self) -> assert_cmd::Command {
        common::nvsn(&nvsn_bin(), self.root())
    }
}

fn nvsn_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_nvsn"))
}

/// Free loopback port that is no longer listening: connection refused.
fn closed_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);
    port
}

fn stderr_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn stdout_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Checks the exit code and that there was no panic.
fn assert_code(out: &Output, expected: i32) {
    let err = stderr_of(out);
    assert!(!err.contains("panicked"), "the binary panicked:\n{err}");
    assert_eq!(
        out.status.code(),
        Some(expected),
        "exit code; stderr:\n{err}"
    );
}

/// Checks that there was no panic, whatever the code.
fn assert_no_panic(out: &Output) {
    let err = stderr_of(out);
    assert!(!err.contains("panicked"), "the binary panicked:\n{err}");
    assert!(out.status.code().is_some(), "killed by a signal:\n{err}");
}

/// All paths under `dir`, recursively. Empty if it does not exist.
fn all_paths(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            out.push(path.clone());
            if path.is_dir() {
                out.extend(all_paths(&path));
            }
        }
    }
    out
}

/// Names of the direct entries of `dir`.
fn names_in(dir: &Path) -> Vec<String> {
    fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// No version, no `.tmp-*` staging and no `.part` left in the store.
fn assert_no_version_and_no_leftovers(sb: &Sandbox) {
    assert!(
        !sb.store().join("versions").join(TAG).exists(),
        "{TAG} must not be installed after a failure"
    );
    for name in names_in(&sb.store().join("versions")) {
        assert!(
            !name.starts_with(".tmp-") && name != TAG,
            "unexpected entry in versions/: {name}"
        );
    }
    for path in all_paths(&sb.store()) {
        assert!(
            path.extension().is_none_or(|ext| ext != "part"),
            "partial download left behind: {}",
            path.display()
        );
    }
}

/// Test platform: Node file names that nvsn expects.
struct Dist {
    /// Index token (`linux-x64`, `win-x64-zip`, `osx-arm64-tar`).
    token: String,
    /// File name published by the mirror.
    file: String,
    /// Root directory inside the archive.
    root: String,
    /// Executable relative to `root`.
    bin: String,
    /// Zip format (Windows) or tar.gz (the rest).
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
            file: format!("node-v20.11.1-{dist}.zip"),
            root: format!("node-v20.11.1-{dist}"),
            bin: "node.exe".to_owned(),
            is_zip: true,
        }
    } else if cfg!(target_os = "macos") {
        let dist = format!("darwin-{arch}");
        Dist {
            token: format!("osx-{arch}-tar"),
            file: format!("node-v20.11.1-{dist}.tar.gz"),
            root: format!("node-v20.11.1-{dist}"),
            bin: "bin/node".to_owned(),
            is_zip: false,
        }
    } else {
        let dist = format!("linux-{arch}");
        Dist {
            token: dist.clone(),
            file: format!("node-v20.11.1-{dist}.tar.gz"),
            root: format!("node-v20.11.1-{dist}"),
            bin: "bin/node".to_owned(),
            is_zip: false,
        }
    }
}

/// `index.json` body with a single valid release for the host.
fn index_json(d: &Dist) -> String {
    format!(
        r#"[{{"version":"v20.11.1","date":"2024-02-14","files":["{}"],"lts":"Iron"}}]"#,
        d.token
    )
}

fn sha256_hex(bytes: &[u8]) -> String {
    common::sha256_hex(bytes)
}

/// SHASUMS256.txt line for `archive` with the name `file`.
fn shasums_line(archive: &[u8], file: &str) -> String {
    format!("{}  {file}\n", sha256_hex(archive))
}

fn shasums_path() -> String {
    format!("/{TAG}/SHASUMS256.txt")
}

fn archive_path(d: &Dist) -> String {
    format!("/{TAG}/{}", d.file)
}

/// Tar.gz with raw entries: the name does not go through the validation of `tar`,
/// so paths with `..` can be crafted.
fn tar_gz_raw(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut ar = tar::Builder::new(enc);
    for (name, content) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o755);
        {
            let old = header.as_old_mut();
            old.name = [0; 100];
            old.name[..name.len()].copy_from_slice(name.as_bytes());
        }
        header.set_cksum();
        ar.append(&header, content.as_slice()).expect("tar entry");
    }
    ar.into_inner()
        .expect("tar finish")
        .finish()
        .expect("gz finish")
}

/// Zip with the given entries.
fn zip_bytes(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, content) in entries {
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .expect("zip entry");
        zip.write_all(content).expect("zip write");
    }
    zip.finish().expect("zip finish").into_inner()
}

/// Node release archive for the host: the executable plus `extra`.
fn release_archive(d: &Dist, extra: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut entries = vec![(format!("{}/{}", d.root, d.bin), b"fake node".to_vec())];
    entries.extend(extra.iter().cloned());
    if d.is_zip {
        zip_bytes(&entries)
    } else {
        tar_gz_raw(&entries)
    }
}

/// Schedules a consistent index, SHASUMS and archive on `server`.
fn serve_release(server: &TestServer, d: &Dist, archive: &[u8]) {
    server.set_route("/index.json", Route::ok(index_json(d).into_bytes()));
    server.set_route(
        &shasums_path(),
        Route::ok(shasums_line(archive, &d.file).into_bytes()),
    );
    server.set_route(&archive_path(d), Route::ok(archive.to_vec()));
}

/// Creates a fake installation of `TAG` with the host executable.
fn fake_installed(store: &Path, d: &Dist) -> PathBuf {
    let dir = store.join("versions").join(TAG);
    let node = dir.join(&d.bin);
    fs::create_dir_all(node.parent().expect("parent")).expect("version dir");
    fs::write(&node, b"fake node").expect("fake node");
    dir
}

/// `install` arguments against the local server.
fn install_args(mirror: &str) -> Vec<String> {
    vec![
        "install".into(),
        VERSION.into(),
        "--mirror".into(),
        mirror.into(),
        "--allow-http".into(),
    ]
}

/// Restores write permissions on drop (so that `TempDir` can delete).
#[cfg(unix)]
struct Restore(Vec<PathBuf>);

#[cfg(unix)]
impl Drop for Restore {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        for path in &self.0 {
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o755));
        }
    }
}

/// Closed port: install fails to connect, with the download code.
// Simulates: the mirror does not respond (connection refused); no version and no .part are left.
#[test]
fn install_with_mirror_down_exits_with_download_code() {
    let sb = Sandbox::new();
    let mirror = format!("http://127.0.0.1:{}", closed_port());
    let out = sb
        .cmd()
        .args(["install", VERSION, "--allow-http", "--mirror", &mirror])
        .output()
        .expect("run nvsn");
    assert_code(&out, EXIT_DOWNLOAD);
    assert!(
        stderr_of(&out).contains("--mirror"),
        "the hint must name --mirror:\n{}",
        stderr_of(&out)
    );
    assert_no_version_and_no_leftovers(&sb);
}

/// Corrupt index.json bodies: truncated JSON and valid JSON with another shape.
const CORRUPT_INDEXES: [&str; 2] = [
    r#"[{"version":"v20.11.1","date":"2024-02-"#,
    r#"{"releases":[{"ver":1}]}"#,
];

/// Runs list-remote and install against a corrupt index; returns the outputs.
fn run_against_corrupt_index(body: &str) -> (Output, Output, Sandbox) {
    let server = TestServer::start();
    server.set_route("/index.json", Route::ok(body.as_bytes().to_vec()));
    let mirror = server.base();
    let sb = Sandbox::new();
    let list = sb
        .cmd()
        .args(["list-remote", "--mirror", &mirror, "--allow-http"])
        .output()
        .expect("run nvsn");
    let install = sb
        .cmd()
        .args(install_args(&mirror))
        .output()
        .expect("run nvsn");
    (list, install, sb)
}

/// Truncated JSON and valid JSON with an incorrect shape in index.json.
// Simulates: the mirror serves a corrupt index; list-remote and install fail without panic,
// without a version, and the cache does not keep the bad body.
#[test]
fn corrupt_index_fails_list_remote_and_install_without_panic() {
    for body in CORRUPT_INDEXES {
        let (list, install, sb) = run_against_corrupt_index(body);
        for out in [&list, &install] {
            assert_no_panic(out);
            assert_ne!(
                out.status.code(),
                Some(0),
                "must fail on corrupt index: {body}"
            );
            assert!(
                stderr_of(out).contains("index"),
                "the message must name the index:\n{}",
                stderr_of(out)
            );
        }
        assert_no_version_and_no_leftovers(&sb);
        for path in all_paths(&sb.store().join("cache")) {
            if let Ok(text) = fs::read_to_string(&path) {
                assert!(
                    text.trim() != body,
                    "the corrupt index was cached: {}",
                    path.display()
                );
            }
        }
    }
}

/// A corrupt index is a general error, not a download failure.
// Simulates: same corrupt index; the code should be general (1), not download (7).
#[test]
fn corrupt_index_must_exit_with_general_code() {
    for body in CORRUPT_INDEXES {
        let (list, install, _sb) = run_against_corrupt_index(body);
        assert_code(&list, EXIT_GENERAL);
        assert_code(&install, EXIT_GENERAL);
    }
}

/// The tarball responds 500 persistently.
// Simulates: release archive with HTTP 500; it is retried and nothing is left half-done.
#[test]
fn tarball_http_500_retries_then_fails_without_partial_version() {
    let d = host_dist();
    let archive = release_archive(&d, &[]);
    let server = TestServer::start();
    serve_release(&server, &d, &archive);
    server.set_route(&archive_path(&d), Route::status(500));
    let sb = Sandbox::new();

    let out = sb
        .cmd()
        .args(install_args(&server.base()))
        .output()
        .expect("run nvsn");
    assert_code(&out, EXIT_DOWNLOAD);
    assert!(
        stderr_of(&out).contains("retries"),
        "the message must say the retries were exhausted:\n{}",
        stderr_of(&out)
    );
    // One initial attempt and three retries.
    assert_eq!(server.request_count(&archive_path(&d)), 4);
    assert_no_version_and_no_leftovers(&sb);
}

/// SHASUMS256.txt does not list the archive: verification is mandatory.
// Simulates: mirror that publishes checksums without the archive entry; nothing is installed.
#[test]
fn shasums_without_the_archive_entry_aborts_install() {
    let d = host_dist();
    let archive = release_archive(&d, &[]);
    let server = TestServer::start();
    server.set_route("/index.json", Route::ok(index_json(&d).into_bytes()));
    let other = format!("{}  other-file.tar.gz\n", "ab".repeat(32));
    server.set_route(&shasums_path(), Route::ok(other.into_bytes()));
    server.set_route(&archive_path(&d), Route::ok(archive));
    let sb = Sandbox::new();

    let out = sb
        .cmd()
        .args(install_args(&server.base()))
        .output()
        .expect("run nvsn");
    assert_code(&out, EXIT_DOWNLOAD);
    assert_eq!(
        server.request_count(&archive_path(&d)),
        0,
        "the archive must not be fetched without a checksum"
    );
    assert_no_version_and_no_leftovers(&sb);
}

/// http:// mirror without --allow-http: it is rejected before touching the network.
// Simulates: insecure mirror without the explicit permission; not even one request arrives.
#[test]
fn http_mirror_without_allow_http_is_rejected_before_the_network() {
    let server = TestServer::start();
    serve_release(&server, &host_dist(), b"unused");
    let sb = Sandbox::new();

    let out = sb
        .cmd()
        .args(["install", VERSION, "--mirror", &server.base()])
        .output()
        .expect("run nvsn");
    assert_no_panic(&out);
    assert_ne!(out.status.code(), Some(0), "install must be rejected");
    assert!(
        stderr_of(&out).contains("--allow-http"),
        "the message must name --allow-http:\n{}",
        stderr_of(&out)
    );
    assert_eq!(server.request_count("/index.json"), 0);
    assert_eq!(server.request_count(&shasums_path()), 0);
    assert_no_version_and_no_leftovers(&sb);
}

/// The rejection of http:// without --allow-http must exit with the usage code (2).
// Simulates: same case; the expected code according to the table in exit.rs is 2.
#[test]
fn http_mirror_without_allow_http_must_exit_with_usage_code() {
    let server = TestServer::start();
    serve_release(&server, &host_dist(), b"unused");
    let sb = Sandbox::new();
    let out = sb
        .cmd()
        .args(["install", VERSION, "--mirror", &server.base()])
        .output()
        .expect("run nvsn");
    assert_code(&out, EXIT_USAGE);
}

/// NVSN_DIR points to a regular file: install fails without panic and does not touch the file.
// Simulates: the store root is a file; the server responds, so the failure
// happens when writing to NVSN_DIR.
#[test]
fn nvsn_dir_pointing_to_a_file_fails_without_panic_or_damage() {
    let d = host_dist();
    let archive = release_archive(&d, &[]);
    let server = TestServer::start();
    serve_release(&server, &d, &archive);
    let sb = Sandbox::new();
    let file = sb.root().join("not-a-dir");
    fs::write(&file, b"i am a file").expect("file");

    let out = sb
        .cmd()
        .env("NVSN_DIR", &file)
        .args(install_args(&server.base()))
        .output()
        .expect("run nvsn");
    assert_no_panic(&out);
    assert_ne!(out.status.code(), Some(0), "install must fail");
    assert!(!stderr_of(&out).is_empty(), "an error message is required");
    assert_eq!(fs::read(&file).expect("still a file"), b"i am a file");
}

/// NVSN_DIR as a file must exit with the general code (1).
// Simulates: same case; the expected code is general (1).
#[test]
fn nvsn_dir_pointing_to_a_file_must_exit_with_general_code() {
    let d = host_dist();
    let archive = release_archive(&d, &[]);
    let server = TestServer::start();
    serve_release(&server, &d, &archive);
    let sb = Sandbox::new();
    let file = sb.root().join("not-a-dir");
    fs::write(&file, b"i am a file").expect("file");
    let out = sb
        .cmd()
        .env("NVSN_DIR", &file)
        .args(install_args(&server.base()))
        .output()
        .expect("run nvsn");
    assert_code(&out, EXIT_GENERAL);
}

/// Reading NVSN_DIR pointing to a file: it must be an error, not an empty store.
// Simulates: list over a root that is a file; on Windows read_dir returns
// ERROR_PATH_NOT_FOUND, which versions.rs takes as "does not exist" and exits with 0.
#[cfg(windows)]
#[test]
fn list_with_nvsn_dir_on_a_file_must_fail() {
    let sb = Sandbox::new();
    let file = sb.root().join("not-a-dir");
    fs::write(&file, b"i am a file").expect("file");
    let out = sb
        .cmd()
        .env("NVSN_DIR", &file)
        .arg("list")
        .output()
        .expect("run nvsn");
    assert_no_panic(&out);
    assert_ne!(
        out.status.code(),
        Some(0),
        "list must fail: {}",
        stdout_of(&out)
    );
}

/// Read-only NVSN_DIR: installing fails without leaving anything half-done.
// Simulates: the store does not accept writes (Unix permissions); Unix only, see the response.
#[cfg(unix)]
#[test]
fn install_into_read_only_store_fails_cleanly() {
    let d = host_dist();
    let archive = release_archive(&d, &[]);
    let server = TestServer::start();
    serve_release(&server, &d, &archive);
    let sb = Sandbox::new();
    let store = sb.store();
    let dirs = [
        store.clone(),
        store.join("versions"),
        store.join("cache"),
        store.join("locks"),
    ];
    for dir in &dirs {
        fs::create_dir_all(dir).expect("dir");
    }
    // Restore is declared after sb: it is dropped first so that TempDir can delete.
    let _restore = Restore(dirs.to_vec());
    for dir in &dirs {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o555)).expect("chmod");
    }

    let out = sb
        .cmd()
        .args(install_args(&server.base()))
        .output()
        .expect("run nvsn");
    assert_no_panic(&out);
    assert_ne!(out.status.code(), Some(0), "install must fail");
    assert_no_version_and_no_leftovers(&sb);
}

/// An orphan lock (file with no process holding it) does not block the installation.
// Simulates: locks/<tag>.lock from a run that died; the OS releases the lock.
#[test]
fn orphan_lock_file_does_not_block_install() {
    let d = host_dist();
    let archive = release_archive(&d, &[]);
    let server = TestServer::start();
    serve_release(&server, &d, &archive);
    let sb = Sandbox::new();
    let locks = sb.store().join("locks");
    fs::create_dir_all(&locks).expect("locks");
    fs::write(locks.join(format!("{TAG}.lock")), b"stale").expect("orphan lock");

    let out = sb
        .cmd()
        .args(install_args(&server.base()))
        .output()
        .expect("run nvsn");
    assert_code(&out, 0);
    assert!(sb.store().join("versions").join(TAG).is_dir());
}

/// A `.tmp-*` from a previous installation is not taken for a version.
// Simulates: orphan staging from a dead process; install works and list ignores it.
#[test]
fn leftover_tmp_staging_dir_is_ignored_by_install_and_list() {
    let d = host_dist();
    let archive = release_archive(&d, &[]);
    let server = TestServer::start();
    serve_release(&server, &d, &archive);
    let sb = Sandbox::new();
    let staging = sb.store().join("versions").join(".tmp-999-stale");
    fs::create_dir_all(staging.join("junk")).expect("staging");
    let stale_bin = staging.join(&d.bin);
    fs::create_dir_all(stale_bin.parent().expect("bin dir")).expect("staging bin dir");
    fs::write(&stale_bin, b"half extracted").expect("junk");

    let out = sb
        .cmd()
        .args(install_args(&server.base()))
        .output()
        .expect("run nvsn");
    assert_code(&out, 0);

    let list = sb.cmd().arg("list").output().expect("run nvsn");
    assert_code(&list, 0);
    let text = stdout_of(&list);
    assert!(
        text.contains(VERSION),
        "the installed version is listed:\n{text}"
    );
    assert!(
        !text.contains(".tmp-"),
        "staging must not be listed:\n{text}"
    );

    let which = sb
        .cmd()
        .args(["which", VERSION])
        .output()
        .expect("run nvsn");
    assert_code(&which, 0);
    assert!(
        !stdout_of(&which).contains(".tmp-"),
        "which must point to the real version"
    );
}

/// .nvmrc with BOM, CRLF and comments; and another one with garbage.
// Simulates: version file with Windows format and BOM; then an invalid one.
#[test]
fn nvmrc_with_bom_crlf_and_comments_is_read_and_garbage_is_rejected() {
    let d = host_dist();
    let archive = release_archive(&d, &[]);
    let server = TestServer::start();
    serve_release(&server, &d, &archive);
    let sb = Sandbox::new();
    let work = sb.root().join("work");
    fs::create_dir_all(&work).expect("work");

    let good: &[u8] = b"\xEF\xBB\xBF# node for the project\r\n  20.11.1  # pinned\r\n";
    fs::write(work.join(".nvmrc"), good).expect("nvmrc");
    let out = sb
        .cmd()
        .current_dir(&work)
        .env("HOME", &work)
        .env("USERPROFILE", &work)
        .args(["install", "--mirror", &server.base(), "--allow-http"])
        .output()
        .expect("run nvsn");
    assert_code(&out, 0);
    assert!(sb.store().join("versions").join(TAG).is_dir());

    fs::write(work.join(".nvmrc"), b"foo bar baz\n").expect("garbage");
    let out = sb
        .cmd()
        .current_dir(&work)
        .env("HOME", &work)
        .env("USERPROFILE", &work)
        .args(["install", "--mirror", &server.base(), "--allow-http"])
        .output()
        .expect("run nvsn");
    assert_code(&out, EXIT_VERSION_FILE);
    assert!(
        stderr_of(&out).contains("foo bar baz") || stderr_of(&out).contains(".nvmrc"),
        "the message must name the file or the value:\n{}",
        stderr_of(&out)
    );
    assert_eq!(
        server.request_count("/index.json"),
        1,
        "garbage must not reach the network"
    );
}

/// Version folder without a node executable: which and run fail with a clear message.
// Simulates: broken installation (folder without a binary); there is no panic and reinstalling is suggested.
#[test]
fn installed_version_without_node_binary_fails_which_and_run() {
    let sb = Sandbox::new();
    let dir = sb.store().join("versions").join(TAG);
    fs::create_dir_all(&dir).expect("dir");
    fs::write(dir.join("README"), b"broken").expect("readme");

    for args in [
        vec!["which", VERSION],
        vec!["run", VERSION, "--", "--version"],
    ] {
        let out = sb.cmd().args(&args).output().expect("run nvsn");
        assert_code(&out, EXIT_GENERAL);
        let err = stderr_of(&out);
        assert!(err.contains("no node executable"), "message:\n{err}");
        assert!(err.contains("reinstall"), "hint:\n{err}");
    }
}

/// Uninstall a version that is not installed.
// Simulates: uninstall over an empty store; not-found code, without panic.
#[test]
fn uninstall_of_missing_version_exits_with_not_found() {
    let sb = Sandbox::new();
    let out = sb
        .cmd()
        .args(["uninstall", VERSION])
        .output()
        .expect("run nvsn");
    assert_code(&out, EXIT_NOT_FOUND);
    assert!(
        stderr_of(&out).contains("nvsn install"),
        "hint:\n{}",
        stderr_of(&out)
    );
}

/// use and run without any installed version.
// Simulates: first use of nvsn without installing anything; code 6 and an install hint.
#[test]
fn use_and_run_without_any_installed_version_suggest_install() {
    let sb = Sandbox::new();
    for args in [vec!["use", "20"], vec!["run", "20"]] {
        let out = sb.cmd().args(&args).output().expect("run nvsn");
        assert_code(&out, EXIT_NOT_FOUND);
        assert!(
            stderr_of(&out).contains("nvsn install 20"),
            "hint for {args:?}:\n{}",
            stderr_of(&out)
        );
    }
}

/// Tar entry that escapes the destination.
// Simulates: malicious archive with ../ that tries to write outside NVSN_DIR.
#[test]
fn archive_entry_escaping_destination_is_rejected() {
    let d = host_dist();
    let evil = format!("../../../{ESCAPE_MARKER}");
    let archive = release_archive(&d, &[(evil, b"pwned".to_vec())]);
    let server = TestServer::start();
    serve_release(&server, &d, &archive);
    let sb = Sandbox::new();

    let out = sb
        .cmd()
        .args(install_args(&server.base()))
        .output()
        .expect("run nvsn");
    assert_no_panic(&out);
    assert_ne!(out.status.code(), Some(0), "install must be rejected");
    assert_no_version_and_no_leftovers(&sb);
    let escaped = all_paths(sb.root())
        .into_iter()
        .any(|p| p.file_name().is_some_and(|n| n == ESCAPE_MARKER));
    assert!(!escaped, "the marker was written outside the store");
}

/// Invalid flags and subcommands: usage code.
// Simulates: clap usage error (nonexistent flag, nonexistent subcommand).
#[test]
fn invalid_flags_and_subcommands_exit_with_usage_code() {
    let sb = Sandbox::new();
    for args in [
        vec!["install", VERSION, "--version-x"],
        vec!["frobnicate"],
        vec!["list", "--bogus"],
    ] {
        let out = sb.cmd().args(&args).output().expect("run nvsn");
        assert_code(&out, EXIT_USAGE);
    }
}

/// uninstall and install of the same version at the same time.
// Simulates: two nvsn processes on the same version; each exits with a
// documented code, without panic, and the final state is consistent.
#[test]
fn concurrent_uninstall_and_install_of_same_version_terminate_cleanly() {
    let d = host_dist();
    let archive = release_archive(&d, &[]);
    let server = TestServer::start();
    serve_release(&server, &d, &archive);
    let mirror = server.base();
    const DOCUMENTED: [i32; 5] = [
        0,
        EXIT_GENERAL,
        EXIT_NOT_FOUND,
        EXIT_DOWNLOAD,
        EXIT_LOCK_BUSY,
    ];

    for round in 0..5 {
        let sb = Sandbox::new();
        fake_installed(&sb.store(), &d);
        let root_a = sb.root().to_path_buf();
        let root_b = sb.root().to_path_buf();
        let mirror_b = mirror.clone();

        let uninstall = thread::spawn(move || {
            common::nvsn(&nvsn_bin(), &root_a)
                .args(["uninstall", VERSION, "--yes"])
                .output()
                .expect("run uninstall")
        });
        let install = thread::spawn(move || {
            common::nvsn(&nvsn_bin(), &root_b)
                .args(install_args(&mirror_b))
                .output()
                .expect("run install")
        });
        let un = uninstall.join().expect("uninstall thread");
        let ins = install.join().expect("install thread");

        for (name, out) in [("uninstall", &un), ("install", &ins)] {
            assert_no_panic(out);
            let code = out.status.code().unwrap_or(-1);
            assert!(
                DOCUMENTED.contains(&code),
                "round {round}: {name} exited with undocumented code {code}:\n{}",
                stderr_of(out)
            );
        }
        let dir = sb.store().join("versions").join(TAG);
        if dir.exists() {
            assert!(
                dir.join(&d.bin).is_file(),
                "round {round}: {TAG} exists without its binary"
            );
        }
    }
}

#[test]
fn run_of_indexed_but_uninstalled_version_says_it_is_not_installed() {
    // The version is in the index but not installed: the message must say so.
    let d = host_dist();
    let archive = release_archive(&d, &[]);
    let server = TestServer::start();
    serve_release(&server, &d, &archive);
    let sb = Sandbox::new();
    let list = sb
        .cmd()
        .args(["list-remote", "--mirror", &server.base(), "--allow-http"])
        .output()
        .expect("run nvsn");
    assert_code(&list, 0);

    let out = sb
        .cmd()
        .args(["run", VERSION, "--mirror", &server.base(), "--allow-http"])
        .output()
        .expect("run nvsn");
    assert_code(&out, EXIT_NOT_FOUND);
    let text = stderr_of(&out);
    assert!(
        text.contains("is not installed"),
        "the message must say the version is not installed:\n{text}"
    );
    assert!(
        text.contains(TAG),
        "the message names the matched tag:\n{text}"
    );
}
