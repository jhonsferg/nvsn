//! `list`, `list available` (`list-remote`) and `version-remote`.

use crate::cli::{ListScope, RemoteFilters};
use crate::ctx::Ctx;
use crate::exit::CliError;
use crate::versions::{list_installed, lts_label, parse_input, read_default, resolve_remote};
use nvsn_core::remote::RemoteRelease;
use serde_json::json;

/// `list [available]`: the installed versions, or with `available` the remote index.
///
/// The filters (`--lts`, `--major`, `--latest`) only apply to `available`.
///
/// # Errors
///
/// `usage` (2) if a filter is used without `available`; the errors of [`list`] and [`list_remote`].
pub fn list_command(
    ctx: &Ctx,
    scope: Option<ListScope>,
    filters: &RemoteFilters,
) -> Result<(), CliError> {
    match scope {
        Some(ListScope::Available) => list_remote(ctx, filters),
        None if filters.lts || filters.latest || filters.major.is_some() => Err(CliError::usage(
            "--lts, --major and --latest apply to `list available`",
        )
        .with_hint("run `nvsn list available --lts`")),
        None => list(ctx),
    }
}

/// Lists the installed versions, marking the default and the active one.
///
/// # Errors
///
/// General error if `versions/` cannot be read.
pub fn list(ctx: &Ctx) -> Result<(), CliError> {
    let installed = list_installed(&ctx.store)?;
    let default = read_default(&ctx.store);
    let current = ctx.active_version();

    let mut lines = Vec::with_capacity(installed.len());
    let mut rows = Vec::with_capacity(installed.len());
    for inst in &installed {
        let is_default = default.as_deref() == Some(inst.tag.as_str());
        let is_current = current.as_deref() == Some(inst.tag.as_str());
        let marker = if is_current { "->" } else { "  " };
        let suffix = if is_default { " (default)" } else { "" };
        lines.push(format!("{marker} {}{suffix}", inst.tag));
        rows.push(json!({
            "version": inst.tag,
            "default": is_default,
            "current": is_current,
        }));
    }

    let human = if installed.is_empty() {
        "No Node.js versions installed. Run `nvsn install <version>`, e.g. `nvsn install 20`."
            .to_owned()
    } else {
        lines.join("\n")
    };
    ctx.out.result(
        "list",
        &human,
        json!({ "versions": rows, "default": default, "current": current }),
    );
    Ok(())
}

/// Lists the versions of the remote index with filters.
///
/// # Errors
///
/// `offline-no-cache` (5) with `--offline` and no cache, `download-failed` (7) if the index cannot be downloaded.
pub fn list_remote(ctx: &Ctx, filters: &RemoteFilters) -> Result<(), CliError> {
    let releases = ctx.load_index()?;
    let installed: Vec<String> = list_installed(&ctx.store)?
        .into_iter()
        .map(|inst| inst.tag)
        .collect();

    let mut selected: Vec<&RemoteRelease> = releases
        .iter()
        .filter(|r| !filters.lts || r.lts.is_lts())
        .filter(|r| filters.major.is_none_or(|m| r.version.major == m))
        .collect();
    selected.sort_by(|a, b| a.version.cmp(&b.version));
    if filters.latest {
        selected = selected.last().copied().into_iter().collect();
    }

    let mut lines = Vec::with_capacity(selected.len());
    let mut rows = Vec::with_capacity(selected.len());
    for release in &selected {
        let tag = release.tag();
        let label = lts_label(&release.lts);
        let is_installed = installed.contains(&tag);
        let mark = if is_installed { "installed" } else { "" };
        lines.push(
            format!(
                "{tag:<12} {:<10} {:<12} {mark}",
                label.as_deref().unwrap_or(""),
                release.date
            )
            .trim_end()
            .to_owned(),
        );
        rows.push(json!({
            "version": tag,
            "date": release.date,
            "lts": label,
            "installed": is_installed,
        }));
    }

    let human = if lines.is_empty() {
        "No releases match these filters.".to_owned()
    } else {
        lines.join("\n")
    };
    ctx.out
        .result("list-remote", &human, json!({ "releases": rows }));
    Ok(())
}

/// Shows the version of the remote index that matches a specification.
///
/// # Errors
///
/// `usage` (2) if the specification is not valid, `version-not-found` (6) if there is no match.
pub fn version_remote(ctx: &Ctx, spec_text: &str) -> Result<(), CliError> {
    let spec = parse_input(spec_text)?;
    if !spec.is_remote() {
        return Err(
            CliError::usage(format!("'{spec_text}' is not a remote version spec"))
                .with_hint("use a version number such as 20 or an lts/* spec"),
        );
    }
    let releases = ctx.load_index()?;
    let release = resolve_remote(&releases, &spec, spec_text)?;
    ctx.out.result(
        "version-remote",
        &release.tag(),
        json!({
            "version": release.tag(),
            "date": release.date,
            "lts": lts_label(&release.lts),
        }),
    );
    Ok(())
}
