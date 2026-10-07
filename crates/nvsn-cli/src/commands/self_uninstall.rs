//! `self-uninstall`: removes the nvsn binary and its data directory (`NVSN_DIR`).
//!
//! It only touches two paths: the data root directory and the executable in use.
//! It refuses to delete the root of the file system or a directory that contains the home.
//! On Windows the executable in use cannot be deleted: it is renamed to
//! `nvsn.exe.old` and deletion is attempted; if it is still locked, it is left to be
//! deleted by hand when the terminal is closed.

use crate::commands::self_update::old_path;
use crate::ctx::Ctx;
use crate::exit::{CliError, ErrorKind};
use crate::prompt::confirm;
use serde_json::json;
use std::io::ErrorKind as IoErrorKind;
use std::path::{Path, PathBuf};

/// Runs `self-uninstall`. Asks for confirmation unless `--yes` is given.
///
/// # Errors
///
/// `confirmation-required` (9) without a TTY and without `--yes`, deleting nothing. `general`
/// (1) if the data root is not safe or cannot be deleted.
pub fn run(ctx: &Ctx) -> Result<(), CliError> {
    let exe = std::env::current_exe()
        .map_err(|err| CliError::general(format!("cannot locate the nvsn binary: {err}")))?;
    let root = absolute(ctx.store.root())?;
    check_removable(&root, ctx.home().as_deref())?;
    let root_exists = std::fs::symlink_metadata(&root).is_ok();

    ctx.out.note("nvsn self-uninstall will remove:");
    if root_exists {
        ctx.out.note(format!(
            "  data directory: {} (installed versions, cache, aliases)",
            root.display()
        ));
    } else {
        ctx.out.note(format!(
            "  data directory: {} (not found, skipped)",
            root.display()
        ));
    }
    ctx.out.note(format!("  binary: {}", exe.display()));
    ctx.out
        .note("  the nvsn blocks that `nvsn init --apply` wrote into shell profiles");

    if !confirm(ctx, "Remove nvsn?")? {
        return Err(CliError::general(
            "uninstall cancelled; nothing was removed",
        ));
    }

    let mut removed: Vec<String> = ctx
        .home()
        .map(|home| remove_profile_blocks(&home))
        .unwrap_or_default();
    removed.extend(remove_user_path_entries(ctx, &root));
    let mut leftover: Option<PathBuf> = None;
    let binary_target = if cfg!(windows) {
        old_path(&exe)
    } else {
        exe.clone()
    };

    #[cfg(windows)]
    {
        let _ = std::fs::remove_file(&binary_target);
        std::fs::rename(&exe, &binary_target).map_err(|err| {
            CliError::general(format!(
                "cannot move {} out of the way: {err}",
                exe.display()
            ))
            .with_hint("check write permissions for the install directory")
        })?;
    }

    if root_exists {
        if let Err(err) = std::fs::remove_dir_all(&root) {
            #[cfg(windows)]
            {
                let _ = std::fs::rename(&binary_target, &exe);
            }
            return Err(
                CliError::general(format!("cannot remove {}: {err}", root.display())).with_hint(
                    "some files may already be gone; check permissions and run it again",
                ),
            );
        }
        removed.push(root.display().to_string());
    }

    match std::fs::remove_file(&binary_target) {
        Ok(()) => removed.push(exe.display().to_string()),
        // Already gone together with the data root (the binary was inside it).
        Err(err) if err.kind() == IoErrorKind::NotFound => {}
        Err(_) if cfg!(windows) => {
            schedule_delete_after_exit(&binary_target);
            leftover = Some(binary_target);
        }
        Err(err) => {
            return Err(
                CliError::general(format!("cannot remove {}: {err}", exe.display()))
                    .with_hint("check write permissions for the install directory"),
            );
        }
    }

    let mut lines = vec!["nvsn has been removed.".to_owned()];
    lines.extend(removed.iter().map(|path| format!("  removed {path}")));
    if let Some(path) = &leftover {
        lines.push(format!(
            "  {} is still in use by this process; it is deleted when this process exits.",
            path.display()
        ));
    }
    let leftover_text = leftover.as_ref().map(|path| path.display().to_string());
    ctx.out.result(
        "self-uninstall",
        &lines.join("\n"),
        json!({ "removed": removed, "leftover": leftover_text }),
    );
    Ok(())
}

