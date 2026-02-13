# 跨版本兼容 fixture

> 来源：M4 阶段三 a（跨版本持久化兼容，§3.4）。
> 此目录提交真实 v0.1.0 tag 生成的段文件 fixture，供 `tests/cross_version_compat.rs` 读取验证。

## 目录结构

```
compat/
├── v0.1.0/
│   └── db/                              # Db::open(vfs, "db", ...) 路径
│       ├── manifest.json                # v0.1.0 manifest（collection "docs" schema + segment_ulids）
│       ├── segments/
│       │   └── seg_01KZRQ9VAJ0000000000000000/
│       │       ├── header.bin          # HEADER_FORMAT_V1
│       │       ├── vectors.bin          # VECTORS_FORMAT_V2（v0.1.0 始终写 v2，含 dim 头）
│       │       ├── stored.bin           # STORED_FORMAT_V1（无 zstd-encode，裸 JSON）
│       │       ├── idmap.bin            # IDMAP_FORMAT_V1
│       │       ├── scalars.col          # SCALARS_FORMAT_V1
│       │       ├── inverted.bin         # FORMAT_VERSION=1
│       │       └── hnsw.bin             # HNSW_FORMAT_V1
│       └── wal.log                      # AddSegment 记录
└── README.md                            # 本文件
```

## fixture 来源

- **生成方式**：用 v0.1.0 tag 的 vane-core API 真实生成（非当前代码模拟）。
- **生成步骤**（可复现）：
  1. `git worktree add --detach /tmp/vane-v010 v0.1.0`
  2. 在 worktree 写 `crates/vane-core/tests/gen_compat_fixture.rs`（用 v0.1.0 API 创建 DB + 加已知文档集 + flush）。
  3. `cd /tmp/vane-v010 && cargo test -p vane-core --test gen_compat_fixture`。
  4. 产物在 `/tmp/v010-fixture/db/`，拷贝至 `crates/vane-core/tests/fixtures/compat/v0.1.0/db/`。
  5. `git worktree remove /tmp/vane-v010 --force`（清理 worktree）。
- **生成脚本镜像**：`scripts/gen_compat_fixture.rs`（非 workspace 编译目标，仅供文档/复现参考）。

## 已知文档集（baseline 断言用）

fixture 含 5 篇中英混排确定性文档（fixture-gen 确定性输入，非随机）：

| external_id | tag | text | vector |
|---|---|---|---|
| v010-d0 | a | "向量检索 混合搜索 hybrid search engine" | [1,0,0,0] |
| v010-d1 | b | "BM25 ranking text retrieval" | [0,1,0,0] |
| v010-d2 | a | "机器学习 与 搜索引擎 ranking" | [0,0,1,0] |
| v010-d3 | c | "cosine similarity vector space" | [1,1,0,0] |
| v010-d4 | b | "全文检索 inverted index 倒排" | [0,0,0,1] |

schema：`body=Text, v=Vector{dim=4, Cosine}, tag=Scalar{Keyword}`。

## 格式版本

fixture 段文件 per-file format_version（v0.1.0 产物，当前版本双模读取）：

| 文件 | format_version | 说明 |
|---|---|---|
| header.bin | V1 | 段元数据入口 |
| vectors.bin | V2 | v0.1.0 始终写 v2（含 dim 头，12 字节） |
| stored.bin | V1 | v0.1.0 无 zstd-encode → 裸 JSON |
| idmap.bin | V1 | external_id → docid 映射 |
| scalars.col | V1 | 标量列存 |
| inverted.bin | 1 | 倒排索引 |
| hnsw.bin | V1 | HNSW 图 |

## 体积约束

- fixture 总体积 36KB（<100KB 约束满足）。
- 最大文件：inverted.bin 709B。最小文件：scalars.col 54B。

## 不要修改

fixture 是冻结的 v0.1.0 产物，**禁止修改**。测试用 `copy_fixture_to_temp()` 复制到临时目录后操作，保持 fixture 源不被修改。如需重新生成，按上述步骤在 v0.1.0 tag worktree 离线运行。
