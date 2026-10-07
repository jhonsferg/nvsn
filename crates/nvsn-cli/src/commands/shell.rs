//! `env`, `init` and `completions`: shell integration.

use crate::cli::{Cli, CompletionShell};
use crate::commands::state;
use crate::ctx::Ctx;
use crate::exit::CliError;
use crate::prompt::confirm;
use crate::versions::{bin_dir, list_installed, read_default, Installed};
use clap::CommandFactory;
use nvsn_shell::{detect, from_str, inject_login_profile, inject_profile, EnvContext, ShellConfig};
use serde_json::json;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Prints the script that configures the session: `NVSN_DIR`, the active version and the hook.
///
/// The active version is the session's `NVSN_VERSION` or, if there is none, the `default`.
///
/// # Errors
///
/// `usage` (2) if the shell is not valid or cannot be detected.
pub fn env(ctx: &Ctx, shell_name: Option<&str>) -> Result<(), CliError> {
    let shell = select_shell(shell_name)?;
    let active = active_installed(ctx)?;
    let bin = active.as_ref().map(|inst| bin_dir(&inst.dir));
    let tag = active.as_ref().map(|inst| inst.tag.as_str());

    let env_ctx = EnvContext {
        nvsn_dir: ctx.store.root(),
        active_bin: bin.as_deref(),
        node_version: tag,
    };
    let script = shell.env_script(&env_ctx);
    ctx.out.result(
        "env",
        script.trim_end(),
        json!({ "shell": shell.name(), "script": script }),
    );
    Ok(())
}

/// Shows, or with `apply` writes into the profile, the integration block of a shell.
///
/// # Errors
///
/// `usage` (2) if the shell is not valid; `confirmation-required` (9) with `--apply`
/// and without a TTY nor `--yes`; general error if the profile cannot be written.
pub fn init(ctx: &Ctx, shell_name: Option<&str>, apply: bool) -> Result<(), CliError> {
    let shell = select_shell(shell_name)?;
    let home = ctx.home().ok_or_else(|| {
        CliError::general("cannot find your home directory").with_hint("set HOME (or USERPROFILE)")
    })?;
    let profile = shell.profile_path(&home).ok_or_else(|| {
        CliError::unsupported(format!("{} has no profile file to update", shell.name()))
    })?;
    let init_line = shell.init_line();
    let wrapper = shell.wrapper_function();

    if !apply {
        let human = format!(
            "Add this to {}:\n\n{init_line}\n\n{wrapper}\n\nOr run `nvsn init {} --apply`.",
            profile.display(),
            shell.name()
        );
        ctx.out.result(
            "init",
            &human,
            json!({
                "shell": shell.name(),
                "profile": profile.display().to_string(),
                "init_line": init_line,
                "wrapper_function": wrapper,
                "applied": false,
            }),
        );
        return Ok(());
    }

    let question = format!("Write the nvsn block into {}?", profile.display());
    if !confirm(ctx, &question)? {
        ctx.out.result(
            "init",
            "Cancelled. Nothing was written.",
            json!({ "shell": shell.name(), "applied": false }),
        );
        return Ok(());
    }

    let changed_profile = inject_profile(shell.as_ref(), &home, current_bin_dir().as_deref())
        .map_err(|err| CliError::general(format!("cannot update the profile: {err}")))?;
    // The login profile lets graphical applications and login shells see the
    // global version through the `<root>/current` link (ADR-034).
    let changed_login = inject_login_profile(shell.as_ref(), &home, Some(ctx.store.root()))
        .map_err(|err| CliError::general(format!("cannot update the login profile: {err}")))?;
    refresh_current_link(ctx)?;
    let changed_path = apply_user_path(ctx)?;
    let changed = changed_profile || changed_login || changed_path;
    let human = if changed {
        format!(
            "Updated {}. Open a new shell to load it.",
            profile.display()
        )
    } else {
        format!("{} is already up to date.", profile.display())
    };
    ctx.out.result(
        "init",
        &human,
        json!({
            "shell": shell.name(),
            "profile": profile.display().to_string(),
            "applied": changed,
        }),
    );
    Ok(())
}

