//! Pieces shared by the POSIX dialects (bash, zsh and sh).
//!
//! All generated code uses only sh builtins: no `local`, no `[[` and no
//! `${var//}`, so that it also works in dash, ash and busybox. The PATH is
//! rebuilt without external processes, because the script is evaluated at every
//! shell startup.

use super::quote;
use std::path::Path;

/// Placeholder for the version's binary path inside the templates.
const BIN_PLACEHOLDER: &str = "__NVSN_BIN__";

/// Placeholder for the shell name that `nvsn deactivate --shell` receives.
const SHELL_PLACEHOLDER: &str = "__NVSN_SHELL__";

/// Removes the previous nvsn entry from `PATH` and prepends `bin`. It is idempotent:
/// running it twice leaves the same PATH, because the previous entry is
/// identified by `NVSN_PATH_ENTRY` and compared by exact line.
const SET_PATH_TEMPLATE: &str = r#"_nvsn_rest=
_nvsn_ifs=$IFS
_nvsn_noglob=0
case $- in *f*) _nvsn_noglob=1 ;; *) set -f ;; esac
IFS=:
for _nvsn_d in $PATH; do
    if [ -n "$_nvsn_d" ] && [ "$_nvsn_d" != "${NVSN_PATH_ENTRY-}" ]; then
        _nvsn_rest="${_nvsn_rest:+$_nvsn_rest:}$_nvsn_d"
    fi
done
IFS=$_nvsn_ifs
[ "$_nvsn_noglob" = 1 ] || set +f
unset _nvsn_ifs _nvsn_noglob _nvsn_d
PATH=__NVSN_BIN__
if [ -n "$_nvsn_rest" ]; then PATH="$PATH:$_nvsn_rest"; fi
export PATH
unset _nvsn_rest
export NVSN_PATH_ENTRY=__NVSN_BIN__
"#;

/// Function `_nvsn_has_version_file`: walks up from `$PWD` checking for `.nvmrc` and
/// `.node-version`. It is the cheap filter of the hook: the binary is only
/// launched if it returns 0 (ADR-017).
pub const HAS_VERSION_FILE_FN: &str = r#"_nvsn_has_version_file() {
    _nvsn_d="$PWD"
    while :; do
        if [ -f "$_nvsn_d/.nvmrc" ] || [ -f "$_nvsn_d/.node-version" ]; then
            return 0
        fi
        case "$_nvsn_d" in
            /) return 1 ;;
        esac
        _nvsn_d="${_nvsn_d%/*}"
        [ -n "$_nvsn_d" ] || _nvsn_d=/
    done
}
"#;

/// Directory-change hook for bash and zsh. It only runs if `$PWD` changed.
/// It calls `nvsn __hook` if there is a version file upwards, or
/// if a previous automatic activation has to be reverted.
///
/// It uses `command nvsn` and not `nvsn`: the `nvsn` function of the wrapper resolves
/// `deactivate` by applying the `eval` inside the `$(...)` subshell, and the
/// environment change was lost.
const HOOK_FN: &str = r#"_nvsn_hook() {
    [ -n "${NVSN_SHELL_VERSION-}" ] && return 0
    [ "$PWD" = "${_NVSN_PREV_PWD-}" ] && return 0
    _NVSN_PREV_PWD="$PWD"
    if _nvsn_has_version_file; then
        _NVSN_AUTO=1
    elif [ -n "${_NVSN_AUTO-}" ]; then
        unset _NVSN_AUTO
        if command -v nvsn > /dev/null 2>&1; then eval "$(command nvsn deactivate --shell __NVSN_SHELL__ 2>/dev/null)"; eval "$(command nvsn env --shell __NVSN_SHELL__ 2>/dev/null)"; fi
        return 0
    else
        return 0
    fi
    command -v nvsn > /dev/null 2>&1 || return 0
    eval "$(command nvsn __hook --shell __NVSN_SHELL__ 2>/dev/null)"
}
"#;

/// Deactivation: applies the output of `nvsn deactivate --shell <sh>`, removes the
/// manual pin and the automatic activation mark, and remembers the current directory
/// so that the hook does not reactivate the version in the same place.
const DEACTIVATE_FN: &str = r#"_nvsn_deactivate() {
    _nvsn_out="$(command nvsn deactivate --shell __NVSN_SHELL__)" || return $?
    eval "$_nvsn_out"
    unset NVSN_SHELL_VERSION _NVSN_AUTO
    _NVSN_PREV_PWD="$PWD"
}
"#;

/// `nvsn` wrapper shared by bash, zsh and sh. `use`, `on` and `off` apply the
/// output of the binary with `eval`. `deactivate` goes through `_nvsn_deactivate`.
/// Everything else goes straight to the binary.
pub const WRAPPER: &str = r#"nvsn() {
    case "${1-}" in
        deactivate)
            _nvsn_deactivate
            return $?
            ;;
        use|on|off)
            _nvsn_out="$(NVSN_INTEGRATION=1 command nvsn "$@")" || return $?
            eval "$_nvsn_out"
            return $?
            ;;
    esac
    command nvsn "$@"
}"#;

/// PATH block for `bin`: removes the previous nvsn entry and prepends `bin`.
pub fn set_path(bin: &Path) -> String {
    let bin_q = quote::posix(&bin.display().to_string());
    SET_PATH_TEMPLATE.replace(BIN_PLACEHOLDER, &bin_q)
}

/// `_nvsn_deactivate` function for the shell `sh` (`bash`, `zsh` or `sh`).
pub fn deactivate_fn(shell: &str) -> String {
    DEACTIVATE_FN.replace(SHELL_PLACEHOLDER, shell)
}

/// Hook definitions (filter, hook and deactivation) for `shell`, inside
/// a block that is only defined once per session. `guard` is the `if`
/// condition that indicates they are already defined. `registration` are the lines
/// that register the hook in the shell.
pub fn hook_block(shell: &str, guard: &str, registration: &str) -> String {
    let hook = HOOK_FN.replace(SHELL_PLACEHOLDER, shell);
    format!(
        "if ! {guard}; then\n{HAS_VERSION_FILE_FN}{hook}{deactivate}{registration}\nfi\n",
        deactivate = deactivate_fn(shell),
    )
}

/// Code that deactivates the session's version in bash, zsh and sh: removes
/// `NVSN_VERSION`, `NVSN_SHELL_VERSION` and the nvsn `PATH` entry.
pub const DEACTIVATE: &str = r#"unset NVSN_VERSION NVSN_SHELL_VERSION
if [ -n "${NVSN_PATH_ENTRY-}" ]; then
    _nvsn_rest="$(printf '%s\n' "$PATH" | tr ':' '\n' | grep -vxF "$NVSN_PATH_ENTRY" | tr '\n' ':' | sed 's/:$//')"
    if [ -n "$_nvsn_rest" ]; then
        export PATH="$_nvsn_rest"
    fi
    unset _nvsn_rest NVSN_PATH_ENTRY
fi
"#;
