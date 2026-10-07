//! Zsh integration.
//!
//! Hook with `add-zsh-hook chpwd` (directory changes) and `precmd` (first
//! prompt, because `chpwd` does not run at startup). ADR-017.

use super::{posix, quote, EnvContext};
use std::path::{Path, PathBuf};

/// Zsh interactive profile.
pub fn profile_path(home: &Path) -> Option<PathBuf> {
    Some(home.join(".zshrc"))
}

/// Generates the Zsh initialization script.
pub fn env_script(ctx: &EnvContext<'_>) -> String {
    let nvsn_dir = quote::posix(&ctx.nvsn_dir.display().to_string());
    let version_stmt = ctx.node_version.map_or_else(String::new, |v| {
        format!("export NVSN_VERSION={}\n", quote::posix(v))
    });
    let path_stmt = ctx.active_bin.map_or_else(String::new, posix::set_path);
    let hook = posix::hook_block(
        "zsh",
        "(( $+functions[_nvsn_hook] ))",
        "autoload -Uz add-zsh-hook\nadd-zsh-hook chpwd _nvsn_hook\nadd-zsh-hook precmd _nvsn_hook",
    );
    format!("export NVSN_DIR={nvsn_dir}\nexport NVSN_SHELL=zsh\n{version_stmt}{path_stmt}\n{hook}")
}

/// `nvsn` wrapper function for Zsh (the same as bash and sh).
pub fn wrapper_function() -> &'static str {
    posix::WRAPPER
}

/// Activation script for the current session (see [`super::ShellConfig::shell_version_script`]).
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
            active_bin: Some(Path::new("/home/jo/Mi Dir/.nvsn/versions/v22.1.0/bin")),
            node_version: Some("v22.1.0"),
        }
    }

    #[test]
    fn env_script_quotes_path_with_space_and_accent() {
        let script = env_script(&ctx());
        assert!(script.contains("export NVSN_DIR='/home/jo/Mi Dir/.nvsn'"));
        assert!(script.contains("export NVSN_VERSION='v22.1.0'"));
    }

    #[test]
    fn hook_uses_chpwd_and_precmd_with_add_zsh_hook() {
        let script = env_script(&ctx());
        assert!(script.contains("add-zsh-hook chpwd _nvsn_hook"));
        assert!(script.contains("add-zsh-hook precmd _nvsn_hook"));
    }

    #[test]
    fn hook_filters_before_calling_binary() {
        let script = env_script(&ctx());
        let filter = script
            .find("_nvsn_has_version_file() {")
            .unwrap_or(usize::MAX);
        let call = script.find("nvsn __hook --shell zsh").unwrap_or(0);
        assert!(filter < call);
    }

    #[test]
    fn deactivate_evals_binary_output_with_zsh_name() {
        let script = env_script(&ctx());
        assert!(script.contains("command nvsn deactivate --shell zsh"));
    }

    #[test]
    fn shell_version_script_sets_version_and_pin() {
        let s = shell_version_script(
            "v22.1.0",
            Path::new("/home/.nvsn/versions/v22.1.0/bin"),
            true,
        );
        assert!(s.contains("export NVSN_VERSION='v22.1.0'"));
        assert!(s.contains("export NVSN_SHELL_VERSION='v22.1.0'"));
    }

    #[test]
    fn wrapper_handles_use_and_deactivate() {
        let w = wrapper_function();
        assert!(w.contains("use|on|off)"));
        assert!(w.contains("deactivate)"));
    }
}
