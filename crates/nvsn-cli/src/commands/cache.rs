//! `cache`: lists and clears the download cache through the nvsn-core `cache` module.
//!
//! The cache has two areas: `downloads/` (resumable downloads) and `index/` (the remote
//! index). Installed versions are never touched.

use crate::cli::CacheAction;
use crate::ctx::Ctx;
use crate::exit::CliError;
use nvsn_core::cache::{cache_clear, cache_dirs, cache_list, CacheKind};
use serde_json::json;
use std::path::Path;

/// Runs one of the `cache` actions.
///
/// # Errors
///
/// General error (1) if the cache cannot be read or cleared.
pub fn run(ctx: &Ctx, action: &CacheAction) -> Result<(), CliError> {
    match action {
        CacheAction::Dir => dir(ctx),
        CacheAction::List => list(ctx),
        CacheAction::Clear => clear(ctx),
    }
}

fn dir(ctx: &Ctx) -> Result<(), CliError> {
    let dirs = cache_dirs(&ctx.store);
    let shown = ctx.store.cache_dir().display().to_string();
    ctx.out.result(
        "cache",
        &shown,
        json!({
            "path": shown,
            "downloads": dirs.downloads.display().to_string(),
            "index": dirs.index.display().to_string(),
        }),
    );
    Ok(())
}

fn list(ctx: &Ctx) -> Result<(), CliError> {
    let entries = cache_list(&ctx.store).map_err(|err| {
        CliError::general(format!("{err:#}")).with_hint("check the permissions of NVSN_DIR/cache")
    })?;
    let root = ctx.store.cache_dir();
    let human = if entries.is_empty() {
        "The cache is empty.".to_owned()
    } else {
        entries
            .iter()
            .map(|entry| {
                format!(
                    "{:>12}  {:<9} {}",
                    entry.bytes,
                    kind_name(entry.kind),
                    relative(&entry.path, &root)
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let rows: Vec<_> = entries
        .iter()
        .map(|entry| {
            json!({
                "path": relative(&entry.path, &root),
                "kind": kind_name(entry.kind),
                "bytes": entry.bytes,
            })
        })
        .collect();
    ctx.out.result("cache", &human, json!({ "entries": rows }));
    Ok(())
}

fn clear(ctx: &Ctx) -> Result<(), CliError> {
    let report = cache_clear(&ctx.store).map_err(|err| {
        CliError::general(format!("{err:#}"))
            .with_hint("close the programs that use the cache and retry")
    })?;
    ctx.out.result(
        "cache",
        &format!(
            "Removed {} cache files ({} bytes)",
            report.files, report.bytes
        ),
        json!({ "removed": report.files, "bytes": report.bytes }),
    );
    Ok(())
}

fn kind_name(kind: CacheKind) -> &'static str {
    match kind {
        CacheKind::Download => "download",
        CacheKind::Index => "index",
    }
}

/// `path` relative to the cache root, with forward slashes, for stable output.
fn relative(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}
