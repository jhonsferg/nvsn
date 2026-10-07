//! Minimal HTTP server on `127.0.0.1` for tests. Compiled only in tests.
//!
//! Serves fixed responses per path and records each request (path and
//! `If-None-Match`). It never leaves loopback, so there is no real network access.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Scheduled response for a path.
#[derive(Debug, Clone)]
pub(crate) struct Route {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Route {
    /// 200 response with `body`.
    pub(crate) fn ok(body: Vec<u8>) -> Self {
        Self {
            status: 200,
            headers: Vec::new(),
            body,
        }
    }

    /// Empty response with the given status.
    pub(crate) fn status(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    /// Adds a header to the response.
    pub(crate) fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }
}

/// Request received by the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Request {
    pub(crate) path: String,
    pub(crate) if_none_match: Option<String>,
}

/// Test server. It stops and joins its thread when dropped.
pub(crate) struct TestServer {
    addr: std::net::SocketAddr,
    routes: Arc<Mutex<HashMap<String, Route>>>,
    requests: Arc<Mutex<Vec<Request>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl TestServer {
    /// Starts the server on a free loopback port.
    pub(crate) fn start() -> Self {
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
    pub(crate) fn base(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Schedules the response for `path`.
    pub(crate) fn set_route(&self, path: &str, route: Route) {
        self.routes
            .lock()
            .expect("routes lock")
            .insert(path.to_owned(), route);
    }

    /// Requests received for `path`, in order.
    pub(crate) fn requests(&self, path: &str) -> Vec<Request> {
        self.requests
            .lock()
            .expect("requests lock")
            .iter()
            .filter(|r| r.path == path)
            .cloned()
            .collect()
    }

    /// Number of requests received for `path`.
    pub(crate) fn request_count(&self, path: &str) -> usize {
        self.requests(path).len()
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

/// Serves one connection: reads the request and answers with the scheduled route.
fn handle(
    stream: TcpStream,
    routes: &Mutex<HashMap<String, Route>>,
    requests: &Mutex<Vec<Request>>,
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

    let mut if_none_match = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("if-none-match") {
                if_none_match = Some(value.trim().to_owned());
            }
        }
    }

    requests.lock().expect("requests lock").push(Request {
        path: path.clone(),
        if_none_match,
    });

    let route = routes
        .lock()
        .expect("routes lock")
        .get(&path)
        .cloned()
        .unwrap_or_else(|| Route::status(404));

    let mut head = format!("HTTP/1.1 {} {}\r\n", route.status, reason(route.status));
    for (name, value) in &route.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        route.body.len()
    ));

    let mut stream = stream;
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&route.body);
    let _ = stream.flush();
}

/// Reason phrase for the status codes used by the tests.
fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        304 => "Not Modified",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Status",
    }
}
