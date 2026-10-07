//! Shell integration: the [`ShellConfig`] trait and one implementation per shell.
//!
//! Each supported shell is a field-less type ([`Bash`], [`Zsh`], [`Fish`],
//! [`PowerShell`]) that implements [`ShellConfig`]. Adding a shell requires a
//! new file and a new variant, without touching the rest (open/closed principle).
//!
//! Text generation is pure. The functions that touch profile files
//! ([`inject_profile`], [`inject_login_profile`], [`strip_profile`]) receive
//! the home directory as a parameter and do not resolve it on their own.

mod bash;
mod fish;
mod posix;
mod powershell;
pub(crate) mod quote;
mod zsh;

use crate::profile;
use std::fmt;
use std::path::{Path, PathBuf};

/// Errors of the shell layer.
#[derive(Debug)]
pub enum ShellError {
    /// The name does not match any supported shell.
    UnknownShell(String),
    /// The shell does not define a profile path.
    NoProfilePath(&'static str),
    /// I/O error on a profile file.
    Io {
        /// Affected file.
        path: PathBuf,
        /// Original system error.
        source: std::io::Error,
    },
}

impl fmt::Display for ShellError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownShell(s) => write!(
                f,
                "Unknown shell '{s}'. Supported: powershell, bash, zsh, fish"
            ),
            Self::NoProfilePath(name) => write!(f, "Cannot determine profile path for {name}"),
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
        }
    }
}

impl std::error::Error for ShellError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Context received by [`ShellConfig::env_script`] to generate the session
/// initialization script.
pub struct EnvContext<'a> {
    /// nvsn root directory (value of `NVSN_DIR`).
    pub nvsn_dir: &'a Path,

    /// Directory that contains the `node` executable of the active version
    /// (`bin/` on Unix; version root on Windows). `None` if there is no
    /// active version.
    pub active_bin: Option<&'a Path>,

    /// Active Node.js version (e.g. `v20.11.1`), exported as
    /// `NVSN_VERSION`. `None` if there is no active version.
    pub node_version: Option<&'a str>,
}

/// Behavior that each supported shell must implement.
///
/// The trait is object-safe and is used as `Box<dyn ShellConfig>` or
/// `&dyn ShellConfig`.
pub trait ShellConfig: std::fmt::Debug {
    /// Short lowercase identifier (e.g. `"bash"`, `"powershell"`).
    fn name(&self) -> &'static str;

    /// Generates the script that defines `NVSN_DIR`, `NVSN_VERSION`, `PATH` and
    /// installs the directory-change hook.
    fn env_script(&self, ctx: &EnvContext<'_>) -> String;

    /// Path of the user profile file where the initialization line is added,
    /// or `None` if the shell has none. `home` is the user's home directory.
    fn profile_path(&self, home: &Path) -> Option<PathBuf>;

    /// Additional profiles where `init --apply` writes the same block. By
    /// default there are none.
    fn extra_profile_paths(&self, _home: &Path) -> Vec<PathBuf> {
        Vec::new()
    }

    /// Path of the login profile where the PATH of the default version is set,
    /// visible to graphical applications and non-interactive login shells.
    /// `None` if the shell has no separate login profile or if the Windows
    /// registry already sets the PATH.
    fn login_profile_path(&self, _home: &Path) -> Option<PathBuf> {
        None
    }

    /// Line added to the profile so that `nvsn env` is evaluated in every
    /// new session.
    fn init_line(&self) -> &'static str;

    /// Function (or alias) that wraps the `nvsn` binary. After `use`,
    /// `deactivate`, `on` or `off` it applies the snippet printed by the binary
    /// (with `NVSN_INTEGRATION=1`) to change the environment of the current session.
    fn wrapper_function(&self) -> &'static str;

    /// Minimal script that activates `version` for the session (`NVSN_VERSION`,
    /// `NVSN_PATH_ENTRY` and `PATH`). If `pin` is `true`, it also marks
    /// `NVSN_SHELL_VERSION`, which makes the hook not change the version when
    /// the directory changes.
    fn shell_version_script(&self, version: &str, bin_dir: &Path, pin: bool) -> String;

    /// Script that removes the `NVSN_SHELL_VERSION` mark and re-evaluates the hook
    /// to restore the version that corresponds to the current directory.
    fn shell_unset_script(&self) -> &'static str;

    /// Code that `nvsn deactivate --shell <sh>` prints to remove the version from
    /// the session: `NVSN_VERSION`, `NVSN_SHELL_VERSION`, `NVSN_AUTO` and the nvsn
    /// `PATH` entry (`NVSN_PATH_ENTRY`).
    fn deactivate_script(&self) -> &'static str;

    /// Name of the executable that indicates whether the shell is installed. By
    /// default it is [`ShellConfig::name`].
    fn binary_name(&self) -> &'static str {
        self.name()
    }

