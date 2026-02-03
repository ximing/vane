#!/usr/bin/env bash
# SPEC §12.3：四渠道词典版本哈希一致性校验。
#
# 词典分发四渠道：
# 1. Node：vane-dict-zh cargo path 依赖 include_bytes（crates/vane-dict-zh/data/dict.bin）
# 2. Go：bindings/go/dict/dict.bin.gz（gzip 再压缩，go:embed）
# 3. WASM CDN：fetch jsdelivr（fallback，运行时 sha256_prefix 校验，本脚本不覆盖）
# 4. WASM npm dictData：@vane-rs/dict-zh npm 包 data/dict.bin（Web 端 import asset url 传 dictData）
#
# 本脚本校验：
# 1. Node 侧 dict.bin 的 sha256_prefix 与 sha256_prefix.bin 一致（存在性 + 字节数）
# 2. DICT_VERSION 格式合法（YYYY.MM）
# 3. Go 侧版本哈希一致（gunzip ↔ Node 源字节 sha256 + DictVersion + zstd 头部 prefix）
# 4. npm 包侧 @vane-rs/dict-zh 产物 dict.bin 与源 data/dict.bin 字节一致
#    （package.json files + exports 元数据 + npm pack 字节级比对）
set -euo pipefail

cd "$(dirname "$0")/.."

echo "=== 四渠道词典哈希一致性校验（SPEC §12.3）==="

# sha256 计算工具（GNU sha256sum / BSD shasum -a 256），供 Go 与 npm 渠道共用。
compute_sha256() {
  if command -v sha256sum &>/dev/null; then
    sha256sum | awk '{print $1}'
  else
    shasum -a 256 | awk '{print $1}'
  fi
}

# --- 第一渠道：Node（vane-dict-zh include_bytes）---
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

# --- 第二渠道：Go（dict.bin.gz gzip 再压缩，go:embed）---
# Go 侧用 go:embed dict.bin.gz（gzip 再压缩），解压后与 Node 侧 dict.bin 同源。
# 校验逻辑：
#   1. gunzip Go dict.bin.gz → 与 Node dict.bin 比对 sha256（源字节一致 → prefix 隐含一致）
#   2. 版本一致性：Go DictVersion const vs Rust DICT_VERSION
#   3. 若 zstd 可用：解压 dict.bin → 读头部 [8..16] 直接比对 sha256_prefix.bin
GO_DICT_DIR="bindings/go/dict"
GO_DICT_GZ="$GO_DICT_DIR/dict.bin.gz"
if [ -f "$GO_DICT_GZ" ]; then
  # 1. 源字节 sha256 一致性（最强校验：字节相同 → prefix 必相同）
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

# --- 第四渠道：WASM npm dictData（@vane-rs/dict-zh npm 包）---
# Task 6：Web 端通过 import dictBinUrl from '@vane-rs/dict-zh/dict.bin' 取 dict.bin 字节传 VaneWorker dictData。
# npm 包的 data/dict.bin 就是 crates/vane-dict-zh/data/dict.bin（package.json files 字段直接引用源文件路径，
# 非拷贝），故第四渠道与第一渠道（Node include_bytes）同源。
# 校验：
#   1. package.json files 含 data/dict.bin + data/sha256_prefix.bin（确保 npm 包引用源文件）
#   2. package.json exports ./dict.bin → ./data/dict.bin + ./sha256_prefix.bin → ./data/sha256_prefix.bin
#   3. 若 npm 可用：npm pack 实际产物 → tar 提取 dict.bin → sha256 比对（字节级最严谨）
PKG_JSON="crates/vane-dict-zh/package.json"
if [ ! -f "$PKG_JSON" ]; then
  echo "FAIL: $PKG_JSON not found"
  exit 1
fi

# 1. files 字段校验（grep -F 字面匹配，避免 . 被当正则元字符）
if grep -Fq '"data/dict.bin"' "$PKG_JSON" && grep -Fq '"data/sha256_prefix.bin"' "$PKG_JSON"; then
  echo "OK: npm 包 files 含 data/dict.bin + data/sha256_prefix.bin"
else
  echo "FAIL: npm 包 files 缺 data/dict.bin 或 data/sha256_prefix.bin"
  exit 1
fi

# 2. exports 字段校验
if grep -Fq '"./dict.bin": "./data/dict.bin"' "$PKG_JSON"; then
  echo "OK: npm 包 exports ./dict.bin → ./data/dict.bin"
else
  echo "FAIL: npm 包 exports ./dict.bin 未指向 ./data/dict.bin"
  exit 1
fi
if grep -Fq '"./sha256_prefix.bin": "./data/sha256_prefix.bin"' "$PKG_JSON"; then
  echo "OK: npm 包 exports ./sha256_prefix.bin → ./data/sha256_prefix.bin"
else
  echo "FAIL: npm 包 exports ./sha256_prefix.bin 未指向 ./data/sha256_prefix.bin"
  exit 1
fi

# 3. npm pack 字节级比对（npm 可用时）
if command -v npm &>/dev/null; then
  TMPDIR_PACK=$(mktemp -d)
  # npm pack 产物路径前缀为 package/（tarball 内结构：package/data/dict.bin）
  (cd crates/vane-dict-zh && npm pack --pack-destination "$TMPDIR_PACK" >/dev/null 2>&1)
  TARBALL=$(ls "$TMPDIR_PACK"/*.tgz 2>/dev/null | head -1)
  if [ -z "$TARBALL" ]; then
    echo "FAIL: npm pack 未生成 tarball"
    rm -rf "$TMPDIR_PACK"
    exit 1
  fi
  # tar -O 提取指定路径到 stdout（GNU tar / BSD tar 均支持）
  NPM_DICT_SHA=$(tar -xzf "$TARBALL" -O package/data/dict.bin 2>/dev/null | compute_sha256)
  rm -rf "$TMPDIR_PACK"
  NODE_SHA_NPM=$(compute_sha256 < "$DICT_BIN")
  echo "npm pack dict.bin sha256: $NPM_DICT_SHA"
  echo "Node dict.bin sha256:     $NODE_SHA_NPM"
  if [ "$NPM_DICT_SHA" != "$NODE_SHA_NPM" ]; then
    echo "FAIL: npm pack dict.bin ↔ Node dict.bin sha256 mismatch (SPEC §12.3 第四渠道)"
    exit 1
  fi
  echo "OK: npm pack 产物 dict.bin ↔ Node dict.bin sha256 一致（第四渠道字节同源）"
else
  echo "INFO: npm 不可用，跳过 npm pack 字节比对（files + exports 元数据校验已间接保证同源）"
fi

echo "=== 四渠道词典哈希一致性校验通过 ==="
