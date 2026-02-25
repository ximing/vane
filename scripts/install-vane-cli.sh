#!/usr/bin/env sh
# Install the native `vane` sidecar CLI from the latest GitHub Release.
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/ximing/vane/main/scripts/install-vane-cli.sh | sh
# Override install prefix with PREFIX (default: $HOME/.local).
set -eu

REPO="${VANE_REPO:-ximing/vane}"
PREFIX="${PREFIX:-$HOME/.local}"
BIN_DIR="${PREFIX}/bin"

os="$(uname -s)"
arch="$(uname -m)"
case "${os}/${arch}" in
  Darwin/arm64) target="aarch64-apple-darwin" ;;
  Darwin/x86_64) target="x86_64-apple-darwin" ;;
  Linux/x86_64) target="x86_64-unknown-linux-gnu" ;;
  *)
    echo "No prebuilt vane CLI for ${os}/${arch}." >&2
    echo "Build from source:" >&2
    echo "  cargo install --git https://github.com/${REPO}.git --locked --bin vane" >&2
    exit 1
    ;;
esac

asset="vane-${target}.tar.gz"
url="https://github.com/${REPO}/releases/latest/download/${asset}"

tmp="$(mktemp -d)"
cleanup() { rm -rf "$tmp"; }
trap cleanup EXIT INT HUP

echo "Downloading ${url}"
if command -v curl >/dev/null 2>&1; then
  curl -fL --retry 3 -o "${tmp}/${asset}" "$url"
elif command -v wget >/dev/null 2>&1; then
  wget -O "${tmp}/${asset}" "$url"
else
  echo "need curl or wget" >&2
  exit 1
fi

tar -C "$tmp" -xzf "${tmp}/${asset}"
mkdir -p "$BIN_DIR"
install -m 0755 "${tmp}/vane-${target}" "${BIN_DIR}/vane"

echo "Installed ${BIN_DIR}/vane"
if ! echo ":$PATH:" | grep -q ":${BIN_DIR}:"; then
  echo "Add ${BIN_DIR} to PATH, then run: vane --help"
fi
"${BIN_DIR}/vane" --version || true
