//! Shared HTTP client, verbose-logging helpers, and download options.
//!
//! Adapted from gvsn 1.5.1 (MIT, same author).
//!
//! All outbound HTTP requests in `nvsn` go through [`HttpClient`].
//! Using a single client ensures consistent timeout and header settings
//! across the nodejs.org index, the SHASUMS256.txt file, and binary downloads.
//!
//! Transport policy: HTTPS is mandatory unless the caller opts in with
//! `allow_http` (the `--allow-http` flag). The client enforces it for every
//! request and redirect hop, and [`ensure_https`] validates URLs up front.
//!
//! When the `--verbose` / `-v` flag is passed, [`log_request`] and
//! [`log_response`] print HTTP negotiation details to stderr so the user
//! can diagnose connectivity, redirects, and server behaviour.

use std::time::Duration;

use anyhow::{bail, Context, Result};
use colored::Colorize;

/// Transport options for the HTTP client.
///
/// `Default` gives the safe configuration: no verbose output, no retries, and
/// HTTPS mandatory (`allow_http = false`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HttpOptions {
    /// Prints the negotiation headers to stderr.
    pub verbose: bool,
    /// Number of network retries per download.
    pub retries: u8,
    /// Allows `http://` in addition to `https://` (`--allow-http`). Defaults to `false`.
    pub allow_http: bool,
}

/// HTTP client configuration shared across all requests.
#[derive(Debug, Clone)]
pub struct HttpClient {
    agent: ureq::Agent,
    verbose: bool,
    retries: u8,
    allow_http: bool,
    /// Proxy URL without credentials, safe to show or log.
    proxy: Option<String>,
}

impl HttpClient {
    /// Creates a new `HttpClient` with the given verbosity and retry settings.
    ///
    /// The client requires HTTPS. To allow `http://` (`--allow-http`), use
    /// [`HttpClient::with_options`].
    ///
    /// # Errors
    ///
    /// Returns an error if the `ureq` agent cannot be built.
    pub fn new(verbose: bool, retries: u8) -> Result<Self> {
        Self::with_options(HttpOptions {
            verbose,
            retries,
            allow_http: false,
        })
    }

    /// Creates a new `HttpClient` from explicit [`HttpOptions`].
    ///
    /// With `allow_http = false` the agent also rejects `http://` on redirects.
    /// The client and the install options (`InstallOptions::allow_http`) must
    /// receive the same value; if they do not match, the operation fails closed.
    ///
    /// # Errors
    ///
    /// Returns an error if the `ureq` agent cannot be built.
    pub fn with_options(options: HttpOptions) -> Result<Self> {
        Self::with_proxy(options, None)
    }

    /// Creates a new `HttpClient` that sends its requests through `proxy`.
    ///
    /// `proxy` must be an `http://` or `https://` URL (see [`validate_proxy`]).
    /// With `None` the agent keeps ureq's default, which reads `HTTPS_PROXY`,
    /// `HTTP_PROXY` and `NO_PROXY` from the environment.
    ///
    /// Credentials in the URL are used for the connection but are never kept
    /// in a form that `Debug` or [`HttpClient::proxy`] would print.
    ///
    /// # Errors
    ///
    /// Returns an error if the proxy URL is invalid or the agent cannot be built.
    pub fn with_proxy(options: HttpOptions, proxy: Option<&str>) -> Result<Self> {
        let proxy = proxy.map(str::trim);
        let mut builder = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(15)))
            // Bounds the whole request (connect + write + read), so a
            // connection that stalls after the handshake (flaky Wi-Fi, a CDN
            // that stops sending bytes) fails and hits the retry/back-off
            // logic instead of hanging forever. Kept generous (5 min) because
            // downloads share this client: `download::fetch` resumes from the
            // last written byte via `Range` on retry, so a merely slow (but
            // still progressing) transfer just continues across a couple of
            // attempts instead of failing outright.
            .timeout_global(Some(Duration::from_secs(300)))
            // Enforced by ureq for the request and every redirect hop, so an
            // https mirror cannot bounce the transfer to plain http.
            .https_only(!options.allow_http)
            .user_agent(format!("nvsn/{}", env!("CARGO_PKG_VERSION")));

        let redacted = match proxy {
            Some(url) => {
                validate_proxy(url)?;
                let parsed = ureq::Proxy::new(url).context("invalid proxy URL")?;
                builder = builder.proxy(Some(parsed));
                Some(redact_proxy(url))
            }
            None => None,
        };

        let agent = builder.build().new_agent();
        Ok(Self {
            agent,
            verbose: options.verbose,
            retries: options.retries,
            allow_http: options.allow_http,
            proxy: redacted,
        })
    }

    /// Returns the proxy in use, without credentials, or `None` if the client
    /// was built without an explicit proxy.
    pub fn proxy(&self) -> Option<&str> {
        self.proxy.as_deref()
    }

    /// Returns the underlying `ureq` agent.
    pub fn agent(&self) -> &ureq::Agent {
        &self.agent
    }

    /// Returns `true` when verbose mode is active.
    pub fn is_verbose(&self) -> bool {
        self.verbose
    }

    /// Returns the configured retry limit.
    pub fn retries(&self) -> u8 {
        self.retries
    }

    /// Returns `true` when plain `http://` URLs are allowed (`--allow-http`).
    pub fn allow_http(&self) -> bool {
        self.allow_http
    }
}

