#!/usr/bin/env bash
# build-go-lib.sh — 构建 vane-ffi staticlib 并复制到 bindings/go/lib/<platform>/ 供 cgo 链接。
#
# 用法：bash bindings/go/scripts/build-go-lib.sh
# 前提：cargo + rustc 已安装。
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
LIB_BASE="$SCRIPT_DIR/../lib"

cd "$REPO_ROOT"

echo "[build-go-lib] cargo build --release -p vane-ffi..."
cargo build --release -p vane-ffi

# 推断 host 平台 → lib 目录
OS="$(uname -s)"
ARCH="$(uname -m)"
case "$OS/$ARCH" in
  Darwin/arm64)  LIB_DIR="darwin-arm64" ;;
  Darwin/x86_64) LIB_DIR="darwin-amd64" ;;
  Linux/aarch64) LIB_DIR="linux-arm64" ;;
  Linux/x86_64)  LIB_DIR="linux-amd64" ;;
  *) echo "[build-go-lib] unsupported host: $OS/$ARCH"; exit 1 ;;
esac

mkdir -p "$LIB_BASE/$LIB_DIR"
cp "target/release/libvane_ffi.a" "$LIB_BASE/$LIB_DIR/"
echo "[build-go-lib] copied to lib/$LIB_DIR/libvane_ffi.a"
ls -lh "$LIB_BASE/$LIB_DIR/libvane_ffi.a"
