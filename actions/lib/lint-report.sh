#!/usr/bin/env bash
# Parse `openshell-annotate lint --format json` stdout into GitHub annotations and a job summary.
# Usage: lint-report.sh FINDINGS.json [failure-threshold]
# failure-threshold: error (default) | warning | never — only affects whether this script exits non-zero;
# the caller should pass --deny-warnings to the CLI when threshold is warning.
set -euo pipefail

FINDINGS="${1:?findings json path}"
THRESHOLD="${2:-error}"

if [ ! -s "$FINDINGS" ]; then
  echo '[]' >"$FINDINGS"
fi

emit_annotation() {
  local sev="$1" file="$2" line="$3" msg="$4"
  [ -z "$file" ] || [ "$file" = "." ] && file=""
  local loc=""
  if [ -n "$file" ] && [ "$line" -gt 0 ] 2>/dev/null; then
    loc="file=${file},line=${line}"
  elif [ -n "$file" ]; then
    loc="file=${file}"
  fi
  case "$sev" in
    error) echo "::error ${loc}::${msg}" ;;
    warning) echo "::warning ${loc}::${msg}" ;;
    info) echo "::notice ${loc}::${msg}" ;;
    *) echo "::notice ${loc}::[${sev}] ${msg}" ;;
  esac
}

if ! command -v jq >/dev/null 2>&1; then
  die() { echo "$*" >&2; exit 1; }
  die "lint-report.sh requires jq"
fi

count_err=0 count_warn=0 count_info=0
while IFS= read -r row; do
  sev=$(jq -r '.severity' <<<"$row")
  file=$(jq -r '.file' <<<"$row")
  line=$(jq -r '.line' <<<"$row")
  cat=$(jq -r '.category' <<<"$row")
  msg=$(jq -r '.message' <<<"$row")
  full="[${cat}] ${msg}"
  emit_annotation "$sev" "$file" "$line" "$full"
  case "$sev" in
    error) count_err=$((count_err + 1)) ;;
    warning) count_warn=$((count_warn + 1)) ;;
    info) count_info=$((count_info + 1)) ;;
  esac
done < <(jq -c '.[]' "$FINDINGS")

summary="${GITHUB_STEP_SUMMARY:-/dev/stdout}"
{
  echo "## openshell-annotate lint"
  echo ""
  echo "| Severity | Count |"
  echo "| --- | ---: |"
  echo "| error | ${count_err} |"
  echo "| warning | ${count_warn} |"
  echo "| info | ${count_info} |"
  echo ""
  if [ "$count_err" -gt 0 ] || [ "$count_warn" -gt 0 ] || [ "$count_info" -gt 0 ]; then
    echo "<details><summary>Findings</summary>"
    echo ""
    echo "| File | Line | Severity | Category | Message |"
    echo "| --- | ---: | --- | --- | --- |"
    jq -r '.[] | "| \(.file) | \(.line) | \(.severity) | \(.category) | \(.message | gsub("\\|"; "/")) |"' "$FINDINGS"
    echo ""
    echo "</details>"
  else
    echo "_No findings._"
  fi
} >>"$summary"

exit_code=0
case "$THRESHOLD" in
  error) [ "$count_err" -eq 0 ] || exit_code=1 ;;
  warning) [ "$count_err" -eq 0 ] && [ "$count_warn" -eq 0 ] || exit_code=1 ;;
  never) exit_code=0 ;;
  *) echo "::warning::unknown failure-threshold '$THRESHOLD'; treating as error" >&2; [ "$count_err" -eq 0 ] || exit_code=1 ;;
esac
exit "$exit_code"
