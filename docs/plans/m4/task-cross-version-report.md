# M4 阶段三 a：跨版本持久化兼容 — 实现报告

> 阶段：M4 阶段三 a（跨版本持久化兼容，§3.4）
> 分支：feat/m4-prod-readiness
> BASE：1fc03af（Phase 1 docs commit）
> 实现者：SubAgent（sonnet）

## 1. fixture 生成方式

### 1.1 cross-tag build
- `git worktree add --detach /tmp/vane-v010 v0.1.0`（detached HEAD at v0.1.0 tag）。
- 在 v0.1.0 worktree 写 `crates/vane-core/tests/gen_compat_fixture.rs`（用 v0.1.0 API）。
- `cd /tmp/vane-v010 && cargo test -p vane-core --test gen_compat_fixture` → 产物在 `/tmp/v010-fixture/db/`。
- 拷贝至主工作树 `crates/vane-core/tests/fixtures/compat/v0.1.0/db/`。
- `git worktree remove /tmp/vane-v010 --force`（清理，无残留）。

### 1.2 已知文档集（确定性输入，非随机）
5 篇中英混排文档，schema=`body=Text, v=Vector{dim:4, Cosine}, tag=Scalar{Keyword}`：

| external_id | tag | text | vector |
|---|---|---|---|
| v010-d0 | a | "向量检索 混合搜索 hybrid search engine" | [1,0,0,0] |
| v010-d1 | b | "BM25 ranking text retrieval" | [0,1,0,0] |
| v010-d2 | a | "机器学习 与 搜索引擎 ranking" | [0,0,1,0] |
| v010-d3 | c | "cosine similarity vector space" | [1,1,0,0] |
| v010-d4 | b | "全文检索 inverted index 倒排" | [0,0,0,1] |

### 1.3 fixture 格式版本（v0.1.0 实测）
| 文件 | format_version | 说明 |
|---|---|---|
| header.bin | V1 | HEADER_FORMAT_V1 |
| vectors.bin | V2 | VECTORS_FORMAT_V2（v0.1.0 始终写 v2，含 dim 头 12 字节） |
| stored.bin | V1 | STORED_FORMAT_V1（无 zstd-encode，裸 JSON） |
| idmap.bin | V1 | IDMAP_FORMAT_V1 |
| scalars.col | V1 | SCALARS_FORMAT_V1 |
| inverted.bin | 1 | FORMAT_VERSION=1 |
| hnsw.bin | V1 | HNSW_FORMAT_V1 |

ULID：`01KZRQ9VAJ0000000000000000`（v0.1.0 确定性产物）。
体积：36KB（<100KB 约束满足）。

### 1.4 v0.1.0 API 与当前差异
`diff -rq v0.1.0/current` vane-core/src：
- `segment/header.rs`：Phase 2 fix（`<8`→`<9` off-by-one → panic-on-corrupt），decode-only，不影响 fixture 格式。
- `vector/mod.rs`/`vector/sq8.rs`：SIMD/scalar 路径注释 + SQ8 微调，不影响 fixture 格式。
- `vfs/fault.rs`：新文件（FaultVfs，test-only，不进生产）。
- `vfs/mod.rs`：FaultVfs mod 声明（test-only）。
- `persistence/`/`wal/`：**完全一致**（diff 为空）。
- `api/`（db.rs/collection.rs/types.rs）：**完全一致**。

结论：v0.1.0 与当前版本的持久化格式（段文件格式）完全一致，fixture 是真实 v0.1.0 产物。

## 2. 3 测试实现摘要

### 2.1 `reads_v0_1_0_fixture`
- `copy_fixture_to_temp()` → `StdFsVfs::with_root(temp)` → `Db::open(vfs, "db", ...)`。
- 验 manifest restore（collection "docs" 可见）。
- 验段文件 format_version 与 fixture 一致（6 文件 per-file version 常量断言：header V1 + vectors V2 + stored V1 + idmap V1 + scalars V1 + hnsw V1）。
- vector search `[1,0,0,0]` top_k=10：5 文档全回填（external_id == 已知集）+ baseline 顺序（d0 score=1.0 > d3 score≈0.707 > d1=d2=d4 score=0.0 按 docid 升序）+ stored fields（tag）回填。
- text search "检索"：命中 d0/d2/d4（非 vacuous）。
- hybrid search "检索" + [1,0,0,0]：5 文档全可见（非 vacuous）。

### 2.2 `v1_and_v2_segments_coexist`
- 复制 fixture 到 temp → open → 验旧段 stored.bin V1。
- add 2 新文档（v010-d5/d6）+ flush → 新段。
- 验新段 stored.bin 版本：`#[cfg(feature="zstd-encode")]` V2（真 v1/v2 共存）；`#[cfg(not(...))]` V1（双段同格式共存）。
- vector search [1,0,0,0] top_k=10：7 文档全可见（5 旧 + 2 新）+ d0 仍排第一 score=1.0（来自旧段）。
- hybrid search "检索 compatibility" + [1,0,0,0]：旧段 d0（含"检索"）+ 新段 d5/d6（含"compatibility"）均命中。

