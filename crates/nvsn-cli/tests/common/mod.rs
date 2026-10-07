//! Common utilities for the integration tests: loopback HTTP server,
//! copy of the binary and test release archives. No real network.
//!
//! Adapted from `nvsn-core/src/install/test_server.rs`.

#![allow(dead_code)]

use assert_cmd::Command;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Scheduled response for a path.
#[derive(Debug, Clone)]
pub struct Route {
    status: u16,
    body: Vec<u8>,
}

impl Route {
    /// 200 response with `body`.
    pub fn ok(body: Vec<u8>) -> Self {
        Self { status: 200, body }
    }

    /// Empty response with the given status.
    pub fn status(status: u16) -> Self {
        Self {
            status,
            body: Vec::new(),
        }
    }
}

/// Test server. It stops and joins its thread when dropped.
pub struct TestServer {
    addr: std::net::SocketAddr,
    routes: Arc<Mutex<HashMap<String, Route>>>,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl TestServer {
    /// Starts the server on a free loopback port.
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");
        listener.set_nonblocking(true).expect("nonblocking");

        let routes = Arc::new(Mutex::new(HashMap::new()));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));

        let (r, q, s) = (
            Arc::clone(&routes),
            Arc::clone(&requests),
            Arc::clone(&stop),
        );
        let thread = thread::spawn(move || {
            while !s.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => handle(stream, &r, &q),
                    Err(e) if e.kind() == ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });

        Self {
            addr,
            routes,
            requests,
            stop,
            thread: Some(thread),
        }
    }

    /// Server base, e.g. `http://127.0.0.1:41234`, without a trailing slash.
    pub fn base(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Schedules the response for `path`.
    pub fn set_route(&self, path: &str, route: Route) {
        self.routes
            .lock()
            .expect("routes lock")
            .insert(path.to_owned(), route);
    }

    /// Number of requests received for `path`.
    pub fn request_count(&self, path: &str) -> usize {
        self.requests
            .lock()
            .expect("requests lock")
            .iter()
            .filter(|p| p.as_str() == path)
            .count()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

/// Serves a connection: records the path and responds with the scheduled route.
fn handle(
    stream: TcpStream,
    routes: &Mutex<HashMap<String, Route>>,
    requests: &Mutex<Vec<String>>,
) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let Ok(clone) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(clone);

    let mut request_line = String::new();
    if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
        return;
    }
    let path = request_line
        .split_whitespace()
        .nth(1)
        .unwrap_or("/")
        .to_owned();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        if line.trim_end().is_empty() {
            break;
        }
    }

    requests.lock().expect("requests lock").push(path.clone());
    let route = routes
        .lock()
        .expect("routes lock")
        .get(&path)
        .cloned()
        .unwrap_or_else(|| Route::status(404));

    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        route.status,
        if route.status == 200 { "OK" } else { "Error" },
        route.body.len()
    );
    let mut stream = stream;
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&route.body);
    let _ = stream.flush();
}

/// nvsn release archive name for the test host.
pub fn host_archive_name() -> &'static str {
    match (
        cfg!(windows),
        cfg!(target_os = "macos"),
        cfg!(target_arch = "aarch64"),
    ) {
        (true, _, true) => "nvsn_windows_arm64.zip",
        (true, _, false) => "nvsn_windows_x86_64.zip",
        (false, true, true) => "nvsn_darwin_aarch64.tar.gz",
        (false, true, false) => "nvsn_darwin_x86_64.tar.gz",
        (false, false, true) => "nvsn_linux_aarch64.tar.gz",
        (false, false, false) => "nvsn_linux_x86_64.tar.gz",
    }
}

/// Binary name inside the release archive on the test host.
pub fn host_binary_name() -> &'static str {
    if cfg!(windows) {
        "nvsn.exe"
    } else {
        "nvsn"
    }
}

/// Release archive (`.zip` or `.tar.gz`) with a single binary `name` containing `content`.
pub fn release_archive(name: &str, content: &[u8]) -> Vec<u8> {
    if name.ends_with(".zip") {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        zip.start_file(host_binary_name(), zip::write::SimpleFileOptions::default())
            .expect("zip entry");
        zip.write_all(content).expect("zip write");
        zip.finish().expect("zip finish").into_inner()
    } else {
        let enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut ar = tar::Builder::new(enc);
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        ar.append_data(&mut header, host_binary_name(), content)
            .expect("tar entry");
        ar.into_inner()
            .expect("tar finish")
            .finish()
            .expect("gz finish")
    }
}

/// SHA-256 in lowercase hexadecimal.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Lock that serializes the tests of a test binary that write and then
/// execute a file (`copy_binary`, fake scripts).
///
/// Cause of `ETXTBSY` ("Text file busy") on Linux: while one thread has the
/// executable open for writing, another thread calls `fork`. The child inherits that
/// descriptor until its `exec`, and the first thread's `exec` fails. Closing the
/// file or calling `sync_all` beforehand is not enough: the problem is any `fork` that
/// happens while the descriptor is open. That is why each test of the binary
/// holds this lock for its whole body: there are never two tests at once.
pub fn exec_lock() -> MutexGuard<'static, ()> {
    static EXEC_LOCK: Mutex<()> = Mutex::new(());
    EXEC_LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Copies the nvsn test binary to `dir/bin/`. This way `self-update` and
/// `self-uninstall` only touch the copy, never the cargo binary.
///
/// Must be called with `exec_lock()` held (see there for the reason).
pub fn copy_binary(dir: &Path) -> PathBuf {
    let bin_dir = dir.join("bin");
    std::fs::create_dir_all(&bin_dir).expect("bin dir");
    let dest = bin_dir.join(host_binary_name());
    std::fs::copy(env!("CARGO_BIN_EXE_nvsn"), &dest).expect("copy nvsn binary");
    dest
}

/// Command for the nvsn copy isolated in `dir`: HOME inside `dir`, no active version,
/// no integration and no network variables from the user's environment.
pub fn nvsn(binary: &Path, dir: &Path) -> Command {
    let mut cmd = Command::new(binary);
    cmd.current_dir(dir)
        .env("NVSN_DIR", dir.join("store"))
        .env("HOME", dir.join("home"))
        .env("USERPROFILE", dir.join("home"))
        .env("NO_COLOR", "1")
        .env_remove("NVSN_VERSION")
        .env_remove("NVSN_INTEGRATION")
        .env_remove("NVSN_NODEJS_ORG_MIRROR")
        .env_remove("NVSN_REPO")
        .env_remove("NVSN_TEST_API_BASE")
        .env_remove("NVSN_TEST_DL_BASE")
        // No update notice: no test command touches the network.
        .env("NVSN_NO_UPDATE_CHECK", "1");
    cmd
}
