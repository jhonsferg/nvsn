//! `install-latest-npm`, `reinstall-packages` and their `install` options.
//!
//! Both run through nvsn-core: the npm of the target version runs with that version's
//! node, and the global prefix stays inside the version directory.

use crate::ctx::Ctx;
use crate::exit::CliError;
use crate::versions::{effective_version, resolve_installed, Installed};
use nvsn_core::packages::{
    install_latest_npm as core_install_latest_npm, reinstall_packages as core_reinstall_packages,
};
use serde_json::json;

/// Installs the latest npm into `version`, or into the active version (or the default).
///
/// # Errors
///
/// `version-not-found` (6) if the version is not installed; `usage` (2) without a version
/// and without an active one; general error (1) if npm fails.
pub fn install_latest_npm(ctx: &Ctx, version: Option<&str>) -> Result<(), CliError> {
    let inst = target(ctx, version, "install-latest-npm")?;
    upgrade_npm(ctx, &inst.tag)?;
    ctx.out.result(
        "install-latest-npm",
        &format!("Latest npm installed into {}", inst.tag),
        json!({ "version": inst.tag }),
    );
    Ok(())
}

/// Reinstalls the global packages of `version` from `from` (or from the active version).
///
/// # Errors
///
/// `version-not-found` (6) if a version is not installed; `usage` (2) without a source;
/// general error (1) if npm fails.
pub fn reinstall_packages(ctx: &Ctx, version: &str, from: Option<&str>) -> Result<(), CliError> {
    let to = resolve_installed(ctx, version)?;
    let source = target(ctx, from, "reinstall-packages")?;
    let names = copy_packages(ctx, &source.tag, &to.tag)?;
    let human = if names.is_empty() {
        format!(
            "{} has no global packages to copy from {}",
            to.tag, source.tag
        )
    } else {
        format!(
            "Reinstalled {} global package(s) from {} into {}: {}",
            names.len(),
            source.tag,
            to.tag,
            names.join(", ")
        )
    };
    ctx.out.result(
        "reinstall-packages",
        &human,
        json!({ "from": source.tag, "to": to.tag, "packages": names }),
    );
    Ok(())
}

/// Upgrades the npm of an installed version (also used by `install --latest-npm`).
///
/// # Errors
///
/// General error (1) with the npm output tail if npm fails.
pub fn upgrade_npm(ctx: &Ctx, tag: &str) -> Result<(), CliError> {
    ctx.out
        .note(format!("Installing the latest npm into {tag}..."));
    core_install_latest_npm(&ctx.store, tag).map_err(npm_error)
}

/// Installs the global packages of `from` into `to` (also used by `install --reinstall-packages-from`).
///
/// # Errors
///
/// General error (1) with the npm output tail if npm fails.
pub fn copy_packages(ctx: &Ctx, from: &str, to: &str) -> Result<Vec<String>, CliError> {
    ctx.out.note(format!(
        "Reinstalling the global packages of {from} into {to}..."
    ));
    core_reinstall_packages(&ctx.store, from, to).map_err(npm_error)
}

fn npm_error(err: anyhow::Error) -> CliError {
    CliError::general(format!("{err:#}"))
        .with_hint("check your connection and the npm registry, then retry")
}

/// Installed version named by `input`, or the active one (or the default) without it.
fn target(ctx: &Ctx, input: Option<&str>, command: &str) -> Result<Installed, CliError> {
    match input {
        Some(text) => resolve_installed(ctx, text),
        None => {
            let (tag, _) = effective_version(ctx).ok_or_else(|| {
                CliError::usage(format!(
                    "{command} needs a version: no version is active and no default is set"
                ))
                .with_hint(format!("pass one, e.g. `nvsn {command} 20`"))
            })?;
            resolve_installed(ctx, &tag)
        }
    }
}
