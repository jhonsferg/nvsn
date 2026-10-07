//! Execution context: resolved global flags, root directory, network and platform.
//!
//! Configuration precedence (ADR-012): flags > environment variables > default values.

use crate::cli::GlobalArgs;
use crate::exit::{CliError, ErrorKind};
use crate::output::Output;
use nvsn_core::config::{load_settings, Overrides};
use nvsn_core::remote::cache::{fetch_index, DEFAULT_INDEX_TTL};
use nvsn_core::remote::RemoteRelease;
use nvsn_core::Store;
use nvsn_net::{HttpClient, HttpOptions};
use nvsn_platform::env::{EnvSource, ProcessEnv};
use nvsn_platform::paths::resolve_nvsn_dir;
use nvsn_platform::{effective_arch, Os, Platform};
use std::io::IsTerminal;
use std::path::PathBuf;

/// Default network retries.
pub const RETRIES: u8 = 3;
/// Variable that holds the active version of the session.
pub const ENV_VERSION: &str = "NVSN_VERSION";
/// Variable that enables the integration mode with the shell wrapper.
pub const ENV_INTEGRATION: &str = "NVSN_INTEGRATION";
/// Variable for the Node distribution mirror.
pub const ENV_MIRROR: &str = "NVSN_NODEJS_ORG_MIRROR";

/// Context shared by all commands.
#[derive(Debug)]
pub struct Ctx {
    /// Output presentation.
    pub out: Output,
    /// Local store (`NVSN_DIR`).
    pub store: Store,
    /// No network: index cache only.
    pub offline: bool,
    /// Automatic confirmations (`--yes`).
    pub yes: bool,
    /// Detailed HTTP log on stderr.
    pub verbose: bool,
    /// Allows `http://` (`--allow-http`). Insecure; disabled by default.
    pub allow_http: bool,
    /// Mirror base without a trailing slash.
    pub mirror: String,
    /// Proxy from `config.toml` (`proxy`). `None` lets the HTTP client read `HTTPS_PROXY`.
    pub proxy: Option<String>,
    /// npm registry from `config.toml` (`npm_mirror`), if set.
    pub npm_mirror: Option<String>,
    /// Architecture override from `NVSN_ARCH` or `config.toml`, already validated.
    pub arch_override: Option<String>,
    /// An input terminal is available (the user can be asked).
    pub interactive: bool,
    /// Show the progress indicator on stderr.
    pub show_progress: bool,
    /// Run by the shell wrapper (`NVSN_INTEGRATION=1`).
    pub integration: bool,
}

impl Ctx {
    /// Resolves the context from the global flags.
    ///
    /// # Errors
    ///
    /// Returns an error if the root directory cannot be determined.
    pub fn new(global: &GlobalArgs) -> Result<Self, CliError> {
        let env = ProcessEnv;
        let root = resolve_nvsn_dir(global.dir.as_deref(), detect_os(), &env).map_err(|e| {
            CliError::general(format!("cannot determine the nvsn directory: {e:#}"))
                .with_hint("set NVSN_DIR or pass --dir <path>")
        })?;

        let stderr_tty = std::io::stderr().is_terminal();
        let no_color_env = std::env::var_os("NO_COLOR").is_some();
        let color = !global.no_color && !no_color_env && stderr_tty && !global.json;
        colored::control::set_override(color);

        // Precedence (ADR-036): --mirror > NVSN_NODEJS_ORG_MIRROR > config.toml > default.
        let flags = Overrides {
            node_mirror: global.mirror.clone(),
            proxy: None,
            arch: None,
        };
        let settings = load_settings(&root, &flags, &env).map_err(|e| {
            CliError::general(format!("{e:#}")).with_hint(format!(
                "fix or remove {} (or the NVSN_* variable it conflicts with)",
                nvsn_core::config::Config::path_in(&root).display()
            ))
        })?;

        Ok(Self {
            out: Output::new(global.json, global.quiet),
            store: Store::new(root),
            offline: global.offline,
            yes: global.yes,
            verbose: global.verbose,
            allow_http: global.allow_http,
            mirror: settings.node_mirror.trim_end_matches('/').to_owned(),
            proxy: settings.proxy,
            npm_mirror: settings.npm_mirror,
            arch_override: settings.arch,
            interactive: std::io::stdin().is_terminal() && !global.json,
            show_progress: !global.json
                && !global.quiet
                && !global.no_progress
                && !no_color_env
                && stderr_tty,
            integration: env.var(ENV_INTEGRATION).as_deref() == Some("1"),
        })
    }

    /// Host platform (`Platform::detect`).
    ///
    /// # Errors
    ///
    /// Returns an error if the operating system or architecture is not supported.
    pub fn platform(&self) -> Result<Platform, CliError> {
        let detected = Platform::detect().map_err(|e| {
            CliError::unsupported(format!("{e:#}")).with_hint(
                "nvsn supports Linux, macOS, Windows and Termux on x64, arm64, armv7l, x86, \
                 ppc64le, s390x and riscv64",
            )
        })?;
        // The override (NVSN_ARCH or config.toml) replaces the detected architecture.
        let arch = effective_arch(self.arch_override.as_deref(), detected.arch)
            .map_err(|e| CliError::usage(format!("{e:#}")))?;
        Ok(Platform { arch, ..detected })
    }

