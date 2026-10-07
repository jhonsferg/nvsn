//! `implode`: removes everything nvsn manages on the system.
//!
//! It removes, in this order:
//! 1. The full data directory (`NVSN_DIR`): versions, aliases, cache.
//! 2. The `# nvsn ...` blocks from the shell profiles that `init --apply`
//!    may have written (interactive, login and extra profile of each shell).
//! 3. On Windows, the nvsn entries of the user PATH
//!    (see [`remove_user_path_entries`]).
//! 4. The `nvsn` binary, always the last one.
//!
//! Safety is the same as `self-uninstall`: it only touches `NVSN_DIR`, the
//! binary in use and the shell profiles, and it refuses to delete the root of the
//! file system or a directory that contains the home.
//!
//! The logic of `self_uninstall.rs` is copied here instead of shared
//! because that file is modified by another change in progress. When that finishes, it
//! can be factored into a common module.

use crate::commands::self_update::old_path;
use crate::ctx::Ctx;
use crate::exit::{CliError, ErrorKind};
use crate::prompt::confirm;
use serde_json::json;
use std::io::ErrorKind as IoErrorKind;
use std::path::{Path, PathBuf};

/// Runs `implode`. Asks for confirmation unless `--force` or `--yes` is given.
///
/// # Errors
///
/// `confirmation-required` (9) without a terminal and without `--force`/`--yes`,
/// deleting nothing. `general` (1) if the data root is not safe or cannot be deleted.
/// Failures when cleaning profiles or the PATH are warnings, not errors.
pub fn run(ctx: &Ctx, force: bool) -> Result<(), CliError> {
    let exe = std::env::current_exe()
        .map_err(|err| CliError::general(format!("cannot locate the nvsn binary: {err}")))?;
    let root = absolute(ctx.store.root())?;
    let home = ctx.home();
    check_removable(&root, home.as_deref())?;
    let root_exists = std::fs::symlink_metadata(&root).is_ok();

    ctx.out.note("nvsn implode will remove:");
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

    if !force && !confirm(ctx, "Remove nvsn and all its data?")? {
        return Err(CliError::general("implode cancelled; nothing was removed"));
    }

    let mut removed: Vec<String> = Vec::new();
    let mut leftover: Option<PathBuf> = None;
    let binary_target = if cfg!(windows) {
        old_path(&exe)
    } else {
        exe.clone()
    };

    // On Windows the executable in use cannot be deleted nor remain inside a
    // root that is removed: it is first moved aside to `<name>.old`.
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

    let mut profiles_changed: Vec<String> = Vec::new();
    if let Some(home) = home.as_deref() {
        for (path, outcome) in clean_profiles(home) {
            match outcome {
                Ok(true) => profiles_changed.push(path.display().to_string()),
                Ok(false) => {}
                Err(err) => ctx.out.note(format!(
                    "warning: could not clean {}: {err}",
                    path.display()
                )),
            }
        }
    }

    if let Err(err) = remove_user_path_entries(ctx.store.root()) {
        ctx.out
            .note(format!("warning: could not clean the user PATH: {err}"));
    }

    let mut lines = vec!["nvsn has been removed.".to_owned()];
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

    lines.extend(removed.iter().map(|path| format!("  removed {path}")));
    lines.extend(
        profiles_changed
            .iter()
            .map(|path| format!("  cleaned {path}")),
    );
    if let Some(path) = &leftover {
        lines.push(format!(
            "  {} is still in use by this process; it is deleted when this process exits.",
            path.display()
        ));
    }
    lines.push("Open a new terminal to apply the profile changes.".to_owned());

    let leftover_text = leftover.as_ref().map(|path| path.display().to_string());
    ctx.out.result(
        "implode",
        &lines.join("\n"),
        json!({
            "removed": removed,
            "profiles_cleaned": profiles_changed,
            "leftover": leftover_text,
        }),
    );
    Ok(())
}

/// Extension point: removes from the Windows user PATH the entries that
/// nvsn added (`HKCU\Environment`).
///
/// For now it does nothing and returns `Ok(())`. The implementation will arrive with the
/// registry work of `nvsn-platform`. On other platforms the PATH is not
/// managed in the registry, so it must also remain a no-op there.
///
/// # Errors
///
/// Will return an error if the user PATH cannot be read or written. The
/// command shows it as a warning and continues with the rest of the cleanup.
#[cfg(windows)]
pub fn remove_user_path_entries(root: &Path) -> Result<(), CliError> {
    use nvsn_platform::user_path::{remove_user_path_under, ENVIRONMENT_KEY};
    // Implode deletes the running binary too, so its directory goes from the
    // user PATH as well (self-uninstall keeps it, because it may be shared).
    let mut dirs = vec![root.to_path_buf()];
    if let Some(bin_dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    {
        dirs.push(bin_dir);
    }
    for dir in &dirs {
        remove_user_path_under(ENVIRONMENT_KEY, dir).map_err(|err| {
            CliError::general(format!("cannot clean the user PATH: {err:#}"))
                .with_hint("check that HKCU\\Environment is writable")
        })?;
    }
    Ok(())
}

/// Outside Windows the user PATH does not live in the registry: it does nothing.
#[cfg(not(windows))]
pub fn remove_user_path_entries(_root: &Path) -> Result<(), CliError> {
    Ok(())
}

/// Removes the nvsn blocks from each profile that `init` may have written. Returns
/// the path and whether it changed, or the cleanup error for that path.
fn clean_profiles(home: &Path) -> Vec<(PathBuf, Result<bool, String>)> {
    let mut outcomes = Vec::new();
    for name in ["bash", "zsh", "fish", "powershell"] {
        let Ok(shell) = nvsn_shell::from_str(name) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = Vec::new();
        paths.extend(shell.profile_path(home));
        paths.extend(shell.login_profile_path(home));
        paths.extend(shell.extra_profile_paths(home));
        for path in paths {
            let outcome = nvsn_shell::strip_profile(&path).map_err(|err| err.to_string());
            outcomes.push((path, outcome));
        }
    }
    outcomes
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
    use super::{check_removable, remove_user_path_entries};
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

    #[test]
    fn user_path_hook_is_a_no_op_for_now() {
        assert!(remove_user_path_entries(Path::new("nvsn-not-installed")).is_ok());
    }
}