/// Checks that `url` can be used under the given transport policy.
///
/// Accepts `https://` always, and `http://` only if `allow_http` is `true`
/// (the explicit `--allow-http` option). Any other scheme is rejected.
/// Localhost and 127.0.0.1 are not an exception: only the flag allows `http://`.
///
/// # Errors
///
/// Returns `insecure URL rejected; use https or --allow-http` if the scheme
/// is not allowed. The message does not include the URL, so credentials it
/// might carry are not leaked.
pub fn ensure_https(url: &str, allow_http: bool) -> Result<()> {
    let scheme = url
        .split_once("://")
        .map(|(scheme, _)| scheme.to_ascii_lowercase());
    match scheme.as_deref() {
        Some("https") => Ok(()),
        Some("http") if allow_http => Ok(()),
        _ => bail!("insecure URL rejected; use https or --allow-http"),
    }
}

/// Checks that `proxy` is an `http://` or `https://` proxy URL with a host.
///
/// Other schemes (`socks5://`, `ftp://`, ...) are rejected, even if ureq could
/// parse them: nvsn only supports HTTP CONNECT proxies. Error messages do not
/// repeat the URL, so credentials it may carry are not leaked.
///
/// # Errors
///
/// Returns an error if the scheme is not `http` or `https`, or if no host is given.
pub fn validate_proxy(proxy: &str) -> Result<()> {
    let Some((scheme, rest)) = proxy.trim().split_once("://") else {
        bail!("invalid proxy URL; expected http:// or https:// followed by host[:port]");
    };
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        bail!("unsupported proxy scheme; use http:// or https://");
    }
    let host = proxy_host_port(rest).split(':').next().unwrap_or("");
    if host.is_empty() {
        bail!("invalid proxy URL; the host is missing");
    }
    Ok(())
}

/// Builds `scheme://host[:port]` from a proxy URL, dropping userinfo and path.
/// The input must already have passed [`validate_proxy`].
fn redact_proxy(proxy: &str) -> String {
    let proxy = proxy.trim();
    let (scheme, rest) = proxy.split_once("://").unwrap_or(("http", proxy));
    format!(
        "{}://{}",
        scheme.to_ascii_lowercase(),
        proxy_host_port(rest)
    )
}

/// Returns `host[:port]` from `rest`, the part of a URL after `://`.
/// The path and any `userinfo@` prefix are dropped.
fn proxy_host_port(rest: &str) -> &str {
    let authority = rest.split('/').next().unwrap_or("");
    authority.rsplit('@').next().unwrap_or("")
}

/// Logs an outgoing HTTP request line to stderr when verbose mode is active.
pub fn log_request(client: &HttpClient, method: &str, url: &str) {
    if client.is_verbose() {
        eprintln!("  {} > {} {}", "[v]".dimmed(), method.bold(), url);
    }
}

