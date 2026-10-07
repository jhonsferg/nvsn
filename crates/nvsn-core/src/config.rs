//! Persistent configuration (`<NVSN_DIR>/config.toml`) and layered settings (ADR-036).
//!
//! Precedence, highest first: command-line flags, environment variables,
//! `config.toml`, built-in defaults.
//!
//! Keys in `config.toml`: `node_mirror`, `npm_mirror`, `proxy`, `arch`.
//! Environment variables: `NVSN_NODEJS_ORG_MIRROR` and, for nvm compatibility,
//! `NVM_NODEJS_ORG_MIRROR` (both override `node_mirror`, the first one wins);
//! `NVSN_ARCH` (overrides `arch`). The proxy has no variable of its own: when
//! `proxy` is not set, ureq reads `HTTPS_PROXY` and `HTTP_PROXY` itself.

use crate::install::DEFAULT_MIRROR;
use anyhow::{bail, Context, Result};
use nvsn_net::validate_proxy;
use nvsn_platform::{parse_arch_override, EnvSource};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

/// File name of the persistent configuration, inside the root directory.
pub const CONFIG_FILE_NAME: &str = "config.toml";

/// Keys accepted by [`Config::set`] and stored in `config.toml`.
pub const CONFIG_KEYS: [&str; 4] = ["node_mirror", "npm_mirror", "proxy", "arch"];

/// Settings stored in `config.toml`. Unset keys are `None` and are not written.
///
/// Unknown keys are rejected on read, so a misspelled key fails loudly
/// instead of being silently ignored.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Base URL of the Node distribution mirror, without trailing slash.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_mirror: Option<String>,
    /// Base URL of the npm registry mirror, without trailing slash.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub npm_mirror: Option<String>,
    /// HTTP(S) proxy URL. Credentials, if any, are stored in plain text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy: Option<String>,
    /// Architecture override, in the names accepted by `parse_arch_override`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arch: Option<String>,
}

impl Config {
    /// Path of `config.toml` inside the nvsn root directory `nvsn_dir`.
    pub fn path_in(nvsn_dir: &Path) -> PathBuf {
        nvsn_dir.join(CONFIG_FILE_NAME)
    }

    /// Reads and validates `path`. A missing file yields the empty configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read, is not valid TOML, has
    /// unknown keys, or holds a value that fails validation.
    pub fn read(path: &Path) -> Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(err) => {
                return Err(err).with_context(|| format!("failed to read {}", path.display()));
            }
        };
        let config: Self =
            toml::from_str(&text).with_context(|| format!("invalid {}", path.display()))?;
        config
            .validate()
            .with_context(|| format!("invalid {}", path.display()))?;
        Ok(config)
    }

    /// Writes the configuration to `path` atomically: a temporary file in the
    /// same directory is written, synced and renamed over the target. A reader
    /// sees either the old file or the new one, never a partial write.
    ///
    /// The parent directory is created if it does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration is invalid or the file cannot be written.
    pub fn write(&self, path: &Path) -> Result<()> {
        self.validate()?;
        let dir = path
            .parent()
            .context("the configuration path has no parent directory")?;
        std::fs::create_dir_all(dir)
            .with_context(|| format!("failed to create {}", dir.display()))?;

        let text = toml::to_string_pretty(self).context("failed to serialize the configuration")?;
        let mut tmp = tempfile::NamedTempFile::new_in(dir)
            .with_context(|| format!("failed to create a temporary file in {}", dir.display()))?;
        tmp.write_all(text.as_bytes())
            .and_then(|()| tmp.as_file().sync_all())
            .context("failed to write the temporary configuration file")?;
        tmp.persist(path)
            .map_err(|err| err.error)
            .with_context(|| format!("failed to replace {}", path.display()))?;
        Ok(())
    }

    /// Sets one key. An empty value or `none` removes the key.
    ///
    /// `node_mirror` and `npm_mirror` are checked and stored without a trailing
    /// slash. `proxy` must be an `http://` or `https://` URL. `arch` must be an
    /// architecture name accepted by `parse_arch_override`.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown key or an invalid value. The
    /// configuration is left unchanged on error.
    pub fn set(&mut self, key: &str, value: &str) -> Result<()> {
        let value = value.trim();
        let removing = value.is_empty() || value.eq_ignore_ascii_case("none");
        match key {
            "node_mirror" => {
                self.node_mirror = optional(removing, || normalize_mirror(value))?;
            }
            "npm_mirror" => {
                self.npm_mirror = optional(removing, || normalize_mirror(value))?;
            }
            "proxy" => {
                self.proxy = parse_proxy_setting(value)?;
            }
            "arch" => {
                self.arch = optional(removing, || {
                    parse_arch_override(value)?;
                    Ok(value.to_ascii_lowercase())
                })?;
            }
            other => bail!(
                "unknown configuration key '{other}'; valid keys are {}",
                CONFIG_KEYS.join(", ")
            ),
        }
        Ok(())
    }

    /// Checks every stored value with the same rules as [`Config::set`].
    fn validate(&self) -> Result<()> {
        if let Some(mirror) = &self.node_mirror {
            normalize_mirror(mirror).context("node_mirror")?;
        }
        if let Some(mirror) = &self.npm_mirror {
            normalize_mirror(mirror).context("npm_mirror")?;
        }
        if let Some(proxy) = &self.proxy {
            parse_proxy_setting(proxy).context("proxy")?;
        }
        if let Some(arch) = &self.arch {
            parse_arch_override(arch).context("arch")?;
        }
        Ok(())
    }
}

