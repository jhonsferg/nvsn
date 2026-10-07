//! `prune`: removes the installed versions that nothing references.
//!
//! A version is referenced if it is:
//! - The default version (`NVSN_DIR/default`) or the session one (`NVSN_VERSION`).
//! - The specification of a `.nvmrc` or `.node-version` found walking up
//!   from the current directory.
//! - The one of a version file found in `--scan-dir`, up to 5 levels.
//!
//! Versions are resolved against the installed ones (a `20` references the
//! highest installed 20.x). Without `--force` it asks for confirmation; without a terminal and
//! without `--force` nothing is deleted and the command exits with 9.

use crate::ctx::Ctx;
use crate::exit::CliError;
use crate::versions::{list_installed, read_default, resolve_installed, Installed};
use nvsn_core::nvmrc::{read_version_file, VERSION_FILE_NAMES};
use nvsn_core::{FileLock, Store};
use serde_json::json;
use std::collections::{BTreeSet, HashSet};
use std::path::Path;

/// Maximum depth of the `--scan-dir` scan.
const SCAN_DEPTH: u8 = 5;

/// Directories that are never the root of a project. They are skipped when scanning.
const SKIPPED_DIRS: [&str; 3] = ["node_modules", "target", "vendor"];

/// Removes the unreferenced versions.
///
/// # Errors
///
/// `usage` (2) if `scan_dir` is not a directory. `confirmation-required` (9)
/// if confirmation is needed and there is no terminal nor `--yes`. `general` (1) if a
/// version cannot be deleted.
pub fn prune(
    ctx: &Ctx,
    dry_run: bool,
    force: bool,
    scan_dir: Option<&Path>,
) -> Result<(), CliError> {
    let installed = list_installed(&ctx.store)?;
    if installed.is_empty() {
        ctx.out.result(
            "prune",
            "No Node.js versions installed.",
            json!({ "referenced": [], "to_remove": [], "removed": [] }),
        );
        return Ok(());
    }
    if let Some(dir) = scan_dir {
        if !dir.is_dir() {
            return Err(CliError::usage(format!(
                "--scan-dir '{}' is not a directory",
                dir.display()
            )));
        }
    }

    let referenced = referenced_versions(ctx, scan_dir);
    let (kept, to_remove): (Vec<&Installed>, Vec<&Installed>) = installed
        .iter()
        .partition(|inst| referenced.contains(&inst.tag));
    let kept_tags: Vec<&str> = kept.iter().map(|inst| inst.tag.as_str()).collect();

    if to_remove.is_empty() {
        ctx.out.result(
            "prune",
            "All installed versions are referenced. Nothing to prune.",
            json!({ "referenced": kept_tags, "to_remove": [], "removed": [] }),
        );
        return Ok(());
    }

    let sizes: Vec<f64> = to_remove
        .iter()
        .map(|inst| dir_size_mb(&inst.dir))
        .collect();
    let total_mb: f64 = sizes.iter().sum();
    let plan = render_plan(&to_remove, &sizes, total_mb);
    let to_remove_tags: Vec<&str> = to_remove.iter().map(|inst| inst.tag.as_str()).collect();

    if dry_run {
        ctx.out.result(
            "prune",
            &format!("{plan}\n\nDry run: nothing removed."),
            json!({
                "referenced": kept_tags,
                "to_remove": to_remove_tags,
                "removed": [],
                "dry_run": true,
            }),
        );
        return Ok(());
    }

    if !force {
        ctx.out.note(&plan);
        let question = format!(
            "Remove {} version{}?",
            to_remove.len(),
            if to_remove.len() == 1 { "" } else { "s" }
        );
        if !crate::prompt::confirm(ctx, &question)? {
            ctx.out.result(
                "prune",
                "Cancelled. Nothing was removed.",
                json!({ "referenced": kept_tags, "to_remove": to_remove_tags, "removed": [] }),
            );
            return Ok(());
        }
    }

    ctx.ensure_store()?;
    let mut removed: Vec<&str> = Vec::new();
    for inst in &to_remove {
        remove_version(&ctx.store, inst)?;
        removed.push(inst.tag.as_str());
    }

    ctx.out.result(
        "prune",
        &format!(
            "Pruned {} version{}.",
            removed.len(),
            if removed.len() == 1 { "" } else { "s" }
        ),
        json!({ "referenced": kept_tags, "to_remove": to_remove_tags, "removed": removed }),
    );
    Ok(())
}

/// Set of installed tags protected by some reference.
fn referenced_versions(ctx: &Ctx, scan_dir: Option<&Path>) -> HashSet<String> {
    let mut referenced = HashSet::new();

    if let Some(tag) = read_default(&ctx.store) {
        referenced.insert(tag);
    }
    if let Some(tag) = ctx.active_version() {
        referenced.insert(tag);
    }

    // Specifications that appear in version files. Each one is resolved only once,
    // because each resolution reads the index cache.
    let mut specs: BTreeSet<String> = BTreeSet::new();
    if let Ok(cwd) = std::env::current_dir() {
        for dir in cwd.ancestors() {
            collect_specs_in(dir, &mut specs);
        }
    }
    if let Some(dir) = scan_dir {
        scan_directory(dir, 0, SCAN_DEPTH, &mut specs);
    }
    for spec in &specs {
        if let Ok(inst) = resolve_installed(ctx, spec) {
            referenced.insert(inst.tag);
        }
    }
    referenced
}

