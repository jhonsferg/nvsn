//! Human and JSON output (ARCHITECTURE 3.5 and 7.4).
//!
//! Rules:
//! - With `--json`, stdout contains only the JSON envelope. Notes go to stderr
//!   only in human mode.
//! - Human results go to stdout; progress notes go to stderr.
//! - Every JSON output carries `schema_version`. This version starts at 1.

use crate::exit::CliError;
use colored::Colorize;
use serde_json::{json, Value};
use std::io::Write;

/// JSON schema version (ADR-012).
pub const SCHEMA_VERSION: u64 = 1;

/// Presentation of results and errors according to the global flags.
#[derive(Debug, Clone, Copy)]
pub struct Output {
    json: bool,
    quiet: bool,
}

impl Output {
    /// Creates the presenter.
    pub fn new(json: bool, quiet: bool) -> Self {
        Self { json, quiet }
    }

    /// Prints the result of a command.
    ///
    /// In human mode it writes `human` to stdout (if not empty). In JSON mode
    /// it writes the envelope `{schema_version, ok, command, data}`.
    pub fn result(&self, command: &str, human: &str, data: Value) {
        let mut out = std::io::stdout().lock();
        if self.json {
            let envelope = json!({
                "schema_version": SCHEMA_VERSION,
                "ok": true,
                "command": command,
                "data": data,
            });
            let _ = writeln!(out, "{envelope}");
        } else if !human.is_empty() {
            let _ = writeln!(out, "{human}");
        }
    }

    /// Status note on stderr. Omitted in JSON and with `--quiet`.
    pub fn note(&self, message: impl AsRef<str>) {
        if !self.json && !self.quiet {
            let mut err = std::io::stderr().lock();
            let _ = writeln!(err, "{}", message.as_ref());
        }
    }

    /// Writes literal text to stdout, without a JSON envelope or an extra newline.
    /// Used by the commands whose output is a script for `eval`.
    pub fn raw(&self, text: &str) {
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(text.as_bytes());
    }

    /// Writes an error: JSON on stdout, or cause and hint on stderr.
    pub fn error(&self, err: &CliError) {
        if self.json {
            let mut error = json!({ "code": err.code(), "message": err.message });
            if let Some(hint) = &err.hint {
                error["hint"] = Value::String(hint.clone());
            }
            let envelope = json!({
                "schema_version": SCHEMA_VERSION,
                "ok": false,
                "error": error,
            });
            let mut out = std::io::stdout().lock();
            let _ = writeln!(out, "{envelope}");
        } else {
            let mut stderr = std::io::stderr().lock();
            let _ = writeln!(stderr, "{} {}", "error:".red().bold(), err.message);
            if let Some(hint) = &err.hint {
                let _ = writeln!(stderr, "{} {}", "hint:".cyan(), hint);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_version_is_one() {
        assert_eq!(SCHEMA_VERSION, 1);
    }
}
