#!/usr/bin/env bash
# M2-14 Demo 构建：产出浏览器可用的 wasm 双产物 + JS 胶水。
#
# 流程：
#   1. cargo build wasm32（simd / scalar 两次，worker feature）
#   2. wasm-bindgen --target web 后处理产出 JS 胶水 + _bg.wasm
#   3. wasm-opt -Oz 优化双 wasm
#   4. 拷贝到 demo/pkg/（vane_wasm.js + vane_wasm_simd.wasm + vane_wasm_scalar.wasm）
#
# 与 scripts/build-wasm-variants.sh 的区别：
#   - 后者仅 cargo build 产出 raw wasm（M2-05 体积门禁 / 特征校验用）。
#   - 本脚本额外跑 wasm-bindgen 后处理，产出浏览器可 import 的 JS 胶水。
#   - 两个 wasm 共享同一份 JS（导出一致，仅 target-feature 不同），由 worker.js
#     运行时 SIMD 探针动态选择加载。
#
# 用法：
#   bash demo/build.sh           # 默认 worker feature
#   FEATURES=worker bash demo/build.sh
set -euo pipefail

cd "$(dirname "$0")/.."

TARGET="wasm32-unknown-unknown"
PKG_CRATE="vane-wasm"
PKG_FILE="vane_wasm"   # cargo build 产物文件名（- 替换为 _）
FEATURES="${FEATURES:-worker}"
OUT="$(dirname "$0")/pkg"
TMP="$(dirname "$0")/.build-tmp"

mkdir -p "$OUT" "$TMP"

# ── 1. cargo build + wasm-bindgen 后处理（simd / scalar）──
build_variant() {
  local label="$1"   # simd | scalar
  local extra_flags="$2"

  echo "=== [$label] cargo build ($extra_flags) ==="
  RUSTFLAGS="$extra_flags" \
    cargo build --release --target "$TARGET" -p "$PKG_CRATE" --features "$FEATURES"

  local src="target/$TARGET/release/${PKG_FILE}.wasm"
  if [ ! -f "$src" ]; then
    echo "FAIL: $src not found" >&2
    exit 1
  fi

  echo "=== [$label] wasm-bindgen --target web ==="
  rm -rf "$TMP/$label"
  wasm-bindgen "$src" --out-dir "$TMP/$label" --target web

  local wasm_src="$TMP/$label/${PKG_FILE}_bg.wasm"
  local js_src="$TMP/$label/${PKG_FILE}.js"
  if [ ! -f "$wasm_src" ] || [ ! -f "$js_src" ]; then
    echo "FAIL: wasm-bindgen output missing for $label" >&2
    ls -la "$TMP/$label" >&2
    exit 1
  fi

  # wasm-opt 优化（可选）
  local wasm_dst="$OUT/vane_wasm_${label}.wasm"
  if command -v wasm-opt &>/dev/null; then
    wasm-opt -Oz "$wasm_src" -o "$wasm_dst"
    echo "(wasm-opt -Oz applied)"
  else
    cp "$wasm_src" "$wasm_dst"
    echo "(wasm-opt not available, copying unoptimized)"
  fi
  echo "→ $wasm_dst"
}

build_variant simd  "-Ctarget-feature=+simd128"
build_variant scalar ""

# ── 2. JS 胶水（两变体相同，保留一份）──
cp "$TMP/simd/${PKG_FILE}.js" "$OUT/vane_wasm.js"
echo "→ $OUT/vane_wasm.js"

# 清理临时目录
rm -rf "$TMP"

# ── 3. 词典数据（demo 用 fetch dictData 加载，非内联编译）──
DICT_SRC="crates/vane-dict-zh/data/dict.bin"
SHA_SRC="crates/vane-dict-zh/data/sha256_prefix.bin"
if [ -f "$DICT_SRC" ]; then
  cp "$DICT_SRC" "$OUT/dict.bin"
  cp "$SHA_SRC" "$OUT/sha256_prefix.bin"
  echo "→ $OUT/dict.bin ($(stat -f%z "$DICT_SRC" 2>/dev/null || stat -c%s "$DICT_SRC") bytes)"
  echo "→ $OUT/sha256_prefix.bin"
else
  echo "WARN: $DICT_SRC not found — demo 将降级 bigram" >&2
fi

# ── 4. 体积门禁（gzip ≤ 800KB，SPEC §13.2-3）──
MAX=$((800 * 1024))
FAIL=0
for v in simd scalar; do
  f="$OUT/vane_wasm_${v}.wasm"
  size=$(gzip -c "$f" | wc -c | tr -d ' ')
  echo "$v gzip: $size bytes (max $MAX)"
  if [ "$size" -gt "$MAX" ]; then
    echo "FAIL: $v gzip > 800KB" >&2
    FAIL=1
  fi
done

echo ""
echo "=== Demo pkg 产出 ==="
ls -la "$OUT"

# ── 5. nodejs 产出（供 e2e smoke 测试用，非浏览器 demo 用）──
NODE_OUT="$(dirname "$0")/pkg-node"
rm -rf "$NODE_OUT"
mkdir -p "$NODE_OUT"
echo ""
echo "=== Building nodejs target (for e2e smoke) ==="
wasm-bindgen "target/$TARGET/release/${PKG_FILE}.wasm" --out-dir "$NODE_OUT" --target nodejs
echo "→ $NODE_OUT/vane_wasm.js (nodejs target, for demo/e2e/run-smoke.mjs)"

exit $FAIL
