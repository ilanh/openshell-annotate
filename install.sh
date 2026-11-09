#!/bin/sh
# Install a prebuilt openshell-annotate from a GitHub release, verifying its SHA-256 checksum.
#
#   curl -fsSL https://raw.githubusercontent.com/ilanh/openshell-annotate/main/install.sh | sh
#   curl -fsSL .../install.sh | sh -s -- --version v0.1.0-rc.1 --dir /usr/local/bin
#
# Options (or environment):
#   -v, --version TAG   release tag to install (default: the latest stable release; OPENSHELL_ANNOTATE_VERSION)
#   -d, --dir DIR       install directory (default: $HOME/.local/bin; INSTALL_DIR)
#       --verify        also run `gh attestation verify` (needs the GitHub CLI): proves the archive was built by this
#                       repository's release workflow. A checksum alone only detects a corrupted download.
#   -h, --help
# OPENSHELL_ANNOTATE_BASE_URL overrides https://github.com/ilanh/openshell-annotate/releases (for testing).
set -eu

REPO="ilanh/openshell-annotate"
BASE="${OPENSHELL_ANNOTATE_BASE_URL:-https://github.com/$REPO/releases}"
VERSION="${OPENSHELL_ANNOTATE_VERSION:-}"
DIR="${INSTALL_DIR:-$HOME/.local/bin}"
VERIFY=0

die() { printf 'install.sh: %s\n' "$*" >&2; exit 1; }
usage() { sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'; }

while [ $# -gt 0 ]; do
  case "$1" in
    -v|--version) [ $# -ge 2 ] || die "$1 needs a value"; VERSION="$2"; shift ;;
    -d|--dir) [ $# -ge 2 ] || die "$1 needs a value"; DIR="$2"; shift ;;
    --verify) VERIFY=1 ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option: $1 (try --help)" ;;
  esac
  shift
done

have() { command -v "$1" > /dev/null 2>&1; }
if have curl; then
  fetch() { curl -fsSL --proto '=https,http' --retry 8 --retry-delay 3 --retry-all-errors -o "$2" "$1"; }
  resolve() { curl -fsSLI -o /dev/null -w '%{url_effective}' "$1"; }
elif have wget; then
  fetch() { wget -q -O "$2" "$1"; }
  resolve() { wget -q --max-redirect=5 -S --spider "$1" 2>&1 | sed -n 's/^ *Location: //p' | tail -n 1; }
else
  die "need curl or wget"
fi

if have sha256sum; then sha() { sha256sum "$1" | cut -d' ' -f1; }
elif have shasum; then sha() { shasum -a 256 "$1" | cut -d' ' -f1; }
elif have openssl; then sha() { openssl dgst -sha256 "$1" | sed 's/^.*= //'; }
else die "need sha256sum, shasum or openssl to verify the download"; fi

case "$(uname -s)" in
  Linux) os=unknown-linux-musl ;;
  Darwin) os=apple-darwin ;;
  *) die "unsupported OS: $(uname -s) (releases cover Linux and macOS)" ;;
esac
case "$(uname -m)" in
  x86_64|amd64) arch=x86_64 ;;
  aarch64|arm64) arch=aarch64 ;;
  *) die "unsupported CPU: $(uname -m)" ;;
esac
target="$arch-$os"

if [ -z "$VERSION" ] || [ "$VERSION" = latest ]; then
  url="$(resolve "$BASE/latest")" || die "could not resolve the latest release (is there a stable release yet? use --version)"
  VERSION="${url##*/}"
fi
case "$VERSION" in v*) ;; *) VERSION="v$VERSION" ;; esac
printf '%s' "$VERSION" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$' || die "not a release tag: $VERSION"
ver="${VERSION#v}"
archive="openshell-annotate-$ver-$target.tar.gz"

tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT INT TERM
echo "Installing openshell-annotate $VERSION ($target) into $DIR"
fetch "$BASE/download/$VERSION/$archive" "$tmp/$archive" || die "download failed: $BASE/download/$VERSION/$archive"
fetch "$BASE/download/$VERSION/SHA256SUMS" "$tmp/SHA256SUMS" || die "download failed: SHA256SUMS"

want="$(awk -v f="$archive" '$2 == f || $2 == "*" f { print $1 }' "$tmp/SHA256SUMS" | head -n 1)"
[ -n "$want" ] || die "$archive is not listed in SHA256SUMS"
got="$(sha "$tmp/$archive")"
[ "$want" = "$got" ] || die "checksum mismatch for $archive (expected $want, got $got); nothing was installed"
echo "checksum ok ($got)"

if [ "$VERIFY" = 1 ]; then
  have gh || die "--verify needs the GitHub CLI (https://cli.github.com)"
  gh attestation verify "$tmp/$archive" --repo "$REPO" > /dev/null || die "attestation verification failed; nothing was installed"
  echo "attestation ok (built by $REPO)"
fi

tar -xzf "$tmp/$archive" -C "$tmp"
bin="$tmp/openshell-annotate-$ver-$target/openshell-annotate"
[ -f "$bin" ] || die "archive does not contain the expected binary"
mkdir -p "$DIR"
cp "$bin" "$DIR/openshell-annotate.new" && chmod 0755 "$DIR/openshell-annotate.new" && mv -f "$DIR/openshell-annotate.new" "$DIR/openshell-annotate"
"$DIR/openshell-annotate" --version
case ":$PATH:" in *":$DIR:"*) ;; *) echo "note: $DIR is not on your PATH" ;; esac
