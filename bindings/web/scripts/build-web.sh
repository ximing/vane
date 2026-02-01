#!/usr/bin/env bash
# @vane-rs/web 构建脚本（M3 阶段一 Task 2）：wasm-bindgen --target web ESM 双变体产物。
#
# 流程（对应 docs/plans/m3/task-1-design.md §7.4）：
#   1. 每变体：cargo build（simd128 / scalar，worker feature）
#   2. 每变体：wasm-bindgen --target web 后处理（产出 _bg.wasm + glue .js + .d.ts）
#   3. 每变体：wasm-opt -Oz 优化 _bg.wasm → vane_wasm_{simd,scalar}.wasm
#   4. 拷贝 JS 胶水 + .d.ts 到 dist/（双变体共享一份，导出一致）
#   5. cp vane_wasm_scalar.wasm vane_wasm_bg.wasm 别名（§7.3 默认 URL 兼容）
#   6. W8 校验：vane_wasm.js 含 __wbg_init + new URL(..., import.meta.url)
#   7. 体积门禁（gzip ≤ 800KB，SPEC §13.2-3）
#
# 不含 tsc 编译 src/*.ts（Task 3 扩展；src/ 尚不存在）。
#
# 技术说明（与 task brief 第 2 步的差异）：
#   task brief 称"scalar 不需要再跑 wasm-bindgen"。但 raw .wasm 的 __wbindgen_*
#   导入需经 wasm-bindgen 重写为 __wbg_* 才匹配 vane_wasm.js glue 的 import object
#   （键名 __wbg_*），否则 WebAssembly.instantiate 报 TypeError。故双变体都必须
#   跑 wasm-bindgen 后处理。glue 只拷一份（simd 与 scalar 的 glue 相同，导出一致）。
#   与 demo/build.sh 同模式（已验证可用）。
#
# 用法：
#   bash bindings/web/scripts/build-web.sh
#   FEATURES=worker bash bindings/web/scripts/build-web.sh
set -euo pipefail

cd "$(dirname "$0")/../../.."  # bindings/web/scripts/ → bindings/web/ → bindings/ → 仓库根

TARGET="wasm32-unknown-unknown"
PKG_CRATE="vane-wasm"
PKG_FILE="vane_wasm"   # cargo build 产物文件名（- 替换为 _）
FEATURES="${FEATURES:-worker}"
DIST="bindings/web/dist"
TMP="bindings/web/.build-tmp"

MAX=$((800 * 1024))

# 保存 simd 变体的 glue 路径（双变体共享一份 glue，只拷 simd 的）
JS_GLUE=""
DTS_GLUE=""

# ---- 辅助：wasm-opt 优化或拷贝 ----
optimize() {
  local src="$1" dst="$2"
  if command -v wasm-opt &>/dev/null; then
    wasm-opt -Oz "$src" -o "$dst"
    echo "(wasm-opt -Oz applied)"
  else
    cp "$src" "$dst"
    echo "(wasm-opt not available, copying unoptimized)" >&2
  fi
}

# ---- 单变体构建：cargo build → wasm-bindgen → wasm-opt ----
# 参数：$1=label (simd|scalar)  $2=extra_rustflags
build_variant() {
  local label="$1"
  local extra_flags="$2"

  echo "=== [$label] cargo build (RUSTFLAGS='$extra_flags', features=$FEATURES) ==="
  RUSTFLAGS="$extra_flags" \
    cargo build --release --target "$TARGET" -p "$PKG_CRATE" --features "$FEATURES"

  local src="target/$TARGET/release/${PKG_FILE}.wasm"
  [ -f "$src" ] || { echo "FAIL: $src not found" >&2; exit 1; }

  # ⚠️ 必须在 cargo build 后立即跑 wasm-bindgen：下一变体的 cargo build 会覆盖
  # target/.../vane_wasm.wasm（同路径），先拿到的 src 指向的是当前变体字节。
  echo "=== [$label] wasm-bindgen --target web ==="
  rm -rf "$TMP/$label"
  wasm-bindgen "$src" --out-dir "$TMP/$label" --target web

  local bg="$TMP/$label/${PKG_FILE}_bg.wasm"
  local js="$TMP/$label/${PKG_FILE}.js"
  local dts="$TMP/$label/${PKG_FILE}.d.ts"
  for f in "$bg" "$js" "$dts"; do
    [ -f "$f" ] || {
      echo "FAIL: $f missing (wasm-bindgen 产出不完整)" >&2
      ls -la "$TMP/$label" >&2
      exit 1
    }
  done

  # wasm-opt 优化 → dist/vane_wasm_{label}.wasm
  local dst="$DIST/vane_wasm_${label}.wasm"
  optimize "$bg" "$dst"
  echo "→ $dst"

  # simd 变体记录 glue 路径（双变体 glue 相同，只拷一份）
  if [ "$label" = "simd" ]; then
    JS_GLUE="$js"
    DTS_GLUE="$dts"
  fi
}

