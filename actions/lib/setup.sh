#!/usr/bin/env bash
# Shared steps for the `setup` and `check` composite actions.
# Usage: setup.sh dir|resolve|install|build|finish
# Paths are resolved from this script's location, never from the caller's workspace, so the actions work
# when consumed remotely (`uses: ilanh/openshell-annotate/actions/check@<ref>`) or from any checkout path.
set -euo pipefail

lib_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_dir="$(cd "$lib_dir/../.." && pwd)"

case "${1:-}" in
  dir)
    # in: INPUT_INSTALL_DIR (optional)
    dir="${INPUT_INSTALL_DIR:-}"
    [ -n "$dir" ] || dir="${RUNNER_TOOL_CACHE:?}/openshell-annotate"
    mkdir -p "$dir"
    echo "dir=$dir" >>"$GITHUB_OUTPUT"
    ;;
  resolve)
    # in: OPENSHELL_ANNOTATE_VERSION (tag, `latest` or `source`; empty means `latest`), OPENSHELL_ANNOTATE_BASE_URL
    # (optional, as for install.sh). out: `version` (the concrete tag to install) and `cacheable` (true/false).
    # `latest` is resolved to the tag GitHub redirects /releases/latest to, so the cache key names the real release
    # and a new stable release is installed instead of the first cached one (#77). If it cannot be resolved (no
    # stable release yet, network error) the cache is skipped and install.sh reports the error.
    want="${OPENSHELL_ANNOTATE_VERSION:-latest}"
    cacheable=true
    case "$want" in
      source)
        cacheable=false
        ;;
      latest)
        base="${OPENSHELL_ANNOTATE_BASE_URL:-https://github.com/ilanh/openshell-annotate/releases}"
        url="$(curl -fsSLI --retry 3 --retry-delay 2 -o /dev/null -w '%{url_effective}' "$base/latest" 2>/dev/null || true)"
        tag="${url##*/}"
        if printf '%s' "$tag" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$'; then
          want="$tag"
          echo "version latest resolved to $want"
        else
          cacheable=false
          echo "::warning::could not resolve version latest to a release tag (is there a stable release yet?); not caching"
        fi
        ;;
      v*) ;;
      *) want="v$want" ;;
    esac
    echo "version=$want" >>"$GITHUB_OUTPUT"
    echo "cacheable=$cacheable" >>"$GITHUB_OUTPUT"
    ;;
  install)
    # in: INSTALL_DIR, OPENSHELL_ANNOTATE_VERSION, VERIFY
    install_sh="$repo_dir/install.sh"
    if [ ! -f "$install_sh" ]; then
      install_sh="${RUNNER_TEMP:-/tmp}/openshell-annotate-install.sh"
      curl -fsSL "https://raw.githubusercontent.com/ilanh/openshell-annotate/${GITHUB_ACTION_REF:-main}/install.sh" \
        -o "$install_sh"
    fi
    args=(--dir "${INSTALL_DIR:?}")
    if [ -n "${OPENSHELL_ANNOTATE_VERSION:-}" ]; then
      args+=(--version "$OPENSHELL_ANNOTATE_VERSION")
    fi
    if [ "${VERIFY:-false}" = "true" ]; then
      args+=(--verify)
    fi
    attempt=0
    max=5
    until sh "$install_sh" "${args[@]}"; do
      attempt=$((attempt + 1))
      [ "$attempt" -ge "$max" ] && exit 1
      echo "install.sh failed (attempt $attempt/$max); retrying in ${attempt}0s..." >&2
      sleep $((attempt * 10))
    done
    ;;
  build)
    # in: INSTALL_DIR. Builds the openshell-annotate sources that ship with this action.
    [ -f "$repo_dir/Cargo.toml" ] || { echo "::error::version=source needs the action sources (no Cargo.toml in $repo_dir)"; exit 1; }
    cd "$repo_dir"
    if ! command -v cargo >/dev/null 2>&1; then
      rustup toolchain install 1.94 --profile minimal
      rustup default 1.94
    fi
    cargo build --release --locked --target-dir "$repo_dir/target"
    install -m 0755 "$repo_dir/target/release/openshell-annotate" "${INSTALL_DIR:?}/openshell-annotate"
    ;;
  finish)
    # in: INSTALL_DIR. Adds it to PATH and records outputs `version` and `install-dir`.
    echo "${INSTALL_DIR:?}" >>"$GITHUB_PATH"
    bin="$INSTALL_DIR/openshell-annotate"
    ver="$("$bin" --version 2>/dev/null || true)"
    echo "version=$ver" >>"$GITHUB_OUTPUT"
    echo "install-dir=$INSTALL_DIR" >>"$GITHUB_OUTPUT"
    echo "openshell-annotate $ver at $bin"
    ;;
  *)
    echo "usage: setup.sh dir|resolve|install|build|finish" >&2
    exit 2
    ;;
esac
