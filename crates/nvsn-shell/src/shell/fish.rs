//! Fish integration.
//!
//! Hook with `--on-variable PWD` (directory changes) and `--on-event
//! fish_prompt` (first prompt). ADR-017.

use super::{quote, EnvContext};
use std::path::{Path, PathBuf};

/// Fish user profile.
pub fn profile_path(home: &Path) -> Option<PathBuf> {
    Some(home.join(".config").join("fish").join("config.fish"))
}

/// Block that puts `bin` at the front of PATH, removing the previous nvsn
/// entry (`NVSN_PATH_ENTRY`) and registering the new one.
fn fish_set_path(bin: &Path) -> String {
    let bin_q = quote::fish(&bin.display().to_string());
    format!(
        "set -l _nvsn_kept\n\
         for _nvsn_d in $PATH\n\
         \x20   if test -n \"$_nvsn_d\"; and test \"$_nvsn_d\" != \"$NVSN_PATH_ENTRY\"\n\
         \x20       set -a _nvsn_kept $_nvsn_d\n\
         \x20   end\n\
         end\n\
         set -gx PATH {bin_q} $_nvsn_kept\n\
         set -gx NVSN_PATH_ENTRY {bin_q}\n\
         set -e _nvsn_kept\n\
         set -e _nvsn_d\n"
    )
}

/// Fish function definitions: filter, hook and deactivation. They are
/// registered only once per session.
const FUNCTIONS: &str = r#"    function _nvsn_has_version_file
        set -l _nvsn_d "$PWD"
        while true
            if test -f "$_nvsn_d/.nvmrc"; or test -f "$_nvsn_d/.node-version"
                return 0
            end
            if test "$_nvsn_d" = /
                return 1
            end
            set _nvsn_d (string replace -r '/[^/]*$' '' -- "$_nvsn_d")
            test -n "$_nvsn_d"; or set _nvsn_d /
        end
    end
    function _nvsn_hook --on-variable PWD --on-event fish_prompt
        if set -q NVSN_SHELL_VERSION
            return
        end
        if set -q _NVSN_PREV_PWD; and test "$_NVSN_PREV_PWD" = "$PWD"
            return
        end
        set -g _NVSN_PREV_PWD "$PWD"
        if _nvsn_has_version_file
            set -g _NVSN_AUTO 1
        else if set -q _NVSN_AUTO
            set -e _NVSN_AUTO
            if command -q nvsn
                nvsn deactivate --shell fish 2>/dev/null | source
                nvsn env --shell fish 2>/dev/null | source
            end
            return
        else
            return
        end
        if command -q nvsn
            nvsn __hook --shell fish 2>/dev/null | source
        end
    end
    function _nvsn_deactivate
        set -l _nvsn_script (command nvsn deactivate --shell fish)
        or return $status
        string join \n -- $_nvsn_script | source
        set -e NVSN_SHELL_VERSION 2>/dev/null
        set -e _NVSN_AUTO 2>/dev/null
        set -g _NVSN_PREV_PWD "$PWD"
    end
"#;

/// Generates the Fish initialization script.
pub fn env_script(ctx: &EnvContext<'_>) -> String {
    let nvsn_dir = quote::fish(&ctx.nvsn_dir.display().to_string());

    let version_stmt = ctx.node_version.map_or_else(String::new, |v| {
        format!("set -gx NVSN_VERSION {}\n", quote::fish(v))
    });

    let path_stmt = ctx.active_bin.map_or_else(String::new, fish_set_path);

    format!(
        "set -gx NVSN_DIR {nvsn_dir}\nset -gx NVSN_SHELL fish\n{version_stmt}{path_stmt}\nif not functions -q _nvsn_hook\n{FUNCTIONS}end\n"
    )
}

/// `nvsn` wrapper function for Fish.
///
/// `use|on|off` runs the binary with `NVSN_INTEGRATION=1` and applies its output
/// with `source`. `deactivate` goes through `_nvsn_deactivate`. Everything else goes straight through.
pub fn wrapper_function() -> &'static str {
    r#"function nvsn
    if contains -- $argv[1] use on off
        set -l _nvsn_script (NVSN_INTEGRATION=1 command nvsn $argv)
        or return $status
        string join \n -- $_nvsn_script | source
        return 0
    end
    if test "$argv[1]" = deactivate
        _nvsn_deactivate
        return $status
    end
    command nvsn $argv
