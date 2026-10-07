//! nvsn-net: HTTP client (ureq + rustls), resumable downloads and retries.
//!
//! It knows nothing about Node or versions. Verifying integrity (SHA-256) is
//! the responsibility of `nvsn-core`. HTTPS is mandatory unless `allow_http` is set.

pub mod download;
pub mod http;

pub use download::fetch;
pub use http::{ensure_https, log_request, log_response, validate_proxy, HttpClient, HttpOptions};
