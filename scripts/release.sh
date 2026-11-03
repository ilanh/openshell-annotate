#!/usr/bin/env bash
# Prepare a release: bump the version in Cargo.toml and Cargo.lock, update CHANGELOG.md, print the tag commands.
#   scripts/release.sh 0.2.0          # or 0.2.0-rc.1 for a prerelease
# It edits files in the working tree only. It never commits, tags or pushes: review the diff, open a PR
# (main is protected), and after it is merged create the tag yourself as printed at the end (docs/RELEASING.md).
set -euo pipefail
cd "$(dirname "$0")/.."

version="${1:-}"; version="${version#v}"
[ -n "$version" ] || { sed -n '2,6p' "$0" | sed 's/^# \{0,1\}//'; exit 2; }
printf '%s' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$' \
  || { echo "release.sh: '$version' is not MAJOR.MINOR.PATCH[-prerelease]" >&2; exit 2; }
[ -z "$(git status --porcelain)" ] || { echo "release.sh: working tree is not clean; commit or stash first" >&2; exit 1; }

current="$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)"
[ "$current" != "$version" ] || { echo "release.sh: Cargo.toml is already at $version" >&2; exit 1; }
if git rev-parse -q --verify "refs/tags/v$version" > /dev/null; then echo "release.sh: tag v$version already exists" >&2; exit 1; fi
branch="$(git rev-parse --abbrev-ref HEAD)"
[ "$branch" = main ] || echo "note: you are on '$branch', not main; releases should be cut from a branch based on current main" >&2

# 1. Cargo.toml (first `version =` line is the [package] one) and Cargo.lock (workspace members only)
sed -i.bak "0,/^version = \".*\"$/s//version = \"$version\"/" Cargo.toml && rm -f Cargo.toml.bak
cargo update --workspace --offline > /dev/null 2>&1 || cargo update --workspace > /dev/null
cargo metadata --locked --format-version 1 --no-deps > /dev/null   # fails if Cargo.lock is not in sync

# 2. CHANGELOG.md: unreleased notes + merged PR titles since the previous tag
prev="$(git describe --tags --abbrev=0 --match 'v[0-9]*' 2>/dev/null || true)"
range="${prev:+$prev..}HEAD"
prs="$(git log --no-merges --pretty='- %s' "$range" | grep -E '\(#[0-9]+\)$' || true)"
notes="$(awk '/^## \[Unreleased\]/{f=1;next} f&&/^## \[/{exit} f' CHANGELOG.md | sed '/./,$!d' | sed -e :a -e '/^[[:space:]]*$/{$d;N;ba' -e '}')"
today="$(date -u +%F)"
tmp="$(mktemp)"
{
  awk '/^## \[Unreleased\]/{print; exit} {print}' CHANGELOG.md
  printf '\n## [%s] - %s\n\n' "$version" "$today"
  [ -z "$notes" ] || printf '%s\n\n' "$notes"
  if [ -n "$prs" ]; then printf 'Merged pull requests%s:\n\n%s\n' "${prev:+ since $prev}" "$prs"; else printf '_No merged pull requests since %s._\n' "${prev:-the start of the history}"; fi
  awk '/^## \[Unreleased\]/{f=1;next} f&&/^## \[/{p=1} p' CHANGELOG.md | sed '1s/^/\n/'
} > "$tmp"
mv "$tmp" CHANGELOG.md

cat <<MSG

Prepared $current -> $version. Changed: Cargo.toml, Cargo.lock, CHANGELOG.md. Review with: git diff

Next (nothing below was run for you):

  git checkout -b chore/release-v$version
  git commit -s -am "chore: release v$version"
  git push -u origin chore/release-v$version        # open a PR, wait for CI, merge it

After the PR is merged (the tag must point at the merged commit on main):

  git checkout main && git pull --ff-only
  git tag -s v$version -m "v$version"                # or: git tag -a v$version -m "v$version" if you do not sign tags
  git push origin v$version                          # this starts .github/workflows/release.yml
MSG
