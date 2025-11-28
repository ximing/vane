#!/usr/bin/env bash
# M2-05 SIMD128 双变体构建脚本（SPEC §12.2/§13.2-3）。
#
# 产出两个 wasm 产物：
#   target/wasm-variants/vane_wasm_simd.wasm   (RUSTFLAGS="-Ctarget-feature=+simd128")
#   target/wasm-variants/vane_wasm_scalar.wasm (默认，无 simd128)
#
# core 算法无平台分支（SPEC v1.4 I-5）——两产物由构建 flag 区分，core 代码完全相同。
# simd 变体中 f32 距离三核走显式 f32x4 intrinsics 路径（post-v0.1.1 Task 1，
# cfg(all(target_arch="wasm32", target_feature="simd128")) 向量化/标量双实现，
# v1.4 释义视为能力开关，归约顺序逐位对齐保证双变体 top-10 一致）；其余代码
# （roaring 位图等）仍靠 -Ctarget-feature=+simd128 启用 LLVM 自动向量化。
#
# 用法：
#   bash scripts/build-wasm-variants.sh           # 默认带 worker feature
#   FEATURES=worker bash scripts/build-wasm-variants.sh
#   FEATURES=opfs bash scripts/build-wasm-variants.sh
#
# 环境变量：
#   FEATURES  — vane-wasm cargo features（默认 worker）
#   OUT_DIR   — 输出目录（默认 target/wasm-variants）
#   NO_OPT    — 设为 1 跳过 wasm-opt -Oz（调试用）
set -euo pipefail

cd "$(dirname "$0")/.."

FEATURES="${FEATURES:-worker}"
OUT_DIR="${OUT_DIR:-target/wasm-variants}"
TARGET="wasm32-unknown-unknown"
PKG="vane-wasm"

mkdir -p "$OUT_DIR"

# ---- 公共：wasm-opt 优化 + 拷贝 ----
optimize_and_copy() {
  local src="$1"
  local dst="$2"
  if [ "${NO_OPT:-0}" = "1" ]; then
    cp "$src" "$dst"
    echo "(wasm-opt skipped)"
  elif command -v wasm-opt &>/dev/null; then
    wasm-opt -Oz "$src" -o "$dst"
    echo "(wasm-opt -Oz applied)"
  else
    cp "$src" "$dst"
    echo "(wasm-opt not available, copying unoptimized)"
  fi
}

# ============================================================
# 1. simd 变体：RUSTFLAGS="-Ctarget-feature=+simd128"
# ============================================================
echo "=== Building vane_wasm_simd.wasm (simd128) ==="
RUSTFLAGS="-Ctarget-feature=+simd128" \
  cargo build --release --target "$TARGET" -p "$PKG" --features "$FEATURES"

SIMD_SRC="target/$TARGET/release/vane_wasm.wasm"
if [ ! -f "$SIMD_SRC" ]; then
  echo "FAIL: $SIMD_SRC not found" >&2
  exit 1
fi
SIMD_DST="$OUT_DIR/vane_wasm_simd.wasm"
optimize_and_copy "$SIMD_SRC" "$SIMD_DST"
echo "→ $SIMD_DST"

# ============================================================
# 2. scalar 变体：默认构建（无 simd128 target-feature）
# ============================================================
echo ""
echo "=== Building vane_wasm_scalar.wasm (scalar) ==="
cargo build --release --target "$TARGET" -p "$PKG" --features "$FEATURES"

SCALAR_SRC="target/$TARGET/release/vane_wasm.wasm"
if [ ! -f "$SCALAR_SRC" ]; then
  echo "FAIL: $SCALAR_SRC not found" >&2
  exit 1
fi
SCALAR_DST="$OUT_DIR/vane_wasm_scalar.wasm"
optimize_and_copy "$SCALAR_SRC" "$SCALAR_DST"
echo "→ $SCALAR_DST"

# ============================================================
# 3. 特征校验（wasm-objdump）
# ============================================================
echo ""
echo "=== Feature verification (wasm-objdump) ==="
if ! command -v wasm-objdump &>/dev/null; then
  echo "WARN: wasm-objdump not available (brew install wabt); skipping feature check" >&2
else
  echo "-- simd variant target_features --"
  wasm-objdump -j target_features -x "$SIMD_DST" 2>/dev/null | grep -E 'simd128|^\s*-\s*name' || echo "(no target_features section)"
  echo "-- scalar variant target_features --"
  wasm-objdump -j target_features -x "$SCALAR_DST" 2>/dev/null | grep -E 'simd128|^\s*-\s*name' || echo "(no target_features section)"

  echo ""
  echo "-- simd128 instruction count (simd variant) --"
  SIMD_HITS=$(wasm-objdump -d "$SIMD_DST" | grep -cE 'f32x4|i32x4|v128' || true)
  echo "simd variant f32x4/i32x4/v128 instruction lines: $SIMD_HITS"
  if [ "$SIMD_HITS" -eq 0 ]; then
    echo "WARN: simd variant has 0 SIMD instructions — LLVM auto-vectorization may be insufficient" >&2
  fi

  echo "-- simd128 instruction count (scalar variant, expect 0) --"
  SCALAR_HITS=$(wasm-objdump -d "$SCALAR_DST" | grep -cE 'f32x4|i32x4|v128' || true)
  echo "scalar variant f32x4/i32x4/v128 instruction lines: $SCALAR_HITS"
fi

# ============================================================
# 4. 体积门禁（gzip ≤ 800KB，SPEC §13.2-3）
# ============================================================
echo ""
echo "=== Size gate (gzip ≤ 800KB) ==="
MAX=$((800 * 1024))

SIMD_SIZE=$(gzip -c "$SIMD_DST" | wc -c | tr -d ' ')
SCALAR_SIZE=$(gzip -c "$SCALAR_DST" | wc -c | tr -d ' ')
echo "simd gzip:   $SIMD_SIZE bytes (max $MAX)"
echo "scalar gzip: $SCALAR_SIZE bytes (max $MAX)"

FAIL=0
if [ "$SIMD_SIZE" -gt "$MAX" ]; then
  echo "FAIL: simd gzip > 800KB" >&2
  FAIL=1
fi
if [ "$SCALAR_SIZE" -gt "$MAX" ]; then
  echo "FAIL: scalar gzip > 800KB" >&2
  FAIL=1
fi

echo ""
echo "=== Summary ==="
echo "simd:   $SIMD_DST ($SIMD_SIZE bytes gzip)"
echo "scalar: $SCALAR_DST ($SCALAR_SIZE bytes gzip)"

exit $FAIL
