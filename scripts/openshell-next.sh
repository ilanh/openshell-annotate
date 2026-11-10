#!/usr/bin/env bash
# Maintain the `openshell-next` integration branch, which builds against the newest NVIDIA/OpenShell tag
# (prereleases included). Used by .github/workflows/openshell-next.yml; safe to run by hand on a local checkout.
#
#   scripts/openshell-next.sh bump <tag>      # pin both OpenShell crates to <tag> in Cargo.toml and re-resolve Cargo.lock
#   scripts/openshell-next.sh update [<tag>]  # bump to <tag> (default: newest upstream tag) only if it is newer than the pin
#   scripts/openshell-next.sh sync [<ref>]    # merge <ref> (default origin/main) into the current branch
#
# `bump` and `update` edit files only; they never commit. `sync` creates a signed-off merge commit. When main and the
# branch both changed the pin, `sync` keeps the newer of the two tags and re-resolves Cargo.lock; any other conflict
# aborts the merge (exit 3) so a human can resolve it.
# `update` prints `previous=`, `tag=`, `changed=` and `stable=` lines (also appended to $GITHUB_OUTPUT when set).
# Env: OPENSHELL_ANNOTATE_ROOT, the checkout to work on (default: the one this script lives in). The workflow runs
#      main's copy of these scripts from a temp dir against the openshell-next checkout, so the branch needs no copy.
# Exit: 0 ok, 2 usage / bad tag, 3 merge conflict that needs a human.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "${OPENSHELL_ANNOTATE_ROOT:-$HERE/..}" && pwd)
export OPENSHELL_ANNOTATE_ROOT="$ROOT" # so openshell-tags.sh reads the same Cargo.toml
cd "$ROOT"
TAGS="$HERE/openshell-tags.sh"
CRATES=(openshell-policy-schema openshell-policy)

bump() {
  local tag="$1" tmp
  [[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-pre\.[0-9]+)?$ ]] || { echo "not a release tag: $tag" >&2; exit 2; }
  tmp=$(mktemp)
  # Repoint both crates (tag = or rev =) and the comment above them; portable sed (no -i).
  sed -E \
    -e "/^openshell-policy(-schema)?[[:space:]]*=/ s/(tag|rev)[[:space:]]*=[[:space:]]*\"[^\"]+\"/tag = \"$tag\"/" \
    -e "s/^# Pinned to the OpenShell [^ ]+ tag;/# Pinned to the OpenShell $tag tag;/" \
    Cargo.toml > "$tmp"
  cat "$tmp" > Cargo.toml; rm -f "$tmp"
  for c in "${CRATES[@]}"; do
    grep -Eq "^${c}[[:space:]]*=.*tag = \"$tag\"" Cargo.toml || { echo "failed to pin $c to $tag in Cargo.toml" >&2; exit 2; }
  done
  cargo update -p openshell-policy-schema -p openshell-policy --quiet
  echo "pinned ${CRATES[*]} to $tag"
}

out() { echo "$1"; if [ -n "${GITHUB_OUTPUT:-}" ]; then echo "$1" >> "$GITHUB_OUTPUT"; fi; }

update() {
  local want="${1:-}" have
  have=$("$TAGS" pinned)
  [ -n "$want" ] || want=$("$TAGS" latest)
  out "previous=$have"
  if "$TAGS" newer "$want" "$have"; then bump "$want" >&2; out "tag=$want"; out "changed=true"
  else out "tag=$have"; out "changed=false"; want="$have"
  fi
  if "$TAGS" is-stable "$want"; then out "stable=true"; else out "stable=false"; fi
}

sync() {
  local ref="${1:-origin/main}" before after bad msg
  before=$("$TAGS" pinned)
  msg="Merge $ref into $(git rev-parse --abbrev-ref HEAD)"
  if git merge --no-ff --no-edit --signoff -m "$msg" "$ref"; then
    echo "merged $ref cleanly"
  else
    bad=$(git diff --name-only --diff-filter=U | grep -vxE 'Cargo\.toml|Cargo\.lock' || true)
    if [ -n "$bad" ]; then
      echo "conflicts outside Cargo.toml/Cargo.lock need a human:" >&2; echo "$bad" >&2
      git merge --abort; exit 3
    fi
    # Both sides moved the pin: start from main's files and re-apply the newer tag.
    git checkout --theirs -- Cargo.toml Cargo.lock 2>/dev/null || git checkout "$ref" -- Cargo.toml Cargo.lock
    after=$("$TAGS" pinned)
    if "$TAGS" newer "$before" "$after"; then bump "$before"; fi
    git add Cargo.toml Cargo.lock
    git commit --signoff -m "$msg" -m "Both sides changed the OpenShell pin; kept the newer tag."
    echo "merged $ref; resolved the pin conflict to $("$TAGS" pinned)"
  fi
  # A clean merge can still leave Cargo.lock stale (e.g. main added a dependency): re-resolve minimally.
  if ! cargo metadata --locked --format-version 1 >/dev/null 2>&1; then
    cargo metadata --format-version 1 >/dev/null
    git add Cargo.lock
    git commit --signoff -m "chore(deps): refresh Cargo.lock after merging $ref"
  fi
}

cmd="${1:-}"; shift || true
case "$cmd" in
  bump) [ $# -eq 1 ] || { echo "usage: $0 bump <tag>" >&2; exit 2; }; bump "$1" ;;
  update) update "${1:-}" ;;
  sync) sync "${1:-}" ;;
  *) sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//' >&2; exit 2 ;;
esac
