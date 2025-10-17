#!/bin/sh
# I-8 薄壳门禁：binding crate 不得含检索逻辑或禁用 IO。命中即 I-8 违规。
# 仅匹配实际代码（排除 // 注释行），跳过字符串字面量中的词面提及。
set -e
cd "$(dirname "$0")/.."

# tokio:: / std::fs:: / std::net:: 禁用运行时与 IO；
# rrf_fuse / linear_fuse / brute_search / cosine_sim / dot_product / hnsw / bm25 禁用算法调用。
matches=$(grep -rnE 'tokio::|std::fs::|std::net::|rrf_fuse\(|linear_fuse\(|brute_search\(|cosine_sim|dot_product|hnsw|bm25' src/ \
  | grep -vE '^\s*[^:]+:\s*//' || true)

if [ -n "$matches" ]; then
  echo "I-8 violation: retrieval logic or forbidden IO found in binding" >&2
  echo "$matches" >&2
  exit 1
fi
echo "OK: vane-node is a thin binding (I-8 clean)"
