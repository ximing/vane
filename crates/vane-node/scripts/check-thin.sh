#!/bin/sh
# I-8 薄壳门禁：binding crate 不得含检索逻辑或禁用 IO。
# 退出码 0 = 薄壳干净。
#
# 仅匹配实际用法（模块路径 / 算法函数调用），跳过注释与字符串字面量中的词面提及。
# 命中即 I-8 违规。
set -e
# 切到 crate 根（脚本位于 <crate>/scripts/），使路径与调用位置无关。
cd "$(dirname "$0")/.."

# 去掉注释行后 grep 实际代码：
# - tokio:: / std::fs:: / std::net:: ：禁用运行时与 IO
# - 算法函数调用：rrf_fuse / linear_fuse / brute_search / cosine_sim / dot_product / hnsw / bm25
matches=$(grep -rnE 'tokio::|std::fs::|std::net::|rrf_fuse\(|linear_fuse\(|brute_search\(|cosine_sim|dot_product|hnsw|bm25' src/ \
  | grep -vE '^\s*[^:]+:\s*//' || true)

if [ -n "$matches" ]; then
  echo "I-8 violation: retrieval logic or forbidden IO found in binding" >&2
  echo "$matches" >&2
  exit 1
fi
echo "OK: vane-node is a thin binding (I-8 clean)"
