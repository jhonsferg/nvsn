//! Single-stream download engine with resume and retry support.
//!
//! Adapted from gvsn 1.5.1 (MIT, same author).
//!
//! [`fetch`] and [`fetch_with_progress`] are the entry points for downloads in
//! `nvsn`. Progress goes to a [`ProgressSink`]; this module draws nothing.
//! Every download uses one HTTP connection end to end: no chunking, no
//! multi-bar coordination, nothing to race against. A large read buffer
//! keeps per-syscall overhead low so the stream can use as much of the
//! link's actual throughput as the OS scheduler allows.
//!
//! There is no separate HEAD probe: nodejs.org's download links 302-redirect to
//! a CDN host, and a HEAD request pays that redirect round-trip just to
//! learn the file size. The GET response carries the same size headers, so
//! [`try_fetch`] reads them off the real transfer instead of a throwaway
//! request.
//!
//! Resume is transparent: if a `.part` file already exists from a previous
//! attempt, the download picks up from the last written byte using a
//! `Range: bytes=N-` request. The resumed answer is only appended when it is
//! a 206 whose `Content-Range` starts exactly at the requested offset. A 416
//! either means the partial file is already complete, or it does not match
//! the origin; in the latter case the download restarts from scratch. If the
//! server ignores the Range header the transfer restarts from scratch too.
//!
//! On a transient network error or a 5xx/408/429 status the download retries
//! up to [`HttpClient::retries`] times using exponential back-off (1 s, 2 s,
//! 4 s, ...). Other 4xx statuses fail at once.
//!
//! Plain `http://` URLs are refused unless the client was built with
//! `allow_http` (see [`ensure_https`]).
//!
//! This module does not verify integrity. The caller checks the SHA-256 of
//! the completed file (see `nvsn-core`) before extracting it.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use colored::Colorize;

use crate::http::{ensure_https, HttpClient};

/// Size of the buffer used to read the response body. Large enough to keep
/// per-syscall overhead low and let the single stream use as much of the
/// link's throughput as possible.
const READ_BUF_SIZE: usize = 1024 * 1024;

/// Receiver of a download's progress. Implement it to show a progress bar;
/// the library draws nothing on its own.
///
/// `on_start` is called at the start of each attempt (also after a retry) and
/// resets the counter. If the download is resumed, `on_bytes` first receives
/// the bytes that were already in the `.part` file.
pub trait ProgressSink {
    /// Expected total size in bytes. `0` if the server does not report it.
    fn on_start(&self, total: u64);

    /// New bytes written to the temporary destination.
    fn on_bytes(&self, n: u64);
}

/// Null receiver: discards progress. It is the default for [`fetch`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoProgress;

impl ProgressSink for NoProgress {
    fn on_start(&self, _total: u64) {}

    fn on_bytes(&self, _n: u64) {}
}

/// Downloads `url` to `dest` over a single HTTP stream, without reporting progress.
///
/// Equivalent to [`fetch_with_progress`] with [`NoProgress`].
///
/// # Errors
///
/// Returns an error if the URL is not https (and the client does not allow
/// http), the server answers with a permanent error (for example 404), all
/// retries are exhausted, or `dest` cannot be written to.
pub fn fetch(client: &HttpClient, url: &str, dest: &Path) -> Result<()> {
    fetch_with_progress(client, url, dest, &NoProgress)
}

/// Downloads `url` to `dest` over a single HTTP stream, reporting progress to `progress`.
///
/// The body is written to `"{dest}.part"` and renamed to `dest` only when the
/// transfer completes. The retry limit is read from the `HttpClient`.
///
/// # Errors
///
/// Same as [`fetch`].
pub fn fetch_with_progress(
    client: &HttpClient,
    url: &str,
    dest: &Path,
    progress: &dyn ProgressSink,
) -> Result<()> {
    ensure_https(url, client.allow_http())?;
    let retries = client.retries();
    fetch_single(client, url, dest, retries, progress)
}

/// Reason why a download attempt did not complete the file.
enum Failure {
    /// Transient failure (network, 5xx, 408, 429): retried with back-off.
    Retry(anyhow::Error),
    /// The `.part` file does not match the origin (416 without a matching total,
    /// or a `Content-Range` that differs from the requested offset). It is
    /// deleted and the download starts from scratch. Does not consume a retry.
    Restart,
    /// Definitive error (other 4xx, unexpected response, disk error).
    Permanent(anyhow::Error),
}

