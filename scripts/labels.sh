#!/usr/bin/env bash
# Create or update the issue labels used by this repo. Idempotent (`gh label create --force`).
#   scripts/labels.sh                 # uses the repo of the current directory
#   REPO=owner/name scripts/labels.sh
set -euo pipefail
REPO="${REPO:-$(gh repo view --json nameWithOwner -q .nameWithOwner)}"

# name|color|description
LABELS=(
  "bug|d73a4a|Something is not working"
  "enhancement|a2eeef|New feature or request"
  "docs|0075ca|Documentation"
  "lint|fbca04|openshell-annotate lint"
  "diff-logs|c5def5|openshell-annotate diff-logs"
  "generate|0e8a16|openshell-annotate generate"
  "scan|1d76db|openshell-annotate scan"
  "providers|5319e7|Provider detection and profiles"
  "languages|bfdadc|Source language and comment support"
  "good first issue|7057ff|Good for newcomers"
  "feedback|ededed|Early-tester feedback and usage reports"
  "upstream-openshell|b60205|Behaviour of OpenShell itself; tracked locally, not filed upstream"
  "upstream-compat|e99695|Breakage against a newer OpenShell (opened by the weekly job)"
  "openshell-next|1d76db|Tracking the next OpenShell release on the openshell-next branch"
)

for entry in "${LABELS[@]}"; do
  IFS='|' read -r name color desc <<<"$entry"
  gh label create "$name" --repo "$REPO" --color "$color" --description "$desc" --force
done
echo "labels synced for $REPO"
