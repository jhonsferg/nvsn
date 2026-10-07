//! Confirmations. They only ask in a TTY; with `--yes` they do not ask.
//!
//! Without a TTY and without `--yes` the answer is not guessed: the command fails with
//! code 9 and says how to continue.

use crate::ctx::Ctx;
use crate::exit::{CliError, ErrorKind};
use std::io::{BufRead, Write};

/// Asks for confirmation. Returns `Ok(true)` if the user accepts or if `--yes` was given.
///
/// # Errors
///
/// Returns `confirmation-required` (code 9) if there is no TTY and `--yes` was not passed.
pub fn confirm(ctx: &Ctx, question: &str) -> Result<bool, CliError> {
    if ctx.yes {
        return Ok(true);
    }
    if !ctx.interactive {
        return Err(CliError::new(
            ErrorKind::ConfirmationRequired,
            format!("{question} (no terminal to ask on)"),
        )
        .with_hint("pass --yes to confirm in scripts"));
    }
    let mut stderr = std::io::stderr().lock();
    let _ = write!(stderr, "{question} [y/N] ");
    let _ = stderr.flush();
    drop(stderr);

    let mut answer = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut answer)
        .map_err(|err| CliError::general(format!("cannot read the answer: {err}")))?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}
