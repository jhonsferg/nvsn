//! CLI definition with clap derive (ADR-001).
//!
//! The help texts are in English (i18n catalog pending). The hidden
//! subcommand `__hook` is called by the shell hook and its output is evaluated.

use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

/// Node.js version manager.
#[derive(Debug, Parser)]
#[command(
    name = "nvsn",
    version,
    about = "Node.js version manager",
    arg_required_else_help = true,
    after_help = "Exit codes: 0 ok, 1 error, 2 usage, 3 version file, 4 no binary for this platform, \
                  5 offline without cache, 6 version not found, 7 download or checksum failed, \
                  8 lock busy, 9 confirmation required (use --yes), 10 unsupported here.\n\n\
                  Default version: `nvsn default node` (alias `nvsn alias default node`) keeps the \
                  default on the newest installed version; it is resolved on every read.\n\n\
                  Settings (arch, proxy, node-mirror, npm-mirror) are stored in NVSN_DIR/config.toml; \
                  NVSN_* variables take precedence. The root cannot be stored: set NVSN_DIR instead."
)]
pub struct Cli {
    /// Options that apply to all commands.
    #[command(flatten)]
    pub global: GlobalArgs,

    /// Command to run.
    #[command(subcommand)]
    pub command: Command,
}

/// Global flags. Each one has an environment variable equivalent where applicable.
#[derive(Debug, Clone, Args)]
pub struct GlobalArgs {
    /// Print machine-readable JSON on stdout (schema_version 1).
    #[arg(long, global = true)]
    pub json: bool,

    /// Suppress status messages (results are still printed).
    #[arg(short, long, global = true)]
    pub quiet: bool,

    /// Show HTTP request and response details on stderr.
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// Disable colors. The NO_COLOR variable is also honored.
    #[arg(long, global = true)]
    pub no_color: bool,

    /// Disable the download progress indicator.
    #[arg(long, global = true)]
    pub no_progress: bool,

    /// Do not touch the network; use the cached index only.
    #[arg(long, global = true)]
    pub offline: bool,

    /// Root directory of nvsn (env: NVSN_DIR).
    #[arg(long, global = true, value_name = "PATH")]
    pub dir: Option<PathBuf>,

    /// Node.js distribution mirror (env: NVSN_NODEJS_ORG_MIRROR).
    #[arg(long, global = true, value_name = "URL")]
    pub mirror: Option<String>,

    /// Permit plain http:// URLs (insecure: checksums come from the same host).
    #[arg(long, global = true)]
    pub allow_http: bool,

    /// Answer yes to confirmations. Required when there is no terminal.
    #[arg(short, long, global = true)]
    pub yes: bool,
}

/// `--lts` (the latest LTS line) or `--lts=<name>` (a named line, such as `--lts=iron` or
/// `--lts=-1`). It replaces the explicit version: clap rejects using both with exit code 2.
#[derive(Debug, Clone, Args)]
pub struct LtsArg {
    /// Use the latest LTS release, or the LTS line named NAME.
    #[arg(
        long,
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "",
        value_name = "NAME"
    )]
    pub lts: Option<String>,
}

impl LtsArg {
    /// Specification equivalent to the flag: `lts/*` or `lts/<name>`. `None` without the flag.
    #[must_use]
    pub fn spec(&self) -> Option<String> {
        self.lts.as_deref().map(|name| {
            if name.is_empty() {
                "lts/*".to_owned()
            } else {
                format!("lts/{name}")
            }
        })
    }
}

/// Filters of `list available` and `list-remote`.
#[derive(Debug, Clone, Args)]
pub struct RemoteFilters {
    /// Only LTS releases.
    #[arg(long)]
    pub lts: bool,

    /// Only releases of this major line.
    #[arg(long, value_name = "MAJOR")]
    pub major: Option<u64>,

    /// Only the most recent release of the selection.
    #[arg(long)]
    pub latest: bool,
}

/// Subject of `list`. `available` lists the remote index instead of the installed versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ListScope {
    /// Versions published in the remote index (same as `list-remote`).
    Available,
}

