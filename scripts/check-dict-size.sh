#!/usr/bin/env bash
# SPEC §13.2-3：词典体积门禁
#   - Node @vane/dict-zh dict.bin gzip ≤ 1.5MB
#   - Go embed dict.bin.gz < 2MB（08-dict-go deferred，门禁就位待产物）
set -euo pipefail

cd "$(dirname "$0")/.."

# --- Node @vane/dict-zh ---
NODE_DICT="crates/vane-dict-zh/data/dict.bin"
if [ ! -f "$NODE_DICT" ]; then
  echo "FAIL: $NODE_DICT not found"
  exit 1
fi
NODE_SIZE=$(gzip -c "$NODE_DICT" | wc -c | tr -d ' ')
NODE_MAX=$((1500 * 1024))
echo "node dict.bin gzip: $NODE_SIZE bytes (max $NODE_MAX)"
if [ "$NODE_SIZE" -gt "$NODE_MAX" ]; then
  echo "FAIL: node dict.bin gzip > 1.5MB (SPEC §13.2-3)"
  exit 1
fi
echo "OK: node dict.bin gzip ≤ 1.5MB"

# --- Go embed dict.bin.gz ---
# 08-dict-go deferred：Go 侧词典产物尚未生成。门禁就位，待 08 落地后自动生效。
GO_DICT="bindings/go/dict/dict.bin.gz"
if [ ! -f "$GO_DICT" ]; then
  echo "SKIP: Go embed dict.bin.gz not found (08-dict-go deferred)"
else
  GO_SIZE=$(stat -c%s "$GO_DICT" 2>/dev/null || stat -f%z "$GO_DICT")
  GO_MAX=$((2 * 1024 * 1024))
  echo "go embed dict.bin.gz: $GO_SIZE bytes (max $GO_MAX)"
  if [ "$GO_SIZE" -gt "$GO_MAX" ]; then
    echo "FAIL: go embed > 2MB (SPEC §13.2-3)"
    exit 1
  fi
  echo "OK: go embed < 2MB"
fi
