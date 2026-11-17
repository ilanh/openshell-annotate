#!/usr/bin/env bash
# Install the pinned security tools that .github/workflows/security.yml uses, verifying each download.
#   scripts/ci-tools.sh install <actionlint|gitleaks|cargo-deny>... [--dir DIR]   # default DIR: ~/.local/bin
# Linux x86_64 only (the CI runner). Each archive must match the SHA-256 pinned below; this is stricter than checking
# against the release's own checksum file, because a replaced release asset and its checksum file would both change.
# actionlint publishes GitHub build provenance, so it is also checked with `gh attestation verify` when `gh` supports
# it (set VERIFY_ATTESTATION=0 to skip). gitleaks and cargo-deny publish no attestations (checked 2026-11-17).
# To bump a tool: change its version and hash together, taken from the release's checksum file, in one PR.
set -euo pipefail

declare -A VERSION=(
  [actionlint]=1.7.12
  [gitleaks]=8.30.1
  [cargo-deny]=0.21.0
)
declare -A SHA256=(
  [actionlint]=8aca8db96f1b94770f1b0d72b6dddcb1ebb8123cb3712530b08cc387b349a3d8
  [gitleaks]=551f6fc83ea457d62a0d98237cbad105af8d557003051f41f3e7ca7b3f2470eb
  [cargo-deny]=a2170b854ff697e79e39c4c538981c9c1bdd803addbae6719a4cb66b4585a903
)

url() {
  local v="${VERSION[$1]}"
  case "$1" in
    actionlint) echo "https://github.com/rhysd/actionlint/releases/download/v$v/actionlint_${v}_linux_amd64.tar.gz" ;;
    gitleaks) echo "https://github.com/gitleaks/gitleaks/releases/download/v$v/gitleaks_${v}_linux_x64.tar.gz" ;;
    cargo-deny) echo "https://github.com/EmbarkStudios/cargo-deny/releases/download/$v/cargo-deny-$v-x86_64-unknown-linux-musl.tar.gz" ;;
  esac
}

install_tool() {
  local tool="$1" dir="$2" tmp archive want got
  [ -n "${VERSION[$tool]:-}" ] || { echo "unknown tool: $tool" >&2; exit 2; }
  tmp="$(mktemp -d)"
  archive="$tmp/$tool.tar.gz"
  curl -fsSL --retry 3 --retry-delay 2 -o "$archive" "$(url "$tool")"
  want="${SHA256[$tool]}"
  got="$(sha256sum "$archive" | cut -d' ' -f1)"
  if [ "$got" != "$want" ]; then
    echo "::error::$tool ${VERSION[$tool]}: SHA-256 mismatch (got $got, want $want); not installed" >&2
    exit 1
  fi
  echo "$tool ${VERSION[$tool]}: sha256 ok ($got)"
  if [ "$tool" = actionlint ] && [ "${VERIFY_ATTESTATION:-1}" = 1 ]; then
    if gh attestation --help >/dev/null 2>&1; then
      gh attestation verify "$archive" --repo rhysd/actionlint >/dev/null
      echo "$tool: attestation ok (built by rhysd/actionlint)"
    else
      echo "::warning::gh has no 'attestation' command (needs gh >= 2.49); skipped the actionlint provenance check"
    fi
  fi
  tar -xzf "$archive" -C "$tmp"
  mkdir -p "$dir"
  install -m 0755 "$(find "$tmp" -type f -name "$tool" | head -n 1)" "$dir/$tool"
  rm -rf "$tmp"
}

[ "${1:-}" = install ] || { echo "usage: $0 install <actionlint|gitleaks|cargo-deny>... [--dir DIR]" >&2; exit 2; }
shift
dir="$HOME/.local/bin"
tools=()
while [ $# -gt 0 ]; do
  case "$1" in
    --dir) dir="$2"; shift 2 ;;
    *) tools+=("$1"); shift ;;
  esac
done
[ "${#tools[@]}" -gt 0 ] || { echo "no tool given" >&2; exit 2; }
for t in "${tools[@]}"; do install_tool "$t" "$dir"; done
if [ -n "${GITHUB_PATH:-}" ]; then echo "$dir" >>"$GITHUB_PATH"; fi
