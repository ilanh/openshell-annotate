#!/usr/bin/env bash
# Append a drift report JSON file to GITHUB_STEP_SUMMARY.
set -euo pipefail

REPORT="${1:?drift json path}"
summary="${GITHUB_STEP_SUMMARY:-/dev/stdout}"

{
  echo "## openshell-annotate drift"
  echo ""
  if [ ! -s "$REPORT" ]; then
    echo "_No drift report (clean or command did not emit JSON)._"
    exit 0
  fi
  count=$(jq '.findings | length' "$REPORT")
  if [ "$count" -eq 0 ]; then
    echo "_No drift: committed policy matches directives._"
    exit 0
  fi
  echo "**${count}** finding(s). Regenerate with \`openshell-annotate generate\`."
  echo ""
  echo "| Change | Section | Item | Detail |"
  echo "| --- | --- | --- | --- |"
  jq -r '.findings[] | "| \(.change) | \(.section) | \(.item | gsub("\\|"; "/")) | \(.detail | gsub("\\|"; "/")) |"' "$REPORT"
} >>"$summary"
