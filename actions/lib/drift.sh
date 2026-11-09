#!/usr/bin/env bash
# Run `openshell-annotate drift` in the current directory (the repository root) and write a job summary.
# in: POLICY (required), PROVIDERS, PROFILES_DIR, STRICT (true|false), PATHS (space-separated)
set -euo pipefail

lib_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root_abs="$(pwd)"
args=(drift --root "$root_abs" --policy "${POLICY:?policy input is required}" --json)
if [ -n "${PROVIDERS:-}" ]; then
  args+=(--providers "$PROVIDERS")
fi
if [ -n "${PROFILES_DIR:-}" ]; then
  args+=(--profiles-dir "$PROFILES_DIR")
fi
if [ "${STRICT:-false}" = "true" ]; then
  args+=(--strict)
fi
# shellcheck disable=SC2206
extra=(${PATHS:-})
if [ "${#extra[@]}" -gt 0 ]; then
  args+=("${extra[@]}")
fi
tmp="${RUNNER_TEMP:-/tmp}"
report="$tmp/drift.json"
set +e
openshell-annotate "${args[@]}" >"$report" 2>"$tmp/drift.stderr"
code=$?
set -e
cat "$tmp/drift.stderr" >&2
bash "$lib_dir/drift-summary.sh" "$report"
if [ "$code" -ne 0 ]; then
  echo "::error::Policy drift detected (exit $code). See the job summary and logs."
  exit "$code"
fi
echo "no drift"
