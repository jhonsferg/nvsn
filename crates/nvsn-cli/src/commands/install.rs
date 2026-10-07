//! `install` and `uninstall`.

use crate::commands::packages;
use crate::commands::shell::refresh_current_link;
use crate::ctx::Ctx;
use crate::exit::{CliError, ErrorKind};
use crate::progress::ByteBar;
use crate::prompt::confirm;
use crate::versions::{
    default_is_dynamic, parse_input, read_default, read_nvmrc, resolve_installed, resolve_remote,
    write_default,
};
use nvsn_core::artifact::artifact_for_release;
use nvsn_core::install::{install_version_with_progress, InstallOptions, DEFAULT_LOCK_TIMEOUT};
use nvsn_core::lock::FileLock;
use nvsn_platform::Os;
use serde_json::json;

/// Options of `install` that go beyond the download.
#[derive(Debug, Clone, Default)]
pub struct InstallFlags {
    /// Do not change the default version (`--no-use`).
    pub no_use: bool,
    /// Install the latest npm afterwards (`--latest-npm`).
    pub latest_npm: bool,
    /// Installed version whose global packages are reinstalled afterwards.
    pub reinstall_from: Option<String>,
}

/// Installs a remote version. Without `requested` it reads `.nvmrc` upwards.
///
/// If there is no default version, the installed one becomes the default, unless
/// `flags.no_use` is set. When the default is `node`, it keeps following the newest version.
///
/// # Errors
///
/// `version-file-missing` (3) without `.nvmrc`, `version-not-found` (6), `no-artifact` (4),
/// `download-failed` or `checksum-mismatch` (7), `lock-busy` (8) and `usage` (2).
pub fn install(ctx: &Ctx, requested: Option<&str>, flags: &InstallFlags) -> Result<(), CliError> {
    // Checked before any download, so that a bad option does not leave a new version behind.
    let reinstall_source = match &flags.reinstall_from {
        Some(from) => Some(resolve_installed(ctx, from)?.tag),
        None => None,
    };

    let (input, spec) = match requested {
        Some(text) => (text.to_owned(), parse_input(text)?),
        None => {
            let file = read_nvmrc(ctx)?;
            ctx.out
                .note(format!("Using {} from {}", file.raw, file.path.display()));
            (file.raw.clone(), file.spec)
        }
    };
    if !spec.is_remote() {
        return Err(CliError::usage(format!("'{input}' cannot be installed"))
            .with_hint("install a version number or an lts/* spec, e.g. `nvsn install 20`"));
    }

    let platform = ctx.platform()?;
    let releases = ctx.load_index()?;
    let release = resolve_remote(&releases, &spec, &input)?;
    let tag = release.tag();

    if let Err(err) = artifact_for_release(release, platform) {
        let error = if platform.os == Os::Android {
            CliError::new(ErrorKind::Unsupported, format!("{err:#}")).with_hint(
                "Node has no official Android binaries; install it with Termux: `pkg install nodejs-lts`",
            )
        } else {
            CliError::new(ErrorKind::NoArtifact, format!("{err:#}"))
                .with_hint("run `nvsn list-remote` to see the versions published for this platform")
        };
        return Err(error);
    }

    ctx.ensure_store()?;
    let http = ctx.http()?;
    let options = InstallOptions {
        mirror: ctx.mirror.clone(),
        lock_timeout: DEFAULT_LOCK_TIMEOUT,
        allow_http: ctx.allow_http,
    };

    let bar = ByteBar::new(ctx.show_progress, format!("Downloading {tag}"));
    let outcome =
        install_version_with_progress(&http, &ctx.store, release, platform, &options, &bar);
    bar.finish();
    let report = outcome.map_err(CliError::classify)?;

    let default_set = if flags.no_use {
        false
    } else {
        adopt_default(ctx, &tag)?
    };

    let path = report.path.display().to_string();
    let data = json!({
        "version": report.version,
        "path": path,
        "already_installed": report.already_installed,
        "default_set": default_set,
    });
    if report.already_installed {
        ctx.out
            .result("install", &format!("{tag} is already installed"), data);
    } else {
        ctx.out
            .result("install", &format!("Installed {tag} at {path}"), data);
        if default_set {
            ctx.out.note(format!("{tag} is now the default version."));
        } else {
            ctx.out.note(format!(
                "Activate it with: nvsn use {}",
                tag.trim_start_matches('v')
            ));
        }
    }

    // The version is installed at this point: a failure below leaves it in place.
    let retry =
        |command: &str| format!("the version is installed; retry with `nvsn {command} {tag}`");
    if flags.latest_npm {
        packages::upgrade_npm(ctx, &tag)
            .map_err(|err| err.with_hint(retry("install-latest-npm")))?;
    }
    if let Some(source) = &reinstall_source {
        packages::copy_packages(ctx, source, &tag)
            .map_err(|err| err.with_hint(retry("reinstall-packages")))?;
    }
    Ok(())
}