/// Prints the `clap_complete` completion script for a shell.
///
/// # Errors
///
/// General error if it cannot write to stdout.
pub fn completions(shell: CompletionShell) -> Result<(), CliError> {
    let mut command = Cli::command();
    let mut buffer = Vec::new();
    let name = "nvsn";
    match shell {
        CompletionShell::Bash => {
            clap_complete::generate(clap_complete::Shell::Bash, &mut command, name, &mut buffer)
        }
        CompletionShell::Zsh => {
            clap_complete::generate(clap_complete::Shell::Zsh, &mut command, name, &mut buffer)
        }
        CompletionShell::Fish => {
            clap_complete::generate(clap_complete::Shell::Fish, &mut command, name, &mut buffer)
        }
        CompletionShell::Powershell => clap_complete::generate(
            clap_complete::Shell::PowerShell,
            &mut command,
            name,
            &mut buffer,
        ),
    }
    std::io::stdout()
        .lock()
        .write_all(&buffer)
        .map_err(|err| CliError::general(format!("cannot write completions: {err}")))
}

/// Shell requested by name, or the one detected from the environment.
pub(crate) fn select_shell(name: Option<&str>) -> Result<Box<dyn ShellConfig>, CliError> {
    match name {
        Some(text) => from_str(text).map_err(|err| {
            CliError::usage(format!("{err}"))
                .with_hint("supported shells: bash, zsh, fish, powershell, nu, elvish, xonsh")
        }),
        None => detect().ok_or_else(|| {
            CliError::usage("cannot detect your shell")
                .with_hint("pass --shell bash|zsh|fish|powershell")
        }),
    }
}

/// Installed active version: the session's or the default. `None` if there is no valid one.
fn active_installed(ctx: &Ctx) -> Result<Option<Installed>, CliError> {
    let installed = list_installed(&ctx.store)?;
    let wanted = ctx.active_version().or_else(|| read_default(&ctx.store));
    Ok(wanted.and_then(|tag| installed.into_iter().find(|inst| inst.tag == tag)))
}

/// Points `<root>/current` to the default version, if there is one. It covers the
/// installations prior to ADR-034, which did not have the link.
pub(crate) fn refresh_current_link(ctx: &Ctx) -> Result<(), CliError> {
    let Some(tag) = read_default(&ctx.store) else {
        return Ok(());
    };
    let installed = list_installed(&ctx.store)?;
    match installed.into_iter().find(|inst| inst.tag == tag) {
        Some(inst) => state::sync_current_link(ctx, &inst),
        None => Ok(()),
    }
}

/// On Windows it adds the nvsn binary folder and `<root>\current` to the user PATH
/// in the registry, so that graphical applications can see them (ADR-034).
/// Returns `true` if anything changed.
#[cfg(windows)]
fn apply_user_path(ctx: &Ctx) -> Result<bool, CliError> {
    use nvsn_platform::user_path::{add_user_path_entries, ENVIRONMENT_KEY};

    // The registry needs absolute paths; `--dir` may be relative.
    let root =
        std::path::absolute(ctx.store.root()).unwrap_or_else(|_| ctx.store.root().to_path_buf());
    let mut entries: Vec<PathBuf> = current_bin_dir().into_iter().collect();
    entries.push(root.join("current"));
    let added = add_user_path_entries(ENVIRONMENT_KEY, &entries)
        .map_err(|err| CliError::general(format!("cannot update the user PATH: {err:#}")))?;
    Ok(!added.is_empty())
}

/// Outside Windows the user PATH does not live in the registry: nothing to do.
#[cfg(not(windows))]
fn apply_user_path(_ctx: &Ctx) -> Result<bool, CliError> {
    Ok(false)
}

/// Directory of the nvsn executable, used as a PATH entry in the profile.
fn current_bin_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
}
