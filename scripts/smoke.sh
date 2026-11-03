#!/usr/bin/env bash
# Smoke-test a built openshell-annotate binary: scan -> generate -> drift on a tiny annotated file.
#   scripts/smoke.sh path/to/openshell-annotate
set -euo pipefail
bin="$(cd "$(dirname "${1:?usage: smoke.sh BINARY}")" && pwd)/$(basename "$1")"
d="$(mktemp -d)"; trap 'rm -rf "$d"' EXIT
"$bin" --version
"$bin" --help > /dev/null
printf '// @openshell egress api.example.com:443 reason="smoke" bin=/usr/bin/node\nfetch(1)\n' > "$d/a.js"
"$bin" --root "$d" scan --format compact | grep 'api.example.com:443' > /dev/null
"$bin" --root "$d" generate -o "$d/policy.yaml" > /dev/null 2>&1
grep -q 'host: "api.example.com"' "$d/policy.yaml"
"$bin" --root "$d" drift --policy "$d/policy.yaml" | grep '^no drift' > /dev/null
echo "smoke ok: $bin"