/// Adds the valid specifications from the version files in `dir`.
fn collect_specs_in(dir: &Path, specs: &mut BTreeSet<String>) {
    for name in VERSION_FILE_NAMES {
        let path = dir.join(name);
        if path.is_file() {
            if let Ok(file) = read_version_file(&path) {
                specs.insert(file.raw);
            }
        }
    }
}

/// Walks `dir` up to `max_depth` levels. Skips hidden directories and those in
/// `SKIPPED_DIRS`, which are not project roots.
fn scan_directory(dir: &Path, depth: u8, max_depth: u8, specs: &mut BTreeSet<String>) {
    if depth > max_depth {
        return;
    }
    collect_specs_in(dir, specs);

    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || SKIPPED_DIRS.contains(&name.as_ref()) {
            continue;
        }
        scan_directory(&entry.path(), depth + 1, max_depth, specs);
    }
}

/// Text of the list of versions to delete, with their size.
fn render_plan(to_remove: &[&Installed], sizes: &[f64], total_mb: f64) -> String {
    let mut lines = vec!["Versions to remove:".to_owned()];
    for (inst, size) in to_remove.iter().zip(sizes) {
        lines.push(format!("  - {}  ({size:.1} MB)", inst.tag));
    }
    lines.push(format!(
        "Total: {} version{}, ~{total_mb:.1} MB",
        to_remove.len(),
        if to_remove.len() == 1 { "" } else { "s" }
    ));
    lines.join("\n")
}

/// Moves the version directory aside (it disappears from the list) and then deletes it.
/// It runs under the same per-version lock as `uninstall`.
fn remove_version(store: &Store, inst: &Installed) -> Result<(), CliError> {
    let lock_path = store.locks_dir().join(format!("{}.lock", inst.tag));
    let _lock = FileLock::lock(&lock_path).map_err(CliError::classify)?;

    let trash = store
        .versions_dir()
        .join(format!(".trash-{}-{}", inst.tag, std::process::id()));
    std::fs::rename(&inst.dir, &trash).map_err(|err| {
        CliError::general(format!("cannot remove {}: {err}", inst.tag))
            .with_hint("close the programs that run this version (node processes) and retry")
    })?;
    std::fs::remove_dir_all(&trash).map_err(|err| {
        CliError::general(format!(
            "{} was removed from the list but its files could not be deleted: {err}",
            inst.tag
        ))
        .with_hint(format!("delete {} by hand", trash.display()))
    })
}

/// Approximate size in megabytes of the tree `dir`. Links count by their
/// own size, without following them.
fn dir_size_mb(dir: &Path) -> f64 {
    fn walk(path: &Path) -> u64 {
        let Ok(entries) = std::fs::read_dir(path) else {
            return 0;
        };
        entries
            .flatten()
            .fold(0u64, |acc, entry| match entry.metadata() {
                Ok(meta) if meta.is_dir() => acc + walk(&entry.path()),
                Ok(meta) => acc + meta.len(),
                Err(_) => acc,
            })
    }
    walk(dir) as f64 / (1024.0 * 1024.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_collects_specs_from_version_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join(".nvmrc"), "20.11.1\n").expect("write");
        let nested = dir.path().join("app").join("api");
        std::fs::create_dir_all(&nested).expect("mkdir");
        std::fs::write(nested.join(".node-version"), "18\n").expect("write");

        let mut specs = BTreeSet::new();
        scan_directory(dir.path(), 0, SCAN_DEPTH, &mut specs);
        assert!(specs.contains("20.11.1"));
        assert!(specs.contains("18"));
    }

    #[test]
    fn scan_skips_hidden_and_build_directories() {
        let dir = tempfile::tempdir().expect("tempdir");
        for name in [".git", "node_modules", "target", "vendor"] {
            let skipped = dir.path().join(name);
            std::fs::create_dir_all(&skipped).expect("mkdir");
            std::fs::write(skipped.join(".nvmrc"), "16.0.0\n").expect("write");
        }
        let mut specs = BTreeSet::new();
        scan_directory(dir.path(), 0, SCAN_DEPTH, &mut specs);
        assert!(specs.is_empty());
    }

    #[test]
    fn scan_stops_at_the_depth_limit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let deep = dir
            .path()
            .join("a")
            .join("b")
            .join("c")
            .join("d")
            .join("e")
            .join("f");
        std::fs::create_dir_all(&deep).expect("mkdir");
        std::fs::write(deep.join(".nvmrc"), "14.0.0\n").expect("write");
        let mut specs = BTreeSet::new();
        scan_directory(dir.path(), 0, SCAN_DEPTH, &mut specs);
        assert!(!specs.contains("14.0.0"));
    }

    #[test]
    fn invalid_version_files_are_ignored() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join(".nvmrc"), "a=b\n").expect("write");
        let mut specs = BTreeSet::new();
        collect_specs_in(dir.path(), &mut specs);
        assert!(specs.is_empty());
    }

    #[test]
    fn dir_size_of_missing_directory_is_zero() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(dir_size_mb(&dir.path().join("absent")), 0.0);
    }
}