    /// `true` if the initialization block must prepend an `export PATH`
    /// with the nvsn directory.
    ///
    /// Needed in bash and zsh: on Linux/macOS the login profile loads the
    /// interactive profile before adding `~/.local/bin` to the PATH, so
    /// `nvsn env` would not be found on every SSH login.
    fn needs_bin_path_in_init(&self) -> bool {
        false
    }
}

// --- Concrete implementations ------------------------------------------------

/// Bash.
#[derive(Debug)]
pub struct Bash;
/// Zsh.
#[derive(Debug)]
pub struct Zsh;
/// Fish.
#[derive(Debug)]
pub struct Fish;
/// PowerShell (5.1 and 7+).
#[derive(Debug)]
pub struct PowerShell;
impl ShellConfig for Bash {
    fn name(&self) -> &'static str {
        "bash"
    }
    fn deactivate_script(&self) -> &'static str {
        posix::DEACTIVATE
    }
    fn env_script(&self, ctx: &EnvContext<'_>) -> String {
        bash::env_script(ctx)
    }
    fn profile_path(&self, home: &Path) -> Option<PathBuf> {
        bash::profile_path(home)
    }
    fn login_profile_path(&self, home: &Path) -> Option<PathBuf> {
        // ~/.profile is loaded by login shells and session managers on
        // Linux, and is not blocked by the interactive-only guard of ~/.bashrc.
        Some(home.join(".profile"))
    }
    fn init_line(&self) -> &'static str {
        r#"eval "$(nvsn env --shell bash)""#
    }
    fn wrapper_function(&self) -> &'static str {
        bash::wrapper_function()
    }
    fn shell_version_script(&self, version: &str, bin_dir: &Path, pin: bool) -> String {
        bash::shell_version_script(version, bin_dir, pin)
    }
    fn shell_unset_script(&self) -> &'static str {
        bash::shell_unset_script()
    }
    fn needs_bin_path_in_init(&self) -> bool {
        true
    }
}

impl ShellConfig for Zsh {
    fn name(&self) -> &'static str {
        "zsh"
    }
    fn deactivate_script(&self) -> &'static str {
        posix::DEACTIVATE
    }
    fn env_script(&self, ctx: &EnvContext<'_>) -> String {
        zsh::env_script(ctx)
    }
    fn profile_path(&self, home: &Path) -> Option<PathBuf> {
        zsh::profile_path(home)
    }
    fn login_profile_path(&self, home: &Path) -> Option<PathBuf> {
        // ~/.zprofile is loaded by zsh login shells (session managers, ssh).
        Some(home.join(".zprofile"))
    }
    fn init_line(&self) -> &'static str {
        r#"eval "$(nvsn env --shell zsh)""#
    }
    fn wrapper_function(&self) -> &'static str {
        zsh::wrapper_function()
    }
    fn shell_version_script(&self, version: &str, bin_dir: &Path, pin: bool) -> String {
        zsh::shell_version_script(version, bin_dir, pin)
    }
    fn shell_unset_script(&self) -> &'static str {
        zsh::shell_unset_script()
    }
    fn needs_bin_path_in_init(&self) -> bool {
        true
    }
}

