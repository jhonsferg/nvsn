//! Background update notice.
//!
//! After most commands the latest GitHub release is queried and, if there is
//! a newer major version, a short note is printed on stderr.
//!
//! - **Cache**: the result is stored in `NVSN_DIR/update-check.json`. At most
//!   one query every 24 h. The attempt is recorded before the network call, so
//!   a process that exits before receiving the response does not cause
//!   queries on every command.
//! - **Does not block**: the query runs in a thread. At the end of the command it
//!   only waits for the remaining budget of [`TOTAL_BUDGET`]; if it has not finished, the
//!   note is skipped without delay.
//! - **No visible failures**: any error (network, JSON, cache) is ignored.
//! - **Can be disabled**: `NVSN_NO_UPDATE_CHECK=1`, `CI` set or `--offline`.
//!   It does not run for `env`, `path`, `__hook`, `deactivate`, `completions` or
//!   `self-update` (that command already reports the version).

use crate::cli::Command;
use crate::ctx::Ctx;
use colored::Colorize;
use nvsn_net::{ensure_https, HttpClient, HttpOptions};
use nvsn_platform::env::{EnvSource, ProcessEnv};
use semver::Version;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Cache file, inside `NVSN_DIR`.
const CACHE_FILE: &str = "update-check.json";

/// Minimum time between two queries to the API.
const TTL_SECS: u64 = 24 * 60 * 60;

/// Maximum time for the query in the background thread.
const CHECK_TIMEOUT: Duration = Duration::from_millis(1500);

/// Maximum time the command waits for the query, counted from when it started.
const TOTAL_BUDGET: Duration = Duration::from_millis(1000);

/// Repository whose releases are queried.
const REPO: &str = "jhonsferg/nvsn";

/// GitHub API base.
const API_BASE: &str = "https://api.github.com";

/// Variable that overrides the API base (tests with a local server).
const API_BASE_ENV: &str = "NVSN_TEST_API_BASE";

/// Variable that disables the notice.
const DISABLE_ENV: &str = "NVSN_NO_UPDATE_CHECK";

/// Content of `update-check.json`. `latest_version` stays empty if a response
/// was never obtained.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Cache {
    last_checked_unix: u64,
    latest_version: String,
}

/// Newer version found: `current` is the one of this binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    current: Version,
    latest: Version,
}

/// Query in progress: result channel and start time.
pub type Pending = (Receiver<Option<Notice>>, Instant);

/// Starts the background query if the command allows it.
///
/// Returns `None` if it must not run, or if the thread could not be created.
pub fn spawn(ctx: &Ctx, command: &Command) -> Option<Pending> {
    let enabled = should_run(
        command,
        ctx.offline,
        disabled_by_env(),
        std::env::var_os("CI").is_some(),
    );
    if !enabled {
        return None;
    }

    let root = ctx.store.root().to_path_buf();
    let allow_http = ctx.allow_http;
    let (tx, rx) = mpsc::channel();
    thread::Builder::new()
        .name("nvsn-update-check".to_owned())
        .spawn(move || {
            let _ = tx.send(check(&root, allow_http));
        })
        .ok()?;
    Some((rx, Instant::now()))
}

/// Waits for the remaining budget and, if there is a new version, prints the note.
pub fn print_if_ready(pending: Option<Pending>) {
    let Some((rx, started)) = pending else {
        return;
    };
    let remaining = TOTAL_BUDGET.saturating_sub(started.elapsed());
    if let Ok(Some(notice)) = rx.recv_timeout(remaining) {
        eprintln!("{}", format_notice(&notice));
    }
}

/// Indicates whether the notice may run for `command`, given the external factors.
fn should_run(command: &Command, offline: bool, disabled: bool, ci: bool) -> bool {
    if offline || disabled || ci {
        return false;
    }
    !matches!(
        command,
        Command::Env { .. }
            | Command::On { .. }
            | Command::Path { .. }
            | Command::Hook { .. }
            | Command::Deactivate { .. }
            | Command::Completions { .. }
            | Command::SelfUpdate { .. }
    )
}

/// `NVSN_NO_UPDATE_CHECK` with any value other than empty and `0`.
fn disabled_by_env() -> bool {
    ProcessEnv
        .var(DISABLE_ENV)
        .is_some_and(|value| !value.is_empty() && value != "0")
}

/// Query with cache. Returns the note if there is a newer version than the current one.
fn check(root: &Path, allow_http: bool) -> Option<Notice> {
    let current = Version::parse(env!("CARGO_PKG_VERSION")).ok()?;
    let cache_path = cache_path(root);
    let now = now_unix();

    let latest_text = match load_cache(&cache_path) {
        Some(cache) if !is_stale(cache.last_checked_unix, now) => cache.latest_version,
        previous => {
            let known = previous
                .map(|cache| cache.latest_version)
                .unwrap_or_default();
            // The attempt is recorded before the network call: if the process exits before
            // the response, it is not queried again until 24 h later.
            save_cache(
                &cache_path,
                &Cache {
                    last_checked_unix: now,
                    latest_version: known.clone(),
                },
            );
            match fetch_latest_tag(allow_http) {
                Some(tag) => {
                    save_cache(
                        &cache_path,
                        &Cache {
                            last_checked_unix: now,
                            latest_version: tag.clone(),
                        },
                    );
                    tag
                }
                None => known,
            }
        }
    };

    let latest = Version::parse(latest_text.trim().trim_start_matches('v')).ok()?;
    (latest > current).then_some(Notice { current, latest })
}

