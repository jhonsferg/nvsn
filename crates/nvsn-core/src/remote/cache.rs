//! On-disk cache of the remote `index.json` index (ADR-012, ARCHITECTURE 6 and 8.2).
//!
//! For each mirror there are two files under `cache/index/`:
//! - `<hash>.json`: the body of `index.json` as is.
//! - `<hash>.meta.json`: mirror base, ETag and fetch time.
//!
//! `<hash>` is the first 16 hexadecimal characters of the SHA-256 of the
//! mirror base. The meta is JSON, not TOML, to keep existing caches readable
//! (`toml` is used for `config.toml` only, see ADR-036).
//!
//! Rules:
//! - Within the TTL there is no network access.
//! - Outside the TTL it is revalidated with `If-None-Match`. A 304 reuses the body
//!   and renews the fetch time.
//! - Atomic writes: temporary file and `rename`.
//! - A corrupt body or an unreadable meta is treated as absent.

use crate::remote::{parse_index, RemoteRelease};
use crate::store::Store;
use anyhow::{bail, Context, Result};
use nvsn_net::{ensure_https, log_request, log_response, HttpClient};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Default TTL of the index (ARCHITECTURE 6: 6 h).
pub const DEFAULT_INDEX_TTL: Duration = Duration::from_secs(6 * 60 * 60);

/// Maximum accepted size for `index.json` (the real one is about 0.3 to 1 MB).
const MAX_INDEX_BYTES: u64 = 64 * 1024 * 1024;

/// Metadata of a cache entry (`<hash>.meta.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CacheMeta {
    /// Mirror base without a trailing slash. Guards against hash collisions.
    base: String,
    /// ETag returned by the server, if any.
    etag: Option<String>,
    /// Unix seconds at which the body was fetched or revalidated.
    fetched_at: u64,
}

/// Cache entry read and already validated.
struct Cached {
    meta: CacheMeta,
    releases: Vec<RemoteRelease>,
}

/// Paths of the two cache files of a mirror.
struct IndexPaths {
    body: PathBuf,
    meta: PathBuf,
}

/// Returns the version index of `base`, using the cache of the `store`.
///
/// - Cache within `ttl`: returned without touching the network.
/// - Cache outside `ttl` and without `offline`: revalidated with `If-None-Match`.
/// - `offline`: the cache is returned even if expired. Without a cache, an error.
///
/// The base is passed as a parameter (it is not fixed) to support mirrors. The
/// final index base is `{base}/index.json`.
///
/// # Errors
///
/// Returns an error if:
/// - `offline` and there is no valid cache.
/// - The network fails, or the server answers with a status other than 200 or 304.
/// - The downloaded body is not a valid `index.json`. In that case nothing
///   is written to the cache.
/// - The base is `http://` and the `HttpClient` does not have `allow_http`. It is
///   rejected before the cache is read.
pub fn fetch_index(
    client: &HttpClient,
    store: &Store,
    base: &str,
    ttl: Duration,
    offline: bool,
) -> Result<Vec<RemoteRelease>> {
    let base = base.trim_end_matches('/');
    ensure_https(base, client.allow_http())?;
    let paths = index_paths(store, base);
    let cached = read_cached(&paths, base);

    if let Some(entry) = &cached {
        if is_fresh(entry.meta.fetched_at, now_secs(), ttl) {
            return Ok(entry.releases.clone());
        }
        if offline {
            return Ok(entry.releases.clone());
        }
    } else if offline {
        bail!(
            "no cached index for {base} and offline mode is on; \
             run once without --offline to download it"
        );
    }

    refresh(client, base, &paths, cached)
}

