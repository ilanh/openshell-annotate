#!/usr/bin/env bash
# Build and test openshell-annotate against a different version of OpenShell's policy-schema crate,
# WITHOUT touching this working tree (Cargo.toml / Cargo.lock stay pinned). Works on a temporary copy.
#
#   scripts/upstream-compat.sh latest-release   # newest stable vX.Y.Z tag of NVIDIA/OpenShell
#   scripts/upstream-compat.sh main             # OpenShell HEAD
#   scripts/upstream-compat.sh v0.1.7-pre.3     # any tag, branch or commit SHA
#
# Env: UPSTREAM_REPO (default https://github.com/NVIDIA/OpenShell), UPSTREAM_COMPAT_LOG (also write the log there),
#      CARGO_TARGET_DIR (reuse a build cache), GITHUB_STEP_SUMMARY / GITHUB_OUTPUT (set automatically in Actions).
# Exit: 0 compatible, 1 build/test failure, 2 usage / could not resolve the ref.
set -euo pipefail
cd "$(dirname "$0")/.."

REPO="${UPSTREAM_REPO:-https://github.com/NVIDIA/OpenShell}"
TARGET="${1:-}"
[ -n "$TARGET" ] || { echo "usage: $0 <latest-release|main|tag|branch|sha>" >&2; exit 2; }

# --- resolve the target to a commit SHA ---------------------------------------------------------
case "$TARGET" in
  latest-release)
    # Newest stable release tag (vX.Y.Z, no -pre suffix). ls-remote avoids API rate limits.
    LABEL=$(git ls-remote --tags --refs "$REPO" | sed 's#.*refs/tags/##' | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' | sort -V | tail -1) || true
    [ -n "${LABEL:-}" ] || { echo "could not find a stable release tag in $REPO" >&2; exit 2; }
    SHA=$(git ls-remote "$REPO" "refs/tags/$LABEL^{}" "refs/tags/$LABEL" | awk 'NR==1{print $1}')
    ;;
  main)
    LABEL="main"
    SHA=$(git ls-remote "$REPO" HEAD | awk '{print $1}')
    ;;
  *)
    LABEL="$TARGET"
    if [[ "$TARGET" =~ ^[0-9a-f]{40}$ ]]; then SHA="$TARGET"
    else SHA=$(git ls-remote "$REPO" "refs/tags/$TARGET^{}" "refs/tags/$TARGET" "refs/heads/$TARGET" | awk 'NR==1{print $1}')
    fi
    ;;
esac
[ -n "${SHA:-}" ] || { echo "could not resolve '$TARGET' in $REPO" >&2; exit 2; }

PINNED=$(grep -E '^openshell-policy-schema' Cargo.toml | sed -E 's/.*tag = "([^"]+)".*/\1/')
echo "==> upstream target: $TARGET ($LABEL) -> $SHA   (pinned in Cargo.toml: $PINNED)"

if [ -n "${GITHUB_OUTPUT:-}" ]; then
  { echo "upstream_label=$LABEL"; echo "upstream_sha=$SHA"; } >> "$GITHUB_OUTPUT"
fi

# --- build on a throwaway copy with the dependency repointed to that commit ---------------------
# Fixed path (not mktemp): the test binaries bake in CARGO_MANIFEST_DIR, so a reused build cache needs a stable one.
WORK="${XDG_CACHE_HOME:-$HOME/.cache}/openshell-annotate-upstream-compat-src"
rm -rf "$WORK"; mkdir -p "$WORK"
tar --exclude=./target --exclude=./.git -cf - . | tar -xf - -C "$WORK"
cd "$WORK"
# tag = "vX" -> rev = "<sha>" on the openshell-policy-schema line only; the lockfile is re-resolved.
sed -i -E "/^openshell-policy-schema/ s/tag = \"[^\"]+\"/rev = \"$SHA\"/" Cargo.toml
grep -q "rev = \"$SHA\"" Cargo.toml || { echo "failed to repoint the dependency in Cargo.toml" >&2; exit 2; }
[ -n "${CARGO_TARGET_DIR:-}" ] || export CARGO_TARGET_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/openshell-annotate-upstream-compat"

run() { printf '\n==> %s\n' "$*"; "$@" || { echo "FAILED: $*"; return 1; }; }
steps() {
  run cargo update -p openshell-policy-schema &&
    run cargo build --all-targets &&
    run cargo test &&
    run scripts/gen-docs.sh --check
}
set +e
steps 2>&1 | tee "${UPSTREAM_COMPAT_LOG:-/dev/null}"
STATUS=${PIPESTATUS[0]}
set -e

RESULT=$([ "$STATUS" = 0 ] && echo "✅ compatible" || echo "❌ BROKEN")
echo; echo "upstream-compat: $TARGET @ ${SHA:0:12}: $RESULT"
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  {
    echo "### Upstream compatibility: \`$TARGET\`"
    echo "- Result: $RESULT"
    echo "- OpenShell ref: \`$LABEL\` → [\`${SHA:0:12}\`](${REPO}/commit/$SHA)"
    echo "- Pinned in Cargo.toml: \`$PINNED\`"
  } >> "$GITHUB_STEP_SUMMARY"
fi
exit "$STATUS"
