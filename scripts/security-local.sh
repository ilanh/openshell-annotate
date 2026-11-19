#!/usr/bin/env bash
# Run the checks of .github/workflows/security.yml locally: actionlint, gitleaks, cargo-deny and the OpenShell pin.
#   scripts/security-local.sh
# Needs actionlint, gitleaks and cargo-deny on PATH (any recent version; on Linux x86_64, `scripts/ci-tools.sh install
# actionlint gitleaks cargo-deny` installs the exact CI versions into ~/.local/bin). shellcheck, if installed, is used
# by actionlint for `run:` scripts.
set -euo pipefail
cd "$(dirname "$0")/.."
run() { printf '\n==> %s\n' "$*"; "$@"; }
run actionlint
run gitleaks git --redact --no-banner --exit-code 1 .
run cargo deny --locked check
run scripts/verify-openshell-pin.sh
echo; echo "security-local: all checks passed"
