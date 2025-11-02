#!/usr/bin/env bash
# SPEC §13.2-3：核心 wasm gzip ≤ 800KB（jieba feature 默认关，wasm32 check 不带 jieba）。
#
# vane-core 无 #[no_mangle] 导出（FFI 导出在 vane-ffi/napi），cdylib 默认产出
# 空壳 wasm（~317 bytes，全部被链接器 dead-code 消除）。为测量真实编译体积，
# 用 -C link-arg=--export-all 强制导出所有符号——这是保守上界（实际 wasm
# 部署只导出子集，体积更小），但能可靠捕捉代码膨胀。
set -euo pipefail

cd "$(dirname "$0")/.."

# 构建 wasm32 cdylib（crate-type 含 cdylib）。--export-all 防止 dead-code 消除。
RUSTFLAGS="-C link-arg=--export-all" \
  cargo build --release --target wasm32-unknown-unknown -p vane-core

WASM="target/wasm32-unknown-unknown/release/vane_core.wasm"

if [ ! -f "$WASM" ]; then
  echo "FAIL: $WASM not found（vane-core 需配置 crate-type = [\"cdylib\", ...]）"
  exit 1
fi

# wasm-opt 若可用则优化（CI 装 binaryen）；本地无则跳过。
if command -v wasm-opt &>/dev/null; then
  wasm-opt -Oz "$WASM" -o "${WASM}.opt"
  WASM="${WASM}.opt"
  echo "(wasm-opt -Oz applied)"
else
  echo "(wasm-opt not available, measuring unoptimized)"
fi

SIZE=$(gzip -c "$WASM" | wc -c | tr -d ' ')
MAX=$((800 * 1024))

echo "wasm gzip size: $SIZE bytes (max $MAX)"
if [ "$SIZE" -gt "$MAX" ]; then
  echo "FAIL: wasm gzip > 800KB (SPEC §13.2-3)"
  exit 1
fi

echo "OK: wasm gzip ≤ 800KB"
