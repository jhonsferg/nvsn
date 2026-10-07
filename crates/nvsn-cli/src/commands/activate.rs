//! `use`, `deactivate` and `__hook`.
//!
//! A process cannot change the environment of its parent. With shell integration
//! (`NVSN_INTEGRATION=1`, set by the function that `nvsn init` generates), the
//! binary prints a script to stdout and the wrapper applies it with `eval`.
//! Without integration, `use` validates and explains, without changing anything.

use crate::commands::settings;
use crate::commands::shell::select_shell;
use crate::ctx::Ctx;
use crate::exit::CliError;
use crate::versions::{bin_dir, parse_input, read_nvmrc, resolve_installed_spec};
use nvsn_core::nvmrc::find_version_file;
use nvsn_shell::{detect, from_str};
use serde_json::json;

/// Activates an installed version. Without `input` it reads `.nvmrc` upwards.
///
/// `input` may also be an architecture word (`x64`, `arm64`, `32`, `64`, ...): then the
/// command changes the architecture instead (see `settings::use_arch`).
///
/// With `silent`, no message is printed. The activation script is still printed when
/// the shell integration is on, because the wrapper needs it.
///
/// # Errors
///
/// `version-file-missing` (3), `version-not-found` (6), `usage` (2), or a general
/// error if the integration cannot detect the shell.
pub fn use_version(ctx: &Ctx, input: Option<&str>, silent: bool) -> Result<(), CliError> {
    if let Some(text) = input {
        if settings::arch_word(text).is_some() {
            return settings::use_arch(ctx, text);
        }
    }
    let (raw, spec) = match input {
        Some(text) => (text.to_owned(), parse_input(text)?),
        None => {
            let file = read_nvmrc(ctx)?;
            if !silent {
                ctx.out
                    .note(format!("Using {} from {}", file.raw, file.path.display()));
            }
            (file.raw.clone(), file.spec)
        }
    };
    let inst = resolve_installed_spec(ctx, &spec, &raw)?;
    let bin = bin_dir(&inst.dir);

    if ctx.integration {
        let shell = detect_shell()?;
        // pin = true: an explicit `use` must not be undone by a directory change.
        ctx.out
            .raw(&shell.shell_version_script(&inst.tag, &bin, true));
        if !silent {
            ctx.out.note(format!("Now using {}", inst.tag));
        }
        return Ok(());
    }

    if silent {
        return Ok(());
    }
    ctx.out.result(
        "use",
        &format!(
            "{} is installed. Changing this session needs shell integration: run `nvsn init <shell>` once, then `nvsn use` again.",
            inst.tag
        ),
        json!({ "version": inst.tag, "activated": false }),
    );
    Ok(())
}

/// Code that the shell hook evaluates when the directory changes (`nvsn __hook --shell <sh>`).
///
/// If the directory (or an ancestor up to the home) has `.nvmrc` or `.node-version`
/// and that version is installed, it prints the code that activates it without pinning it
/// (`NVSN_SHELL_VERSION` is not marked). In any other case it prints nothing:
/// no file, an invalid file or a version that is not installed. The hook runs
/// at every prompt, so it must not pollute the output or fail.
///
/// # Errors
///
/// `usage` (2) only if `shell_name` is not a supported shell.
pub fn hook(ctx: &Ctx, shell_name: &str) -> Result<(), CliError> {
    let shell = select_shell(Some(shell_name))?;
    let Ok(cwd) = std::env::current_dir() else {
        return Ok(());
    };
    let home = ctx.home();
    let file = match find_version_file(&cwd, home.as_deref()) {
        Ok(Some(file)) => file,
        Ok(None) | Err(_) => return Ok(()),
    };
    let Ok(inst) = resolve_installed_spec(ctx, &file.spec, &file.raw) else {
        return Ok(());
    };
    let bin = bin_dir(&inst.dir);
    ctx.out
        .raw(&shell.shell_version_script(&inst.tag, &bin, false));
    Ok(())
}

/// Deactivates the version of the current session: prints the code that removes
/// `NVSN_VERSION`, `NVSN_SHELL_VERSION` and the nvsn entry in `PATH`.
///
/// With `shell` it uses that syntax. Without it, it uses the detected shell and, if
/// none is detected, a generic POSIX output.
///
/// # Errors
///
/// `usage` (2) if `shell` is not a supported shell.
pub fn deactivate(ctx: &Ctx, shell: Option<&str>) -> Result<(), CliError> {
    let config = match shell {
        Some(text) => select_shell(Some(text))?,
        None => match detect() {
            Some(found) => found,
            None => from_str("sh").map_err(|err| CliError::general(format!("{err}")))?,
        },
    };
    ctx.out.raw(config.deactivate_script());
    Ok(())
}

/// Shell of the current session, used to generate the activation script.
fn detect_shell() -> Result<Box<dyn nvsn_shell::ShellConfig>, CliError> {
    detect().ok_or_else(|| {
        CliError::general("cannot detect the shell for activation")
            .with_hint("run `nvsn env --shell <bash|zsh|fish|powershell>` or `nvsn init <shell>`")
    })
}