/// Logs an incoming HTTP response status and all headers to stderr when
/// verbose mode is active.
///
/// `headers` is the `http::HeaderMap` returned by `ureq::Response::headers()`.
pub fn log_response(
    client: &HttpClient,
    status: u16,
    reason: &str,
    headers: &ureq::http::HeaderMap,
) {
    if client.is_verbose() {
        eprintln!(
            "  {} < {} {}",
            "[v]".dimmed(),
            status.to_string().bold(),
            reason
        );
        for (name, value) in headers {
            eprintln!(
                "  {} < {}: {}",
                "[v]".dimmed(),
                name,
                value.to_str().unwrap_or("<binary>")
            );
        }
        eprintln!();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_client_stores_verbose_and_retries() {
        let client = HttpClient::new(true, 5).unwrap();
        assert!(client.is_verbose());
        assert_eq!(client.retries(), 5);
        assert!(!client.allow_http());

        let client = HttpClient::new(false, 0).unwrap();
        assert!(!client.is_verbose());
        assert_eq!(client.retries(), 0);
    }

    #[test]
    fn with_options_stores_allow_http() {
        let client = HttpClient::with_options(HttpOptions {
            verbose: false,
            retries: 2,
            allow_http: true,
        })
        .unwrap();
        assert!(client.allow_http());
        assert_eq!(client.retries(), 2);
    }

    #[test]
    fn validate_proxy_accepts_http_and_https_with_host() {
        assert!(validate_proxy("http://proxy.example:3128").is_ok());
        assert!(validate_proxy("HTTPS://proxy.example").is_ok());
        assert!(validate_proxy("http://user:pass@10.0.0.1:8080/").is_ok());
        assert!(validate_proxy("  http://[::1]:3128  ").is_ok());
    }

    #[test]
    fn validate_proxy_rejects_other_schemes_and_malformed_values() {
        assert!(validate_proxy("socks5://proxy.example:1080").is_err());
        assert!(validate_proxy("socks5h://proxy.example:1080").is_err());
        assert!(validate_proxy("ftp://proxy.example").is_err());
        assert!(validate_proxy("proxy.example:3128").is_err());
        assert!(validate_proxy("http://").is_err());
        assert!(validate_proxy("http://:3128").is_err());
        assert!(validate_proxy("").is_err());
    }

    #[test]
    fn validate_proxy_error_does_not_echo_credentials() {
        let err = validate_proxy("socks5://user:hunter2@proxy.example").unwrap_err();
        assert!(!err.to_string().contains("hunter2"));
    }

    #[test]
    fn with_proxy_builds_agent_with_the_proxy_and_hides_credentials() {
        let client = HttpClient::with_proxy(
            HttpOptions::default(),
            Some("http://user:secret@proxy.example:3128/path"),
        )
        .unwrap();
        assert!(client.agent().config().proxy().is_some());
        assert_eq!(client.proxy(), Some("http://proxy.example:3128"));
        assert!(!format!("{client:?}").contains("secret"));
    }

    #[test]
    fn with_proxy_accepts_https_proxies() {
        let client =
            HttpClient::with_proxy(HttpOptions::default(), Some("https://proxy.example")).unwrap();
        assert_eq!(client.proxy(), Some("https://proxy.example"));
    }

    #[test]
    fn with_proxy_rejects_invalid_proxy_before_building_agent() {
        assert!(HttpClient::with_proxy(HttpOptions::default(), Some("socks5://p:1080")).is_err());
        assert!(HttpClient::with_proxy(HttpOptions::default(), Some("http://")).is_err());
    }

    #[test]
    fn with_options_has_no_explicit_proxy() {
        let client = HttpClient::new(false, 0).unwrap();
        assert_eq!(client.proxy(), None);
    }

    #[test]
    fn agent_is_accessible() {
        let client = HttpClient::new(false, 3).unwrap();
        // Just confirm the accessor returns a usable agent reference; no
        // network call is made.
        let _agent: &ureq::Agent = client.agent();
    }

    #[test]
    fn ensure_https_accepts_https_and_rejects_http_by_default() {
        assert!(ensure_https("https://nodejs.org/dist/index.json", false).is_ok());
        assert!(ensure_https("HTTPS://nodejs.org/dist", false).is_ok());

        let err = ensure_https("http://nodejs.org/dist", false).unwrap_err();
        assert_eq!(
            err.to_string(),
            "insecure URL rejected; use https or --allow-http"
        );
    }

    #[test]
    fn ensure_https_allows_http_only_with_flag() {
        assert!(ensure_https("http://127.0.0.1:8080/dist", true).is_ok());
        assert!(ensure_https("http://localhost/dist", false).is_err());
    }

    #[test]
    fn ensure_https_rejects_other_schemes_and_bad_input() {
        assert!(ensure_https("ftp://example.com/x", true).is_err());
        assert!(ensure_https("nodejs.org/dist", true).is_err());
        assert!(ensure_https("", false).is_err());
    }

    #[test]
    fn log_request_and_response_are_silent_when_not_verbose() {
        // These should not panic and should simply do nothing when verbose
        // mode is off. There is no way to assert on stderr output directly,
        // but exercising the non-verbose branch still covers the
        // early-return path.
        let client = HttpClient::new(false, 3).unwrap();
        log_request(&client, "GET", "https://example.invalid/");

        let headers = ureq::http::HeaderMap::new();
        log_response(&client, 200, "OK", &headers);
    }

    #[test]
    fn log_request_and_response_do_not_panic_when_verbose() {
        let client = HttpClient::new(true, 3).unwrap();
        log_request(&client, "GET", "https://example.invalid/");

        let mut headers = ureq::http::HeaderMap::new();
        headers.insert(
            ureq::http::header::CONTENT_TYPE,
            "application/json".parse().unwrap(),
        );
        log_response(&client, 404, "Not Found", &headers);
    }
}
