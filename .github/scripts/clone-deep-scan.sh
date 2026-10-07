#!/usr/bin/env bash
# Second-stage forensic scan for a clone-watch candidate that already cleared
# clone-similarity.sh's threshold. Deliberately NOT run against every name
# search hit - it costs roughly one GitHub API call per commit, so it only
# makes sense against the small number of candidates already worth a human's
# attention, to hand them concrete evidence instead of just a score.
#
# Runs three checks, each aimed at a *behavior* a malicious redistribution has
# to exhibit almost by definition, rather than at any detail specific to one
# single incident. The point of keeping these behavior-based rather than
# signature-based is that a differently-structured clone, or an unrelated
# attacker with different tooling, still has to do at least one of these
# things to actually get a payload onto a victim's machine:
#
#   A. Links to a binary/archive from README/install scripts that does NOT
#      come from a GitHub Release - i.e. some other hosting entirely
#      (a raw source-control path, a personal domain, a paste site, a URL
#      shortener - the check doesn't care which). Bypassing this project's
#      actual release pipeline is the one thing every version of this attack
#      needs to do, independent of payload format or delivery technique.
#   B. A commit that ADDS a file with an executable/archive/script extension
#      anywhere in the tree. Legitimate Rust projects essentially never
#      commit binaries into source control - release artifacts belong in
#      GitHub Releases.
#   C. A file whose *first appearance* in the visible commit history is a
#      removal, with no earlier add/modify for that same path - only possible
#      if history was rewritten (rebase + force-push, or a squash) to erase
#      how the file got there in the first place, regardless of what that
#      file was.
#
# Usage: clone-deep-scan.sh <owner> <repo>
# Prints a human-readable report to stdout. Exits 0 even when nothing is
# found (absence of evidence here is not evidence of absence - it just means
# this heuristic didn't catch anything, not that the repo is clean).

set -euo pipefail

OWNER_CANDIDATE="${1:?usage: clone-deep-scan.sh <owner> <repo>}"
REPO_CANDIDATE="${2:?usage: clone-deep-scan.sh <owner> <repo>}"
MAX_COMMITS="${MAX_COMMITS:-300}"

# Two different extension lists for two different contexts - conflating them
# causes real false positives (verified below):
#
# - BINARY_EXT_RE: compiled/packaged artifacts only. Used for Check A (a
#   download link that isn't a GitHub Release). Deliberately excludes script
#   extensions like .ps1/.sh: this project's own install.ps1/install.sh are
#   legitimately fetched straight from a raw source URL as the documented
#   bootstrap one-liner (see SECURITY.md) - that pattern is normal for the
#   whole install.sh/curl-pipe-sh ecosystem, not a red flag. Also excludes
#   `.com` and `.pif`: legacy DOS executable extensions with negligible
#   real-world use, and in `.com`'s case actively dangerous to include here
#   since it collides with the `.com` TLD - it matched every single bare
#   `https://github.com`/`https://api.github.com` reference until this was
#   caught while validating against the real install.ps1.
BINARY_EXT_RE='\.(exe|dll|msi|jar|apk|zip|7z|rar)$'
#
# - LOOSE_FILE_EXT_RE: same set plus scripts (bat/cmd/scr/vbs/ps1). Used for
#   Check B (a file added to source control). A *loose* script buried in the
#   tree is still suspicious even though a top-level install.ps1/install.sh is
#   not - CANONICAL_INSTALLER_PATHS below exempts the latter explicitly
#   rather than trying to exclude scripts from this list wholesale.
LOOSE_FILE_EXT_RE='\.(exe|dll|bat|cmd|scr|vbs|jar|apk|ps1|msi|zip|7z|rar)$'
CANONICAL_INSTALLER_PATHS='^install/(install|setup|bootstrap)\.(sh|ps1)$'

fetch_file() {
  gh api "repos/${OWNER_CANDIDATE}/${REPO_CANDIDATE}/contents/$1" --jq '.content' 2>/dev/null \
    | tr -d '\n' | base64 -d 2>/dev/null || true
}

echo "### Deep scan: ${OWNER_CANDIDATE}/${REPO_CANDIDATE}"
echo

