#!/usr/bin/env bash
# Shared steps for the `setup` and `check` composite actions.
# Usage: setup.sh dir|install|build|finish
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
    echo "usage: setup.sh dir|install|build|finish" >&2
    exit 2
    ;;
esac
