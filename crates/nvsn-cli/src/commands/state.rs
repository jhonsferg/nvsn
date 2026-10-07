//! Local state: `current`, `default`, `alias`, `unalias`, `which` and `version`.

use crate::ctx::Ctx;
use crate::exit::CliError;
use crate::versions::{
    effective_version, latest_installed, list_aliases, node_path, parse_input, remove_alias,
    resolve_installed, validate_alias_name, write_alias, write_default, Installed, DYNAMIC_DEFAULT,
};
use nvsn_core::spec::VersionSpec;
use serde_json::json;

/// Shows the active version of the session, or `none`.
///
/// # Errors
///
/// Never fails; always returns Ok.
pub fn current(ctx: &Ctx) -> Result<(), CliError> {
    let effective = effective_version(ctx);
    let human = effective
        .as_ref()
        .map_or_else(|| "none".to_owned(), |(tag, _)| tag.clone());
    let source = effective.as_ref().map(|(_, source)| *source);
    ctx.out.result(
        "current",
        &human,
        json!({ "version": effective.as_ref().map(|(tag, _)| tag), "source": source }),
    );
    Ok(())
}

/// Sets the default version. It must be installed.
///
/// `node` (or `latest`, `newest`) makes the default follow the newest installed
/// stable version: the `default` file stores [`DYNAMIC_DEFAULT`] and every read resolves it.
///
/// It first moves the `<NVSN_DIR>/current` link (ADR-034) and then writes the
/// `default` file: if the link fails, the previous value remains in force.
///
/// # Errors
///
/// `version-not-found` (6) if no installed version matches, `usage` (2) if the spec is not valid.
pub fn set_default(ctx: &Ctx, input: &str) -> Result<(), CliError> {
    if matches!(parse_input(input)?, VersionSpec::Latest) {
        return set_default_latest(ctx);
    }
    let inst = resolve_installed(ctx, input)?;
    sync_current_link(ctx, &inst)?;
    write_default(ctx, &inst.tag)?;
    ctx.out.result(
        "default",
        &format!("Default version set to {}", inst.tag),
        json!({ "default": inst.tag }),
    );
    Ok(())
}

/// `default node`: stores the dynamic marker and points the link to the newest installed version.
fn set_default_latest(ctx: &Ctx) -> Result<(), CliError> {
    let inst = latest_installed(&ctx.store).ok_or_else(|| {
        CliError::not_found("no Node.js version is installed")
            .with_hint("install one first, e.g. `nvsn install 20`")
    })?;
    sync_current_link(ctx, &inst)?;
    write_default(ctx, DYNAMIC_DEFAULT)?;
    ctx.out.result(
        "default",
        &format!(
            "Default version set to node (currently {}, the newest installed)",
            inst.tag
        ),
        json!({ "default": DYNAMIC_DEFAULT, "version": inst.tag }),
    );
    Ok(())
}

/// Points `<NVSN_DIR>/current` to the installation `inst` (junction on Windows,
/// symlink on Unix). It is the fixed path that all applications see (ADR-034).
///
/// # Errors
///
/// `general` (1) if the link cannot be created or activated.
pub(crate) fn sync_current_link(ctx: &Ctx, inst: &Installed) -> Result<(), CliError> {
    ctx.ensure_store()?;
    let link = ctx.store.root().join("current");
    nvsn_platform::set_version_link(&link, &inst.dir)
        .map_err(|err| CliError::general(format!("cannot update {}: {err:#}", link.display())))
}

/// Creates or replaces an alias. Without arguments, lists the aliases.
///
/// # Errors
///
/// `usage` (2) for an invalid name or no active version, `version-not-found` (6).
pub fn alias(ctx: &Ctx, name: Option<&str>, version: Option<&str>) -> Result<(), CliError> {
    let Some(name) = name else {
        return list_alias_table(ctx);
    };
    // `alias default node` is the spelling of `default node` (the newest installed version).
    // Other values keep the reserved-name error: use `nvsn default <version>` for them.
    if name == "default" {
        if let Some(text) = version {
            if matches!(parse_input(text)?, VersionSpec::Latest) {
                return set_default(ctx, text);
            }
        }
    }
    validate_alias_name(name)?;

    let target = match version {
        Some(text) => resolve_installed(ctx, text)?.tag,
        None => {
            let active = effective_version(ctx).map(|(tag, _)| tag).ok_or_else(|| {
                CliError::usage("no version given and no version is active")
                    .with_hint(format!("pass one: `nvsn alias {name} <version>`"))
            })?;
            resolve_installed(ctx, &active)?.tag
        }
    };
    write_alias(ctx, name, &target)?;
    ctx.out.result(
        "alias",
        &format!("{name} -> {target}"),
        json!({ "name": name, "version": target }),
    );
    Ok(())
}

fn list_alias_table(ctx: &Ctx) -> Result<(), CliError> {
    let aliases = list_aliases(&ctx.store)?;
    let human = if aliases.is_empty() {
        "No aliases defined. Create one with `nvsn alias <name> <version>`.".to_owned()
    } else {
        aliases
            .iter()
            .map(|(name, tag)| format!("{name} -> {tag}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let rows: Vec<_> = aliases
        .iter()
        .map(|(name, tag)| json!({ "name": name, "version": tag }))
        .collect();
    ctx.out.result("alias", &human, json!({ "aliases": rows }));
    Ok(())
}

/// Removes an alias.
///
/// # Errors
///
/// `version-not-found` (6) if the alias does not exist.
pub fn unalias(ctx: &Ctx, name: &str) -> Result<(), CliError> {
    if !remove_alias(ctx, name)? {
        return Err(
            CliError::not_found(format!("alias '{name}' is not defined"))
                .with_hint("run `nvsn alias` to list the aliases"),
        );
    }
    ctx.out.result(
        "unalias",
        &format!("Removed alias {name}"),
        json!({ "name": name, "removed": true }),
    );
    Ok(())
}

/// Prints the path of the `node` executable of a version.
///
/// # Errors
///
/// `version-not-found` (6) without a version or without an active one; general error if the binary is missing.
pub fn which(ctx: &Ctx, input: Option<&str>) -> Result<(), CliError> {
    let inst = match input {
        Some(text) => resolve_installed(ctx, text)?,
        None => {
            let active = effective_version(ctx).map(|(tag, _)| tag).ok_or_else(|| {
                CliError::not_found("no version is active and no default is set")
                    .with_hint("pass a version: `nvsn which 20`")
            })?;
            resolve_installed(ctx, &active)?
        }
    };
    let node = node_path(&inst.dir).ok_or_else(|| {
        CliError::general(format!("{} has no node executable", inst.tag)).with_hint(format!(
            "reinstall it: `nvsn uninstall {0}` and `nvsn install {0}`",
            inst.tag.trim_start_matches('v')
        ))
    })?;
    let path = node.display().to_string();
    ctx.out
        .result("which", &path, json!({ "version": inst.tag, "path": path }));
    Ok(())
}

/// Prints the installed version that matches a specification.
///
/// # Errors
///
/// `version-not-found` (6) if there is no installed match.
pub fn version(ctx: &Ctx, spec: &str) -> Result<(), CliError> {
    let inst = resolve_installed(ctx, spec)?;
    ctx.out
        .result("version", &inst.tag, json!({ "version": inst.tag }));
    Ok(())
}