end"#
}

/// Activation script for the current session (see [`super::ShellConfig::shell_version_script`]).
pub fn shell_version_script(version: &str, bin_dir: &Path, pin: bool) -> String {
    let pin_stmt = if pin {
        format!("set -gx NVSN_SHELL_VERSION {}\n", quote::fish(version))
    } else {
        String::new()
    };
    format!(
        "set -gx NVSN_VERSION {}\n{pin_stmt}{}",
        quote::fish(version),
        fish_set_path(bin_dir)
    )
}

/// Removes the session mark, forgets the last directory seen and re-evaluates the hook.
pub fn shell_unset_script() -> &'static str {
    "set -e NVSN_SHELL_VERSION 2>/dev/null\nset -e _NVSN_AUTO 2>/dev/null\nset -e _NVSN_PREV_PWD 2>/dev/null\n_nvsn_hook 2>/dev/null\n"
}

/// fish code that deactivates the session's version: removes `NVSN_VERSION`,
/// `NVSN_SHELL_VERSION` and the nvsn `PATH` entry.
pub const DEACTIVATE: &str = r#"set -q NVSN_VERSION; and set -e NVSN_VERSION
set -q NVSN_SHELL_VERSION; and set -e NVSN_SHELL_VERSION
if set -q NVSN_PATH_ENTRY
    set -l keep
    for entry in $PATH
        if test "$entry" != "$NVSN_PATH_ENTRY"
            set -a keep $entry
        end
    end
    set -gx PATH $keep
    set -e NVSN_PATH_ENTRY
end
"#;

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
        assert!(script.contains("set -gx NVSN_DIR '/home/jo/Mi Dir/.nvsn'"));
        assert!(script.contains("set -gx NVSN_VERSION 'v20.11.1'"));
        assert!(script
            .contains("set -gx NVSN_PATH_ENTRY '/home/jo/Mi Dir/.nvsn/versions/v20.11.1/bin'"));
    }

    #[test]
    fn env_script_escapes_backslash_in_windows_style_path() {
        let ctx = EnvContext {
            nvsn_dir: Path::new(r"C:\Users\José Ñu\.nvsn"),
            active_bin: None,
            node_version: None,
        };
        assert!(env_script(&ctx).contains(r"set -gx NVSN_DIR 'C:\\Users\\José Ñu\\.nvsn'"));
    }

    #[test]
    fn hook_registered_on_pwd_and_prompt() {
        let script = env_script(&ctx());
        assert!(script.contains("--on-variable PWD --on-event fish_prompt"));
    }

    #[test]
    fn hook_filters_before_calling_binary() {
        let script = env_script(&ctx());
        let filter = script
            .find("function _nvsn_has_version_file")
            .unwrap_or(usize::MAX);
        let call = script.find("nvsn __hook --shell fish").unwrap_or(0);
        assert!(filter < call);
        assert!(script.contains("command -q nvsn"));
    }

    #[test]
    fn deactivate_function_evals_binary_output() {
        let script = env_script(&ctx());
        assert!(script.contains("command nvsn deactivate --shell fish"));
        assert!(script.contains("string join \\n -- $_nvsn_script | source"));
    }

    #[test]
    fn shell_version_script_uses_fish_syntax() {
        let s = shell_version_script(
            "v22.1.0",
            Path::new("/home/.nvsn/versions/v22.1.0/bin"),
            true,
        );
        assert!(s.contains("set -gx NVSN_VERSION 'v22.1.0'"));
        assert!(s.contains("set -gx NVSN_SHELL_VERSION 'v22.1.0'"));
    }

    #[test]
    fn wrapper_handles_use_and_deactivate() {
        let w = wrapper_function();
        assert!(w.contains("contains -- $argv[1] use on off"));
        assert!(w.contains("_nvsn_deactivate"));
        assert!(w.contains("NVSN_INTEGRATION=1"));
    }
}
