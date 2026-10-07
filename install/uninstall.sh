#!/bin/sh
# nvsn (Node Version Manager) -- Linux and macOS uninstaller
#
# Removes every trace of nvsn from the system:
#   - The data directory: all installed Node.js versions, the default and
#     alias pointers, and the staging area. Location: $NVSN_DIR if set, else
#     $XDG_DATA_HOME/nvsn or ~/.local/share/nvsn (~/.nvsn on Termux).
#   - The nvsn binary itself.
#   - Every nvsn-managed block (# nvsn init, # nvsn wrapper, # nvsn path) from
#     the shell profiles nvsn can write to, wherever they are found.
#   - Leftover temp files from an install that was interrupted mid-way.
#
# Usage (one-liner):
#   curl -fsSL https://raw.githubusercontent.com/jhonsferg/nvsn/main/install/uninstall.sh | sh
#
# Pass flags through when piping by using `sh -s --`:
#   curl -fsSL .../uninstall.sh | sh -s -- --force
#   curl -fsSL .../uninstall.sh | sh -s -- --dry-run
#
# Or set these before piping instead:
#   NVSN_UNINSTALL_FORCE=1 curl -fsSL .../uninstall.sh | sh
#   NVSN_UNINSTALL_DRY_RUN=1 curl -fsSL .../uninstall.sh | sh
#
# Customise the locations to clean (only needed if you used these at
# install time):
#   NVSN_DIR=$HOME/.nvsn NVSN_INSTALL_DIR=$HOME/.local/bin sh uninstall.sh

set -eu

INSTALL_DIR="${NVSN_INSTALL_DIR:-$HOME/.local/bin}"

FORCE=0
[ "${NVSN_UNINSTALL_FORCE:-}" = "1" ] && FORCE=1
DRY_RUN=0
[ "${NVSN_UNINSTALL_DRY_RUN:-}" = "1" ] && DRY_RUN=1
for arg in "$@"; do
    case "$arg" in
        --force | -f) FORCE=1 ;;
        --dry-run | -n) DRY_RUN=1 ;;
    esac
done

# -- Terminal helpers ----------------------------------------------------------
if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
    C_CYAN='\033[0;36m'
    C_GREEN='\033[0;32m'
    C_YELLOW='\033[1;33m'
    C_RED='\033[0;31m'
    C_BOLD='\033[1m'
    C_RESET='\033[0m'
else
    C_CYAN='' C_GREEN='' C_YELLOW='' C_RED='' C_BOLD='' C_RESET=''
fi

step() { printf "  ${C_CYAN}->${C_RESET} %s\n" "$1"; }
ok()   { printf "  ${C_GREEN}v ${C_RESET} %s\n" "$1"; }
die()  { printf "  ${C_RED}x ${C_RESET} %b\n" "$1" >&2; exit 1; }

printf "\n  ${C_BOLD}${C_RED}nvsn${C_RESET}${C_BOLD} -- uninstaller${C_RESET}\n\n"

# -- Resolve the data directory ------------------------------------------------
# Mirrors nvsn-platform's default_nvsn_dir: NVSN_DIR wins; Termux uses
# ~/.nvsn; other systems use $XDG_DATA_HOME/nvsn when absolute.
is_termux() {
    [ -n "${TERMUX_VERSION:-}" ] || [ -d "/data/data/com.termux" ]
}

if [ -n "${NVSN_DIR:-}" ]; then
    NVSN_DIR_PATH="$NVSN_DIR"
elif is_termux; then
    NVSN_DIR_PATH="$HOME/.nvsn"