/// Single-connection download with retry and transparent resume.
fn fetch_single(
    client: &HttpClient,
    url: &str,
    dest: &Path,
    retries: u8,
    progress: &dyn ProgressSink,
) -> Result<()> {
    let part = part_path(dest);
    let mut attempt = 0u8;
    let mut restarted = false;

    loop {
        let existing = part.metadata().map(|m| m.len()).unwrap_or(0);

        match try_fetch(client, url, &part, existing, progress) {
            Ok(()) => {
                std::fs::rename(&part, dest)
                    .with_context(|| format!("Cannot finalise {}", dest.display()))?;
                return Ok(());
            }
            Err(Failure::Restart) if !restarted => {
                // An attempt without Range never produces Restart, so a single
                // courtesy retry is enough to avoid an infinite loop.
                restarted = true;
                let _ = std::fs::remove_file(&part);
                eprintln!(
                    "  {} Partial download does not match the server, starting over",
                    "!".yellow()
                );
            }
            Err(Failure::Restart) => {
                let _ = std::fs::remove_file(&part);
                bail!("{url} rejected the download twice; giving up");
            }
            Err(Failure::Permanent(err)) => {
                let _ = std::fs::remove_file(&part);
                return Err(err);
            }
            Err(Failure::Retry(err)) => {
                if attempt >= retries {
                    let _ = std::fs::remove_file(&part);
                    return Err(err).context(format!("Download failed after {retries} retries"));
                }
                attempt += 1;
                eprintln!(
                    "  {} Network error, retrying ({}/{retries})...",
                    "!".yellow(),
                    attempt
                );
                thread::sleep(Duration::from_secs(backoff(attempt)));
            }
        }
    }
}

/// Performs a single GET request and streams the body to `part` with a
/// large read buffer ([`READ_BUF_SIZE`]) to keep per-syscall overhead low.
///
/// Reports the total size from the GET response's own headers: no separate
/// HEAD request, so there is only ever one redirect round-trip per attempt
/// instead of two.
fn try_fetch(
    client: &HttpClient,
    url: &str,
    part: &Path,
    offset: u64,
    progress: &dyn ProgressSink,
) -> Result<(), Failure> {
    crate::http::log_request(client, "GET", url);

    let mut req = client
        .agent()
        .get(url)
        // Prevent transparent gzip so the raw binary body is never decoded.
        .header("Accept-Encoding", "identity");
    if offset > 0 {
        req = req.header("Range", &format!("bytes={offset}-"));
    }

    // An HTTP status is not a ureq error: it is classified here to tell apart
    // 416 (restart), 5xx (retry) and 404 (definitive).
    let mut response = req
        .config()
        .http_status_as_error(false)
        .build()
        .call()
        .with_context(|| format!("Failed to connect to {url}"))
        .map_err(Failure::Retry)?;

    let status = response.status().as_u16();
    crate::http::log_response(
        client,
        status,
        response.status().canonical_reason().unwrap_or(""),
        response.headers(),
    );

    if status == 416 {
        return range_not_satisfiable(response.headers(), offset, url);
    }
    if status != 200 && status != 206 {
        let err = anyhow!("{url} answered HTTP {status}");
        return Err(if is_transient(status) {
            Failure::Retry(err)
        } else {
            Failure::Permanent(err)
        });
    }

    // 206 is only valid as a response to our Range, and it must start exactly
    // at the requested offset; otherwise the .part file cannot be resumed.
    let resuming = if status == 206 {
        if offset == 0 {
            return Err(Failure::Permanent(anyhow!(
                "{url} answered 206 Partial Content to a request without Range"
            )));
        }
        let start = content_range(response.headers()).and_then(|(start, _)| start);
        if start != Some(offset) {
            return Err(Failure::Restart);
        }
        true
    } else {
        false
    };

    // The CDN normally reports the full file size via
    // x-identity-content-length, even on a 206 response, so it doesn't need
    // adjusting for the current offset. Kept as a harmless fallback. Plain
    // content-length is only the size of the remaining bytes on a 206, so add
    // offset back to get the total for the progress bar.
    let headers = response.headers();
    let total_length = headers
        .get("x-identity-content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or_else(|| {
            let len = headers
                .get("content-length")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);
            if resuming {
                offset + len
            } else {
                len
            }
        });

    let mut file: File = if resuming {
        OpenOptions::new()
            .append(true)
            .open(part)
            .with_context(|| format!("Cannot open {} for appending", part.display()))
            .map_err(Failure::Permanent)?
    } else {
        File::create(part)
            .with_context(|| format!("Cannot create {}", part.display()))
            .map_err(Failure::Permanent)?
    };

    progress.on_start(total_length);
    if resuming {
        progress.on_bytes(offset);
    }

    let mut reader = response.body_mut().as_reader();
    let mut buf = vec![0u8; READ_BUF_SIZE];
    loop {
        let n = match reader.read(&mut buf) {
            Ok(n) => n,
            Err(e) => {
                break Err(Failure::Retry(
                    anyhow::Error::from(e).context("Download interrupted"),
                ))
            }
        };
        if n == 0 {
            break Ok(());
        }
        if let Err(e) = file.write_all(&buf[..n]) {
            break Err(Failure::Permanent(
                anyhow::Error::from(e).context("Write failed"),
            ));
        }
        progress.on_bytes(n as u64);
    }
}

