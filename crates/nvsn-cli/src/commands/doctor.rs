//! `doctor`: diagnostics for the directory, platform, shell, PATH, default, profile
//! hook, the `.nvmrc` of the current directory and the index.
//!
//! Exits with 1 if any check is `fail`. `warn` results do not change the exit
//! code. The `fail` checks follow gvsn: binary outside the PATH, no global version
//! or not installed, hook missing from the profile and `.nvmrc` not installed.

use crate::ctx::Ctx;
use crate::exit::CliError;
use crate::versions::{list_installed, read_default, resolve_installed};
use nvsn_core::nvmrc::{read_version_file, VERSION_FILE_NAMES};
use nvsn_platform::Platform;
use nvsn_shell::{detect, is_dir_in_path};
use serde_json::{json, Value};

/// Marker that `init --apply` writes into the shell profile.
const HOOK_MARKER: &str = "# nvsn init";

/// Result of a check.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Check {
    name: &'static str,
    status: Status,
    detail: String,
}

/// Status of a check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Ok,
    Warn,
    Fail,
}

impl Status {
    fn label(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Warn => "warn",
            Status::Fail => "fail",
        }
    }
}

/// Runs the checks and shows them.
///
/// # Errors
///
/// `doctor-failed` (1) if any check fails.
pub fn doctor(ctx: &Ctx) -> Result<(), CliError> {
    let checks = run_checks(ctx);
    let failed = checks.iter().any(|c| c.status == Status::Fail);
    let failed_count = checks.iter().filter(|c| c.status == Status::Fail).count();

    let human = checks
        .iter()
        .map(|c| format!("[{:<4}] {:<12} {}", c.status.label(), c.name, c.detail))
        .collect::<Vec<_>>()
        .join("\n");
    let data: Vec<Value> = checks
        .iter()
        .map(|c| json!({ "name": c.name, "status": c.status.label(), "detail": c.detail }))
        .collect();
    ctx.out
        .result("doctor", &human, json!({ "ok": !failed, "checks": data }));

    if failed {
        return Err(CliError::general(format!(
            "doctor found {failed_count} problem{}; see the [fail] lines above",
            if failed_count == 1 { "" } else { "s" }
        )));
    }
    Ok(())
}

fn run_checks(ctx: &Ctx) -> Vec<Check> {
    let mut checks = Vec::new();

    let root = ctx.store.root();
    checks.push(if root.is_dir() {
        check("store", Status::Ok, format!("{} exists", root.display()))
    } else {
        check(
            "store",
            Status::Warn,
            format!("{} is created on the first install", root.display()),
        )
    });

    checks.push(match Platform::detect() {
        Ok(platform) => {
            let mut detail = format!(
                "{}-{} libc={:?}",
                platform.os.dist_name(),
                platform.arch.dist_name(),
                platform.libc
            );
            if platform.os == nvsn_platform::Os::Android {
                detail.push_str(" (Node comes from Termux: `pkg install nodejs-lts`)");
            }
            check("platform", Status::Ok, detail)
        }
        Err(err) => check("platform", Status::Fail, format!("{err:#}")),
    });

    checks.push(match detect() {
        Some(shell) => check("shell", Status::Ok, shell.name().to_owned()),
        None => check(
            "shell",
            Status::Warn,
            "not detected; pass --shell to `env` and `init`".to_owned(),
        ),
    });

    let integrated = std::env::var_os("NVSN_DIR").is_some() || ctx.integration;
    checks.push(if integrated {
        check(
            "integration",
            Status::Ok,
            "shell integration is loaded".to_owned(),
        )
    } else {
        check(
            "integration",
            Status::Warn,
            "not loaded in this session; run `nvsn init <shell>` once".to_owned(),
        )
    });

    let installed = list_installed(&ctx.store).unwrap_or_default();
    checks.push(if installed.is_empty() {
        check(
            "installed",
            Status::Warn,
            "no versions installed; run `nvsn install 20`".to_owned(),
        )
    } else {
        check(
            "installed",
            Status::Ok,
            format!("{} version(s)", installed.len()),
        )
    });

    checks.push(match read_default(&ctx.store) {
        Some(tag) if installed.iter().any(|i| i.tag == tag) => check("default", Status::Ok, tag),
        Some(tag) => check(
            "default",
            Status::Fail,
            format!("{tag} is set as default but is not installed; run `nvsn install {tag}`"),
        ),
        None => check(
            "default",
            Status::Fail,
            "no default version; run `nvsn default <version>`".to_owned(),
        ),
    });

    checks.push(match ctx.cached_index() {
        Some(releases) => check(
            "index",
            Status::Ok,
            format!("cached ({} releases)", releases.len()),
        ),
        None => check(
            "index",
            Status::Warn,
            "no cached index; run `nvsn list-remote`".to_owned(),
        ),
    });

    checks.push(path_check());
    checks.push(hook_check(ctx));
    if let Some(local) = local_version_check(ctx) {
        checks.push(local);
    }
    checks
}

