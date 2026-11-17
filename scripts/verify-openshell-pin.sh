#!/usr/bin/env bash
# Verify the NVIDIA/OpenShell git dependency pin (run by the `openshell-pin` job in .github/workflows/security.yml).
#   scripts/verify-openshell-pin.sh
# CI does not download OpenShell release binaries: the two policy crates are git dependencies on
# https://github.com/NVIDIA/OpenShell by tag (Cargo.toml), and Cargo.lock records the commit cargo fetched. Release
# attestations (`gh attestation verify --repo NVIDIA/OpenShell`) cover OpenShell's built artifacts such as the .deb, not
# its source tree, and those artifacts can be built from a different commit than the tag (v0.1.8's .deb came from 22209de,
# its tag points at f3d9765), so they cannot vouch for what cargo builds here. What this script checks instead:
#   1. every OpenShell crate in Cargo.toml uses the same `tag = "vX.Y.Z[-pre.N]"` (no branch or rev pins);
#   2. every NVIDIA/OpenShell package in Cargo.lock comes from that tag and one commit;
#   3. that commit is what the tag points to upstream right now (`git ls-remote`, annotated tags peeled), so a moved or
#      re-created tag, or a lock file edited by hand, fails;
#   4. with `gh` available, GitHub reports the commit's signature as verified (a warning, not a failure, if not).
# Env: UPSTREAM_REPO (default https://github.com/NVIDIA/OpenShell).
set -euo pipefail
cd "$(dirname "$0")/.."
REPO="${UPSTREAM_REPO:-https://github.com/NVIDIA/OpenShell}"
fail() { echo "::error::$*" >&2; exit 1; }

tags="$(grep -E '^[a-z0-9_-]+ *= *\{[^}]*git *= *"https://github.com/NVIDIA/OpenShell"' Cargo.toml || true)"
[ -n "$tags" ] || fail "no NVIDIA/OpenShell git dependency found in Cargo.toml"
if printf '%s\n' "$tags" | grep -Eq '(branch|rev) *='; then
  fail "Cargo.toml pins NVIDIA/OpenShell by branch or rev; use a release tag"
fi
tag="$(printf '%s\n' "$tags" | sed -nE 's/.*tag *= *"([^"]+)".*/\1/p' | sort -u)"
[ "$(printf '%s\n' "$tag" | grep -c .)" = 1 ] || fail "OpenShell crates use different tags in Cargo.toml: $(printf "%s " "$tag")"
printf '%s' "$tag" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$' || fail "unexpected OpenShell tag: $tag"

sources="$(grep -E '^source = "git\+https://github.com/NVIDIA/OpenShell' Cargo.lock | sort -u)"
[ -n "$sources" ] || fail "no NVIDIA/OpenShell package in Cargo.lock"
[ "$(printf '%s\n' "$sources" | grep -c .)" = 1 ] || fail "Cargo.lock has more than one NVIDIA/OpenShell source:
$sources"
locked_tag="$(printf '%s' "$sources" | sed -nE 's/.*\?tag=([^#]+)#.*/\1/p')"
commit="$(printf '%s' "$sources" | sed -nE 's/.*#([0-9a-f]{40})"$/\1/p')"
[ "$locked_tag" = "$tag" ] || fail "Cargo.lock uses tag '${locked_tag:-none}', Cargo.toml uses $tag (run cargo update)"
[ -n "$commit" ] || fail "no commit hash in Cargo.lock source: $sources"

refs="$(git ls-remote "$REPO" "refs/tags/$tag" "refs/tags/$tag^{}")" || fail "git ls-remote $REPO failed"
upstream="$(printf '%s\n' "$refs" | awk -v t="refs/tags/$tag^{}" '$2 == t { print $1 }')"
[ -n "$upstream" ] || upstream="$(printf '%s\n' "$refs" | awk -v t="refs/tags/$tag" '$2 == t { print $1 }')"
[ -n "$upstream" ] || fail "tag $tag not found in $REPO"
[ "$upstream" = "$commit" ] || fail "OpenShell $tag points to $upstream upstream, but Cargo.lock has $commit"
echo "OpenShell pin ok: $tag = $commit (Cargo.toml, Cargo.lock and $REPO agree)"

if command -v gh >/dev/null 2>&1; then
  slug="${REPO#https://github.com/}"
  if verified="$(gh api "repos/$slug/commits/$commit" --jq '.commit.verification | "\(.verified) \(.reason)"' 2>/dev/null)"; then
    case "$verified" in
      true*) echo "commit signature: verified by GitHub ($verified)" ;;
      *) echo "::warning::OpenShell $tag commit $commit signature is not verified by GitHub ($verified)" ;;
    esac
  else
    echo "::warning::could not query GitHub for the signature of $commit (no token or rate limit); skipped"
  fi
fi
