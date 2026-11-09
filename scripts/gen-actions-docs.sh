#!/usr/bin/env bash
# Regenerate the inputs/outputs tables in docs/GITHUB_ACTIONS.md from actions/*/action.yml.
#   scripts/gen-actions-docs.sh           # rewrite the tables in place
#   scripts/gen-actions-docs.sh --check   # fail if the tables are stale or an action has no section (CI)
#
# Each action gets one generated block in the doc, between
#   <!-- BEGIN actions-docs NAME -->  and  <!-- END actions-docs NAME -->
# Everything outside the markers is hand-written. The parser only understands the plain action.yml layout used
# in this repository: top-level `inputs:`/`outputs:`, two-space item names, four-space `description:`,
# `required:` and `default:` keys (single-line values or a folded `>` description).
set -euo pipefail
cd "$(dirname "$0")/.."

DOC="docs/GITHUB_ACTIONS.md"
check=false
case "${1:-}" in
  --check) check=true ;;
  "") ;;
  *) echo "usage: $0 [--check]" >&2; exit 2 ;;
esac

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# Print the Markdown tables for one action.yml.
tables() {
  awk -v q="'" '
    function unquote(v) {
      sub(/^[ \t]+/, "", v); sub(/[ \t]+$/, "", v)
      if (v ~ /^".*"$/ || v ~ ("^" q ".*" q "$")) v = substr(v, 2, length(v) - 2)
      return v
    }
    function esc(v) { gsub(/\|/, "\\|", v); return v }
    function flush() {
      if (name == "") return
      if (sec == "inputs") {
        d = (def != "") ? "`" def "`" : ((req == "true") ? "—" : "_(empty)_")
        in_rows[++n_in] = "| `" name "` | " (req == "true" ? "yes" : "no") " | " d " | " esc(desc) " |"
      } else {
        out_rows[++n_out] = "| `" name "` | " esc(desc) " |"
      }
      name = ""
    }
    /^[^ #]/ {
      flush(); folding = 0
      sec = ($0 ~ /^inputs:[ \t]*$/) ? "inputs" : (($0 ~ /^outputs:[ \t]*$/) ? "outputs" : "")
      next
    }
    sec == "" { next }
    /^  [A-Za-z0-9_-]+:[ \t]*$/ {
      flush(); folding = 0
      name = $0; sub(/^  /, "", name); sub(/:.*$/, "", name)
      desc = ""; req = "false"; def = ""
      next
    }
    name != "" && /^    [a-z-]+:/ {
      folding = 0
      key = $0; sub(/^    /, "", key); sub(/:.*$/, "", key)
      val = $0; sub(/^    [a-z-]+:/, "", val); val = unquote(val)
      if (key == "description") {
        if (val == ">" || val == ">-" || val == "|" || val == "|-") { desc = ""; folding = 1 } else desc = val
      } else if (key == "required") req = val
      else if (key == "default") def = val
      next
    }
    folding && /^      / {
      line = $0; sub(/^[ \t]+/, "", line); sub(/[ \t]+$/, "", line)
      desc = (desc == "") ? line : desc " " line
      next
    }
    END {
      flush()
      if (n_in > 0) {
        print "| Input | Required | Default | Description |"
        print "| --- | --- | --- | --- |"
        for (i = 1; i <= n_in; i++) print in_rows[i]
      } else print "_No inputs._"
      if (n_out > 0) {
        print ""
        print "| Output | Description |"
        print "| --- | --- |"
        for (i = 1; i <= n_out; i++) print out_rows[i]
      }
    }
  ' "$1"
}

names=()
for yml in actions/*/action.yml; do
  name="$(basename "$(dirname "$yml")")"
  names+=("$name")
  tables "$yml" >"$tmp/$name.md"
done

status=0
for name in "${names[@]}"; do
  if ! grep -qx "<!-- BEGIN actions-docs $name -->" "$DOC" || ! grep -qx "<!-- END actions-docs $name -->" "$DOC"; then
    echo "error: $DOC has no generated section for actions/$name; add" >&2
    echo "  <!-- BEGIN actions-docs $name -->" >&2
    echo "  <!-- END actions-docs $name -->" >&2
    status=1
  fi
done
while IFS= read -r name; do
  if [ ! -f "actions/$name/action.yml" ]; then
    echo "error: $DOC has a generated section for actions/$name, which does not exist" >&2
    status=1
  fi
done < <(sed -n 's/^<!-- BEGIN actions-docs \(.*\) -->$/\1/p' "$DOC")
[ "$status" -eq 0 ] || exit 1

awk -v dir="$tmp" '
  /^<!-- BEGIN actions-docs .* -->$/ {
    print
    name = $0; sub(/^<!-- BEGIN actions-docs /, "", name); sub(/ -->$/, "", name)
    print ""
    file = dir "/" name ".md"
    while ((getline line < file) > 0) print line
    close(file)
    print ""
    skipping = 1
    next
  }
  /^<!-- END actions-docs .* -->$/ { skipping = 0 }
  !skipping { print }
' "$DOC" >"$tmp/doc.md"

if $check; then
  if ! diff -u "$DOC" "$tmp/doc.md"; then
    echo "error: $DOC is out of date with actions/*/action.yml; run scripts/gen-actions-docs.sh and commit it" >&2
    exit 1
  fi
  echo "actions docs are up to date"
else
  cp "$tmp/doc.md" "$DOC"
  echo "updated $DOC"
fi