else
    case "${XDG_DATA_HOME:-}" in
        /*) NVSN_DIR_PATH="$XDG_DATA_HOME/nvsn" ;;
        *)  NVSN_DIR_PATH="$HOME/.local/share/nvsn" ;;
    esac
fi

# Refuse to delete anything obviously dangerous, whatever the variables say.
case "$NVSN_DIR_PATH" in
    "" | "/" | "$HOME" | "$HOME/")
        die "Refusing to remove '$NVSN_DIR_PATH' as the nvsn data directory. Set NVSN_DIR to a dedicated folder." ;;
esac

# -- Strip nvsn-managed blocks from a profile file -----------------------------
#
# A block starts at one of the nvsn marker lines and ends at the next blank
# line (or EOF). Everything else is left exactly as it was. The file is
# rewritten in place (cat, not mv) so its permissions and inode are kept.
#
# Returns success (0) if the file was changed, failure (1) otherwise.
strip_nvsn_blocks() {
    file="$1"
    [ -f "$file" ] || return 1
    tmp="$(mktemp)"
    awk '
        /^# nvsn init$/ || /^# nvsn wrapper$/ || /^# nvsn path$/ {
            in_block = 1
            next
        }
        in_block && NF == 0 { in_block = 0; next }
        in_block { next }
        { print }
    ' "$file" > "$tmp"
    if cmp -s "$file" "$tmp"; then
        rm -f "$tmp"
        return 1
    fi
    cat "$tmp" > "$file"
    rm -f "$tmp"
    return 0
}

# -- Locate the binary ---------------------------------------------------------
NVSN_BIN="$(command -v nvsn 2> /dev/null || true)"
if [ -z "$NVSN_BIN" ] && [ -x "$INSTALL_DIR/nvsn" ]; then
    NVSN_BIN="$INSTALL_DIR/nvsn"
fi

# -- Profiles that nvsn init can write to --------------------------------------
PROFILES="$HOME/.bashrc $HOME/.zshrc $HOME/.config/fish/config.fish $HOME/.profile $HOME/.zprofile"

FOUND_PROFILES=""
for p in $PROFILES; do
    if [ -f "$p" ] && grep -Eq '^# nvsn (init|wrapper|path)$' "$p" 2> /dev/null; then
        FOUND_PROFILES="$FOUND_PROFILES $p"
    fi
done

VERSION_COUNT=0
if [ -d "$NVSN_DIR_PATH/versions" ]; then
    VERSION_COUNT="$(find "$NVSN_DIR_PATH/versions" -mindepth 1 -maxdepth 1 -type d 2> /dev/null | wc -l | tr -d ' ')"
fi

# -- Print removal plan --------------------------------------------------------
printf "  ${C_BOLD}This will permanently remove:${C_RESET}\n\n"
if [ -d "$NVSN_DIR_PATH" ]; then
    plural="s"
    [ "$VERSION_COUNT" = "1" ] && plural=""
    printf "  ${C_CYAN}->${C_RESET} %s (%s installed version%s, plus cache/tmp)\n" "$NVSN_DIR_PATH" "$VERSION_COUNT" "$plural"
else
    printf "  ${C_CYAN}->${C_RESET} %s (not found)\n" "$NVSN_DIR_PATH"
fi
if [ -n "$NVSN_BIN" ]; then
    printf "  ${C_CYAN}->${C_RESET} %s\n" "$NVSN_BIN"
else
    printf "  ${C_CYAN}->${C_RESET} nvsn binary (not found in PATH or %s)\n" "$INSTALL_DIR"
fi
if [ -n "$FOUND_PROFILES" ]; then
    for p in $FOUND_PROFILES; do
        printf "  ${C_CYAN}->${C_RESET} %s (nvsn lines removed)\n" "$p"
    done
else
    printf "  ${C_CYAN}->${C_RESET} no nvsn entries found in any shell profile\n"
fi
printf "  ${C_CYAN}->${C_RESET} any leftover temp files from an interrupted install\n"
printf "\n"

if [ "$DRY_RUN" = "1" ]; then
    printf "  ${C_YELLOW}Dry run - nothing was removed.${C_RESET}\n\n"
    exit 0
fi

# -- Confirm -------------------------------------------------------------------
if [ "$FORCE" != "1" ]; then
    if { : < /dev/tty; } 2> /dev/null; then
        printf "  Type ${C_BOLD}yes${C_RESET} to confirm: "
        read -r REPLY < /dev/tty
    else
        die "No interactive terminal to confirm on. Re-run with --force (or NVSN_UNINSTALL_FORCE=1) to proceed non-interactively."
    fi
    case "$(printf '%s' "$REPLY" | tr '[:upper:]' '[:lower:]')" in
        y | yes) ;;
        *) die "Aborted." ;;
    esac
    printf "\n"
fi

# -- Remove data directory -----------------------------------------------------
if [ -d "$NVSN_DIR_PATH" ]; then
    rm -rf "$NVSN_DIR_PATH"
    ok "Removed $NVSN_DIR_PATH"
fi

# -- Clean shell profiles ------------------------------------------------------
for p in $PROFILES; do
    if strip_nvsn_blocks "$p"; then
        ok "Cleaned $p"
    fi
done

# -- Sweep leftover temp files from interrupted installs -----------------------
TMP_BASE="${TMPDIR:-/tmp}"
for f in "$TMP_BASE"/nvsn-install.*; do
    [ -e "$f" ] || continue
    rm -rf "$f"
done
for f in "$INSTALL_DIR"/.nvsn-install.*; do
    [ -e "$f" ] || continue
    rm -f "$f"
done

# -- Remove the binary (last, so earlier steps still had it available) ---------
if [ -n "$NVSN_BIN" ] && [ -f "$NVSN_BIN" ]; then
    rm -f "$NVSN_BIN"
    ok "Removed $NVSN_BIN"
fi
if [ -f "$INSTALL_DIR/nvsn" ] && [ "$INSTALL_DIR/nvsn" != "$NVSN_BIN" ]; then
    rm -f "$INSTALL_DIR/nvsn"
    ok "Removed $INSTALL_DIR/nvsn"
fi

printf "\n  ${C_GREEN}${C_BOLD}nvsn has been completely removed.${C_RESET}\n"
printf "  Open a new terminal for the profile changes to take effect.\n\n"
printf "  ${C_YELLOW}Note:${C_RESET} if you saved shell completions manually (nvsn completions ...),\n"
printf "  remove that file yourself - nvsn does not track where it was written.\n\n"
