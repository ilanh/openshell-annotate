#!/usr/bin/env bash
# Package one release binary as a deterministic tar.gz (same inputs -> same bytes).
#   scripts/package.sh <version> <target> <binary> [outdir]
# Writes <outdir>/openshell-annotate-<version>-<target>.tar.gz containing
# openshell-annotate-<version>-<target>/{openshell-annotate,LICENSE,README.md}. Needs GNU tar.
set -euo pipefail
[ $# -ge 3 ] || { sed -n '2,5p' "$0" | sed 's/^# \{0,1\}//'; exit 2; }
version="${1#v}"; target="$2"; bin="$3"; out="${4:-dist}"
cd "$(dirname "$0")/.."
[ -f "$bin" ] || { echo "package.sh: no such binary: $bin" >&2; exit 1; }
name="openshell-annotate-$version-$target"
stage="$(mktemp -d)"; trap 'rm -rf "$stage"' EXIT
mkdir -p "$stage/$name" "$out"
install -m 0755 "$bin" "$stage/$name/openshell-annotate"
cp LICENSE README.md "$stage/$name/"
epoch="${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct 2>/dev/null || echo 0)}"
tar --sort=name --mtime="@$epoch" --owner=0 --group=0 --numeric-owner -C "$stage" -cf - "$name" | gzip -9n > "$out/$name.tar.gz"
echo "$out/$name.tar.gz"