### 2.3 `migrates_v0_1_0_via_reindex`
- `#[ignore]` 占位，骨架已建。
- 验 v1 段可读（无需迁移）。
- 注释：未来 v3+ 格式升级时实现迁移器（遍历旧段 → 读 v1/v2 → flush 新段 → manifest 切换 → WAL → 删旧段）。

## 3. format-freeze note 摘要
`docs/plans/m4/format-freeze-note.md` 供 Phase 6 SPEC §6.2 修订：
- per-file format_version 冻结/演进矩阵（header/idmap/scalars/inverted/hnsw 冻结 V1 不可变；vectors/stored v1 冻结 v2 可演进）。
- 迁移策略（双模读 + merge 自然迁移 + 迁移器占位；不做原地迁移）。
- 格式冻结承诺（v1 不可变 / v2 可演进 / manifest/wal/BM25 冻结）。
- cross_version_compat.rs 测试覆盖表。
- fixture 生成方式（可复现）。

## 4. 文件清单
| 文件 | 类型 | 说明 |
|---|---|---|
| `crates/vane-core/tests/fixtures/compat/v0.1.0/db/manifest.json` | fixture | v0.1.0 manifest |
| `crates/vane-core/tests/fixtures/compat/v0.1.0/db/segments/seg_01KZRQ9VAJ0000000000000000/header.bin` | fixture | HEADER_FORMAT_V1 |
| `crates/vane-core/tests/fixtures/compat/v0.1.0/db/segments/seg_01KZRQ9VAJ0000000000000000/vectors.bin` | fixture | VECTORS_FORMAT_V2 |
| `crates/vane-core/tests/fixtures/compat/v0.1.0/db/segments/seg_01KZRQ9VAJ0000000000000000/stored.bin` | fixture | STORED_FORMAT_V1 |
| `crates/vane-core/tests/fixtures/compat/v0.1.0/db/segments/seg_01KZRQ9VAJ0000000000000000/idmap.bin` | fixture | IDMAP_FORMAT_V1 |
| `crates/vane-core/tests/fixtures/compat/v0.1.0/db/segments/seg_01KZRQ9VAJ0000000000000000/scalars.col` | fixture | SCALARS_FORMAT_V1 |
| `crates/vane-core/tests/fixtures/compat/v0.1.0/db/segments/seg_01KZRQ9VAJ0000000000000000/inverted.bin` | fixture | FORMAT_VERSION=1 |
| `crates/vane-core/tests/fixtures/compat/v0.1.0/db/segments/seg_01KZRQ9VAJ0000000000000000/hnsw.bin` | fixture | HNSW_FORMAT_V1 |
| `crates/vane-core/tests/fixtures/compat/v0.1.0/db/wal.log` | fixture | AddSegment 记录 |
| `crates/vane-core/tests/fixtures/compat/README.md` | doc | fixture 来源 + 生成方式 + 已知文档集 + 格式版本 |
| `crates/vane-core/tests/cross_version_compat.rs` | test | 3 测试（reads_v0_1_0_fixture + v1_and_v2_segments_coexist + migrates 占位） |
| `scripts/gen_compat_fixture.rs` | script | fixture-gen 镜像（非 workspace 编译目标） |
| `docs/plans/m4/format-freeze-note.md` | doc | 格式冻结承诺（供 Phase 6 SPEC §6.2 修订） |
| `docs/plans/m4/task-cross-version-report.md` | doc | 本报告 |

## 5. 各门禁真实输出

### 5.1 `cargo fmt --all -- --check`
```
（无输出，rc=0）
```

### 5.2 `cargo clippy --all-targets --all-features --workspace --exclude vane-fuzz -- -D warnings`
```
Checking vane-core v0.2.0
Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.90s
（rc=0，0 warnings）
```

### 5.3 `cargo test -p vane-core --all-features --test cross_version_compat`
```
running 3 tests
test migrates_v0_1_0_via_reindex ... ignored, 当前 v1/v2 双模读取覆盖兼容；未来格式升级（v3+）时实现迁移器
test reads_v0_1_0_fixture ... ok
test v1_and_v2_segments_coexist ... ok

test result: ok. 2 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.27s
```

### 5.4 `cargo test --workspace --all-features --exclude vane-fuzz`
```
test result: ok. 322 passed; 0 failed; 1 ignored; ... (vane-core unit)
test result: ok. 2 passed; 0 failed; 1 ignored; ... (cross_version_compat)
（全集成测试 0 FAILED，含 proptest 3 不变量 + crash_recovery 5 场景 + corpus_compat + recall 等）
```

