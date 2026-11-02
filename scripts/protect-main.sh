#!/usr/bin/env bash
# Apply the `main` protection ruleset (.github/rulesets/protect-main.json) and the repo merge settings.
#   scripts/protect-main.sh [owner/repo]        # default: the current repo (needs `gh` logged in with admin rights)
# Requires a plan that supports rulesets: a public repo, or GitHub Pro/Team for a private one
# (otherwise GitHub answers HTTP 403 "Upgrade to GitHub Pro or make this repository public").
# Idempotent: updates the ruleset named "protect-main" if it already exists.
set -euo pipefail
cd "$(dirname "$0")/.."
REPO="${1:-$(gh repo view --json nameWithOwner --jq .nameWithOwner)}"
FILE=.github/rulesets/protect-main.json

# Merge settings: squash only, delete head branches after merge.
gh api -X PATCH "repos/$REPO" \
  -F allow_squash_merge=true -F allow_merge_commit=false -F allow_rebase_merge=false \
  -F delete_branch_on_merge=true \
  -f squash_merge_commit_title=PR_TITLE -f squash_merge_commit_message=PR_BODY >/dev/null
echo "merge settings applied to $REPO"

ID=$(gh api "repos/$REPO/rulesets" --jq '.[] | select(.name=="protect-main") | .id' || true)
if [ -n "$ID" ]; then
  gh api -X PUT "repos/$REPO/rulesets/$ID" --input "$FILE" >/dev/null
  echo "ruleset protect-main updated (id $ID)"
else
  gh api -X POST "repos/$REPO/rulesets" --input "$FILE" >/dev/null
  echo "ruleset protect-main created"
fi
echo "verify: gh api repos/$REPO/rules/branches/main --jq '.[].type'"
