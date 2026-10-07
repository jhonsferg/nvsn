//! Dispatches a `Command` to the function of the matching command.

use clap::CommandFactory;

use crate::cli::{Cli, Command, LtsArg};
use crate::commands::install::InstallFlags;
use crate::commands::{
    activate, cache, doctor, exec, implode, install, list, local, outdated, packages, path, prune,
    run, self_uninstall, self_update, settings, shell, state, update_check,
};
use crate::ctx::Ctx;
use crate::exit::CliError;

/// Runs the requested command. `completions` does not need the context (it does not touch the disk).
///
/// After the command, a notice about a new version may be printed (see `update_check`).
/// It never changes the result of the command.
///
/// # Errors
///
/// Returns the command's `CliError`; `main` shows it and uses its exit code.
pub fn run(cli: &Cli) -> Result<(), CliError> {
    self_update::remove_stale_old();
    if let Command::Completions { shell } = &cli.command {
        return shell::completions(*shell);
    }
    if let Some(code) = help_or_version_sentinel(&cli.command) {
        std::process::exit(code.into());
    }
    let ctx = Ctx::new(&cli.global)?;
    let notice = update_check::spawn(&ctx, &cli.command);
    let result = dispatch(&ctx, &cli.command);
    update_check::print_if_ready(notice);
    result
}

/// `run` and `exec` disable clap's own `--help`/`-h`/`--version`/`-V` so those reach
/// node/the child when a real version precedes them (`nvsn run 20 --help`). But with
/// no `--lts` and no real version, one of those four landing alone in the version
/// slot (`allow_hyphen_values` on that field lets it parse instead of failing) means
/// the user wants nvsn's own help or version for the subcommand, not a forward to a
/// child that was never identified. Returns the exit code to use in that case.
fn help_or_version_sentinel(command: &Command) -> Option<u8> {
    let (name, version, lts) = match command {
        Command::Run { version, lts, .. } => ("run", version.as_deref(), lts),
        Command::Exec { version, lts, .. } => ("exec", version.as_deref(), lts),
        _ => return None,
    };
    if lts.spec().is_some() {
        // Every word, including a leading --help, is forwarded to the child.
        return None;
    }
    match version {
        Some("--help" | "-h") => {
            let mut cmd = Cli::command();
            if let Some(sub) = cmd.find_subcommand_mut(name) {
                let _ = sub.print_help();
            }
            Some(crate::exit::SUCCESS)
        }
        Some("--version" | "-V") => {
            print!("{}", Cli::command().render_version());
            Some(crate::exit::SUCCESS)
        }
        _ => None,
    }
}

/// Version input of a command that accepts a version or `--lts`: the LTS spec, the
/// explicit version, or nothing. Clap rejects both at once.
fn target(version: Option<&str>, lts: &LtsArg) -> Option<String> {
    lts.spec().or_else(|| version.map(str::to_owned))
}

/// Like [`target`], for commands that need one.
fn required(version: Option<&str>, lts: &LtsArg, command: &str) -> Result<String, CliError> {
    target(version, lts).ok_or_else(|| {
        CliError::usage(format!("{command} needs a version or --lts")).with_hint(format!(
            "e.g. `nvsn {command} 20` or `nvsn {command} --lts`"
        ))
    })
}

/// Version argument of `run` or `exec` that is required when there is no `--lts`.
fn explicit<'a>(version: Option<&'a str>, command: &str) -> Result<&'a str, CliError> {
    version.ok_or_else(|| {
        CliError::usage(format!("{command} needs a version or --lts")).with_hint(format!(
            "e.g. `nvsn {command} 20 ...` or `nvsn {command} --lts ...`"
        ))
    })
}