impl ShellConfig for Fish {
    fn name(&self) -> &'static str {
        "fish"
    }
    fn deactivate_script(&self) -> &'static str {
        fish::DEACTIVATE
    }
    fn env_script(&self, ctx: &EnvContext<'_>) -> String {
        fish::env_script(ctx)
    }
    fn profile_path(&self, home: &Path) -> Option<PathBuf> {
        fish::profile_path(home)
    }
    fn init_line(&self) -> &'static str {
        "if command -q nvsn; nvsn env --shell fish | source; end"
    }
    fn wrapper_function(&self) -> &'static str {
        fish::wrapper_function()
    }
    fn shell_version_script(&self, version: &str, bin_dir: &Path, pin: bool) -> String {
        fish::shell_version_script(version, bin_dir, pin)
    }
    fn shell_unset_script(&self) -> &'static str {
        fish::shell_unset_script()
    }
}

impl ShellConfig for PowerShell {
    fn name(&self) -> &'static str {
        "powershell"
    }
    fn extra_profile_paths(&self, home: &Path) -> Vec<PathBuf> {
        // Windows PowerShell 5.1 reads its own profile, different from the one of PowerShell 7.
        vec![home
            .join("Documents")
            .join("WindowsPowerShell")
            .join("Microsoft.PowerShell_profile.ps1")]
    }
    fn deactivate_script(&self) -> &'static str {
        powershell::DEACTIVATE
    }
    fn binary_name(&self) -> &'static str {
        // The executable is `pwsh` (PowerShell 7+) on all platforms.
        // On older Windows it may be `powershell.exe`; `is_available` handles it separately.
        "pwsh"
    }
    fn env_script(&self, ctx: &EnvContext<'_>) -> String {
        powershell::env_script(ctx)
    }
    fn profile_path(&self, home: &Path) -> Option<PathBuf> {
        powershell::profile_path(home)
    }
    fn init_line(&self) -> &'static str {
        // Guarded, unlike bash/zsh: invoking a blocked or missing binary (AppLocker,
        // WDAC, PATH without an entry) throws a terminating error in PowerShell,
        // which would appear in every new shell.
        "if (Get-Command nvsn -ErrorAction SilentlyContinue) { try { nvsn env --shell powershell | Out-String | Invoke-Expression } catch {} }"
    }
    fn wrapper_function(&self) -> &'static str {
        powershell::wrapper_function()
    }
    fn shell_version_script(&self, version: &str, bin_dir: &Path, pin: bool) -> String {
        powershell::shell_version_script(version, bin_dir, pin)
    }
    fn shell_unset_script(&self) -> &'static str {
        powershell::shell_unset_script()
    }
}

// --- Factory -----------------------------------------------------------------

/// Detects the running shell from the environment.
///
/// Detection order (from most to least specific):
///
/// 1. `PSModulePath` variable, present in every PowerShell process.
/// 2. `SHELL` variable, standard on Unix. The executable name is compared.
/// 3. `cfg!(target_os = "windows")` as a last resort (PowerShell).
///
/// Returns `None` if the shell cannot be identified.
pub fn detect() -> Option<Box<dyn ShellConfig>> {
    // `NVSN_SHELL` is set by the session environment and the wrappers; it is the most
    // reliable source, because on Windows `PSModulePath` exists in any process.
    if let Ok(name) = std::env::var("NVSN_SHELL") {
        if let Ok(shell) = from_str(&name) {
            return Some(shell);
        }
    }
    if std::env::var("PSModulePath").is_ok() {
        return Some(Box::new(PowerShell));
    }
    if let Ok(shell) = std::env::var("SHELL") {
        let name = shell.rsplit('/').next().unwrap_or(shell.as_str());
        if name.contains("zsh") {
            return Some(Box::new(Zsh));
        }
        if name.contains("fish") {
            return Some(Box::new(Fish));
        }
        if name.contains("bash") {
            return Some(Box::new(Bash));
        }
    }
    if cfg!(target_os = "windows") {
        return Some(Box::new(PowerShell));
    }
    None
}