/// Classifies a 416 response to a resume request.
///
/// If the `Content-Range` (`bytes */TOTAL`) matches what is already written, the
/// `.part` file is complete and is accepted (SHA-256 validates it later).
/// In any other case the `.part` file does not match the origin and is restarted.
fn range_not_satisfiable(
    headers: &ureq::http::HeaderMap,
    offset: u64,
    url: &str,
) -> Result<(), Failure> {
    if offset == 0 {
        return Err(Failure::Permanent(anyhow!(
            "{url} answered 416 Range Not Satisfiable to a full request"
        )));
    }
    if let Some((None, Some(total))) = content_range(headers) {
        if total == offset {
            return Ok(());
        }
    }
    Err(Failure::Restart)
}

/// Parses the `Content-Range` header into `(start, total)`.
///
/// Accepts `bytes START-END/TOTAL` and `bytes */TOTAL`. A `*` is returned as
/// `None`. Returns `None` if the header is missing or does not have that form.
fn content_range(headers: &ureq::http::HeaderMap) -> Option<(Option<u64>, Option<u64>)> {
    let value = headers.get("content-range")?.to_str().ok()?;
    let spec = value.trim().strip_prefix("bytes ")?;
    let (range, total) = spec.split_once('/')?;
    let total = if total == "*" {
        None
    } else {
        Some(total.parse::<u64>().ok()?)
    };
    let start = if range == "*" {
        None
    } else {
        Some(range.split_once('-')?.0.parse::<u64>().ok()?)
    };
    Some((start, total))
}

/// Indicates whether an HTTP status deserves a retry: 408, 429 and 5xx.
fn is_transient(status: u16) -> bool {
    status == 408 || status == 429 || (500..600).contains(&status)
}

/// Returns the path used for the in-progress download: `"{dest}.part"`.
fn part_path(dest: &Path) -> PathBuf {
    PathBuf::from(format!("{}.part", dest.to_string_lossy()))
}