# ---- 清理 + 建目录 ----
rm -rf "$TMP" "$DIST"
mkdir -p "$TMP" "$DIST"

# ============================================================
# 1-3. 双变体构建（每变体：cargo build → wasm-bindgen → wasm-opt）
# ============================================================
build_variant simd  "-Ctarget-feature=+simd128"
echo ""
build_variant scalar ""

# ============================================================
# 4. 拷贝 JS 胶水 + .d.ts（双变体共享一份）
# ============================================================
cp "$JS_GLUE" "$DIST/vane_wasm.js"
cp "$DTS_GLUE" "$DIST/vane_wasm.d.ts"
echo "→ $DIST/vane_wasm.js"
echo "→ $DIST/vane_wasm.d.ts"

# ============================================================
# 5. cp vane_wasm_scalar.wasm vane_wasm_bg.wasm 别名（§7.3 默认 URL 兼容）
#    wasm-bindgen 生成的 vane_wasm.js 末尾默认 new URL('vane_wasm_bg.wasm', import.meta.url)。
#    双变体重命名为 _simd/_scalar 后无 _bg.wasm，bundler 静态分析会报错。
#    cp scalar 别名保守默认 scalar；worker.js 显式传 URL 覆盖默认。
# ============================================================
cp "$DIST/vane_wasm_scalar.wasm" "$DIST/vane_wasm_bg.wasm"
echo "→ $DIST/vane_wasm_bg.wasm (scalar 别名，默认 URL 兼容)"

# ============================================================
# 6. W8 校验：vane_wasm.js 含 __wbg_init + new URL(..., import.meta.url)
# ============================================================
echo ""
echo "=== W8 wasm-bindgen 生成校验 ==="
if ! grep -q '__wbg_init' "$DIST/vane_wasm.js"; then
  echo "FAIL: vane_wasm.js 缺 __wbg_init（wasm-bindgen 生成结构异常，W8）" >&2
  exit 1
fi
if ! grep -qE 'new URL\([^)]*import\.meta\.url\)' "$DIST/vane_wasm.js"; then
  echo "FAIL: vane_wasm.js 缺 new URL(..., import.meta.url)（默认 URL 解析异常，W8）" >&2
  exit 1
fi
echo "OK: __wbg_init + new URL(..., import.meta.url) 均存在"

# ============================================================
# 7. 体积门禁（gzip ≤ 800KB，SPEC §13.2-3）
# ============================================================
echo ""
echo "=== Size gate (gzip ≤ 800KB) ==="
FAIL=0
for v in simd scalar; do
  f="$DIST/vane_wasm_${v}.wasm"
  size=$(gzip -c "$f" | wc -c | tr -d ' ')
  raw=$(stat -f%z "$f" 2>/dev/null || stat -c%s "$f")
  echo "$v: raw=$raw bytes, gzip=$size bytes (max $MAX)"
  if [ "$size" -gt "$MAX" ]; then
    echo "FAIL: $v gzip > 800KB" >&2
    FAIL=1
  fi
done

# bg.wasm 别名体积（= scalar，仅日志，不入门禁）
BG_SIZE=$(gzip -c "$DIST/vane_wasm_bg.wasm" | wc -c | tr -d ' ')
echo "bg (alias of scalar): gzip=$BG_SIZE bytes (不入门禁，别名)"

echo ""
echo "=== dist 产出 ==="
ls -la "$DIST"

# 清理临时目录
rm -rf "$TMP"

exit $FAIL
