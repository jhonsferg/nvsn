//! PowerShell integration (5.1 and 7+).
//!
//! It wraps the `prompt` function, because there is no reliable location-change
//! event (ADR-017). It does not override `Set-Location` or `cd`.

use super::{quote, EnvContext};
use std::path::{Path, PathBuf};

/// PowerShell user profile.
pub fn profile_path(home: &Path) -> Option<PathBuf> {
    Some(
        home.join("Documents")
            .join("PowerShell")
            .join("Microsoft.PowerShell_profile.ps1"),
    )
}

/// Block that puts `bin` at the front of `$env:PATH`, removing the previous nvsn
/// entry (`$env:NVSN_PATH_ENTRY`) and registering the new one. The list is
/// built as an array, so no trailing `;` is left if PATH was empty.
fn ps_set_path(bin: &Path) -> String {
    let bin_q = quote::powershell(&bin.display().to_string());
    format!(
        "$env:PATH = (@({bin_q}) + @($env:PATH -split ';' | Where-Object {{ $_ -ne '' -and $_ -ne $env:NVSN_PATH_ENTRY }})) -join ';'\n\
         $env:NVSN_PATH_ENTRY = {bin_q}\n"
    )
}

/// Hook and deactivation functions. They are defined only once per session.
const FUNCTIONS: &str = r##"    function global:_nvsn_has_version_file {
        $info = [System.IO.DirectoryInfo]::new($PWD.ProviderPath)
        while ($null -ne $info) {
            $base = $info.FullName
            if ([System.IO.File]::Exists([System.IO.Path]::Combine($base, '.nvmrc')) -or [System.IO.File]::Exists([System.IO.Path]::Combine($base, '.node-version'))) {
                return $true
            }
            $info = $info.Parent
        }
        return $false
    }
    function global:_nvsn_hook {
        if ($env:NVSN_SHELL_VERSION) { return }
        $here = $PWD.ProviderPath
        if ($global:_NVSN_PREV_PWD -eq $here) { return }
        $global:_NVSN_PREV_PWD = $here
        if (_nvsn_has_version_file) {
            $global:_NVSN_AUTO = $true
        } elseif ($global:_NVSN_AUTO) {
            _nvsn_deactivate
            $nvsnEnv = (& nvsn env --shell powershell 2>$null | Out-String)
            if ($nvsnEnv.Trim()) { Invoke-Expression $nvsnEnv }
            return
        } else {
            return
        }
        if (-not (Get-Command nvsn -CommandType Application -ErrorAction SilentlyContinue)) { return }
        $nvsnSnippet = (& nvsn __hook --shell powershell 2>$null | Out-String)
        if ($nvsnSnippet.Trim()) { Invoke-Expression $nvsnSnippet }
    }
    function global:_nvsn_deactivate {
        $nvsnBin = (Get-Command nvsn -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1).Source
        if (-not $nvsnBin) { Write-Error 'nvsn binary not found in PATH'; return }
        $nvsnScript = (& $nvsnBin deactivate --shell powershell | Out-String)
        if ($LASTEXITCODE -eq 0 -and $nvsnScript.Trim()) { Invoke-Expression $nvsnScript }
        Remove-Item env:NVSN_SHELL_VERSION -ErrorAction SilentlyContinue
        $global:_NVSN_AUTO = $false
        $global:_NVSN_PREV_PWD = $PWD.ProviderPath
    }
    # The ScriptBlock is stored, not the FunctionInfo: the FunctionInfo is a
    # live reference and, after redefining `prompt`, would point to the wrapper (recursion).
    $global:_nvsn_original_prompt = (Get-Command prompt -CommandType Function -ErrorAction SilentlyContinue).ScriptBlock
    function global:prompt {
        _nvsn_hook
        if ($global:_nvsn_original_prompt) {
            & $global:_nvsn_original_prompt
        } else {
            "PS $($PWD.Path)> "
        }
    }
"##;

/// Generates the PowerShell initialization script.
pub fn env_script(ctx: &EnvContext<'_>) -> String {
    let nvsn_dir = quote::powershell(&ctx.nvsn_dir.display().to_string());

    let version_stmt = ctx.node_version.map_or_else(String::new, |v| {
        format!("$env:NVSN_VERSION = {}\n", quote::powershell(v))
    });

    let path_stmt = ctx.active_bin.map_or_else(String::new, ps_set_path);

    format!(
        "$env:NVSN_DIR = {nvsn_dir}\n$env:NVSN_SHELL = 'powershell'\n{version_stmt}{path_stmt}\n\
         if (-not (Get-Command _nvsn_hook -CommandType Function -ErrorAction SilentlyContinue)) {{\n{FUNCTIONS}}}\n"
    )
}

