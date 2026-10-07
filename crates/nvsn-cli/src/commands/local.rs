//! `local`: pins the Node version of the current directory by writing `.nvmrc`.
//!
//! The file contains only the specification as plain text (`20.11.1`,
//! `lts/*`, `latest`), without a `v` prefix, so that it can be versioned and read by
//! `nvsn use`, `nvsn install` and `nvsn prune`. If the version is not installed
//! a warning is shown, but the file is written anyway.

use crate::ctx::Ctx;
use crate::exit::CliError;
use crate::versions::{parse_input, resolve_installed};
use nvsn_core::spec::VersionSpec;
use serde_json::json;

/// Name of the file that the command writes.
pub const NVMRC: &str = ".nvmrc";

/// Writes `.nvmrc` in the current directory with `input`.
///
/// # Errors
///
/// `usage` (2) if the specification is not valid or cannot be pinned
/// (`system`, `current`). `general` (1) if the file cannot be written.
pub fn local(ctx: &Ctx, input: &str) -> Result<(), CliError> {
    let spec = parse_input(input)?;
    if matches!(spec, VersionSpec::System | VersionSpec::Current) {
        return Err(
            CliError::usage(format!("'{input}' cannot be pinned in {NVMRC}"))
                .with_hint("pass a version such as 20, 20.11.1, lts/* or latest"),
        );
    }

    let text = pin_text(input);
    let installed = resolve_installed(ctx, input).is_ok();
    if !installed {
        ctx.out.note(format!(
            "warning: {text} is not installed yet. Run `nvsn install {text}` first."
        ));
    }

    let cwd = std::env::current_dir()
        .map_err(|err| CliError::general(format!("cannot read the current directory: {err}")))?;
    let file = cwd.join(NVMRC);
    std::fs::write(&file, format!("{text}\n")).map_err(|err| {
        CliError::general(format!("cannot write {}: {err}", file.display()))
            .with_hint("check write permissions for the current directory")
    })?;

    ctx.out.result(
        "local",
        &format!("Local version set to {text} ({NVMRC})"),
        json!({
            "version": text,
            "file": file.display().to_string(),
            "installed": installed,
        }),
    );
    Ok(())
}

/// Text written to `.nvmrc`: the specification without the `v` prefix of
/// an exact version (`v20.11.1` -> `20.11.1`). Anything else is written as is.
pub fn pin_text(input: &str) -> String {
    let trimmed = input.trim();
    match trimmed.strip_prefix('v') {
        Some(rest) if rest.starts_with(|c: char| c.is_ascii_digit()) => rest.to_owned(),
        _ => trimmed.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::pin_text;

    #[test]
    fn strips_the_v_prefix_of_exact_versions() {
        assert_eq!(pin_text("v20.11.1"), "20.11.1");
        assert_eq!(pin_text("20.11.1"), "20.11.1");
    }

    #[test]
    fn keeps_specs_that_are_not_tags() {
        assert_eq!(pin_text("latest"), "latest");
        assert_eq!(pin_text("lts/*"), "lts/*");
        assert_eq!(pin_text(" 20 "), "20");
    }
}
