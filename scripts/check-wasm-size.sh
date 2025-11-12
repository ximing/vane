#!/usr/bin/env bash
# SPEC §13.2-3：核心 wasm gzip ≤ 800KB（jieba feature 默认关，wasm32 check 不带 jieba）。
#
# 两口径测量：
# 1. vane-wasm default（真实 deliverable 口径，M2-01）：cdylib + wasm-bindgen 胶水，
#    含 ruzstd decode + 真实 API 胶水。这是浏览器实际加载的 wasm。
# 2. vane-core --export-all（保守上界对照）：无 #[no_mangle] 导出，用 --export-all
#    强制导出防 dead-code 消除。捕捉代码膨胀。
set -euo pipefail

cd "$(dirname "$0")/.."

MAX=$((800 * 1024))

# ---- 口径 1：vane-wasm default（真实 deliverable）----
echo "=== vane-wasm default (real deliverable) ==="
cargo build --release --target wasm32-unknown-unknown -p vane-wasm

WASM_WASM="target/wasm32-unknown-unknown/release/vane_wasm.wasm"
if [ ! -f "$WASM_WASM" ]; then
  echo "FAIL: $WASM_WASM not found"
  exit 1
fi

WASM_DEFAULT="$WASM_WASM"
if command -v wasm-opt &>/dev/null; then
  wasm-opt -Oz "$WASM_WASM" -o "${WASM_WASM}.opt"
  WASM_DEFAULT="${WASM_WASM}.opt"
  echo "(wasm-opt -Oz applied)"
else
  echo "(wasm-opt not available, measuring unoptimized)"
fi

SIZE_DEFAULT=$(gzip -c "$WASM_DEFAULT" | wc -c | tr -d ' ')
echo "vane-wasm default gzip size: $SIZE_DEFAULT bytes (max $MAX)"
if [ "$SIZE_DEFAULT" -gt "$MAX" ]; then
  echo "FAIL: vane-wasm default gzip > 800KB (SPEC §13.2-3)"
  exit 1
fi
echo "OK: vane-wasm default gzip ≤ 800KB"

# ---- 口径 2：vane-core --export-all（保守上界对照）----
echo ""
echo "=== vane-core --export-all (conservative upper bound) ==="
# vane-core 无 #[no_mangle] 导出（FFI 导出在 vane-ffi/napi），cdylib 默认产出
# 空壳 wasm（~317 bytes，全部被链接器 dead-code 消除）。为测量真实编译体积，
# 用 -C link-arg=--export-all 强制导出所有符号——这是保守上界（实际 wasm
# 部署只导出子集，体积更小），但能可靠捕捉代码膨胀。
RUSTFLAGS="-C link-arg=--export-all" \
  cargo build --release --target wasm32-unknown-unknown -p vane-core

WASM_CORE="target/wasm32-unknown-unknown/release/vane_core.wasm"
if [ ! -f "$WASM_CORE" ]; then
  echo "FAIL: $WASM_CORE not found（vane-core 需配置 crate-type = [\"cdylib\", ...]）"
  exit 1
fi

WASM_CORE_OPT="$WASM_CORE"
if command -v wasm-opt &>/dev/null; then
  wasm-opt -Oz "$WASM_CORE" -o "${WASM_CORE}.opt"
  WASM_CORE_OPT="${WASM_CORE}.opt"
  echo "(wasm-opt -Oz applied)"
else
  echo "(wasm-opt not available, measuring unoptimized)"
fi

SIZE_CORE=$(gzip -c "$WASM_CORE_OPT" | wc -c | tr -d ' ')
echo "vane-core --export-all gzip size: $SIZE_CORE bytes (max $MAX)"
if [ "$SIZE_CORE" -gt "$MAX" ]; then
  echo "FAIL: vane-core --export-all gzip > 800KB (SPEC §13.2-3)"
  exit 1
fi
echo "OK: vane-core --export-all gzip ≤ 800KB"

echo ""
echo "=== Summary ==="
echo "vane-wasm default:      $SIZE_DEFAULT bytes (gzip)"
echo "vane-core --export-all: $SIZE_CORE bytes (gzip)"