/// `nvsn` wrapper function for PowerShell.
///
/// `use|on|off` runs the binary with `NVSN_INTEGRATION=1` and applies its output
/// with `Invoke-Expression`. `deactivate` goes through `_nvsn_deactivate`.
pub fn wrapper_function() -> &'static str {
    r#"function nvsn {
    $nvsnBin = (Get-Command nvsn -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1).Source
    if (-not $nvsnBin) { Write-Error 'nvsn binary not found in PATH'; return }
    if ($args.Count -gt 0 -and $args[0] -eq 'deactivate') {
        _nvsn_deactivate
        $global:LASTEXITCODE = 0
        return
    }
    if ($args.Count -gt 0 -and $args[0] -in @('use', 'on', 'off')) {
        $env:NVSN_INTEGRATION = '1'
        try {
            $nvsnScript = & $nvsnBin @args
        } finally {
            Remove-Item env:NVSN_INTEGRATION -ErrorAction SilentlyContinue
        }
        $nvsnExit = $LASTEXITCODE
        if ($nvsnExit -eq 0 -and $nvsnScript) {
            $nvsnScript | Out-String | Invoke-Expression
        }
    } else {
        & $nvsnBin @args
        $nvsnExit = $LASTEXITCODE
    }
    $global:LASTEXITCODE = $nvsnExit
}"#
}

/// Activation script for the current session (see [`super::ShellConfig::shell_version_script`]).
pub fn shell_version_script(version: &str, bin_dir: &Path, pin: bool) -> String {
    let pin_stmt = if pin {
        format!("$env:NVSN_SHELL_VERSION = {}\n", quote::powershell(version))
    } else {
        String::new()
    };
    format!(
        "$env:NVSN_VERSION = {}\n{pin_stmt}{}",
        quote::powershell(version),
        ps_set_path(bin_dir)
    )
}

/// Removes the session mark, forgets the last directory seen and re-evaluates the hook.
pub fn shell_unset_script() -> &'static str {
    "Remove-Item env:NVSN_SHELL_VERSION -ErrorAction SilentlyContinue\n$global:_NVSN_AUTO = $false\n$global:_NVSN_PREV_PWD = $null\n_nvsn_hook 2>$null\n"
}

/// PowerShell code that deactivates the session's version: removes
/// `NVSN_VERSION`, `NVSN_SHELL_VERSION` and the nvsn `PATH` entry.
pub const DEACTIVATE: &str = r#"Remove-Item Env:NVSN_VERSION -ErrorAction SilentlyContinue
Remove-Item Env:NVSN_SHELL_VERSION -ErrorAction SilentlyContinue
if ($env:NVSN_PATH_ENTRY) {
    $env:PATH = (($env:PATH -split ';') | Where-Object { $_ -and $_ -ne $env:NVSN_PATH_ENTRY }) -join ';'
    Remove-Item Env:NVSN_PATH_ENTRY -ErrorAction SilentlyContinue
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> EnvContext<'static> {
        EnvContext {
            nvsn_dir: Path::new(r"C:\Users\José Ñu\.nvsn"),
            active_bin: Some(Path::new(r"C:\Users\José Ñu\.nvsn\versions\v20.11.1")),
            node_version: Some("v20.11.1"),
        }
    }

    #[test]
    fn env_script_quotes_windows_path_with_accent_and_space() {
        let script = env_script(&ctx());
        assert!(script.contains(r"$env:NVSN_DIR = 'C:\Users\José Ñu\.nvsn'"));
        assert!(
            script.contains(r"$env:NVSN_PATH_ENTRY = 'C:\Users\José Ñu\.nvsn\versions\v20.11.1'")
        );
        assert!(script.contains("$env:NVSN_VERSION = 'v20.11.1'"));
    }

    #[test]
    fn env_script_doubles_typographic_quote_in_path() {
        let ctx = EnvContext {
            nvsn_dir: Path::new("C:\\Users\\O\u{2019}Brien\\.nvsn"),
            active_bin: None,
            node_version: None,
        };
        assert!(env_script(&ctx).contains("'C:\\Users\\O\u{2019}\u{2019}Brien\\.nvsn'"));
    }

    #[test]
    fn hook_is_wrapped_around_prompt_not_set_location() {
        let script = env_script(&ctx());
        assert!(script.contains("function global:prompt"));
        assert!(
            !script.contains("Set-Location"),
            "must not override Set-Location"
        );
        assert!(!script.contains("Set-Alias"), "must not override cd");
    }

    #[test]
    fn hook_filters_before_calling_binary() {
        let script = env_script(&ctx());
        let filter = script
            .find("function global:_nvsn_has_version_file")
            .unwrap_or(usize::MAX);
        let call = script.find("nvsn __hook --shell powershell").unwrap_or(0);
        assert!(filter < call);
    }

    #[test]
    fn path_update_has_no_trailing_separator_source() {
        let script = ps_set_path(Path::new(r"C:\x"));
        assert!(script.contains("@('C:\\x') + @("));
        assert!(!script.contains("\"C:\\x;\""));
    }

    #[test]
    fn shell_version_script_uses_ps_syntax() {
        let s = shell_version_script(
            "v22.4.0",
            Path::new(r"C:\Users\user\.nvsn\versions\v22.4.0"),
            true,
        );
        assert!(s.contains("$env:NVSN_VERSION = 'v22.4.0'"));
        assert!(s.contains("$env:NVSN_SHELL_VERSION = 'v22.4.0'"));
        assert!(s.contains("$env:PATH"));
    }

    #[test]
    fn wrapper_handles_use_and_deactivate() {
        let w = wrapper_function();
        assert!(w.contains("'use', 'on', 'off'"));
        assert!(w.contains("_nvsn_deactivate"));
        assert!(w.contains("NVSN_INTEGRATION"));
    }
}