### 5.5 `cargo deny check`
```
advisories ok, bans ok, licenses ok, sources ok
（1 pre-existing warning: regex/napi-derive-backend unmatched wrapper，非本任务引入）
```

### 5.6 `cargo check --target wasm32-unknown-unknown -p vane-core`
```
Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.08s
（rc=0，fixture/tests 不进 wasm）
```

### 5.7 fixture + worktree 确认
- fixture 文件：9 文件 in `crates/vane-core/tests/fixtures/compat/v0.1.0/db/`，36KB（<100KB）。
- `git worktree list`：仅 `/Users/ximing/project/mygithub/vane 1fc03af [feat/m4-prod-readiness]`，无 v0.1.0 worktree 残留。
- `/tmp/vane-v010` 不存在（已清理）。
- `git status` 无 Cargo.lock 变化（fixture 是数据非依赖）。

## 6. commit hash
`c07600e`（`c07600e1146252e38f4f9f4a0af942bc3d6f49e5`，amend 后）
- 分支：`feat/m4-prod-readiness`
- 提交信息：`test(core): cross_version_compat v0.1.0 fixture + v1/v2 共存（M4 阶段三 a）`
- 无 Co-Authored-By trailer
- 无 push

## 7. 自审

### 7.1 v0.1.0 API 与当前差异
- v0.1.0 与当前的 `api/`（db.rs/collection.rs/types.rs）、`persistence/`、`wal/` 完全一致（diff 为空）。
- `segment/header.rs` 差异是 Phase 2 fix（decode-only `<8`→`<9`），不影响 fixture 格式（fixture header 91 字节 ≥9 字节门限）。
- `segment/mod.rs` 完全一致（diff 为空）→ v0.1.0 始终写 vectors.bin V2，与当前一致。
- 结论：v0.1.0 fixture 是真实 v0.1.0 产物，格式与当前版本一致，当前版本双模读取通过。

### 7.2 fixture 真实性确认
- fixture 由 v0.1.0 tag 的 vane-core API 真实生成（非当前代码模拟）。
- v0.1.0 worktree 编译成功（`cargo build -p vane-core --tests` rc=0）。
- fixture-gen test 在 v0.1.0 运行通过（`gen_v0_1_0_fixture ... ok`）。
- 段文件 format_version 经 xxd 确认（header V1 / vectors V2 / stored V1 / 其余 V1）。
- ULID `01KZRQ9VAJ0000000000000000` 是 v0.1.0 确定性产物。

### 7.3 worktree 清理确认
- `git worktree remove /tmp/vane-v010 --force` 成功。
- `git worktree list` 仅显示主工作树。
- `/tmp/vane-v010` 目录不存在。
- `/tmp/v010-fixture` 临时产物已删除。
- 主工作树 git 状态无 v0.1.0 worktree 残留污染。

### 7.4 测试非 vacuous 确认
- `reads_v0_1_0_fixture`：6 per-file format_version 常量断言 + 5 external_id 集合断言 + 3 模式 search 命中断言 + baseline 顺序断言（d0/d3 位置 + score 精确值）→ 全 non-vacuous。
- `v1_and_v2_segments_coexist`：旧段 V1 + 新段 V2（zstd-encode 时）格式版本断言 + 7 文档全可见 + 旧段 d0 仍排第一 → non-vacuous。
- `migrates_v0_1_0_via_reindex`：`#[ignore]` 占位，骨架验证 v1 段可读（5 文档）。

### 7.5 不改 M0-M3 冻结 pub API
- 无 Rust 源码修改（仅新增 fixture + 测试 + 脚本 + 文档）。
- 无 Cargo.toml 修改（无新依赖）。
- 无 SPEC.md / CI yml / fault.rs / crash_recovery.rs / vane-fuzz / proptest 修改。

### 7.6 concerns
1. **v0.1.0 vectors.bin 实际为 V2（非 V1）**：设计文档 §3.4 称 "v1 fixture（v0.1.0 产物）" 措辞不精确——v0.1.0 的 `segment/mod.rs` 始终写 vectors.bin V2（与当前一致），不存在已发布的 V1 vectors.bin 产物。VECTORS_FORMAT_V1 读路径保留但无已发布 fixture 覆盖（仅 types.rs 单测常量断言）。本任务的 "v1/v2 共存" 在 stored.bin 维度（fixture V1 + 新 flush V2 仅 zstd-encode 时）。这是格式事实，非缺陷——fixture 真实反映 v0.1.0 产物，测试覆盖当前版本双模读取。
2. **`v1_and_v2_segments_coexist` 无 zstd-encode 时双段同格式**：不启 zstd-encode 时新段 stored.bin 也是 V1，"v1/v2 共存" 退化为 "双段共存"。CI `--all-features` 含 zstd-encode，真 v1/v2 共存被测；非 zstd-encode 配置下测双段共存（仍 non-vacuous）。这是 feature 门控的预期行为，非缺陷。
