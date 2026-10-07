//! Settings: `arch`, `proxy`, `node-mirror`, `npm-mirror`, `root` and `set-colors`.
//!
//! Values are stored in `<NVSN_DIR>/config.toml` through nvsn-core. The environment
//! takes precedence over the file (ADR-036), and the commands say so when it applies.
//!
//! `root` is the exception: `config.toml` lives inside the root, so it cannot tell
//! which root to use. The root is chosen with `NVSN_DIR` or `--dir` only.

use crate::ctx::Ctx;
use crate::exit::CliError;
use crate::versions::effective_version;
use nvsn_core::artifact::artifact_for_release;
use nvsn_core::config::Config;
use nvsn_core::install::DEFAULT_MIRROR;
use nvsn_platform::env::{EnvSource, ProcessEnv};
use nvsn_platform::{effective_arch, parse_arch_override, Arch, Platform};
use serde_json::json;
use std::path::{Path, PathBuf};

/// Registry that npm uses when nothing else is configured.
const NPM_DEFAULT_REGISTRY: &str = "https://registry.npmjs.org/";

/// Variables that the HTTP client reads for its proxy, in priority order.
const PROXY_VARS: [&str; 4] = ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy"];

/// Variables that take precedence over `node_mirror` in `config.toml`.
const MIRROR_VARS: [&str; 2] = ["NVSN_NODEJS_ORG_MIRROR", "NVM_NODEJS_ORG_MIRROR"];

/// Shows the architecture in use, or stores an override in `config.toml`.
///
/// `none` removes the stored override. `NVSN_ARCH`, when set, still takes precedence.
///
/// # Errors
///
/// `usage` (2) for an unknown architecture; general error if `config.toml` cannot be written.
pub fn arch(ctx: &Ctx, value: Option<&str>) -> Result<(), CliError> {
    let Some(text) = value else {
        let effective = ctx.platform()?.arch;
        let source = if ctx.arch_override.is_some() {
            "override"
        } else {
            "detected"
        };
        ctx.out.result(
            "arch",
            effective.dist_name(),
            json!({ "arch": effective.dist_name(), "source": source }),
        );
        return Ok(());
    };

    let (path, mut config) = load(ctx)?;
    set_key(&mut config, "arch", text)?;
    save(&config, &path)?;

    let host = Platform::detect().map_err(|err| CliError::unsupported(format!("{err:#}")))?;
    let env_override = ProcessEnv
        .var("NVSN_ARCH")
        .filter(|value| !value.trim().is_empty());
    if env_override.is_some() {
        ctx.out
            .note("note: NVSN_ARCH is set and takes precedence over config.toml");
    }
    let override_value = env_override.or_else(|| config.arch.clone());
    let effective = effective_arch(override_value.as_deref(), host.arch)
        .map_err(|err| CliError::usage(format!("{err:#}")))?;

    if let Some(warning) = missing_binary_warning(
        ctx,
        Platform {
            arch: effective,
            ..host
        },
    ) {
        ctx.out.note(format!("warning: {warning}"));
    }
    let human = match &config.arch {
        Some(stored) => format!("Architecture set to {stored} in {}", path.display()),
        None => format!(
            "Architecture override removed; using {}",
            host.arch.dist_name()
        ),
    };
    ctx.out.result(
        "arch",
        &human,
        json!({ "arch": effective.dist_name(), "stored": config.arch }),
    );
    Ok(())
}

/// `use x64|arm64|32|64` (and the other architecture names): the same as `arch <value>`.
///
/// # Errors
///
/// Same as [`arch`].
pub fn use_arch(ctx: &Ctx, text: &str) -> Result<(), CliError> {
    arch(ctx, Some(text))
}

/// Architecture named by a word of the command line. `32` is x86 and `64` is x64.
pub fn arch_word(text: &str) -> Option<Arch> {
    parse_arch_override(text).ok()
}

