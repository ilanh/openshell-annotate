#!/usr/bin/env bash
# Run the same checks as .github/workflows/ci.yml (the `check` job) locally.
#   scripts/ci-local.sh            # fmt, clippy, test, docs checks
#   TOOLCHAIN=1.94 scripts/ci-local.sh   # pin the toolchain CI uses (needs `rustup toolchain install 1.94`)
set -euo pipefail
cd "$(dirname "$0")/.."
TC="${TOOLCHAIN:+ +$TOOLCHAIN}"
[ -n "${TOOLCHAIN:-}" ] && export RUSTUP_TOOLCHAIN="$TOOLCHAIN"
run() { printf '\n==> %s\n' "$*"; "$@"; }
run cargo $TC fmt --check
run cargo $TC clippy --all-targets --locked -- -D warnings
run cargo $TC test --locked
run cargo $TC test --locked --test gateway_policies
run scripts/gen-docs.sh --check
run scripts/gen-actions-docs.sh --check
echo; echo "ci-local: all checks passed"
