#!/usr/bin/env bash
# List and compare NVIDIA/OpenShell release tags in release order (a stable vX.Y.Z sorts after its vX.Y.Z-pre.N).
#
#   scripts/openshell-tags.sh latest          # newest tag, prereleases included (what `openshell-next` tracks)
#   scripts/openshell-tags.sh latest-stable   # newest vX.Y.Z
#   scripts/openshell-tags.sh list            # every release tag, oldest first
#   scripts/openshell-tags.sh pinned          # the tag Cargo.toml pins openshell-policy-schema to
#   scripts/openshell-tags.sh newer A B       # exit 0 when tag A is newer than tag B, else 1
#   scripts/openshell-tags.sh is-stable A     # exit 0 when A is vX.Y.Z (no prerelease suffix)
#
# Only vX.Y.Z and vX.Y.Z-pre.N tags count; anything else upstream (`dev`, `vm-runtime`, ...) is ignored.
# Env: UPSTREAM_REPO (default https://github.com/NVIDIA/OpenShell); OPENSHELL_TAGS (newline-separated tag list to use
#      instead of asking the remote, for tests and offline runs); OPENSHELL_ANNOTATE_ROOT (checkout whose Cargo.toml
#      `pinned` reads; default: the checkout this script lives in).
# Exit: 0 ok / true, 1 false, 2 usage or no tags found.
set -euo pipefail
cd "${OPENSHELL_ANNOTATE_ROOT:-$(dirname "$0")/..}"

REPO="${UPSTREAM_REPO:-https://github.com/NVIDIA/OpenShell}"
RE='^v[0-9]+\.[0-9]+\.[0-9]+(-pre\.[0-9]+)?$'

usage() { sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//' >&2; exit 2; }

valid() { [[ "$1" =~ $RE ]]; }

# "v0.1.8-pre.3" -> "0 1 8 3 v0.1.8-pre.3"; a stable tag gets 999999 so it sorts after its prereleases.
key() {
  local t="${1#v}" pre=999999
  case "$t" in *-pre.*) pre="${t##*-pre.}"; t="${t%-pre.*}" ;; esac
  printf '%s %s %s %s %s\n' "${t%%.*}" "$(echo "$t" | cut -d. -f2)" "${t##*.}" "$pre" "$1"
}

sorted() { while read -r t; do valid "$t" && key "$t"; done | sort -k1,1n -k2,2n -k3,3n -k4,4n | awk '{print $5}'; }

tags() {
  if [ -n "${OPENSHELL_TAGS:-}" ]; then printf '%s\n' "$OPENSHELL_TAGS"
  else git ls-remote --tags --refs "$REPO" | sed 's#.*refs/tags/##'
  fi | sorted
}

need() { [ -n "$1" ] || { echo "no matching release tag in $REPO" >&2; exit 2; }; echo "$1"; }

cmd="${1:-}"; [ -n "$cmd" ] || usage; shift
case "$cmd" in
  list) tags ;;
  latest) need "$(tags | tail -1)" ;;
  latest-stable) need "$(tags | grep -v -- '-pre\.' | tail -1)" ;;
  pinned)
    t=$(sed -n -E 's/^openshell-policy-schema[[:space:]]*=.*tag[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/p' Cargo.toml)
    [ -n "$t" ] || { echo "Cargo.toml does not pin openshell-policy-schema to a tag" >&2; exit 2; }
    echo "$t" ;;
  newer)
    if [ $# -ne 2 ] || ! valid "$1" || ! valid "$2"; then echo "newer: need two release tags, got: $*" >&2; exit 2; fi
    [ "$1" != "$2" ] && [ "$(printf '%s\n%s\n' "$1" "$2" | sorted | tail -1)" = "$1" ] ;;
  is-stable)
    if [ $# -ne 1 ] || ! valid "$1"; then echo "is-stable: need one release tag" >&2; exit 2; fi
    [[ "$1" != *-pre.* ]] ;;
  -h|--help|help) usage ;;
  *) echo "unknown command: $cmd" >&2; usage ;;
esac
