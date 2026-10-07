#!/usr/bin/env bash
# Looks for unauthorized clones/impersonations of this repository elsewhere on GitHub.
# Run on a schedule (see .github/workflows/clone-watch.yml), independent of the build/CI flow.
#
# Two detection strategies:
#   1. Repos with the same name, not owned by us, not a fork.
#   2. Commits authored by the maintainer's email in repos owned by someone else
#      (catches renamed clones that still carry over the original commit history).
#
# Strategy 1 alone can be noisy: a plain name search can turn up unrelated
# repos that merely share part of the name. Strategy 1 candidates are
# therefore run through clone-similarity.sh, which checks actual file content (a
# distinctive source fingerprint, README overlap, Cargo.toml package name)
# against this repository, and are only kept if the score clears
# $CONTENT_SCORE_THRESHOLD. Strategy 2 candidates are not scored - sharing
# actual commit history authored by the maintainer's email is already
# decisive on its own.
#
# Findings are filtered against .github/clone-watch-allowlist.txt (known legitimate
# collaborations) and deduplicated against already-open `clone-watch` issues before
# a new issue is filed.

set -euo pipefail

OWNER="${OWNER:?OWNER env var required}"
REPO_NAME="${REPO_NAME:?REPO_NAME env var required}"
AUTHOR_EMAIL="${AUTHOR_EMAIL:?AUTHOR_EMAIL env var required}"
ALLOWLIST_FILE="${ALLOWLIST_FILE:-.github/clone-watch-allowlist.txt}"
LABEL="clone-watch"
CONTENT_SCORE_THRESHOLD="${CONTENT_SCORE_THRESHOLD:-40}"
SIMILARITY_SCRIPT="$(dirname "${BASH_SOURCE[0]}")/clone-similarity.sh"
DEEP_SCAN_SCRIPT="$(dirname "${BASH_SOURCE[0]}")/clone-deep-scan.sh"

is_allowlisted() {
  local candidate_owner_lower
  candidate_owner_lower=$(echo "$1" | tr '[:upper:]' '[:lower:]')
  if [ -f "$ALLOWLIST_FILE" ]; then
    while IFS= read -r line; do
      line="${line%%#*}"
      line="$(echo "$line" | xargs || true)"
      [ -z "$line" ] && continue
      if [ "$(echo "$line" | tr '[:upper:]' '[:lower:]')" = "$candidate_owner_lower" ]; then
        return 0
      fi
    done < "$ALLOWLIST_FILE"
  fi
  return 1
}

echo "== Strategy 1: repos named '$REPO_NAME' not owned by $OWNER =="
name_match_candidates=$(gh api "search/repositories?q=${REPO_NAME}+in:name&per_page=50" \
  --jq ".items[] | select(.owner.login != \"$OWNER\") | select(.fork == false) | \"\(.full_name)|\(.owner.login)|\(.html_url)|\(.created_at)\"")

echo "== Strategy 1b: scoring name-match candidates by content similarity (threshold: $CONTENT_SCORE_THRESHOLD) =="
name_matches=""
while IFS= read -r candidate; do
  [ -z "$candidate" ] && continue
  cand_full_name="$(echo "$candidate" | cut -d'|' -f1)"
  cand_owner="$(echo "$candidate" | cut -d'|' -f2)"

  score_line="$(bash "$SIMILARITY_SCRIPT" "$cand_owner" "$(echo "$cand_full_name" | cut -d'/' -f2)")"
  score="$(echo "$score_line" | grep -oE 'SCORE=[0-9]+' | cut -d'=' -f2)"
  signals="$(echo "$score_line" | grep -oE 'SIGNALS=.*' | cut -d'=' -f2-)"

  if [ "${score:-0}" -lt "$CONTENT_SCORE_THRESHOLD" ]; then
    echo "skip (content score ${score:-0} < ${CONTENT_SCORE_THRESHOLD}, signals: ${signals:-none}): $cand_full_name"
    continue
  fi

  echo "keep (content score ${score}, signals: ${signals}): $cand_full_name"
  name_matches="${name_matches}${candidate}|name-match (content-score=${score}: ${signals})"$'\n'
done <<< "$name_match_candidates"
name_matches=$(echo "$name_matches" | sed '/^$/d')