/// Builds a [`ShellConfig`] from its name.
///
/// Accepted values (case insensitive, hyphens ignored):
/// `powershell`, `pwsh`, `bash`, `zsh`, `fish`.
///
/// # Errors
///
/// Returns [`ShellError::UnknownShell`] if the name is not supported.
pub fn from_str(s: &str) -> Result<Box<dyn ShellConfig>, ShellError> {
    match s.to_lowercase().replace('-', "").as_str() {
        "powershell" | "pwsh" => Ok(Box::new(PowerShell)),
        "bash" => Ok(Box::new(Bash)),
        "zsh" => Ok(Box::new(Zsh)),
        "fish" => Ok(Box::new(Fish)),
        _ => Err(ShellError::UnknownShell(s.to_string())),
    }
}

// --- Availability ------------------------------------------------------------

/// Indicates whether the shell binary exists in `PATH`.
///
/// On Windows PowerShell is always the host and is considered available.
pub fn is_available(shell: &dyn ShellConfig) -> bool {
    // On Windows, PowerShell is the host: there is no need to look up the binary.
    #[cfg(windows)]
    if shell.name() == "powershell" {
        return true;
    }
    find_binary(shell.binary_name())
}

/// Returns the names of the supported shells that are installed.
///
/// Order: powershell, bash, zsh, fish.
pub fn available_shells() -> Vec<&'static str> {
    let candidates: &[(&dyn ShellConfig, &'static str)] = &[
        (&PowerShell, "powershell"),
        (&Bash, "bash"),
        (&Zsh, "zsh"),
        (&Fish, "fish"),
    ];
    candidates
        .iter()
        .filter(|(sh, _)| is_available(*sh))
        .map(|(_, name)| *name)
        .collect()
}

/// Indicates whether `dir` appears in the list of directories `path_var`, using
/// the platform separator (`:` or `;`).
///
/// The check is textual over the entries: it does not resolve links.
pub fn is_dir_in_path(dir: &Path, path_var: &str) -> bool {
    std::env::split_paths(path_var).any(|p| p == dir)
}

/// Looks for an executable with that name in `PATH`. On Windows it also tries
/// `<name>.exe`.
fn find_binary(name: &str) -> bool {
    let Some(path_var) = std::env::var_os("PATH") else {
        return false;
    };
    for dir in std::env::split_paths(&path_var) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        if dir.join(name).exists() {
            return true;
        }
        #[cfg(windows)]
        if dir.join(format!("{name}.exe")).exists() {
            return true;
        }
    }
    false
}

// --- Profile injection -------------------------------------------------------

/// Content of the `# nvsn init` block.
///
/// In bash and zsh (not Windows) it prepends an `export PATH` with the directory of
/// `nvsn` so that the block works even when it is loaded before that directory
/// is in PATH.
fn build_init_content(shell: &dyn ShellConfig, home: &Path, bin_dir: Option<&Path>) -> String {
    if shell.needs_bin_path_in_init() {
        #[cfg(not(target_os = "windows"))]
        if let Some(dir) = bin_dir {
            let path_expr = profile::home_relative(dir, home);
            return format!("export PATH={path_expr}:$PATH\n{}", shell.init_line());
        }
        #[cfg(target_os = "windows")]
        let _ = (home, bin_dir);
    }
    shell.init_line().to_string()
}

/// Adds or updates the `# nvsn init` and `# nvsn wrapper` blocks in the
/// shell profile.
///
/// Two independent markers allow each block to be injected and removed
/// separately. Repeating the call does not modify the file if it is up to date;
/// an old version of the wrapper is replaced.
///
/// Returns `Ok(true)` if the file changed. `bin_dir` is the `nvsn` directory
/// that is prepended to the PATH in shells that need it.
///
/// # Errors
///
/// Returns [`ShellError::NoProfilePath`] if the shell has no profile, or
/// [`ShellError::Io`] if the file cannot be read or written.
pub fn inject_profile(
    shell: &dyn ShellConfig,
    home: &Path,
    bin_dir: Option<&Path>,
) -> Result<bool, ShellError> {
    let path = shell
        .profile_path(home)
        .ok_or(ShellError::NoProfilePath(shell.name()))?;
    let init_content = build_init_content(shell, home, bin_dir);
    let mut changed = profile::ensure_profile(&path, &init_content, shell.wrapper_function())?;
    for extra in shell.extra_profile_paths(home) {
        changed |= profile::ensure_profile(&extra, &init_content, shell.wrapper_function())?;
    }
    Ok(changed)
}