/// Exponential back-off in seconds for retry attempt `n` (1-based): 1, 2, 4, 8 ...
///
/// The shift is capped so large retry counts saturate instead of overflowing.
fn backoff(n: u8) -> u64 {
    let shift = u32::from(n.saturating_sub(1)).min(63);
    1u64 << shift
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::HttpOptions;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::time::Instant;
    use tempfile::tempdir;

    /// Client for this test's loopback server.
    ///
    /// `allow_http` is enabled only here: the server is `http://127.0.0.1` without
    /// TLS. It never leaves loopback, so there is no real network access.
    fn local_client(retries: u8) -> HttpClient {
        HttpClient::with_options(HttpOptions {
            verbose: false,
            retries,
            allow_http: true,
        })
        .unwrap()
    }

    /// Serializes an HTTP/1.1 response with `Connection: close`.
    fn raw(status: &str, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
        let mut head = format!("HTTP/1.1 {status}\r\n");
        for (name, value) in headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        head.push_str(&format!(
            "Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        ));
        let mut bytes = head.into_bytes();
        bytes.extend_from_slice(body);
        bytes
    }

    /// Serves raw responses in order, one per connection. Returns the base URL and
    /// a thread that, when finished, yields the `Range` header of each request
    /// (`None` if it did not carry one).
    fn serve(responses: Vec<Vec<u8>>) -> (String, thread::JoinHandle<Vec<Option<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let mut ranges = Vec::new();
            for response in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut range = None;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap() == 0 {
                        break;
                    }
                    if line == "\r\n" || line == "\n" {
                        break;
                    }
                    let is_range =
                        line.len() >= 6 && line.as_bytes()[..6].eq_ignore_ascii_case(b"range:");
                    if is_range {
                        range = Some(line[6..].trim().to_owned());
                    }
                }
                ranges.push(range);
                stream.write_all(&response).unwrap();
                stream.flush().unwrap();
            }
            ranges
        });
        (format!("http://{addr}"), handle)
    }

    #[test]
    fn part_path_appends_suffix() {
        let dest = Path::new("node-v22.11.0-linux-x64.tar.gz");
        assert_eq!(
            part_path(dest),
            PathBuf::from("node-v22.11.0-linux-x64.tar.gz.part")
        );
    }

    #[test]
    fn part_path_handles_paths_without_extension() {
        let dest = Path::new("node-archive");
        assert_eq!(part_path(dest), PathBuf::from("node-archive.part"));
    }

    #[test]
    fn backoff_doubles_each_attempt() {
        assert_eq!(backoff(1), 1);
        assert_eq!(backoff(2), 2);
        assert_eq!(backoff(3), 4);
        assert_eq!(backoff(4), 8);
    }

    #[test]
    fn backoff_saturates_instead_of_overflowing() {
        assert_eq!(backoff(0), 1);
        assert_eq!(backoff(255), 1u64 << 63);
    }

    /// Sink that records the total and the sum of bytes received.
    #[derive(Default)]
    struct Recorder {
        total: std::cell::Cell<Option<u64>>,
        bytes: std::cell::Cell<u64>,
    }

    impl ProgressSink for Recorder {
        fn on_start(&self, total: u64) {
            self.total.set(Some(total));
            self.bytes.set(0);
        }

        fn on_bytes(&self, n: u64) {
            self.bytes.set(self.bytes.get() + n);
        }
    }

    #[test]
    fn fetch_with_progress_reports_total_and_all_bytes() {
        let full = b"0123456789ABCDEF".to_vec();
        let (base, server) = serve(vec![raw("200 OK", &[], &full)]);

        let dir = tempdir().unwrap();
        let dest = dir.path().join("out.bin");
        let recorder = Recorder::default();

        fetch_with_progress(
            &local_client(0),
            &format!("{base}/file.bin"),
            &dest,
            &recorder,
        )
        .unwrap();

        server.join().unwrap();
        assert_eq!(recorder.total.get(), Some(16));
        assert_eq!(recorder.bytes.get(), 16);
        assert_eq!(std::fs::read(&dest).unwrap(), full);
    }

    #[test]
    fn fetch_with_progress_counts_resumed_bytes_once() {
        let full = b"0123456789ABCDEF".to_vec();
        let (base, server) = serve(vec![raw(
            "206 Partial Content",
            &[("Content-Range", "bytes 5-15/16")],
            &full[5..],
        )]);

        let dir = tempdir().unwrap();
        let dest = dir.path().join("out.bin");
        std::fs::write(part_path(&dest), &full[..5]).unwrap();
        let recorder = Recorder::default();

        fetch_with_progress(
            &local_client(0),
            &format!("{base}/file.bin"),
            &dest,
            &recorder,
        )
        .unwrap();

        server.join().unwrap();
        assert_eq!(recorder.total.get(), Some(16));
        assert_eq!(recorder.bytes.get(), 16);
        assert_eq!(std::fs::read(&dest).unwrap(), full);
    }

    #[test]
    fn no_progress_is_a_silent_default() {
        let sink: &dyn ProgressSink = &NoProgress;
        sink.on_start(10);
        sink.on_bytes(10);
    }

    #[test]
    fn content_range_parses_partial_and_unsatisfied_forms() {
        let mut headers = ureq::http::HeaderMap::new();
        headers.insert("content-range", "bytes 5-31/32".parse().unwrap());
        assert_eq!(content_range(&headers), Some((Some(5), Some(32))));

        headers.insert("content-range", "bytes */32".parse().unwrap());
        assert_eq!(content_range(&headers), Some((None, Some(32))));

        headers.insert("content-range", "items 5-31/32".parse().unwrap());
        assert_eq!(content_range(&headers), None);
    }

    #[test]
    fn fetch_rejects_plain_http_without_allow_http() {
        let dir = tempdir().unwrap();
        let dest = dir.path().join("out.bin");
        let client = HttpClient::new(false, 0).unwrap();

        // Never connects: the URL is rejected before any network access.
        let err = fetch(&client, "http://127.0.0.1:1/file.bin", &dest).unwrap_err();
        assert!(err.to_string().contains("insecure URL rejected"), "{err:#}");
        assert!(!dest.exists());
        assert!(!part_path(&dest).exists());
    }

    /// End-to-end regression test for resume: a `.part` file with some bytes
    /// already on disk must produce a `Range` request for the remaining
    /// bytes, and the response must be appended (not overwritten) so the
    /// final file matches the full original content. Runs against a minimal
    /// HTTP server bound to the loopback interface only (no external network).
    #[test]
    fn fetch_resumes_from_existing_partial_file_via_range_request() {
        use std::io::Write;

        let full_content = b"Hello, resumable download world!".to_vec();
        let already_written = 5usize; // "Hello"
        let remaining = full_content[already_written..].to_vec();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let remaining_for_server = remaining.clone();
        let total_len = full_content.len();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());

            let mut request_line = String::new();
            reader.read_line(&mut request_line).unwrap();

            let mut saw_range_header = false;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" || line.is_empty() {
                    break;
                }
                if line.to_ascii_lowercase().starts_with("range:") && line.contains("bytes=5-") {
                    saw_range_header = true;
                }
            }
            assert!(
                saw_range_header,
                "expected a 'Range: bytes=5-' header on the resumed request"
            );

            let mut stream = stream;
            let response = format!(
                "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 5-{}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                total_len - 1,
                total_len,
                remaining_for_server.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            stream.write_all(&remaining_for_server).unwrap();
            stream.flush().unwrap();
        });

        let dir = tempdir().unwrap();
        let dest = dir.path().join("out.bin");
        std::fs::write(part_path(&dest), &full_content[..already_written]).unwrap();

        let client = local_client(0);
        let url = format!("http://{addr}/file.bin");
        fetch(&client, &url, &dest).unwrap();

        server.join().unwrap();

        assert_eq!(std::fs::read(&dest).unwrap(), full_content);
        assert!(!part_path(&dest).exists());
    }

    #[test]
    fn fetch_restarts_from_scratch_when_416_does_not_match_the_origin() {
        // A .part file larger than the remote file: the origin answers 416 without a
        // matching total. It must restart from scratch without consuming retries
        // (retries = 0 proves it).
        let full = b"0123456789ABCDEF".to_vec();
        let (base, server) = serve(vec![
            raw("416 Range Not Satisfiable", &[], b""),
            raw("200 OK", &[], &full),
        ]);

        let dir = tempdir().unwrap();
        let dest = dir.path().join("out.bin");
        std::fs::write(part_path(&dest), vec![b'X'; 40]).unwrap();

        fetch(&local_client(0), &format!("{base}/file.bin"), &dest).unwrap();

        let ranges = server.join().unwrap();
        assert_eq!(ranges, vec![Some("bytes=40-".to_owned()), None]);
        assert_eq!(std::fs::read(&dest).unwrap(), full);
        assert!(!part_path(&dest).exists());
    }

    #[test]
    fn fetch_accepts_416_when_partial_file_is_already_complete() {
        let full = b"0123456789ABCDEF".to_vec();
        let (base, server) = serve(vec![raw(
            "416 Range Not Satisfiable",
            &[("Content-Range", "bytes */16")],
            b"",
        )]);

        let dir = tempdir().unwrap();
        let dest = dir.path().join("out.bin");
        std::fs::write(part_path(&dest), &full).unwrap();

        fetch(&local_client(0), &format!("{base}/file.bin"), &dest).unwrap();

        let ranges = server.join().unwrap();
        assert_eq!(ranges, vec![Some("bytes=16-".to_owned())]);
        assert_eq!(std::fs::read(&dest).unwrap(), full);
    }

    #[test]
    fn fetch_restarts_when_content_range_does_not_start_at_offset() {
        // The proxy answers 206 but from byte 3 instead of the requested 5. Appending
        // that would corrupt the file; it must restart from scratch.
        let full = b"0123456789ABCDEF".to_vec();
        let (base, server) = serve(vec![
            raw(
                "206 Partial Content",
                &[("Content-Range", "bytes 3-15/16")],
                &full[3..],
            ),
            raw("200 OK", &[], &full),
        ]);

        let dir = tempdir().unwrap();
        let dest = dir.path().join("out.bin");
        std::fs::write(part_path(&dest), &full[..5]).unwrap();

        fetch(&local_client(0), &format!("{base}/file.bin"), &dest).unwrap();

        let ranges = server.join().unwrap();
        assert_eq!(ranges, vec![Some("bytes=5-".to_owned()), None]);
        assert_eq!(std::fs::read(&dest).unwrap(), full);
    }

    #[test]
    fn fetch_does_not_retry_permanent_client_errors() {
        let (base, server) = serve(vec![raw("404 Not Found", &[], b"missing")]);

        let dir = tempdir().unwrap();
        let dest = dir.path().join("out.bin");

        // With retries = 3, one retry would cost at least 1 s of back-off.
        let started = Instant::now();
        let err = fetch(&local_client(3), &format!("{base}/file.bin"), &dest).unwrap_err();
        let elapsed = started.elapsed();

        assert!(err.to_string().contains("404"), "{err:#}");
        assert!(elapsed < Duration::from_secs(1), "retried: {elapsed:?}");
        assert_eq!(server.join().unwrap().len(), 1);
        assert!(!dest.exists());
    }
}
