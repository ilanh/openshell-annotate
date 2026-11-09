#!/usr/bin/env bash
# Run `openshell-annotate lint` in the current directory (the repository root); annotations + job summary.
# in: POLICY, PATHS (space-separated), THRESHOLD (error|warning|never)
set -euo pipefail

lib_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root_abs="$(pwd)"
threshold="${THRESHOLD:-error}"
args=(lint --root "$root_abs" --format json)
if [ -n "${POLICY:-}" ]; then
  args+=(--policy "$POLICY")
fi
if [ "$threshold" = "warning" ]; then
  args+=(--deny-warnings)
fi
# shellcheck disable=SC2206
extra=(${PATHS:-})
if [ "${#extra[@]}" -gt 0 ]; then
  args+=("${extra[@]}")
fi
tmp="${RUNNER_TEMP:-/tmp}"
findings="$tmp/lint.json"
stderr="$tmp/lint.stderr"
set +e
openshell-annotate "${args[@]}" >"$findings" 2>"$stderr"
cli_code=$?
set -e
cat "$stderr" >&2
set +e
bash "$lib_dir/lint-report.sh" "$findings" "$threshold"
report_code=$?
set -e
if [ "$threshold" = "never" ]; then
  if [ "$cli_code" -ne 0 ]; then
    echo "::notice::lint reported findings but failure-threshold=never (step succeeds)."
  fi
  exit 0
fi
if [ "$report_code" -ne 0 ]; then
  exit "$report_code"
fi
if [ "$cli_code" -eq 2 ]; then
  echo "::error::openshell-annotate lint failed with a usage or I/O error (exit 2)."
  exit 2
fi