/// Revalidates or downloads the index and updates the cache.
fn refresh(
    client: &HttpClient,
    base: &str,
    paths: &IndexPaths,
    cached: Option<Cached>,
) -> Result<Vec<RemoteRelease>> {
    let url = format!("{base}/index.json");
    log_request(client, "GET", &url);

    let mut request = client
        .agent()
        .get(&url)
        .header("Accept-Encoding", "identity");
    if let Some(etag) = cached.as_ref().and_then(|c| c.meta.etag.as_deref()) {
        request = request.header("If-None-Match", etag);
    }
    let mut response = request
        .config()
        .http_status_as_error(false)
        .build()
        .call()
        .with_context(|| format!("Failed to fetch {url}"))?;

    let status = response.status().as_u16();
    log_response(
        client,
        status,
        response.status().canonical_reason().unwrap_or(""),
        response.headers(),
    );

    if status == 304 {
        let Some(entry) = cached else {
            bail!("{url} answered 304 Not Modified but no cached copy exists");
        };
        let meta = CacheMeta {
            fetched_at: now_secs(),
            ..entry.meta
        };
        write_atomic(&paths.meta, &serde_json::to_vec(&meta)?)?;
        return Ok(entry.releases);
    }
    if status != 200 {
        bail!("{url} answered HTTP {status}; check the mirror (--mirror)");
    }

    let etag = response
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let mut body = Vec::new();
    response
        .body_mut()
        .with_config()
        .limit(MAX_INDEX_BYTES)
        .reader()
        .read_to_end(&mut body)
        .with_context(|| format!("Failed to read the body of {url}"))?;
    let text = String::from_utf8(body).with_context(|| format!("{url} is not UTF-8"))?;

    // Validate before writing: a bad body must not end up in the cache.
    let releases = parse_index(&text)?;

    write_atomic(&paths.body, text.as_bytes())?;
    let meta = CacheMeta {
        base: base.to_owned(),
        etag,
        fetched_at: now_secs(),
    };
    write_atomic(&paths.meta, &serde_json::to_vec(&meta)?)?;
    Ok(releases)
}

/// Reads the cache at `paths`. Returns `None` if something is missing or invalid.
fn read_cached(paths: &IndexPaths, base: &str) -> Option<Cached> {
    let meta: CacheMeta = serde_json::from_slice(&std::fs::read(&paths.meta).ok()?).ok()?;
    if meta.base != base {
        return None;
    }
    let body = std::fs::read_to_string(&paths.body).ok()?;
    let releases = parse_index(&body).ok()?;
    Some(Cached { meta, releases })
}

/// Indicates whether an entry fetched at `fetched_at` is still within `ttl`.
///
/// A `fetched_at` in the future (clock moved backwards) counts as expired.
fn is_fresh(fetched_at: u64, now: u64, ttl: Duration) -> bool {
    now >= fetched_at && now - fetched_at < ttl.as_secs()
}

/// Computes the cache paths of a mirror.
fn index_paths(store: &Store, base: &str) -> IndexPaths {
    let dir = store.cache_dir().join("index");
    let key = cache_key(base);
    IndexPaths {
        body: dir.join(format!("{key}.json")),
        meta: dir.join(format!("{key}.meta.json")),
    }
}