fn dispatch(ctx: &Ctx, command: &Command) -> Result<(), CliError> {
    match command {
        Command::Install {
            version,
            lts,
            no_use,
            latest_npm,
            reinstall_packages_from,
        } => {
            let flags = InstallFlags {
                no_use: *no_use,
                latest_npm: *latest_npm,
                reinstall_from: reinstall_packages_from.clone(),
            };
            let input = target(version.as_deref(), lts);
            install::install(ctx, input.as_deref(), &flags)
        }
        Command::Uninstall { version, lts } => {
            install::uninstall(ctx, &required(version.as_deref(), lts, "uninstall")?)
        }
        Command::Use {
            version,
            lts,
            silent,
        } => {
            let input = target(version.as_deref(), lts);
            activate::use_version(ctx, input.as_deref(), *silent)
        }
        Command::List {
            scope,
            filters,
            _no_colors: _,
        } => list::list_command(ctx, *scope, filters),
        Command::ListRemote {
            filters,
            _no_colors: _,
        } => list::list_remote(ctx, filters),
        Command::Current => state::current(ctx),
        Command::Default { version } => state::set_default(ctx, version),
        Command::Alias { name, version } => state::alias(ctx, name.as_deref(), version.as_deref()),
        Command::Unalias { name } => state::unalias(ctx, name),
        Command::Which { version, lts } => {
            let input = target(version.as_deref(), lts);
            state::which(ctx, input.as_deref())
        }
        Command::Deactivate { shell } => activate::deactivate(ctx, shell.as_deref()),
        Command::On { shell } => shell::env(ctx, shell.as_deref()),
        Command::Hook { shell } => activate::hook(ctx, shell),
        Command::Version { spec } => state::version(ctx, spec),
        Command::VersionRemote { spec, lts } => {
            list::version_remote(ctx, &required(spec.as_deref(), lts, "version-remote")?)
        }
        Command::Run { version, lts, args } => match lts.spec() {
            // With --lts every argument belongs to node: there is no version argument.
            Some(spec) => {
                let words: Vec<String> = version
                    .iter()
                    .cloned()
                    .chain(args.iter().cloned())
                    .collect();
                run::run_node(ctx, &spec, &words)
            }
            None => run::run_node(ctx, explicit(version.as_deref(), "run")?, args),
        },
        Command::Exec {
            version,
            command,
            lts,
            args,
        } => match lts.spec() {
            // With --lts the words are the command and its arguments.
            Some(spec) => {
                let words: Vec<String> = version
                    .iter()
                    .chain(command.iter())
                    .cloned()
                    .chain(args.iter().cloned())
                    .collect();
                let (name, rest) = words.split_first().ok_or_else(|| {
                    CliError::usage("exec needs a command after --lts")
                        .with_hint("e.g. `nvsn exec --lts npm --version`")
                })?;
                exec::exec_command(ctx, &spec, name, rest)
            }
            None => {
                let version = explicit(version.as_deref(), "exec")?;
                let name = command.as_deref().ok_or_else(|| {
                    CliError::usage("exec needs a command")
                        .with_hint("e.g. `nvsn exec 20 node --version`")
                })?;
                exec::exec_command(ctx, version, name, args)
            }
        },
        Command::Env { shell } => shell::env(ctx, shell.as_deref()),
        Command::Init { shell, apply } => shell::init(ctx, shell.as_deref(), *apply),
        Command::Completions { shell } => shell::completions(*shell),
        Command::Local { version } => local::local(ctx, version),
        Command::Path { version } => path::path(ctx, version.as_deref()),
        Command::Outdated => outdated::outdated(ctx),
        Command::Prune {
            dry_run,
            force,
            scan_dir,
        } => prune::prune(ctx, *dry_run, *force, scan_dir.as_deref()),
        Command::Doctor => doctor::doctor(ctx),
        Command::SelfUpdate {
            check,
            force,
            retries,
        } => self_update::run(ctx, *check, *force, *retries),
        Command::SelfUninstall => self_uninstall::run(ctx),
        Command::Implode { force } => implode::run(ctx, *force),
        Command::Arch { value } => settings::arch(ctx, value.as_deref()),
        Command::Proxy { value } => settings::proxy(ctx, value.as_deref()),
        Command::NodeMirror { url } => settings::node_mirror(ctx, url.as_deref()),
        Command::NpmMirror { url } => settings::npm_mirror(ctx, url.as_deref()),
        Command::Root { path } => settings::root(ctx, path.as_deref()),
        Command::SetColors { .. } => settings::set_colors(),
        Command::ReinstallPackages { version, from } => {
            packages::reinstall_packages(ctx, version, from.as_deref())
        }
        Command::InstallLatestNpm { version } => {
            packages::install_latest_npm(ctx, version.as_deref())
        }
        Command::Cache { action } => cache::run(ctx, action),
    }
}