/// Requests the latest release tag from the API. No retries: the notice is
/// best-effort and must not delay the command.
fn fetch_latest_tag(allow_http: bool) -> Option<String> {
    let client = HttpClient::with_options(HttpOptions {
        verbose: false,
        retries: 0,
        allow_http,
    })
    .ok()?;
    let base = ProcessEnv
        .var(API_BASE_ENV)
        .map(|value| value.trim_end_matches('/').to_owned())
        .unwrap_or_else(|| API_BASE.to_owned());
    let url = format!("{base}/repos/{REPO}/releases/latest");
    ensure_https(&url, allow_http).ok()?;

    let mut response = client
        .agent()
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .config()
        .timeout_global(Some(CHECK_TIMEOUT))
        .http_status_as_error(true)
        .build()
        .call()
        .ok()?;
    let text = response.body_mut().read_to_string().ok()?;
    let body: serde_json::Value = serde_json::from_str(&text).ok()?;
    body.get("tag_name")?.as_str().map(str::to_owned)
}

/// Reads the cache. A missing or corrupt file counts as no cache.
fn load_cache(path: &Path) -> Option<Cache> {
    let content = std::fs::read_to_string(path).ok()?;
    let value: Value = serde_json::from_str(&content).ok()?;
    Some(Cache {
        last_checked_unix: value.get("last_checked_unix")?.as_u64()?,
        latest_version: value.get("latest_version")?.as_str()?.to_owned(),
    })
}

/// Writes the cache. Errors are ignored: the notice must never fail.
fn save_cache(path: &Path, cache: &Cache) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let value = json!({
        "last_checked_unix": cache.last_checked_unix,
        "latest_version": cache.latest_version,
    });
    let _ = std::fs::write(path, value.to_string());
}

/// Indicates whether a query made at `last_checked_unix` has already expired relative to `now_unix`.
fn is_stale(last_checked_unix: u64, now_unix: u64) -> bool {
    now_unix.saturating_sub(last_checked_unix) >= TTL_SECS
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

/// One-line note with the current version, the new one and the command to update.
fn format_notice(notice: &Notice) -> String {
    format!(
        "\n  {} nvsn v{} -> v{}. Run {} to update.\n",
        "Update available:".yellow().bold(),
        notice.current,
        notice.latest,
        "nvsn upgrade".cyan()
    )
}

/// Path of the cache file inside the root directory `root`.
fn cache_path(root: &Path) -> PathBuf {
    root.join(CACHE_FILE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_stale_before_ttl_is_false() {
        assert!(!is_stale(1000, 1000 + TTL_SECS - 1));
    }

    #[test]
    fn is_stale_at_or_after_ttl_is_true() {
        assert!(is_stale(1000, 1000 + TTL_SECS));
        assert!(is_stale(1000, 1000 + TTL_SECS + 1));
    }

    #[test]
    fn clock_going_backwards_is_not_stale() {
        assert!(!is_stale(1000, 500));
    }

    #[test]
    fn cache_round_trips_and_corrupt_files_are_absent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = cache_path(dir.path());
        let cache = Cache {
            last_checked_unix: 12345,
            latest_version: "9.9.9".to_owned(),
        };
        save_cache(&path, &cache);
        assert_eq!(load_cache(&path), Some(cache));

        std::fs::write(&path, "not json").expect("write");
        assert_eq!(load_cache(&path), None);
    }

    #[test]
    fn save_creates_a_missing_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = cache_path(&dir.path().join("store"));
        save_cache(
            &path,
            &Cache {
                last_checked_unix: 1,
                latest_version: String::new(),
            },
        );
        assert!(path.is_file());
    }

    #[test]
    fn excluded_commands_never_run_the_check() {
        let env = Command::Env { shell: None };
        let path = Command::Path { version: None };
        let update = Command::SelfUpdate {
            check: false,
            force: false,
            retries: 3,
        };
        assert!(!should_run(&env, false, false, false));
        assert!(!should_run(&path, false, false, false));
        assert!(!should_run(&update, false, false, false));
        assert!(!should_run(
            &Command::Completions {
                shell: crate::cli::CompletionShell::Bash
            },
            false,
            false,
            false
        ));
    }

    #[test]
    fn regular_commands_run_unless_disabled() {
        assert!(should_run(&Command::Outdated, false, false, false));
        assert!(!should_run(&Command::Outdated, false, true, false));
        assert!(!should_run(&Command::Outdated, false, false, true));
        assert!(!should_run(&Command::Outdated, true, false, false));
    }

    #[test]
    fn notice_names_both_versions_and_the_command() {
        let notice = Notice {
            current: Version::new(0, 1, 0),
            latest: Version::new(0, 2, 0),
        };
        let text = format_notice(&notice);
        assert!(text.contains("v0.1.0"));
        assert!(text.contains("v0.2.0"));
        assert!(text.contains("nvsn upgrade"));
    }
}