/// Short hash of the mirror base: 16 hexadecimal characters.
fn cache_key(base: &str) -> String {
    Sha256::digest(base.as_bytes())
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Writes `bytes` to `path` atomically: a temporary file in the same
/// directory, then `rename`. A reader never sees a partially written file.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path
        .parent()
        .with_context(|| format!("{} has no parent directory", path.display()))?;
    std::fs::create_dir_all(dir).with_context(|| format!("Failed to create {}", dir.display()))?;

    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("cache");
    let tmp = dir.join(format!(".{name}.{}.tmp", unique_id()));

    let result = (|| -> Result<()> {
        let mut file = std::fs::File::create(&tmp)
            .with_context(|| format!("Failed to create {}", tmp.display()))?;
        file.write_all(bytes)
            .with_context(|| format!("Failed to write {}", tmp.display()))?;
        file.sync_all()
            .with_context(|| format!("Failed to sync {}", tmp.display()))?;
        std::fs::rename(&tmp, path)
            .with_context(|| format!("Failed to move {} into place", path.display()))
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Identifier unique per process and moment, for temporary names.
pub(crate) fn unique_id() -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(
        "{}-{nanos}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    )
}

/// Current Unix seconds (0 if the clock is before 1970).
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install::test_server::{Route, TestServer};
    use nvsn_net::HttpOptions;
    use tempfile::tempdir;

    const INDEX_V1: &str = r#"[
        {"version":"v22.11.0","date":"2024-10-25","files":["linux-x64"],"lts":"Jod"},
        {"version":"v20.11.1","date":"2024-02-14","files":["linux-x64"],"lts":"Iron"}
    ]"#;

    /// Client with an explicit `allow_http`: `TestServer` is `http://127.0.0.1`
    /// without TLS and never leaves loopback.
    fn client() -> HttpClient {
        HttpClient::with_options(HttpOptions {
            verbose: false,
            retries: 0,
            allow_http: true,
        })
        .unwrap()
    }

    #[test]
    fn http_base_is_rejected_before_touching_the_cache() {
        let store_dir = tempdir().unwrap();
        let store = Store::new(store_dir.path());
        let strict = HttpClient::new(false, 0).unwrap();

        // Never connects: the base is rejected before the network and the cache.
        let err = fetch_index(
            &strict,
            &store,
            "http://127.0.0.1:1",
            DEFAULT_INDEX_TTL,
            false,
        )
        .unwrap_err();
        assert!(err.to_string().contains("insecure URL rejected"), "{err}");
        assert!(!store.cache_dir().join("index").exists());
    }

    fn index_route(etag: &str, body: &str) -> Route {
        Route::ok(body.as_bytes().to_vec()).with_header("ETag", etag)
    }

    #[test]
    fn cache_key_is_16_hex_chars_and_depends_on_base() {
        let a = cache_key("https://nodejs.org/dist");
        assert_eq!(a.len(), 16);
        assert!(a.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(a, cache_key("https://mirror.example/dist"));
    }

    #[test]
    fn freshness_respects_ttl_and_clock_skew() {
        let ttl = Duration::from_secs(100);
        assert!(is_fresh(1000, 1050, ttl));
        assert!(!is_fresh(1000, 1100, ttl));
        assert!(
            !is_fresh(1000, 999, ttl),
            "future timestamp counts as stale"
        );
        assert!(!is_fresh(1000, 1000, Duration::ZERO));
    }

    #[test]
    fn cold_fetch_then_fresh_cache_makes_no_request() {
        let server = TestServer::start();
        server.set_route("/index.json", index_route("\"v1\"", INDEX_V1));
        let store_dir = tempdir().unwrap();
        let store = Store::new(store_dir.path());

        let first =
            fetch_index(&client(), &store, &server.base(), DEFAULT_INDEX_TTL, false).unwrap();
        assert_eq!(first.len(), 2);
        assert_eq!(server.request_count("/index.json"), 1);

        let second =
            fetch_index(&client(), &store, &server.base(), DEFAULT_INDEX_TTL, false).unwrap();
        assert_eq!(second, first);
        assert_eq!(
            server.request_count("/index.json"),
            1,
            "a fresh cache must not touch the network"
        );

        let dir = store.cache_dir().join("index");
        let key = cache_key(&server.base());
        assert!(dir.join(format!("{key}.json")).is_file());
        assert!(dir.join(format!("{key}.meta.json")).is_file());

        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temporary files left: {leftovers:?}");
    }

    #[test]
    fn stale_cache_revalidates_with_if_none_match_and_reuses_body_on_304() {
        let server = TestServer::start();
        server.set_route("/index.json", index_route("\"v1\"", INDEX_V1));
        let store_dir = tempdir().unwrap();
        let store = Store::new(store_dir.path());

        // Zero TTL: every entry is already expired.
        let first = fetch_index(&client(), &store, &server.base(), Duration::ZERO, false).unwrap();
        server.set_route("/index.json", Route::status(304));

        let second = fetch_index(&client(), &store, &server.base(), Duration::ZERO, false).unwrap();
        assert_eq!(second, first);

        let requests = server.requests("/index.json");
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].if_none_match.as_deref(), Some("\"v1\""));
    }

    #[test]
    fn stale_cache_is_replaced_on_200() {
        let server = TestServer::start();
        server.set_route("/index.json", index_route("\"v1\"", INDEX_V1));
        let store_dir = tempdir().unwrap();
        let store = Store::new(store_dir.path());
        fetch_index(&client(), &store, &server.base(), Duration::ZERO, false).unwrap();

        let v2 = r#"[{"version":"v24.0.0","date":"2025-05-06","files":["linux-x64"],"lts":false}]"#;
        server.set_route("/index.json", index_route("\"v2\"", v2));
        let refreshed =
            fetch_index(&client(), &store, &server.base(), Duration::ZERO, false).unwrap();
        assert_eq!(refreshed.len(), 1);
        assert_eq!(refreshed[0].tag(), "v24.0.0");
    }

    #[test]
    fn offline_without_cache_fails_with_clear_message() {
        let store_dir = tempdir().unwrap();
        let store = Store::new(store_dir.path());

        // Never connects: the base does not exist.
        let err = fetch_index(
            &client(),
            &store,
            "http://127.0.0.1:1",
            DEFAULT_INDEX_TTL,
            true,
        )
        .unwrap_err();
        assert!(err.to_string().contains("offline"), "{err}");
    }

    #[test]
    fn offline_uses_stale_cache_without_network() {
        let server = TestServer::start();
        server.set_route("/index.json", index_route("\"v1\"", INDEX_V1));
        let store_dir = tempdir().unwrap();
        let store = Store::new(store_dir.path());
        fetch_index(&client(), &store, &server.base(), DEFAULT_INDEX_TTL, false).unwrap();

        let offline = fetch_index(&client(), &store, &server.base(), Duration::ZERO, true).unwrap();
        assert_eq!(offline.len(), 2);
        assert_eq!(server.request_count("/index.json"), 1);
    }

    #[test]
    fn http_error_is_reported_and_nothing_is_cached() {
        let server = TestServer::start();
        server.set_route("/index.json", Route::status(500));
        let store_dir = tempdir().unwrap();
        let store = Store::new(store_dir.path());

        let err =
            fetch_index(&client(), &store, &server.base(), DEFAULT_INDEX_TTL, false).unwrap_err();
        assert!(err.to_string().contains("HTTP 500"), "{err}");
        assert!(!store.cache_dir().join("index").exists());
    }

    #[test]
    fn invalid_body_is_not_cached() {
        let server = TestServer::start();
        server.set_route("/index.json", index_route("\"bad\"", "not json"));
        let store_dir = tempdir().unwrap();
        let store = Store::new(store_dir.path());

        assert!(fetch_index(&client(), &store, &server.base(), DEFAULT_INDEX_TTL, false).is_err());
        assert!(!store.cache_dir().join("index").exists());
    }

    #[test]
    fn corrupt_cache_is_ignored_and_refetched_when_online() {
        let server = TestServer::start();
        server.set_route("/index.json", index_route("\"v1\"", INDEX_V1));
        let store_dir = tempdir().unwrap();
        let store = Store::new(store_dir.path());
        fetch_index(&client(), &store, &server.base(), DEFAULT_INDEX_TTL, false).unwrap();

        let key = cache_key(&server.base());
        let dir = store.cache_dir().join("index");
        std::fs::write(dir.join(format!("{key}.json")), b"garbage").unwrap();

        let releases =
            fetch_index(&client(), &store, &server.base(), DEFAULT_INDEX_TTL, false).unwrap();
        assert_eq!(releases.len(), 2);
        assert_eq!(server.request_count("/index.json"), 2);
    }
}