/// Warning when the effective version has no binary for the platform. Only checked when
/// the index is in the cache; without it there is nothing to compare with.
fn missing_binary_warning(ctx: &Ctx, platform: Platform) -> Option<String> {
    let (tag, _) = effective_version(ctx)?;
    let index = ctx.cached_index()?;
    let release = index.iter().find(|release| release.tag() == tag)?;
    artifact_for_release(release, platform).err().map(|err| {
        format!(
            "{tag} has no binary for {}: {err:#}",
            platform.arch.dist_name()
        )
    })
}

/// Shows the proxy in use, or stores one (`none` removes it).
///
/// Without a stored proxy, the HTTP client reads `HTTPS_PROXY` or `HTTP_PROXY` itself.
///
/// # Errors
///
/// `usage` (2) for an invalid URL; general error if `config.toml` cannot be written.
pub fn proxy(ctx: &Ctx, value: Option<&str>) -> Result<(), CliError> {
    let Some(text) = value else {
        if let Some(url) = &ctx.proxy {
            let shown = redact_credentials(url);
            ctx.out.result(
                "proxy",
                &shown,
                json!({ "proxy": shown, "source": "config" }),
            );
            return Ok(());
        }
        let found = PROXY_VARS
            .iter()
            .find_map(|name| ProcessEnv.var(name).map(|url| ((*name).to_owned(), url)));
        match found {
            Some((source, url)) => {
                let shown = redact_credentials(&url);
                ctx.out
                    .result("proxy", &shown, json!({ "proxy": shown, "source": source }));
            }
            None => ctx
                .out
                .result("proxy", "none", json!({ "proxy": null, "source": null })),
        }
        return Ok(());
    };

    let (path, mut config) = load(ctx)?;
    set_key(&mut config, "proxy", text)?;
    save(&config, &path)?;
    let human = match &config.proxy {
        Some(url) => format!("Proxy set to {}", redact_credentials(url)),
        None => "Proxy removed; HTTPS_PROXY and HTTP_PROXY apply again".to_owned(),
    };
    ctx.out.result(
        "proxy",
        &human,
        json!({ "proxy": config.proxy.as_deref().map(redact_credentials) }),
    );
    Ok(())
}

/// Hides the user and password of a URL, so that a proxy can be shown in logs.
pub fn redact_credentials(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_owned();
    };
    match rest.rsplit_once('@') {
        Some((_, host)) => format!("{scheme}://***@{host}"),
        None => url.to_owned(),
    }
}

/// Shows the Node.js distribution mirror in use, or stores one (`none` restores the default).
///
/// # Errors
///
/// `usage` (2) for an invalid URL; general error if `config.toml` cannot be written.
pub fn node_mirror(ctx: &Ctx, url: Option<&str>) -> Result<(), CliError> {
    let Some(text) = url else {
        ctx.out
            .result("node-mirror", &ctx.mirror, json!({ "mirror": ctx.mirror }));
        return Ok(());
    };

    let (path, mut config) = load(ctx)?;
    set_key(&mut config, "node_mirror", text)?;
    save(&config, &path)?;
    warn_if_env_overrides(ctx, &MIRROR_VARS);
    let stored = config
        .node_mirror
        .clone()
        .unwrap_or_else(|| DEFAULT_MIRROR.to_owned());
    ctx.out.result(
        "node-mirror",
        &format!("Node mirror set to {stored}"),
        json!({ "mirror": stored }),
    );
    Ok(())
}

/// Shows the npm registry in use, or stores one (`none` restores npm's default).
///
/// # Errors
///
/// `usage` (2) for an invalid URL; general error if `config.toml` cannot be written.
pub fn npm_mirror(ctx: &Ctx, url: Option<&str>) -> Result<(), CliError> {
    let Some(text) = url else {
        let shown = ctx
            .npm_mirror
            .clone()
            .unwrap_or_else(|| NPM_DEFAULT_REGISTRY.to_owned());
        let source = if ctx.npm_mirror.is_some() {
            "config"
        } else {
            "default"
        };
        ctx.out.result(
            "npm-mirror",
            &shown,
            json!({ "registry": shown, "source": source }),
        );
        return Ok(());
    };

    let (path, mut config) = load(ctx)?;
    set_key(&mut config, "npm_mirror", text)?;
    save(&config, &path)?;
    let stored = config
        .npm_mirror
        .clone()
        .unwrap_or_else(|| NPM_DEFAULT_REGISTRY.to_owned());
    ctx.out.result(
        "npm-mirror",
        &format!("npm registry set to {stored}"),
        json!({ "registry": stored }),
    );
    Ok(())
}