/// Updates the `# nvsn path` block of the login profile with
/// `export PATH="<root>/current/bin:$PATH"` (ADR-034), so that graphical
/// applications see the global version through the `current` link.
///
/// Does nothing if the shell has no login profile or if `root` is `None`.
/// Returns `Ok(true)` if the file changed.
///
/// # Errors
///
/// Returns [`ShellError::Io`] if the file cannot be read or written.
pub fn inject_login_profile(
    shell: &dyn ShellConfig,
    home: &Path,
    root: Option<&Path>,
) -> Result<bool, ShellError> {
    let (Some(path), Some(root)) = (shell.login_profile_path(home), root) else {
        return Ok(false);
    };
    profile::update_path_block(&path, home, root)
}

/// Removes all the blocks managed by nvsn from `path`.
///
/// Returns `Ok(true)` if the file changed, `Ok(false)` if it contained no
/// blocks or does not exist.
///
/// # Errors
///
/// Returns [`ShellError::Io`] if the file cannot be read or written.
pub fn strip_profile(path: &Path) -> Result<bool, ShellError> {
    profile::strip_nvsn_blocks(path)
}

// --- Tests -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    /// [`ShellConfig`] for tests that wraps [`Bash`] but redirects the profile path
    /// to a temporary directory, to exercise `inject_profile`,
    /// `inject_login_profile` and `strip_profile` against the production code.
    #[derive(Debug)]
    struct TempShell {
        profile: PathBuf,
        login_profile: PathBuf,
    }

    impl ShellConfig for TempShell {
        fn name(&self) -> &'static str {
            "bash"
        }
        fn env_script(&self, ctx: &EnvContext<'_>) -> String {
            bash::env_script(ctx)
        }
        fn profile_path(&self, _home: &Path) -> Option<PathBuf> {
            Some(self.profile.clone())
        }
        fn login_profile_path(&self, _home: &Path) -> Option<PathBuf> {
            Some(self.login_profile.clone())
        }
        fn init_line(&self) -> &'static str {
            r#"eval "$(nvsn env --shell bash)""#
        }
        fn wrapper_function(&self) -> &'static str {
            bash::wrapper_function()
        }
        fn shell_version_script(&self, version: &str, bin_dir: &Path, pin: bool) -> String {
            bash::shell_version_script(version, bin_dir, pin)
        }
        fn shell_unset_script(&self) -> &'static str {
            bash::shell_unset_script()
        }
        fn deactivate_script(&self) -> &'static str {
            posix::DEACTIVATE
        }
        fn needs_bin_path_in_init(&self) -> bool {
            true
        }
    }

    fn temp_shell(dir: &Path) -> TempShell {
        TempShell {
            profile: dir.join("profile"),
            login_profile: dir.join("login_profile"),
        }
    }

    #[test]
    fn inject_profile_writes_init_and_wrapper_blocks() {
        let dir = tempdir().unwrap();
        let sh = temp_shell(dir.path());

        assert!(inject_profile(&sh, dir.path(), None).unwrap());

        let content = fs::read_to_string(&sh.profile).unwrap();
        assert!(content.contains("# nvsn init"));
        assert!(content.contains("# nvsn wrapper"));
        assert!(content.contains(r#"eval "$(nvsn env --shell bash)""#));
    }

    #[test]
    fn inject_profile_is_idempotent() {
        let dir = tempdir().unwrap();
        let sh = temp_shell(dir.path());

        inject_profile(&sh, dir.path(), None).unwrap();
        let first = fs::read_to_string(&sh.profile).unwrap();
        let changed = inject_profile(&sh, dir.path(), None).unwrap();
        let second = fs::read_to_string(&sh.profile).unwrap();

        assert!(!changed, "second run must report no change");
        assert_eq!(first, second, "second run must not change the file");
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn inject_profile_prepends_bin_path_when_needed() {
        let dir = tempdir().unwrap();
        let sh = temp_shell(dir.path());
        let bin_dir = dir.path().join("bin");

        inject_profile(&sh, dir.path(), Some(&bin_dir)).unwrap();

        let content = fs::read_to_string(&sh.profile).unwrap();
        assert!(content.contains("export PATH="));
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn inject_login_profile_points_at_the_current_link() {
        let dir = tempdir().unwrap();
        let sh = temp_shell(dir.path());
        let root = dir.path().join(".nvsn");

        assert!(inject_login_profile(&sh, dir.path(), Some(&root)).unwrap());

        let content = fs::read_to_string(&sh.login_profile).unwrap();
        assert!(content.contains("# nvsn path"));
        assert!(
            content.contains("export PATH=\"$HOME/.nvsn/current/bin:$PATH\""),
            "static line pointing to current/bin: {content}"
        );
        assert!(
            !content.contains("default"),
            "the profile no longer reads the default file: {content}"
        );
        assert!(
            !content.contains("v20.11.1"),
            "the version is not pinned in the profile"
        );
    }

    #[test]
    fn inject_login_profile_is_noop_without_root() {
        let dir = tempdir().unwrap();
        let sh = temp_shell(dir.path());
        assert!(!inject_login_profile(&sh, dir.path(), None).unwrap());
        assert!(!sh.login_profile.exists());
    }

    #[test]
    fn strip_profile_removes_nvsn_blocks_after_inject() {
        let dir = tempdir().unwrap();
        let sh = temp_shell(dir.path());
        fs::write(&sh.profile, "# user config\nexport FOO=bar\n").unwrap();

        inject_profile(&sh, dir.path(), None).unwrap();
        assert!(fs::read_to_string(&sh.profile)
            .unwrap()
            .contains("# nvsn init"));

        let changed = strip_profile(&sh.profile).unwrap();
        assert!(changed);

        let content = fs::read_to_string(&sh.profile).unwrap();
        assert!(!content.contains("# nvsn init"));
        assert!(!content.contains("# nvsn wrapper"));
        assert!(
            content.contains("export FOO=bar"),
            "user content must survive"
        );
    }

    #[test]
    fn strip_profile_returns_false_for_missing_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("does-not-exist");
        assert!(!strip_profile(&path).unwrap());
    }

    #[test]
    fn setup_injects_both_blocks_into_empty_profile() {
        let dir = tempdir().unwrap();
        let sh = temp_shell(dir.path());
        inject_profile(&sh, dir.path(), None).unwrap();
        let result = fs::read_to_string(&sh.profile).unwrap();
        assert!(result.contains("# nvsn init"));
        assert!(result.contains("# nvsn wrapper"));
        assert!(
            result.contains("deactivate)"),
            "wrapper must route deactivate"
        );
    }

    #[test]
    fn setup_updates_stale_bash_wrapper() {
        let dir = tempdir().unwrap();
        let sh = temp_shell(dir.path());
        fs::write(
            &sh.profile,
            "# nvsn init\neval \"$(nvsn env --shell bash)\"\n\n# nvsn wrapper\nnvsn() { command nvsn \"$@\"; }\n",
        )
        .unwrap();

        inject_profile(&sh, dir.path(), None).unwrap();
        let result = fs::read_to_string(&sh.profile).unwrap();
        assert!(
            result.contains("NVSN_INTEGRATION=1"),
            "new wrapper must be injected"
        );
        assert!(
            !result.contains("nvsn() { command nvsn \"$@\"; }"),
            "old stub must be removed"
        );
        assert_eq!(result.matches("# nvsn init").count(), 1);
    }

    #[test]
    fn setup_updates_stale_zsh_wrapper() {
        let dir = tempdir().unwrap();
        let sh = Zsh;
        let path = dir.path().join(".zshrc");
        fs::write(
            &path,
            "# nvsn init\neval \"$(nvsn env --shell zsh)\"\n\n# nvsn wrapper\nnvsn() { command nvsn \"$@\"; }\n",
        )
        .unwrap();

        assert!(profile::ensure_profile(&path, sh.init_line(), sh.wrapper_function()).unwrap());
        let result = fs::read_to_string(&path).unwrap();
        assert!(result.contains("--shell zsh"));
        assert!(result.contains("NVSN_INTEGRATION=1"));
    }

    #[test]
    fn setup_updates_stale_fish_wrapper() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.fish");
        let stale = "# nvsn init\nnvsn env --shell fish | source\n\n# nvsn wrapper\nfunction nvsn\n    command nvsn $argv\nend\n";
        fs::write(&path, stale).unwrap();

        profile::ensure_profile(&path, Fish.init_line(), Fish.wrapper_function()).unwrap();
        let result = fs::read_to_string(&path).unwrap();
        assert!(
            result.contains("string join"),
            "updated fish wrapper must use string join"
        );
        assert!(
            result.contains("command -q nvsn"),
            "new guard must be present"
        );
        assert_eq!(result.matches("# nvsn init").count(), 1);
    }

    #[test]
    fn setup_is_idempotent_for_fish() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.fish");
        profile::ensure_profile(&path, Fish.init_line(), Fish.wrapper_function()).unwrap();
        let first = fs::read_to_string(&path).unwrap();
        let changed =
            profile::ensure_profile(&path, Fish.init_line(), Fish.wrapper_function()).unwrap();
        assert!(!changed);
        assert_eq!(first, fs::read_to_string(&path).unwrap());
    }

    #[test]
    fn profile_paths_are_derived_from_home() {
        let home = Path::new("/home/user");
        assert_eq!(Bash.profile_path(home), Some(home.join(".bashrc")));
        assert_eq!(Zsh.profile_path(home), Some(home.join(".zshrc")));
        assert_eq!(
            Fish.profile_path(home),
            Some(home.join(".config").join("fish").join("config.fish"))
        );
        assert_eq!(
            PowerShell.profile_path(home),
            Some(
                home.join("Documents")
                    .join("PowerShell")
                    .join("Microsoft.PowerShell_profile.ps1")
            )
        );
    }

    #[test]
    fn from_str_accepts_aliases_and_rejects_unknown() {
        assert_eq!(from_str("pwsh").unwrap().name(), "powershell");
        assert_eq!(from_str("Power-Shell").unwrap().name(), "powershell");
        assert_eq!(from_str("zsh").unwrap().name(), "zsh");
        let err = from_str("tcsh").unwrap_err();
        assert!(matches!(err, ShellError::UnknownShell(ref s) if s == "tcsh"));
    }

    #[test]
    fn deactivate_script_is_defined_for_every_shell() {
        let shells: [&dyn ShellConfig; 4] = [&Bash, &Zsh, &Fish, &PowerShell];
        for shell in shells {
            assert!(
                shell.deactivate_script().contains("NVSN_"),
                "{} has no deactivation code",
                shell.name()
            );
        }
    }

    #[test]
    fn posix_profile_block_is_idempotent() {
        let dir = tempdir().unwrap();
        let home = dir.path();
        let bin = home.join("Mi Dir").join("bin");
        assert!(inject_profile(&Bash, home, Some(&bin)).unwrap());
        let first = fs::read_to_string(home.join(".bashrc")).unwrap();
        assert!(first.contains("# nvsn init"));
        assert!(!inject_profile(&Bash, home, Some(&bin)).unwrap());
        assert_eq!(first, fs::read_to_string(home.join(".bashrc")).unwrap());
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn posix_profile_path_with_space_is_double_quoted_under_home() {
        let dir = tempdir().unwrap();
        let home = dir.path();
        let bin = home.join("Mi Dir").join("bin");
        inject_profile(&Bash, home, Some(&bin)).unwrap();
        let content = fs::read_to_string(home.join(".bashrc")).unwrap();
        assert!(content.contains("export PATH=\"$HOME/Mi Dir/bin\":$PATH"));
    }

    #[test]
    fn is_dir_in_path_matches_exact_entries() {
        let sep = if cfg!(windows) { ";" } else { ":" };
        let var = format!("/usr/bin{sep}/opt/nvsn/bin");
        assert!(is_dir_in_path(Path::new("/opt/nvsn/bin"), &var));
        assert!(!is_dir_in_path(Path::new("/opt/nvsn"), &var));
    }
}