/// Comandos disponibles.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Install a Node.js version. Without a version, reads .nvmrc upwards.
    ///
    /// If there is no default version, the installed one becomes the default,
    /// unless --no-use is given.
    #[command(visible_alias = "i")]
    Install {
        /// Version spec: 20, 20.11.1, lts/*, lts/iron, ^20, latest, newest.
        #[arg(conflicts_with = "lts")]
        version: Option<String>,

        #[command(flatten)]
        lts: LtsArg,

        /// Never change the default version, even when there is none.
        #[arg(long)]
        no_use: bool,

        /// Install the latest npm into the new version after the install.
        #[arg(long)]
        latest_npm: bool,

        /// Reinstall the global packages of this installed version into the new one
        /// after the install.
        #[arg(long, value_name = "VERSION")]
        reinstall_packages_from: Option<String>,
    },

    /// Remove an installed Node.js version.
    #[command(visible_aliases = ["un", "rm"])]
    Uninstall {
        /// Installed version or alias.
        #[arg(conflicts_with = "lts")]
        version: Option<String>,

        #[command(flatten)]
        lts: LtsArg,
    },

    /// Activate an installed version in the current shell (needs `nvsn init`).
    ///
    /// `latest`, `newest`, `node` and `lts` resolve to installed versions. `use x64`,
    /// `use arm64`, `use 32` or `use 64` store the architecture, as `nvsn arch <value>` does.
    Use {
        /// Installed version or alias. Without it, reads .nvmrc upwards.
        #[arg(conflicts_with = "lts")]
        version: Option<String>,

        #[command(flatten)]
        lts: LtsArg,

        /// Print no message. The activation script is still printed when integrated.
        #[arg(long)]
        silent: bool,
    },

    /// List installed versions. With `available`, list the remote index instead.
    #[command(visible_alias = "ls")]
    List {
        /// `available` lists the versions of the remote index (like `list-remote`).
        #[arg(value_enum)]
        scope: Option<ListScope>,

        #[command(flatten)]
        filters: RemoteFilters,

        /// Accepted for nvm compatibility; has no effect.
        #[arg(long = "no-colors")]
        _no_colors: bool,
    },

    /// List versions available from the remote index.
    #[command(visible_alias = "ls-remote")]
    ListRemote {
        #[command(flatten)]
        filters: RemoteFilters,

        /// Accepted for nvm compatibility; has no effect.
        #[arg(long = "no-colors")]
        _no_colors: bool,
    },

    /// Show the version active in this shell.
    Current,

    /// Set the default version. `node` keeps it on the newest installed version.
    Default {
        /// Installed version, alias, or `node` (newest installed, resolved on every read).
        version: String,
    },

    /// Create or show aliases. Without arguments, lists them.
    ///
    /// `alias default <version>` is the same as `default <version>`, and
    /// `alias default node` is the same as `default node`.
    Alias {
        /// Alias name.
        name: Option<String>,
        /// Installed version. Without it, uses the active version.
        version: Option<String>,
    },

    /// Remove an alias.
    Unalias {
        /// Alias name.
        name: String,
    },

    /// Print the path of the node binary of a version.
    Which {
        /// Installed version or alias. Without it, uses the active version.
        #[arg(conflicts_with = "lts")]
        version: Option<String>,

        #[command(flatten)]
        lts: LtsArg,
    },

    /// Print the shell code that deactivates the version of this shell.
    #[command(visible_alias = "off")]
    Deactivate {
        /// Target shell: bash, zsh, fish or powershell. Detected when omitted.
        #[arg(long, value_name = "SHELL")]
        shell: Option<String>,
    },

    /// Reactivate the shell integration: print the env script (the default version
    /// when no version is active in this session).
    On {
        /// Target shell: bash, zsh, fish or powershell. Detected when omitted.
        #[arg(long, value_name = "SHELL")]
        shell: Option<String>,
    },

    /// Internal: print the code that applies the .nvmrc of the current directory.
    #[command(name = "__hook", hide = true)]
    Hook {
        /// Shell that evaluates the output: bash, zsh, fish or powershell.
        #[arg(long, value_name = "SHELL")]
        shell: String,
    },

    /// Print the installed version that matches a spec.
    Version {
        /// Version spec or alias.
        spec: String,
    },

    /// Print the version a spec resolves to in the remote index.
    VersionRemote {
        /// Version spec.
        #[arg(conflicts_with = "lts")]
        spec: Option<String>,

        #[command(flatten)]
        lts: LtsArg,
    },

    /// Print the shell script that configures the session.
    Env {
        /// Target shell: bash, zsh, fish or powershell. Detected when omitted.
        #[arg(long, value_name = "SHELL")]
        shell: Option<String>,
    },

    /// Show, or with --apply write, the shell integration for a shell.
    Init {
        /// Target shell: bash, zsh, fish or powershell.
        shell: String,

        /// Write the block into the shell profile (asks unless --yes).
        #[arg(long)]
        apply: bool,
    },

    /// Run the node binary of an installed version. Does not change this session.
    ///
    /// With --lts there is no version argument: every argument is passed to node.
    /// Everything after the version is the child's: clap must not treat `--help`,
    /// `-h`, `--version` or `-V` among those words as nvsn's own flags, or a call
    /// such as `nvsn run 20 --help` would print nvsn's help instead of node's.
    #[command(disable_help_flag = true, disable_version_flag = true)]
    Run {
        /// Installed version or spec: 20, 20.11.1, lts/*, alias name. Omit it with --lts.
        ///
        /// `allow_hyphen_values` lets a bare `--help`/`-h`/`--version`/`-V` (with no
        /// real version before it) land here instead of failing to parse; `dispatch`
        /// recognizes those four values and shows nvsn's own help/version for `run`.
        #[arg(allow_hyphen_values = true)]
        version: Option<String>,

        #[command(flatten)]
        lts: LtsArg,

        /// Arguments passed to node unchanged. Use `--` before them if they start
        /// with a dash and clash with nvsn flags.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Run a command with the bin directory of a version first in PATH.
    ///
    /// With --lts there is no version argument: the first word is the command.
    ///
    /// Same reasoning as `Run`: the child's own `--help`/`-h`/`--version`/`-V`
    /// must reach it unchanged, not be swallowed by clap as nvsn's own flags.
    #[command(disable_help_flag = true, disable_version_flag = true)]
    Exec {
        /// Installed version or spec: 20, 20.11.1, lts/*, alias name. With --lts, the command.
        ///
        /// Same `allow_hyphen_values` reasoning as `Run::version`.
        #[arg(allow_hyphen_values = true)]
        version: Option<String>,

        /// Command to run (looked up in the PATH of the version first).
        command: Option<String>,

        #[command(flatten)]
        lts: LtsArg,

        /// Arguments passed to the command unchanged.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Print the completion script for a shell.
    Completions {
        /// Target shell.
        shell: CompletionShell,
    },

    /// Write a .nvmrc in the current directory pinning a version.
    Local {
        /// Version to pin: 20, 20.11.1, lts/*, latest. Written as plain text.
        version: String,
    },

    /// Print the bin directory of the active version, or of the given one.
    Path {
        /// Installed version or alias. Without it, uses the active version.
        version: Option<String>,
    },

    /// Show installed versions that have a newer patch release in the index.
    Outdated,

    /// Remove installed versions that nothing references.
    Prune {
        /// Only list the versions that would be removed.
        #[arg(long)]
        dry_run: bool,

        /// Remove without asking for confirmation.
        #[arg(long)]
        force: bool,

        /// Also look for .nvmrc and .node-version files under this directory (5 levels).
        #[arg(long, value_name = "PATH")]
        scan_dir: Option<PathBuf>,
    },

    /// Check the installation and report problems. Exits 1 when a check fails.
    Doctor,

    /// Update nvsn to the latest GitHub release. The SHA-256 is always verified.
    #[command(visible_alias = "upgrade")]
    SelfUpdate {
        /// Only report whether a newer release exists.
        #[arg(long)]
        check: bool,

        /// Reinstall the latest release even when nvsn is up to date.
        #[arg(long)]
        force: bool,

        /// Network retries with exponential backoff (1, 2, 4 s ...). Default 3.
        #[arg(long, value_name = "N", default_value_t = 3, value_parser = clap::value_parser!(u8).range(0..=10))]
        retries: u8,
    },

    /// Remove the nvsn binary and its data directory (NVSN_DIR).
    SelfUninstall,

    /// Remove everything nvsn manages: data directory, binary and shell profile blocks.
    Implode {
        /// Remove without asking for confirmation.
        #[arg(long)]
        force: bool,
    },

    /// Show the architecture, or store an override with a value: x64, arm64, armv7l, x86,
    /// ppc64le, s390x, riscv64, 32 (x86) or 64 (x64). `none` removes the override.
    Arch {
        /// Architecture to use. Without it, shows the effective one.
        value: Option<String>,
    },

    /// Show the HTTP(S) proxy, or store one with a URL (`none` removes it). Without a stored
    /// proxy, HTTPS_PROXY and HTTP_PROXY are used.
    Proxy {
        /// Proxy URL, or `none`.
        value: Option<String>,
    },

    /// Show the Node.js distribution mirror, or store one with a URL (`none` restores the default).
    NodeMirror {
        /// Mirror base URL.
        url: Option<String>,
    },

    /// Show the npm registry, or store one with a URL (`none` restores npm's default).
    NpmMirror {
        /// Registry URL.
        url: Option<String>,
    },

    /// Show the nvsn root directory (NVSN_DIR). A root cannot be stored: set NVSN_DIR instead.
    Root {
        /// Not accepted: the root is chosen with NVSN_DIR or --dir.
        path: Option<PathBuf>,
    },

    /// Reinstall the global npm packages of a version from another installed version.
    ReinstallPackages {
        /// Installed version that receives the packages.
        version: String,

        /// Installed version that provides the packages. Defaults to the active version.
        #[arg(long, value_name = "VERSION")]
        from: Option<String>,
    },

    /// Install the latest npm into an installed version.
    InstallLatestNpm {
        /// Installed version. Defaults to the active version.
        version: Option<String>,
    },

    /// Show or clear the download cache (NVSN_DIR/cache).
    #[command(subcommand_required = true)]
    Cache {
        #[command(subcommand)]
        action: CacheAction,
    },

    /// Not supported: nvsn does not customize colors. Use NO_COLOR or --no-color.
    SetColors {
        /// Ignored. The command always fails with exit code 10.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

/// Actions of `cache`.
#[derive(Debug, Subcommand)]
pub enum CacheAction {
    /// Print the cache directory.
    Dir,
    /// Remove every entry in the cache directory.
    Clear,
    /// List the entries of the cache directory with their sizes.
    List,
}

/// Shells for which completions are generated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CompletionShell {
    /// Bash.
    Bash,
    /// Zsh.
    Zsh,
    /// Fish.
    Fish,
    /// PowerShell.
    Powershell,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("nvsn").chain(args.iter().copied()))
    }

    #[test]
    fn lts_flag_without_value_is_the_latest_lts() {
        let cli = parse(&["use", "--lts"]).expect("valid");
        let Command::Use { lts, .. } = cli.command else {
            panic!("expected use");
        };
        assert_eq!(lts.spec().as_deref(), Some("lts/*"));
    }

    #[test]
    fn lts_flag_with_name_is_the_codename() {
        let cli = parse(&["install", "--lts=iron"]).expect("valid");
        let Command::Install { lts, .. } = cli.command else {
            panic!("expected install");
        };
        assert_eq!(lts.spec().as_deref(), Some("lts/iron"));
    }

    #[test]
    fn lts_flag_accepts_negative_offsets() {
        let cli = parse(&["which", "--lts=-1"]).expect("valid");
        let Command::Which { lts, .. } = cli.command else {
            panic!("expected which");
        };
        assert_eq!(lts.spec().as_deref(), Some("lts/-1"));
    }

    #[test]
    fn lts_and_explicit_version_conflict_with_usage_code() {
        for args in [
            vec!["install", "20", "--lts"],
            vec!["uninstall", "20", "--lts"],
            vec!["use", "20", "--lts"],
            vec!["which", "20", "--lts"],
            vec!["version-remote", "20", "--lts"],
        ] {
            let err = parse(&args).expect_err("conflict");
            assert_eq!(err.kind(), ErrorKind::ArgumentConflict, "{args:?}");
            assert_eq!(err.exit_code(), 2, "{args:?}");
        }
    }

    #[test]
    fn list_scope_and_filters_parse() {
        let cli = parse(&["list", "available", "--lts", "--major", "20"]).expect("valid");
        let Command::List { scope, filters, .. } = cli.command else {
            panic!("expected list");
        };
        assert_eq!(scope, Some(ListScope::Available));
        assert!(filters.lts);
        assert_eq!(filters.major, Some(20));
    }

    #[test]
    fn ls_and_list_remote_aliases_are_accepted() {
        assert!(matches!(
            parse(&["ls"]).expect("valid").command,
            Command::List { .. }
        ));
        assert!(matches!(
            parse(&["list"]).expect("valid").command,
            Command::List { .. }
        ));
        assert!(matches!(
            parse(&["ls-remote"]).expect("valid").command,
            Command::ListRemote { .. }
        ));
        assert!(matches!(
            parse(&["list-remote"]).expect("valid").command,
            Command::ListRemote { .. }
        ));
    }

    #[test]
    fn un_and_off_aliases_are_accepted() {
        assert!(matches!(
            parse(&["un", "20"]).expect("valid").command,
            Command::Uninstall { .. }
        ));
        assert!(matches!(
            parse(&["off"]).expect("valid").command,
            Command::Deactivate { .. }
        ));
        assert!(matches!(
            parse(&["on"]).expect("valid").command,
            Command::On { .. }
        ));
    }

    #[test]
    fn cache_requires_an_action() {
        let err = parse(&["cache"]).expect_err("action required");
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn run_with_lts_passes_every_argument_to_node() {
        // Mirrors `dispatch`: with --lts the positional `version` is the first argument.
        let cli = parse(&["run", "--lts=iron", "--", "--version"]).expect("valid");
        let Command::Run { version, lts, args } = cli.command else {
            panic!("expected run");
        };
        assert_eq!(lts.spec().as_deref(), Some("lts/iron"));
        let words: Vec<String> = version.into_iter().chain(args).collect();
        assert_eq!(
            crate::commands::run::strip_separator(&words),
            ["--version".to_owned()]
        );
    }
}