/// Returns `None` when `removing`, otherwise the result of `make`.
fn optional(removing: bool, make: impl FnOnce() -> Result<String>) -> Result<Option<String>> {
    if removing {
        Ok(None)
    } else {
        make().map(Some)
    }
}

/// Validates a mirror base URL and removes trailing slashes.
///
/// # Errors
///
/// Returns an error unless the URL uses `http://` or `https://` and has a host.
pub fn normalize_mirror(value: &str) -> Result<String> {
    let trimmed = value.trim().trim_end_matches('/');
    let Some((scheme, rest)) = trimmed.split_once("://") else {
        bail!("invalid mirror URL; expected http:// or https:// followed by host and path");
    };
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        bail!("unsupported mirror scheme; use http:// or https://");
    }
    let host = rest.split('/').next().unwrap_or("");
    if host.is_empty() {
        bail!("invalid mirror URL; the host is missing");
    }
    Ok(trimmed.to_string())
}

/// Parses a proxy value. `none` or an empty value means no proxy (`Ok(None)`);
/// anything else must pass [`nvsn_net::validate_proxy`].
///
/// # Errors
///
/// Returns an error if the value is neither `none` nor a valid http(s) proxy URL.
pub fn parse_proxy_setting(value: &str) -> Result<Option<String>> {
    let value = value.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("none") {
        return Ok(None);
    }
    validate_proxy(value)?;
    Ok(Some(value.to_string()))
}

/// Command-line flags that override every other layer. `None` means the flag was not given.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Overrides {
    /// `--mirror` value.
    pub node_mirror: Option<String>,
    /// `--proxy` value. `none` disables the proxy stored in `config.toml`.
    pub proxy: Option<String>,
    /// `--arch` value.
    pub arch: Option<String>,
}

/// Settings after all layers are applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// Node distribution base URL, without trailing slash. Never empty.
    pub node_mirror: String,
    /// npm registry mirror, from `config.toml` only.
    pub npm_mirror: Option<String>,
    /// Proxy URL, or `None` to let ureq read `HTTPS_PROXY` and `HTTP_PROXY`.
    pub proxy: Option<String>,
    /// Architecture override, already validated. Resolve it with
    /// `nvsn_platform::effective_arch` against the detected architecture.
    pub arch: Option<String>,
}

/// Loads `nvsn_dir/config.toml` and combines it with the environment and the flags.
///
/// # Errors
///
/// Returns an error if the file is invalid or any resolved value is invalid.
pub fn load_settings(nvsn_dir: &Path, flags: &Overrides, env: &dyn EnvSource) -> Result<Settings> {
    let file = Config::read(&Config::path_in(nvsn_dir))?;
    resolve_settings(flags, env, &file)
}