    /// HTTP client with the default retries, through the configured proxy if there is one.
    ///
    /// # Errors
    ///
    /// Returns an error if the TLS client cannot be built.
    pub fn http(&self) -> Result<HttpClient, CliError> {
        HttpClient::with_proxy(
            HttpOptions {
                verbose: self.verbose,
                retries: RETRIES,
                allow_http: self.allow_http,
            },
            self.proxy.as_deref(),
        )
        .map_err(|e| CliError::general(format!("cannot create the HTTP client: {e:#}")))
    }

    /// The user's home directory (`HOME` or `USERPROFILE`).
    pub fn home(&self) -> Option<PathBuf> {
        let env = ProcessEnv;
        env.var("HOME")
            .or_else(|| env.var("USERPROFILE"))
            .map(PathBuf::from)
    }

    /// Active version of the session (`NVSN_VERSION`), if it exists.
    pub fn active_version(&self) -> Option<String> {
        ProcessEnv.var(ENV_VERSION)
    }

    /// Creates the top-level directories of the store.
    ///
    /// # Errors
    ///
    /// Returns an error if `NVSN_DIR` cannot be written.
    pub fn ensure_store(&self) -> Result<(), CliError> {
        self.store.create_dirs().map_err(|e| {
            CliError::general(format!("{e:#}")).with_hint(format!(
                "check write permissions for {}",
                self.store.root().display()
            ))
        })
    }

    /// Remote index with cache (ETag and TTL). With `--offline` it does not touch the network.
    ///
    /// # Errors
    ///
    /// Returns a cache error without network (code 5) or a download error (code 7).
    pub fn load_index(&self) -> Result<Vec<RemoteRelease>, CliError> {
        let http = self.http()?;
        fetch_index(
            &http,
            &self.store,
            &self.mirror,
            DEFAULT_INDEX_TTL,
            self.offline,
        )
        .map_err(|e| index_error(e, self.offline))
    }

    /// Index from the cache, without network and without failing. `None` if there is no cache.
    pub fn cached_index(&self) -> Option<Vec<RemoteRelease>> {
        let http = self.http().ok()?;
        fetch_index(&http, &self.store, &self.mirror, DEFAULT_INDEX_TTL, true).ok()
    }
}

/// Operating system for the default value of `NVSN_DIR`.
fn detect_os() -> Os {
    match Platform::detect() {
        Ok(platform) => platform.os,
        Err(_) => match std::env::consts::OS {
            "windows" => Os::Windows,
            "macos" => Os::MacOs,
            "android" => Os::Android,
            _ => Os::Linux,
        },
    }
}

/// Classifies the failure of loading the index according to its real origin: the network
/// (code 7), an insecure URL (usage, 2) or a corrupt index or a local failure of the
/// cache (general, 1). The network goes first: its errors also contain
/// `io::Error`, and a dead mirror must not look like a disk failure.
fn index_error(err: anyhow::Error, offline: bool) -> CliError {
    if offline {
        return CliError::new(ErrorKind::OfflineNoCache, format!("{err:#}"))
            .with_hint("run once without --offline to download the index");
    }
    let text = format!("{err:#}");
    let chain = || err.chain();
    if chain().any(|cause| cause.downcast_ref::<ureq::Error>().is_some()) {
        return CliError::new(ErrorKind::DownloadFailed, text).with_hint(format!(
            "check your connection, or set --mirror / {ENV_MIRROR}; retry with -v for details"
        ));
    }
    if text.contains("insecure URL rejected") {
        return CliError::new(ErrorKind::Usage, text)
            .with_hint("use an https:// mirror, or pass --allow-http for a local test server");
    }
    if chain().any(|cause| cause.downcast_ref::<serde_json::Error>().is_some()) {
        return CliError::new(ErrorKind::General, text).with_hint(format!(
            "the mirror returned an invalid release index; check --mirror / {ENV_MIRROR}"
        ));
    }
    if chain().any(|cause| cause.downcast_ref::<std::io::Error>().is_some()) {
        return CliError::new(ErrorKind::General, text).with_hint(
            "the local cache could not be written; check NVSN_DIR permissions and free space",
        );
    }
    CliError::new(ErrorKind::DownloadFailed, text).with_hint(format!(
        "check your connection, or set --mirror / {ENV_MIRROR}; retry with -v for details"
    ))
}

#[cfg(test)]
mod tests {
    use crate::cli::Cli;
    use clap::Parser;

    /// Context built from an argument line, with the store in `dir`.
    fn ctx_from(dir: &std::path::Path, extra: &[&str]) -> super::Ctx {
        let dir_arg = dir.display().to_string();
        let mut args = vec!["nvsn", "--dir", dir_arg.as_str()];
        args.extend_from_slice(extra);
        args.push("list");
        let cli = Cli::try_parse_from(args).expect("valid arguments");
        super::Ctx::new(&cli.global).expect("context")
    }

    #[test]
    fn no_progress_flag_disables_progress() {
        let dir = tempfile::tempdir().expect("temporary directory");
        assert!(!ctx_from(dir.path(), &["--no-progress"]).show_progress);
    }

    #[test]
    fn json_and_quiet_disable_progress() {
        let dir = tempfile::tempdir().expect("temporary directory");
        assert!(!ctx_from(dir.path(), &["--json"]).show_progress);
        assert!(!ctx_from(dir.path(), &["--quiet"]).show_progress);
    }
}
