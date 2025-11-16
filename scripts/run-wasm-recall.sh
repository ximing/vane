#!/usr/bin/env bash
# M2-06 §8.4 双变体召回回归 CI 编排脚本。
#
# 流程：
#   1. simd 变体（RUSTFLAGS=-Ctarget-feature=+simd128）构建 + wasm-bindgen-test（node）
#      跑五档×三模式 recall@10≥0.95 + Jaccard 探针。
#   2. scalar 变体（默认 RUSTFLAGS）同上。
#   3. 两变体 Jaccard 探针输出跨变体比对（Jaccard ≥0.99 硬断言）。
#
# SPEC §8.4（双变体召回回归）/ §13.2-1（recall@10≥0.95 五档）。
# 依赖：rustup wasm32-unknown-unknown target + wasm-bindgen-cli（0.2.x）+ node ≥18。
#
# 用法：
#   bash scripts/run-wasm-recall.sh
# 环境变量：
#   NO_BUILD=1 — 跳过构建（复用已有 target 缓存，调试用）。
set -euo pipefail

cd "$(dirname "$0")/.."

TARGET="wasm32-unknown-unknown"
PKG="vane-wasm"
PROBE_TEST="recall_jaccard_probe"
SIMD_BIN="recall_regression_simd"
SCALAR_BIN="recall_regression_scalar"
WORK_DIR="${WORK_DIR:-target/wasm-recall}"

mkdir -p "$WORK_DIR"

# ---------------------------------------------------------------------------
# 跑单个变体：构建 + 测试 + 捕获 Jaccard 探针 JSON。
# 传参：$1=变体名(simd/scalar)  $2=RUSTFLAGS(可空)  $3=测试二进制名
# 输出：$WORK_DIR/<variant>_probe.json
# ---------------------------------------------------------------------------
run_variant() {
  local name="$1"
  local rustflags="${2:-}"
  local bin="$3"

  echo ""
  echo "============================================================"
  echo "=== ${name} 变体：构建 + wasm-bindgen-test (node) ==="
  echo "============================================================"

  if [ "${NO_BUILD:-0}" != "1" ]; then
    if [ -n "$rustflags" ]; then
      RUSTFLAGS="$rustflags" cargo test --target "$TARGET" -p "$PKG" --test "$bin" --no-run
    else
      cargo test --target "$TARGET" -p "$PKG" --test "$bin" --no-run
    fi
  fi

  # 跑全部测试（recall 五档×三模式 + Jaccard 探针），--nocapture 暴露 console.log。
  local out
  if [ -n "$rustflags" ]; then
    out=$(RUSTFLAGS="$rustflags" cargo test --target "$TARGET" -p "$PKG" --test "$bin" -- --nocapture 2>&1)
  else
    out=$(cargo test --target "$TARGET" -p "$PKG" --test "$bin" -- --nocapture 2>&1)
  fi

  echo "$out" | tail -20

  # 提取 Jaccard 探针 JSON 行。
  local probe
  probe=$(echo "$out" | grep '^JACCARD_PROBE ' | head -1 | sed 's/^JACCARD_PROBE //')
  if [ -z "$probe" ]; then
    echo "FAIL: ${name} 变体未产出 JACCARD_PROBE 行" >&2
    exit 1
  fi
  printf '%s' "$probe" > "$WORK_DIR/${name}_probe.json"
  echo "→ ${name} 探针 JSON 写入 $WORK_DIR/${name}_probe.json"
}

# ---------------------------------------------------------------------------
# 1. simd 变体
# ---------------------------------------------------------------------------
run_variant "simd" "-Ctarget-feature=+simd128" "$SIMD_BIN"

# ---------------------------------------------------------------------------
# 2. scalar 变体
# ---------------------------------------------------------------------------
run_variant "scalar" "" "$SCALAR_BIN"

# ---------------------------------------------------------------------------
# 3. 跨变体 Jaccard ≥0.99 硬断言
# ---------------------------------------------------------------------------
echo ""
echo "============================================================"
echo "=== 跨变体 Jaccard 比对 (≥0.99 硬断言) ==="
echo "============================================================"
node scripts/wasm-recall-jaccard.mjs "$WORK_DIR/simd_probe.json" "$WORK_DIR/scalar_probe.json"

echo ""
echo "✅ M2-06 双变体召回回归全部通过"