/// Removes the nvsn blocks from all the profiles that `init` may have written
/// (interactive and login profile of each supported shell). Returns the files
/// that changed. It does not touch anything else in the user's profile.
fn remove_profile_blocks(home: &Path) -> Vec<String> {
    let mut changed = Vec::new();
    for name in ["bash", "zsh", "fish", "powershell"] {
        let Ok(shell) = nvsn_shell::from_str(name) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = Vec::new();
        paths.extend(shell.profile_path(home));
        paths.extend(shell.login_profile_path(home));
        paths.extend(shell.extra_profile_paths(home));
        for path in paths {
            if matches!(nvsn_shell::strip_profile(&path), Ok(true)) {
                changed.push(path.display().to_string());
            }
        }
    }
    changed
}

/// Removes from the user PATH (registry) the entries that `init --apply` added
/// under the nvsn directory, that is `<root>\current`. Returns what was removed.
///
/// The binary's folder is not removed: it may be shared (e.g.
/// `~/.local/bin`), as gvsn also does. If it fails, it warns and continues.
#[cfg(windows)]
fn remove_user_path_entries(ctx: &Ctx, root: &Path) -> Vec<String> {
    use nvsn_platform::user_path::{remove_user_path_under, ENVIRONMENT_KEY};
    match remove_user_path_under(ENVIRONMENT_KEY, root) {
        Ok(entries) => entries
            .into_iter()
            .map(|entry| format!("user PATH entry {entry}"))
            .collect(),
        Err(err) => {
            ctx.out
                .note(format!("warning: cannot clean the user PATH: {err:#}"));
            Vec::new()
        }
    }
}

/// Outside Windows the user PATH does not live in the registry.
#[cfg(not(windows))]
fn remove_user_path_entries(_ctx: &Ctx, _root: &Path) -> Vec<String> {
    Vec::new()
}

/// On Windows the executable in use cannot be deleted. It launches a separate `cmd`
/// that waits for this process to exit and deletes it; it does not block the command.
#[cfg(windows)]
fn schedule_delete_after_exit(path: &Path) {
    use std::os::windows::process::CommandExt;
    let line = format!(
        "/C \"ping -n 3 127.0.0.1 >nul & del /f /q \"{}\"\"",
        path.display()
    );
    let _ = std::process::Command::new("cmd").raw_arg(line).spawn();
}

#[cfg(not(windows))]
fn schedule_delete_after_exit(_path: &Path) {}

/// Converts the data root into an absolute path relative to the current directory.
fn absolute(root: &Path) -> Result<PathBuf, CliError> {
    if root.is_absolute() {
        return Ok(root.to_path_buf());
    }
    std::env::current_dir()
        .map(|dir| dir.join(root))
        .map_err(|err| CliError::general(format!("cannot resolve {}: {err}", root.display())))
}

/// Rejects roots that are not a directory owned by nvsn.
fn check_removable(root: &Path, home: Option<&Path>) -> Result<(), CliError> {
    let unsafe_root = |reason: &str| {
        CliError::new(
            ErrorKind::General,
            format!("refusing to remove {}: {reason}", root.display()),
        )
        .with_hint("point NVSN_DIR (or --dir) to a dedicated directory such as ~/.nvsn")
    };
    if !root.is_absolute() || root.parent().is_none() {
        return Err(unsafe_root("it is a filesystem root"));
    }
    if let Some(home) = home {
        if home.starts_with(root) {
            return Err(unsafe_root("it contains your home directory"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::check_removable;
    use std::path::Path;

    #[test]
    fn refuses_filesystem_root() {
        let root = Path::new(if cfg!(windows) { "C:\\" } else { "/" });
        let err = check_removable(root, None).expect_err("root must be refused");
        assert!(err.message.contains("filesystem root"));
    }

    #[test]
    fn refuses_a_directory_that_contains_home() {
        let dir = tempfile::tempdir().expect("temp dir");
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).expect("mkdir");
        let err = check_removable(dir.path(), Some(&home)).expect_err("ancestor refused");
        assert!(err.message.contains("home directory"));
        assert!(check_removable(&home, Some(&home)).is_err());
    }

    #[test]
    fn accepts_a_dedicated_directory() {
        let dir = tempfile::tempdir().expect("temp dir");
        let home = dir.path().join("home");
        let store = dir.path().join("store");
        assert!(check_removable(&store, Some(&home)).is_ok());
    }
}
