//! Bash integration.
//!
//! Hook through `PROMPT_COMMAND`, comparing `$PWD` (ADR-017). `cd` is not
//! overridden, so that `cd -` and `CDPATH` keep working.

use super::{posix, quote, EnvContext};
use std::path::{Path, PathBuf};

/// Bash interactive profile.
pub fn profile_path(home: &Path) -> Option<PathBuf> {
    Some(home.join(".bashrc"))
}

/// Generates the Bash initialization script.
pub fn env_script(ctx: &EnvContext<'_>) -> String {
    let nvsn_dir = quote::posix(&ctx.nvsn_dir.display().to_string());
    let version_stmt = ctx.node_version.map_or_else(String::new, |v| {
        format!("export NVSN_VERSION={}\n", quote::posix(v))
    });
    let path_stmt = ctx.active_bin.map_or_else(String::new, posix::set_path);
    let hook = posix::hook_block(
        "bash",
        "declare -f _nvsn_hook >/dev/null 2>&1",
        "PROMPT_COMMAND=\"_nvsn_hook${PROMPT_COMMAND:+;$PROMPT_COMMAND}\"",
    );
    format!("export NVSN_DIR={nvsn_dir}\nexport NVSN_SHELL=bash\n{version_stmt}{path_stmt}\n{hook}")
}

/// `nvsn` wrapper function for Bash (the same as zsh and sh).
pub fn wrapper_function() -> &'static str {
    posix::WRAPPER
}

/// Activation script for the current session, emitted by `nvsn use`.
///
/// With `pin = true` it marks `NVSN_SHELL_VERSION`, which makes the hook not change
/// the version when the directory changes.
pub fn shell_version_script(version: &str, bin_dir: &Path, pin: bool) -> String {
    let pin_stmt = if pin {
        format!("export NVSN_SHELL_VERSION={}\n", quote::posix(version))
    } else {
        String::new()
    };
    format!(
        "export NVSN_VERSION={}\n{pin_stmt}{}",
        quote::posix(version),
        posix::set_path(bin_dir)
    )
}

/// Removes the session mark, forgets the last directory seen and re-evaluates the hook.
pub fn shell_unset_script() -> &'static str {
    "unset NVSN_SHELL_VERSION _NVSN_AUTO\n_NVSN_PREV_PWD=\n_nvsn_hook 2>/dev/null || true\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> EnvContext<'static> {
        EnvContext {
            nvsn_dir: Path::new("/home/jo/Mi Dir/.nvsn"),
            active_bin: Some(Path::new("/home/jo/Mi Dir/.nvsn/versions/v20.11.1/bin")),
            node_version: Some("v20.11.1"),
        }
    }

    #[test]
    fn env_script_quotes_path_with_space_and_accent() {
        let script = env_script(&ctx());
        assert!(script.contains("export NVSN_DIR='/home/jo/Mi Dir/.nvsn'"));
        assert!(
            script.contains("export NVSN_PATH_ENTRY='/home/jo/Mi Dir/.nvsn/versions/v20.11.1/bin'")
        );
    }

    #[test]
    fn hook_filters_before_calling_binary() {
        let script = env_script(&ctx());
        let filter = script
            .find("_nvsn_has_version_file() {")
            .unwrap_or(usize::MAX);
        let call = script.find("nvsn __hook --shell bash").unwrap_or(0);
        assert!(
            filter < call,
            "filter must be defined before the __hook call"
        );
        assert!(script.contains("command -v nvsn"));
    }

    #[test]
    fn hook_calls_binary_bypassing_wrapper_function() {
        // With a bare `nvsn`, `deactivate` would go to the wrapper and its `eval` would stay
        // inside the `$(...)` subshell, with no effect on the session.
        let script = env_script(&ctx());
        assert!(script.contains("eval \"$(command nvsn deactivate --shell bash"));
        assert!(script.contains("eval \"$(command nvsn __hook --shell bash"));
    }

    #[test]
    fn env_script_records_shell_name() {
        assert!(env_script(&ctx()).contains("export NVSN_SHELL=bash"));
    }

    #[test]
    fn hook_does_not_override_cd() {
        let script = env_script(&ctx());
        assert!(
            !script.contains("cd()"),
            "bash must use PROMPT_COMMAND only"
        );
    }

    #[test]
    fn registers_prompt_command_once_per_session() {
        let script = env_script(&ctx());
        assert!(script.contains("PROMPT_COMMAND=\"_nvsn_hook"));
        assert!(script.contains("if ! declare -f _nvsn_hook"));
    }

    #[test]
    fn deactivate_evals_binary_output_with_shell_name() {
        let script = env_script(&ctx());
        assert!(script.contains("command nvsn deactivate --shell bash"));
    }

    #[test]
    fn env_script_without_version_omits_version_export() {
        let ctx = EnvContext {
            nvsn_dir: Path::new("/home/user/.nvsn"),
            active_bin: None,
            node_version: None,
        };
        let script = env_script(&ctx);
        assert!(!script.contains("NVSN_VERSION="));
    }

    #[test]
    fn shell_version_script_pins_when_requested() {
        let bin = Path::new("/home/user/.nvsn/versions/v20.11.1/bin");
        let pinned = shell_version_script("v20.11.1", bin, true);
        assert!(pinned.contains("export NVSN_SHELL_VERSION='v20.11.1'"));
        let free = shell_version_script("v20.11.1", bin, false);
        assert!(!free.contains("NVSN_SHELL_VERSION"));
    }

    #[test]
    fn wrapper_delegates_deactivate_and_evals_use() {
        let w = wrapper_function();
        assert!(w.contains("deactivate)"));
        assert!(w.contains("_nvsn_deactivate"));
        assert!(w.contains("NVSN_INTEGRATION=1"));
    }
}
