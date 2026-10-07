//! `run`: runs the `node` of an installed version without changing the session.
//!
//! The child process inherits stdin, stdout and stderr. Its exit code is
//! propagated as is: if the child exits with an error, the `nvsn` process exits
//! with the same code (see [`finish`]).

use crate::ctx::Ctx;
use crate::exit::CliError;
use crate::versions::{node_path, resolve_installed};
use std::process::{Command, ExitStatus, Stdio};

/// Runs the `node` of the version `version` with `args`.
///
/// # Errors
///
/// `version-not-found` (6) if the version is not installed, `usage` (2) if the
/// specification is not valid, general error if the `node` executable is missing or
/// cannot be launched. If the child exits with an error, it does not return: it exits with the
/// same code.
pub fn run_node(ctx: &Ctx, version: &str, args: &[String]) -> Result<(), CliError> {
    let inst = resolve_installed(ctx, version)?;
    let node = node_path(&inst.dir).ok_or_else(|| {
        CliError::general(format!("{} has no node executable", inst.tag)).with_hint(format!(
            "reinstall it: `nvsn uninstall {0}` and `nvsn install {0}`",
            inst.tag.trim_start_matches('v')
        ))
    })?;

    let status = Command::new(&node)
        .args(strip_separator(args))
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|err| CliError::general(format!("cannot run {}: {err}", node.display())))?;
    finish(status);
    Ok(())
}

/// Removes the `--` that clap leaves at the start of the trailing arguments, if present.
///
/// Only the first one is removed: a later `--` belongs to the child command.
pub fn strip_separator(args: &[String]) -> &[String] {
    match args.first() {
        Some(first) if first == "--" => &args[1..],
        _ => args,
    }
}

/// If the child exited with an error, exits `nvsn` with the same exit code.
///
/// The exit uses `std::process::exit`, because [`CliError`] only admits the
/// fixed codes of the table in `exit.rs`. Nothing is left pending to write:
/// `nvsn` does not print anything before launching the child.
pub fn finish(status: ExitStatus) {
    if !status.success() {
        std::process::exit(child_exit_code(status));
    }
}

/// Code to propagate: the child's, or `128 + signal` if a signal killed it (Unix).
fn child_exit_code(status: ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return 128 + signal;
        }
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_one_leading_separator() {
        let args: Vec<String> = ["--", "a", "--"].iter().map(|s| (*s).to_owned()).collect();
        assert_eq!(strip_separator(&args), &args[1..]);
    }

    #[test]
    fn keeps_arguments_without_separator() {
        let args: Vec<String> = ["a", "--"].iter().map(|s| (*s).to_owned()).collect();
        assert_eq!(strip_separator(&args), args.as_slice());
    }

    #[test]
    fn empty_arguments_stay_empty() {
        assert!(strip_separator(&[]).is_empty());
    }
}
