#!/usr/bin/env bash
# Append human-readable diff-logs output (from a log file) to the job summary.
set -euo pipefail

LOG="${1:?path to captured diff-logs stdout}"
summary="${GITHUB_STEP_SUMMARY:-/dev/stdout}"

{
  echo "## openshell-annotate diff-logs"
  echo ""
  if [ ! -s "$LOG" ]; then
    echo "_No diff-logs output._"
  else
    echo '```text'
    head -c 120000 "$LOG"
    echo '```'
  fi
} >>"$summary"