/// Shows the root directory (`NVSN_DIR`).
///
/// A stored root is not supported: `config.toml` lives inside the root, so it cannot say
/// which root to open. Set `NVSN_DIR` (or `--dir`) in the shell profile instead.
///
/// # Errors
///
/// `usage` (2) when a path is given.
pub fn root(ctx: &Ctx, path: Option<&Path>) -> Result<(), CliError> {
    if let Some(path) = path {
        return Err(CliError::usage(
            "the root cannot be stored in config.toml: that file lives inside the root",
        )
        .with_hint(format!(
            "export NVSN_DIR='{}' in your shell profile (or pass --dir); no data is moved",
            path.display()
        )));
    }
    let shown = ctx.store.root().display().to_string();
    ctx.out.result("root", &shown, json!({ "root": shown }));
    Ok(())
}

/// `set-colors` is not supported: colors follow `NO_COLOR` and `--no-color`.
///
/// # Errors
///
/// Always `unsupported` (10).
pub fn set_colors() -> Result<(), CliError> {
    Err(
        CliError::unsupported("set-colors is not supported: nvsn does not customize colors")
            .with_hint("disable colors with NO_COLOR=1 or --no-color"),
    )
}

/// Reads `config.toml` of the root.
fn load(ctx: &Ctx) -> Result<(PathBuf, Config), CliError> {
    let path = Config::path_in(ctx.store.root());
    let config = Config::read(&path).map_err(|err| {
        CliError::general(format!("{err:#}")).with_hint(format!("fix or remove {}", path.display()))
    })?;
    Ok((path, config))
}

/// Applies one key to `config`. Invalid values are usage errors.
fn set_key(config: &mut Config, key: &str, value: &str) -> Result<(), CliError> {
    config.set(key, value).map_err(|err| {
        CliError::usage(format!("{err:#}"))
            .with_hint(format!("`{key}` takes a value or `none` to remove it"))
    })
}

/// Writes `config.toml` atomically.
fn save(config: &Config, path: &Path) -> Result<(), CliError> {
    config.write(path).map_err(|err| {
        CliError::general(format!("{err:#}"))
            .with_hint(format!("check write permissions for {}", path.display()))
    })
}

/// Notes that an environment variable takes precedence over the value just stored.
fn warn_if_env_overrides(ctx: &Ctx, names: &[&str]) {
    if let Some(name) = names.iter().find(|name| ProcessEnv.var(name).is_some()) {
        ctx.out.note(format!(
            "note: {name} is set and takes precedence over config.toml"
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn architecture_words_map_to_node_names() {
        assert_eq!(arch_word("x64"), Some(Arch::X64));
        assert_eq!(arch_word("64"), Some(Arch::X64));
        assert_eq!(arch_word("32"), Some(Arch::X86));
        assert_eq!(arch_word("ARM64"), Some(Arch::Arm64));
        assert_eq!(arch_word("armv7l"), Some(Arch::Armv7l));
        assert_eq!(arch_word("mips"), None);
        assert_eq!(arch_word("20"), None);
    }

    #[test]
    fn proxy_credentials_are_hidden() {
        assert_eq!(
            redact_credentials("http://user:secret@proxy.local:3128"),
            "http://***@proxy.local:3128"
        );
        assert_eq!(
            redact_credentials("http://proxy.local:3128"),
            "http://proxy.local:3128"
        );
    }
}
