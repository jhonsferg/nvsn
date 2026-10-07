#!/usr/bin/env bash
# Scores how likely a candidate GitHub repository is an actual content clone of
# this repository, as opposed to an unrelated project that merely shares part of
# its name.
#
# A plain name search (see clone-watch.sh Strategy 1) can still surface
# unrelated repos that merely share part of the name, so name-matching alone
# is not enough signal to file an issue over - this script adds a
# content-based check.
#
# Must be run from a checkout of this repository - it reads the local
# crates/nvsn-shell/src/profile.rs, crates/nvsn-shell/src/shell/posix.rs,
# README.md and crates/nvsn-cli/Cargo.toml as the canonical fingerprints and
# fetches the candidate's copies via the GitHub Contents API for comparison.
#
# Usage: clone-similarity.sh <owner> <repo>
# Prints one line to stdout: SCORE=<0-100> SIGNALS=<comma-separated,or none>
#
# Signals and their weight:
#   source-fingerprint (70) - candidate's profile.rs and shell/posix.rs contain
#                             both of this project's distinctive internal
#                             markers ("# nvsn init", "_nvsn_hook"). This is
#                             the load-bearing signal: those two literal
#                             strings together, at those exact paths, are not
#                             something an unrelated project would produce by
#                             chance - only an actual copy (full or partial)
#                             of this repository's source would have them.
#   readme-overlap      (20) - at least 50% of this repo's README.md lines
#                             appear verbatim in the candidate's README.md.
#                             Deliberately high bar: generic "Installation" /
#                             "Usage" section headers overlap between any two
#                             CLI tool READMEs, so a low bar produces noise.
#   cargo-package-name  (10) - candidate's crates/nvsn-cli/Cargo.toml declares
#                             the exact package name "nvsn-cli". Kept as a
#                             weak, corroborating-only signal - deliberately
#                             NOT enough by itself to move the needle, since a
#                             matching crate name alone doesn't prove the
#                             source was actually copied.
#
# A real, unmodified or lightly-modified clone (the common case: someone
# cloned the git history or downloaded a release snapshot and re-pushed it)
# reliably hits source-fingerprint. An unrelated project with a similar name,
# even one that also happens to be a Rust CLI tool, does not.

set -euo pipefail

OWNER_CANDIDATE="${1:?usage: clone-similarity.sh <owner> <repo>}"
REPO_CANDIDATE="${2:?usage: clone-similarity.sh <owner> <repo>}"

fetch_file() {
  # $1 = path in the candidate repo. Prints decoded content, or nothing on
  # 404 / any error (deliberately swallowed - a missing file is a "no match"
  # signal, not a script failure).
  gh api "repos/${OWNER_CANDIDATE}/${REPO_CANDIDATE}/contents/$1" --jq '.content' 2>/dev/null \
    | tr -d '\n' | base64 -d 2>/dev/null || true
}

score=0
signals=()

# --- Signal: distinctive source fingerprint (dominant) ---
their_profile="$(fetch_file "crates/nvsn-shell/src/profile.rs")"
their_posix="$(fetch_file "crates/nvsn-shell/src/shell/posix.rs")"
if [ -n "$their_profile" ] && [ -n "$their_posix" ] \
  && echo "$their_profile" | grep -q '# nvsn init' \
  && echo "$their_posix" | grep -q '_nvsn_hook'; then
  score=$((score + 70))
  signals+=("source-fingerprint:profile.rs+shell/posix.rs")
fi

# --- Signal: README.md line-level overlap (high bar) ---
their_readme="$(fetch_file "README.md")"
if [ -n "$their_readme" ]; then
  our_lines=$(grep -c . README.md || true)
  if [ "${our_lines:-0}" -gt 0 ]; then
    shared_lines=$(comm -12 <(sort -u README.md) <(echo "$their_readme" | sort -u) | grep -c . || true)
    pct=$(( shared_lines * 100 / our_lines ))
    if [ "$pct" -ge 50 ]; then
      score=$((score + 20))
      signals+=("readme-overlap:${pct}%")
    fi
  fi
fi

# --- Signal: Cargo.toml package identity (weak, corroborating only) ---
their_cargo="$(fetch_file "crates/nvsn-cli/Cargo.toml")"
if [ -n "$their_cargo" ] && echo "$their_cargo" | grep -qE '^[[:space:]]*name[[:space:]]*=[[:space:]]*"nvsn-cli"[[:space:]]*$'; then
  score=$((score + 10))
  signals+=("cargo-package-name")
fi

if [ "$score" -gt 100 ]; then score=100; fi

signals_joined=$(IFS=,; echo "${signals[*]:-none}")
echo "SCORE=${score} SIGNALS=${signals_joined}"
