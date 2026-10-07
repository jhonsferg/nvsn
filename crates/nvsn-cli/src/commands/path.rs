//! `path`: prints the `bin` directory of the active version or of the given one.
//!
//! The output is only the path, without decoration, for use in scripts:
//!
//! ```sh
//! export PATH="$(nvsn path):$PATH"
//! ```
//!
//! It does not touch the network: it resolves against the installed versions and the index cache.

use crate::ctx::Ctx;
use crate::exit::CliError;
use crate::versions::{bin_dir, effective_version, resolve_installed};
use serde_json::json;

/// Prints the `bin` directory of `input`, or of the active version if none is given.
///
/// # Errors
///
/// `version-not-found` (6) if there is no active version or the given one is not
/// installed. `usage` (2) if the specification is not valid.
pub fn path(ctx: &Ctx, input: Option<&str>) -> Result<(), CliError> {
    let inst = match input {
        Some(text) => resolve_installed(ctx, text)?,
        None => {
            let (tag, _) = effective_version(ctx).ok_or_else(|| {
                CliError::not_found("no version is active and no default is set")
                    .with_hint("pass a version, or set a default with `nvsn default <version>`")
            })?;
            resolve_installed(ctx, &tag)?
        }
    };
    let dir = bin_dir(&inst.dir).display().to_string();
    ctx.out
        .result("path", &dir, json!({ "version": inst.tag, "path": dir }));
    Ok(())
}
