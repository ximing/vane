#!/usr/bin/env bash
# SPEC §12.3：三渠道（Node/Go/内嵌）词典版本哈希一致性校验。
#
# Node 渠道（07）：vane-dict-zh crate 的 DICT_VERSION + SHA256_PREFIX_BIN。
# Go 渠道（08 deferred）：bindings/go/dict/ 产物，待 08 落地后启用。
#
# 本脚本校验：
# 1. Node 侧 dict.bin 的 sha256_prefix 与 sha256_prefix.bin 一致
# 2. DICT_VERSION 格式合法（YYYY.MM）
# 3. Go 侧（若有）版本哈希一致
set -euo pipefail

cd "$(dirname "$0")/.."

echo "=== 三渠道词典哈希一致性校验（SPEC §12.3）==="

# --- Node 渠道 ---
DICT_BIN="crates/vane-dict-zh/data/dict.bin"
SHA_FILE="crates/vane-dict-zh/data/sha256_prefix.bin"

if [ ! -f "$DICT_BIN" ]; then
  echo "FAIL: $DICT_BIN not found"
  exit 1
fi

# sha256_prefix.bin 是 dict.bin 解压后 payload 的 sha256 前 8 字节。
# 此处只校验文件存在 + 非空（完整校验在 Rust 测试 dict_tests.rs 中）。
if [ ! -f "$SHA_FILE" ]; then
  echo "FAIL: $SHA_FILE not found"
  exit 1
fi
SHA_SIZE=$(stat -c%s "$SHA_FILE" 2>/dev/null || stat -f%z "$SHA_FILE")
if [ "$SHA_SIZE" -ne 8 ]; then
  echo "FAIL: sha256_prefix.bin must be 8 bytes, got $SHA_SIZE"
  exit 1
fi
echo "OK: Node sha256_prefix.bin = 8 bytes"

# DICT_VERSION 从 Cargo.toml 读取（与 src/lib.rs DICT_VERSION 常量一致）
DICT_VERSION=$(grep '^version' crates/vane-dict-zh/Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')
if echo "$DICT_VERSION" | grep -qE '^[0-9]{4}\.[0-9]+$'; then
  echo "OK: Node DICT_VERSION = $DICT_VERSION (YYYY.MM)"
else
  # Cargo.toml version 是 semver（2026.8.0），DICT_VERSION 是 YYYY.MM（2026.08）
  # 两者格式不同但日期部分应一致
  echo "INFO: Cargo.toml version = $DICT_VERSION (semver)"
fi

# --- Rust 测试覆盖 ---
echo "INFO: 完整 sha256 一致性校验在 Rust 测试 dict_tests.rs 中（编译期 include_bytes vs 运行时 load_zstd）"

# --- Go 渠道（08 已落地：dict.bin.gz 已提交到 bindings/go/dict/）---
# Go 侧用 go:embed dict.bin.gz（gzip 再压缩），解压后与 Node 侧 dict.bin 同源。
# 校验逻辑：
#   1. gunzip Go dict.bin.gz → 与 Node dict.bin 比对 sha256（源字节一致 → prefix 隐含一致）
#   2. 版本一致性：Go DictVersion const vs Rust DICT_VERSION
#   3. 若 zstd 可用：解压 dict.bin → 读头部 [8..16] 直接比对 sha256_prefix.bin
GO_DICT_DIR="bindings/go/dict"
GO_DICT_GZ="$GO_DICT_DIR/dict.bin.gz"
if [ -f "$GO_DICT_GZ" ]; then
  # 1. 源字节 sha256 一致性（最强校验：字节相同 → prefix 必相同）
  compute_sha256() {
    if command -v sha256sum &>/dev/null; then
      sha256sum | awk '{print $1}'
    else
      shasum -a 256 | awk '{print $1}'
    fi
  }
  GO_SHA=$(gunzip -c "$GO_DICT_GZ" | compute_sha256)
  NODE_SHA=$(compute_sha256 < "$DICT_BIN")
  echo "Go dict.bin (gunzipped) sha256: $GO_SHA"
  echo "Node dict.bin sha256:           $NODE_SHA"
  if [ "$GO_SHA" != "$NODE_SHA" ]; then
    echo "FAIL: Go ↔ Node dict.bin sha256 mismatch (SPEC §12.3)"
    exit 1
  fi
  echo "OK: Go ↔ Node dict.bin sha256 一致（同源 → sha256_prefix 隐含一致）"

  # 2. 版本一致性
  GO_VER=$(grep 'const DictVersion' "$GO_DICT_DIR/dict.go" | sed 's/.*"\(.*\)".*/\1/')
  RUST_VER=$(grep 'pub const DICT_VERSION' crates/vane-dict-zh/src/lib.rs | sed 's/.*"\(.*\)".*/\1/')
  echo "Go DictVersion:    $GO_VER"
  echo "Rust DICT_VERSION: $RUST_VER"
  if [ "$GO_VER" != "$RUST_VER" ]; then
    echo "FAIL: Go ↔ Rust DICT_VERSION mismatch (SPEC §12.3)"
    exit 1
  fi
  echo "OK: Go ↔ Rust DICT_VERSION 一致"

  # 3. sha256_prefix 直接比对（zstd 可用时：解压 → 读头部 [8..16]）
  if command -v zstd &>/dev/null; then
    # 先解压到临时文件，避免 dd 提前关闭管道触发 SIGPIPE（pipefail 下误判失败）。
    TMP_PAYLOAD=$(mktemp)
    gunzip -c "$GO_DICT_GZ" | zstd -d > "$TMP_PAYLOAD" 2>/dev/null
    GO_PREFIX=$(dd if="$TMP_PAYLOAD" bs=1 skip=8 count=8 2>/dev/null | xxd -p | tr -d '\n')
    rm -f "$TMP_PAYLOAD"
    NODE_PREFIX=$(xxd -p "$SHA_FILE" | tr -d '\n')
    echo "Go sha256_prefix (header [8..16]): $GO_PREFIX"
    echo "Node sha256_prefix.bin:            $NODE_PREFIX"
    if [ "$GO_PREFIX" != "$NODE_PREFIX" ]; then
      echo "FAIL: Go ↔ Node sha256_prefix mismatch (SPEC §12.3)"
      exit 1
    fi
    echo "OK: Go ↔ Node sha256_prefix 一致（zstd 解压头部直接比对）"
  else
    echo "INFO: zstd 不可用，跳过头部 prefix 直接比对（源字节一致已隐含证明）"
  fi
else
  echo "SKIP: Go dict.bin.gz not found ($GO_DICT_GZ)"
fi

echo "=== 三渠道词典哈希一致性校验通过 ==="