echo "== Strategy 2: commits authored by $AUTHOR_EMAIL in repos not owned by $OWNER =="
commit_matches=$(gh api "search/commits?q=author-email:${AUTHOR_EMAIL}&per_page=50" \
  --jq ".items[] | select(.repository.owner.login != \"$OWNER\") | select(.repository.fork == false) | \"\(.repository.full_name)|\(.repository.owner.login)|\(.repository.html_url)|\(.repository.created_at // \"unknown\")|commit-history-match\"")

all_matches=$(printf '%s\n%s\n' "$name_matches" "$commit_matches" | sed '/^$/d' | sort -u -t'|' -k1,1)

if [ -z "$all_matches" ]; then
  echo "No matches found at all (allowlisted or not). Nothing to do."
  exit 0
fi

echo "== Existing open clone-watch issues (for dedup) =="
existing_issues_body=$(gh issue list --label "$LABEL" --state open --json body --jq '[.[].body] | join("\n---\n")' || true)

new_findings=""
while IFS= read -r match; do
  [ -z "$match" ] && continue
  full_name="${match%%|*}"
  owner="$(echo "$match" | cut -d'|' -f2)"

  if is_allowlisted "$owner"; then
    echo "skip (allowlisted): $full_name"
    continue
  fi

  if echo "$existing_issues_body" | grep -qF "$full_name"; then
    echo "skip (already reported in an open issue): $full_name"
    continue
  fi

  echo "NEW finding: $match"
  new_findings="${new_findings}${match}"$'\n'
done <<< "$all_matches"

new_findings=$(echo "$new_findings" | sed '/^$/d')

if [ -z "$new_findings" ]; then
  echo "All matches are allowlisted or already tracked. Nothing new to report."
  exit 0
fi

body="Automated scan found repositories that may be unauthorized clones of \`${OWNER}/${REPO_NAME}\`.

This does **not** automatically mean malicious intent - review each one manually before taking any action (e.g. reporting to GitHub via https://github.com/contact/report-abuse). See [SECURITY.md](../blob/main/SECURITY.md#malicious-forks--clones) for the criteria and process.

Name-only matches are additionally scored by actual content similarity against this repository (source fingerprint, README overlap, \`Cargo.toml\` package name) before being listed here - see the score/signals in \"Detected via\" for each finding, and \`.github/scripts/clone-similarity.sh\` for the methodology. Commit-history matches are not scored, since sharing this project's actual commit history is already conclusive on its own.

If a finding here is actually a legitimate collaboration, add its owner to \`.github/clone-watch-allowlist.txt\` to silence it.

| Repository | Owner | Detected via | Created | Link |
|---|---|---|---|---|
"

while IFS= read -r match; do
  [ -z "$match" ] && continue
  full_name="$(echo "$match" | cut -d'|' -f1)"
  owner="$(echo "$match" | cut -d'|' -f2)"
  url="$(echo "$match" | cut -d'|' -f3)"
  created="$(echo "$match" | cut -d'|' -f4)"
  reason="$(echo "$match" | cut -d'|' -f5)"
  body="${body}| \`${full_name}\` | \`${owner}\` | ${reason} | ${created} | ${url} |
"
done <<< "$new_findings"

echo "== Deep-scanning new findings for concrete evidence (this may take a while) =="
body="${body}
## Deep scan

Behavior-based forensic checks against each finding above - see
\`.github/scripts/clone-deep-scan.sh\` for what each check looks for and why. Best-effort: a scan
that fails or times out is skipped rather than blocking the whole run, and its absence here is not
evidence the repository is clean.
"

while IFS= read -r match; do
  [ -z "$match" ] && continue
  full_name="$(echo "$match" | cut -d'|' -f1)"
  owner="$(echo "$match" | cut -d'|' -f2)"
  repo_name_only="$(echo "$full_name" | cut -d'/' -f2)"

  echo "-- deep scan: $full_name --"
  scan_report="$(timeout 240 bash "$DEEP_SCAN_SCRIPT" "$owner" "$repo_name_only" 2>&1 || echo "_Deep scan failed or timed out for ${full_name} - skipped._")"

  body="${body}
<details>
<summary>\`${full_name}\`</summary>

${scan_report}

</details>
"
done <<< "$new_findings"

echo "Creating issue with $(echo "$new_findings" | wc -l) new finding(s)..."
gh issue create \
  --title "[clone-watch] Possible unauthorized clone(s) detected - $(date -u +%Y-%m-%d)" \
  --label "$LABEL" \
  --body "$body"