/// Makes `tag` the default when there is none, and keeps the `node` default pointing at
/// the newest version. Returns `true` if `tag` became the default.
///
/// A failure of the `current` link is a warning: the installation already succeeded.
fn adopt_default(ctx: &Ctx, tag: &str) -> Result<bool, CliError> {
    let became_default = read_default(&ctx.store).is_none();
    if became_default {
        write_default(ctx, tag)?;
    }
    if became_default || default_is_dynamic(&ctx.store) {
        if let Err(err) = refresh_current_link(ctx) {
            ctx.out.note(format!(
                "warning: the default could not be linked: {}",
                err.message
            ));
        }
    }
    Ok(became_default)
}

/// Removes an installed version. Asks for confirmation if it is the default or the active one.
///
/// # Errors
///
/// `version-not-found` (6), `confirmation-required` (9) without a TTY and without `--yes`,
/// `lock-busy` (8) and a general error if the files cannot be deleted.
pub fn uninstall(ctx: &Ctx, input: &str) -> Result<(), CliError> {
    let inst = resolve_installed(ctx, input)?;
    let is_default = read_default(&ctx.store).as_deref() == Some(inst.tag.as_str());
    let follows_newest = default_is_dynamic(&ctx.store);
    let is_active = ctx.active_version().as_deref() == Some(inst.tag.as_str());

    if is_default || is_active {
        let mut reasons = Vec::new();
        if is_default {
            reasons.push("the default version");
        }
        if is_active {
            reasons.push("active in this session");
        }
        let question = format!("{} is {}. Remove it?", inst.tag, reasons.join(" and "));
        if !confirm(ctx, &question)? {
            ctx.out.result(
                "uninstall",
                "Cancelled.",
                json!({ "version": inst.tag, "removed": false }),
            );
            return Ok(());
        }
    }

    ctx.ensure_store()?;
    let lock_path = ctx.store.locks_dir().join(format!("{}.lock", inst.tag));
    let _lock = FileLock::lock(&lock_path).map_err(CliError::classify)?;

    // The directory is first moved aside (it disappears from the list) and then deleted.
    let trash =
        ctx.store
            .versions_dir()
            .join(format!(".trash-{}-{}", inst.tag, std::process::id()));
    std::fs::rename(&inst.dir, &trash).map_err(|err| {
        CliError::general(format!("cannot remove {}: {err}", inst.tag))
            .with_hint("close the programs that run this version (node processes) and retry")
    })?;
    std::fs::remove_dir_all(&trash).map_err(|err| {
        CliError::general(format!(
            "{} was removed from the list but its files could not be deleted: {err}",
            inst.tag
        ))
        .with_hint(format!("delete {} by hand", trash.display()))
    })?;

    if is_default && follows_newest {
        // `default node` keeps working: it now resolves to the next newest version.
        if let Err(err) = refresh_current_link(ctx) {
            ctx.out.note(format!(
                "warning: the default could not be relinked: {}",
                err.message
            ));
        }
    } else if is_default {
        match std::fs::remove_file(ctx.store.default_file()) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(CliError::general(format!(
                    "cannot clear the default version: {err}"
                )))
            }
        }
    }

    ctx.out.result(
        "uninstall",
        &format!("Removed {}", inst.tag),
        json!({ "version": inst.tag, "removed": true, "was_default": is_default }),
    );
    Ok(())
}