/// Checks that the directory of the `nvsn` executable is in PATH.
fn path_check() -> Check {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf));
    let path_var = std::env::var_os("PATH")
        .map(|v| v.to_string_lossy().into_owned())
        .unwrap_or_default();
    match exe_dir {
        Some(dir) if is_dir_in_path(&dir, &path_var) => {
            check("path", Status::Ok, format!("{} is in PATH", dir.display()))
        }
        Some(dir) => check(
            "path",
            Status::Fail,
            format!("{} is not in PATH; add it to your PATH", dir.display()),
        ),
        None => check(
            "path",
            Status::Fail,
            "cannot locate the nvsn executable".to_owned(),
        ),
    }
}

/// Checks that the profile of the detected shell contains the `# nvsn init` block.
fn hook_check(ctx: &Ctx) -> Check {
    let Some(shell) = detect() else {
        return check(
            "hook",
            Status::Warn,
            "no shell detected; run `nvsn init <shell> --apply` by hand".to_owned(),
        );
    };
    let Some(home) = ctx.home() else {
        return check(
            "hook",
            Status::Warn,
            "no home directory; cannot find the shell profile".to_owned(),
        );
    };
    let Some(profile) = shell.profile_path(&home) else {
        return check(
            "hook",
            Status::Warn,
            format!("{} has no profile file", shell.name()),
        );
    };
    match std::fs::read_to_string(&profile) {
        Ok(text) if text.contains(HOOK_MARKER) => check(
            "hook",
            Status::Ok,
            format!("configured in {}", profile.display()),
        ),
        _ => check(
            "hook",
            Status::Fail,
            format!(
                "{HOOK_MARKER} missing from {}; run `nvsn init {} --apply`",
                profile.display(),
                shell.name()
            ),
        ),
    }
}

/// Checks the `.nvmrc` (or `.node-version`) of the current directory. Without a file
/// there is no check. With a file, it must be valid and its version must be installed.
fn local_version_check(ctx: &Ctx) -> Option<Check> {
    let cwd = std::env::current_dir().ok()?;
    let (path, name) = VERSION_FILE_NAMES
        .iter()
        .map(|name| (cwd.join(name), *name))
        .find(|(path, _)| path.is_file())?;

    let file = match read_version_file(&path) {
        Ok(file) => file,
        Err(err) => {
            return Some(check(
                "local version",
                Status::Fail,
                format!("{name} is invalid: {err:#}"),
            ))
        }
    };
    Some(match resolve_installed(ctx, &file.raw) {
        Ok(inst) => check(
            "local version",
            Status::Ok,
            format!("{name} = {} (installed as {})", file.raw, inst.tag),
        ),
        Err(err) => check(
            "local version",
            Status::Fail,
            format!(
                "{name} = {}: {}; run `nvsn install {}`",
                file.raw, err.message, file.raw
            ),
        ),
    })
}

fn check(name: &'static str, status: Status, detail: String) -> Check {
    Check {
        name,
        status,
        detail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_labels_are_stable() {
        assert_eq!(Status::Ok.label(), "ok");
        assert_eq!(Status::Warn.label(), "warn");
        assert_eq!(Status::Fail.label(), "fail");
    }
}