# --- Check A: binary downloads that bypass GitHub Releases ---------------
#
# Deliberately not keyed to any specific attacker's file names or hosting
# choice (raw.githubusercontent.com, a personal domain, a paste site, a URL
# shortener - all look the same to this check). What every one of these
# clones needs, regardless of payload or technique, is to get a binary onto
# the victim's machine somehow. The one thing a legitimate release of THIS
# project never does is serve one from anywhere but a GitHub Release, so any
# link to a file with a binary/archive extension that isn't a
# ".../releases/download/..." URL is suspicious on its own - independent of
# what generated it, what it's named, or which specific malware family (if
# any) it turns out to be.
echo "**Check A: binary/archive download links bypassing GitHub Releases**"
bypass_urls=()
for path in "README.md" "install/install.sh" "install/install.ps1"; do
  content="$(fetch_file "$path")"
  [ -z "$content" ] && continue
  while IFS= read -r url; do
    [ -z "$url" ] && continue
    if echo "$url" | grep -qiE "$BINARY_EXT_RE" && ! echo "$url" | grep -qE '/releases/download/'; then
      bypass_urls+=("${path}: ${url}")
    fi
  done < <(echo "$content" | grep -oE "https?://[^[:space:]\")>]+")
done
echo "Found: ${#bypass_urls[@]}"
if [ "${#bypass_urls[@]}" -gt 0 ]; then
  printf -- '- `%s`\n' "${bypass_urls[@]}"
fi
echo

# Oldest-first, capped at MAX_COMMITS: history-rewrite detection below needs
# chronological order, and this bounds worst-case API usage/runtime since
# this script makes one extra call per commit.
shas="$(gh api "repos/${OWNER_CANDIDATE}/${REPO_CANDIDATE}/commits?per_page=100" --paginate \
  --jq '.[].sha' 2>/dev/null | tail -n "$MAX_COMMITS" | tac || true)"

if [ -z "$shas" ]; then
  echo "_Could not list commits (private/empty/rate-limited/deleted). Skipping._"
  exit 0
fi

total_commits=$(echo "$shas" | grep -c . || true)

# --- Checks B & C: commit history walk -------------------------------------
#
# Both look at *behavior* (what changed, and whether the history is internally
# consistent), not at any fixed list of "known bad" file names - so they hold
# up against a differently-structured clone or a different attacker's tooling,
# not just a repeat of one specific incident.
echo "**Checks B & C:** scanned ${total_commits} commit(s) (oldest-first, capped at ${MAX_COMMITS})."
echo

declare -A seen_added=()
suspicious_adds=()
orphan_removals=()

while IFS= read -r sha; do
  [ -z "$sha" ] && continue
  files_json="$(gh api "repos/${OWNER_CANDIDATE}/${REPO_CANDIDATE}/commits/${sha}" \
    --jq '.files[]? | "\(.status)|\(.filename)"' 2>/dev/null || true)"
  [ -z "$files_json" ] && continue

  while IFS='|' read -r status filename; do
    [ -z "$filename" ] && continue

    if [ "$status" = "added" ]; then
      seen_added["$filename"]=1
      if echo "$filename" | grep -qiE "$LOOSE_FILE_EXT_RE" && ! echo "$filename" | grep -qE "$CANONICAL_INSTALLER_PATHS"; then
        suspicious_adds+=("${sha:0:10}  added     ${filename}")
      fi
    elif [ "$status" = "modified" ] || [ "$status" = "renamed" ] || [ "$status" = "copied" ]; then
      seen_added["$filename"]=1
    elif [ "$status" = "removed" ]; then
      if [ -z "${seen_added[$filename]:-}" ]; then
        orphan_removals+=("${sha:0:10}  removed   ${filename}  (no earlier add/modify in this scan)")
      fi
    fi
  done <<< "$files_json"
done <<< "$shas"

echo "**Check B: binary/archive/script files ever added to source control:** ${#suspicious_adds[@]}"
if [ "${#suspicious_adds[@]}" -gt 0 ]; then
  printf -- '- `%s`\n' "${suspicious_adds[@]}"
fi
echo
echo "**Check C: files whose first appearance in history is a removal (possible history rewrite):** ${#orphan_removals[@]}"
if [ "${#orphan_removals[@]}" -gt 0 ]; then
  printf -- '- `%s`\n' "${orphan_removals[@]}"
fi
