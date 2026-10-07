//! `nvsn`: Node.js version manager. CLI entry point.
//!
//! Exit codes (full table and JSON error codes in `exit.rs`):
//! 0 ok, 1 general error, 2 usage, 3 `.nvmrc` missing or invalid, 4 no binary
//! for the platform, 5 `--offline` without cache, 6 version not found,
//! 7 download or checksum failed, 8 lock busy, 9 confirmation required
//! (`--yes`), 10 not supported here.
//!
//! Output: with `--json`, stdout contains only the versioned envelope (`schema_version`
//! 1). Without `--json`, results go to stdout and notes to stderr.

mod cli;
mod commands;
mod ctx;
mod dispatch;
mod exit;
mod output;
mod progress;
mod prompt;
mod versions;

use clap::Parser;
use cli::Cli;
use std::process::ExitCode;

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            // Help and --version exit with 0; usage errors exit with 2.
            let code = u8::try_from(err.exit_code()).unwrap_or(exit::USAGE);
            let _ = err.print();
            return ExitCode::from(code);
        }
    };

    let output = output::Output::new(cli.global.json, cli.global.quiet);
    match dispatch::run(&cli) {
        Ok(()) => ExitCode::from(exit::SUCCESS),
        Err(err) => {
            output.error(&err);
            ExitCode::from(err.exit_code())
        }
    }
}