/// Applies the precedence rules to an already read configuration. It does no I/O.
///
/// # Errors
///
/// Returns an error if a resolved mirror, proxy or architecture is invalid.
pub fn resolve_settings(flags: &Overrides, env: &dyn EnvSource, file: &Config) -> Result<Settings> {
    let mirror_raw = flags
        .node_mirror
        .clone()
        .or_else(|| env.var("NVSN_NODEJS_ORG_MIRROR"))
        .or_else(|| env.var("NVM_NODEJS_ORG_MIRROR"))
        .or_else(|| file.node_mirror.clone());
    let node_mirror = match mirror_raw {
        Some(value) => normalize_mirror(&value).context("node mirror")?,
        None => DEFAULT_MIRROR.to_string(),
    };

    let proxy = match &flags.proxy {
        Some(value) => parse_proxy_setting(value).context("--proxy")?,
        None => file.proxy.clone(),
    };

    let arch = flags
        .arch
        .clone()
        .or_else(|| env.var("NVSN_ARCH"))
        .or_else(|| file.arch.clone());
    if let Some(value) = &arch {
        parse_arch_override(value).context("architecture override")?;
    }

    Ok(Settings {
        node_mirror,
        npm_mirror: file.npm_mirror.clone(),
        proxy,
        arch,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// Environment with a fixed set of variables, for tests.
    #[derive(Default)]
    struct TestEnv(BTreeMap<String, String>);

    impl TestEnv {
        fn with(pairs: &[(&str, &str)]) -> Self {
            Self(
                pairs
                    .iter()
                    .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                    .collect(),
            )
        }
    }

    impl EnvSource for TestEnv {
        fn var(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
    }

    fn file_with(text: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CONFIG_FILE_NAME);
        std::fs::write(&path, text).unwrap();
        (dir, path)
    }

    #[test]
    fn defaults_apply_without_file_or_env() {
        let dir = tempfile::tempdir().unwrap();
        let settings =
            load_settings(dir.path(), &Overrides::default(), &TestEnv::default()).unwrap();
        assert_eq!(settings.node_mirror, DEFAULT_MIRROR);
        assert_eq!(settings.npm_mirror, None);
        assert_eq!(settings.proxy, None);
        assert_eq!(settings.arch, None);
    }

    #[test]
    fn file_values_apply_when_no_flag_or_env_is_set() {
        let (_dir, path) = file_with(
            "node_mirror = \"https://mirror.example/dist/\"\n\
             npm_mirror = \"https://npm.example\"\n\
             proxy = \"http://proxy.example:3128\"\n\
             arch = \"arm64\"\n",
        );
        let file = Config::read(&path).unwrap();
        let settings = resolve_settings(&Overrides::default(), &TestEnv::default(), &file).unwrap();
        assert_eq!(settings.node_mirror, "https://mirror.example/dist");
        assert_eq!(settings.npm_mirror.as_deref(), Some("https://npm.example"));
        assert_eq!(settings.proxy.as_deref(), Some("http://proxy.example:3128"));
        assert_eq!(settings.arch.as_deref(), Some("arm64"));
    }

    #[test]
    fn env_overrides_file_for_mirror_and_arch() {
        let file = Config {
            node_mirror: Some("https://file.example".into()),
            arch: Some("x64".into()),
            ..Config::default()
        };
        let env = TestEnv::with(&[
            ("NVSN_NODEJS_ORG_MIRROR", "https://env.example/dist"),
            ("NVSN_ARCH", "arm64"),
        ]);
        let settings = resolve_settings(&Overrides::default(), &env, &file).unwrap();
        assert_eq!(settings.node_mirror, "https://env.example/dist");
        assert_eq!(settings.arch.as_deref(), Some("arm64"));
    }

    #[test]
    fn nvsn_mirror_variable_wins_over_nvm_compatibility_variable() {
        let env = TestEnv::with(&[
            ("NVM_NODEJS_ORG_MIRROR", "https://nvm.example/dist"),
            ("NVSN_NODEJS_ORG_MIRROR", "https://nvsn.example/dist"),
        ]);
        let settings = resolve_settings(&Overrides::default(), &env, &Config::default()).unwrap();
        assert_eq!(settings.node_mirror, "https://nvsn.example/dist");

        let env = TestEnv::with(&[("NVM_NODEJS_ORG_MIRROR", "https://nvm.example/dist/")]);
        let settings = resolve_settings(&Overrides::default(), &env, &Config::default()).unwrap();
        assert_eq!(settings.node_mirror, "https://nvm.example/dist");
    }

    #[test]
    fn flags_override_env_and_file() {
        let file = Config {
            node_mirror: Some("https://file.example".into()),
            proxy: Some("http://file-proxy:1".into()),
            arch: Some("x64".into()),
            ..Config::default()
        };
        let env = TestEnv::with(&[
            ("NVSN_NODEJS_ORG_MIRROR", "https://env.example"),
            ("NVSN_ARCH", "arm64"),
        ]);
        let flags = Overrides {
            node_mirror: Some("https://flag.example".into()),
            proxy: Some("https://flag-proxy:2".into()),
            arch: Some("x86".into()),
        };
        let settings = resolve_settings(&flags, &env, &file).unwrap();
        assert_eq!(settings.node_mirror, "https://flag.example");
        assert_eq!(settings.proxy.as_deref(), Some("https://flag-proxy:2"));
        assert_eq!(settings.arch.as_deref(), Some("x86"));
    }

    #[test]
    fn proxy_none_flag_disables_the_stored_proxy() {
        let file = Config {
            proxy: Some("http://file-proxy:1".into()),
            ..Config::default()
        };
        let flags = Overrides {
            proxy: Some("none".into()),
            ..Overrides::default()
        };
        let settings = resolve_settings(&flags, &TestEnv::default(), &file).unwrap();
        assert_eq!(settings.proxy, None);
    }

    #[test]
    fn invalid_values_fail_resolution() {
        let env = TestEnv::with(&[("NVSN_ARCH", "sparc")]);
        assert!(resolve_settings(&Overrides::default(), &env, &Config::default()).is_err());

        let flags = Overrides {
            proxy: Some("socks5://proxy:1080".into()),
            ..Overrides::default()
        };
        assert!(resolve_settings(&flags, &TestEnv::default(), &Config::default()).is_err());

        let flags = Overrides {
            node_mirror: Some("ftp://mirror.example".into()),
            ..Overrides::default()
        };
        assert!(resolve_settings(&flags, &TestEnv::default(), &Config::default()).is_err());
    }

    #[test]
    fn missing_file_reads_as_empty_configuration() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::read(&Config::path_in(dir.path())).unwrap();
        assert_eq!(config, Config::default());
    }

    #[test]
    fn unknown_key_is_rejected_with_the_file_name() {
        let (_dir, path) = file_with("node_mirrr = \"https://x.example\"\n");
        let err = format!("{:#}", Config::read(&path).unwrap_err());
        assert!(err.contains("config.toml"));
        assert!(err.contains("node_mirrr"));
    }

    #[test]
    fn invalid_values_in_file_are_rejected() {
        let (_dir, path) = file_with("proxy = \"socks5://proxy:1080\"\n");
        assert!(Config::read(&path).is_err());

        let (_dir, path) = file_with("arch = \"mips\"\n");
        assert!(Config::read(&path).is_err());

        let (_dir, path) = file_with("npm_mirror = \"not a url\"\n");
        assert!(Config::read(&path).is_err());
    }

    #[test]
    fn set_normalizes_mirrors_and_removes_keys_with_none() {
        let mut config = Config::default();
        config
            .set("node_mirror", "https://mirror.example/dist//")
            .unwrap();
        assert_eq!(
            config.node_mirror.as_deref(),
            Some("https://mirror.example/dist")
        );

        config.set("node_mirror", "none").unwrap();
        assert_eq!(config.node_mirror, None);

        config.set("npm_mirror", "https://npm.example/").unwrap();
        config.set("npm_mirror", "").unwrap();
        assert_eq!(config.npm_mirror, None);
    }

    #[test]
    fn set_validates_proxy_and_arch() {
        let mut config = Config::default();
        config.set("proxy", "http://proxy.example:3128").unwrap();
        assert!(config.set("proxy", "socks5://proxy:1080").is_err());
        assert_eq!(config.proxy.as_deref(), Some("http://proxy.example:3128"));

        config.set("proxy", "none").unwrap();
        assert_eq!(config.proxy, None);

        config.set("arch", "ARM64").unwrap();
        assert_eq!(config.arch.as_deref(), Some("arm64"));
        assert!(config.set("arch", "mips").is_err());
        config.set("arch", "none").unwrap();
        assert_eq!(config.arch, None);
    }

    #[test]
    fn set_rejects_unknown_keys() {
        let mut config = Config::default();
        let err = config.set("root", "/tmp").unwrap_err().to_string();
        assert!(err.contains("root"));
        assert!(err.contains("node_mirror"));
    }

    #[test]
    fn write_then_read_round_trips_and_omits_unset_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = Config::path_in(&dir.path().join("nested"));
        let mut config = Config::default();
        config.set("node_mirror", "https://mirror.example").unwrap();
        config.set("arch", "x86").unwrap();

        config.write(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("node_mirror"));
        assert!(!text.contains("proxy"));
        assert_eq!(Config::read(&path).unwrap(), config);
    }

    #[test]
    fn write_replaces_an_existing_file_and_leaves_no_temporary_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = Config::path_in(dir.path());

        let mut first = Config::default();
        first.set("arch", "x64").unwrap();
        first.write(&path).unwrap();

        let mut second = Config::default();
        second.set("proxy", "http://proxy.example").unwrap();
        second.write(&path).unwrap();

        assert_eq!(Config::read(&path).unwrap(), second);
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec![CONFIG_FILE_NAME.to_string()]);
    }

    #[test]
    fn write_refuses_invalid_configuration() {
        let dir = tempfile::tempdir().unwrap();
        let path = Config::path_in(dir.path());
        let config = Config {
            proxy: Some("ftp://proxy".into()),
            ..Config::default()
        };
        assert!(config.write(&path).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn normalize_mirror_requires_http_scheme_and_host() {
        assert_eq!(
            normalize_mirror(" https://a.example/x/ ").unwrap(),
            "https://a.example/x"
        );
        assert!(normalize_mirror("nodejs.org/dist").is_err());
        assert!(normalize_mirror("file:///tmp/dist").is_err());
        assert!(normalize_mirror("https://").is_err());
    }
}
