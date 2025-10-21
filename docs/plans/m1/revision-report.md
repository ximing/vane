# M1 计划修订报告（plan-splitter 第二轮）

> 修订日期：2026-08-09
> 修订依据：`docs/plans/m1/revision-brief.md`（用户已批准 SPEC 修订 + M1 全串行策略）
> 基线：第一轮产出（README + 12 模块）+ 双视角审查报告（review-spec.md / review-feasibility.md）
> 目标：应用 SPEC v1.0→v1.1 三处修订 + 逐项计划修订，产出无阻塞版本。

---

## 一、SPEC 修订落实（docs/SPEC.md v1.0 → v1.1）

| 项 | 落实情况 |
|---|---|
| **S1** §5.4 `builtin_dict_version` 语义澄清 | ✅ 补一段明确「编译期词典格式 spec 版本常量」（如 `b"jieba-fmt-v1"`），仅 DAT/HMM 格式变更递增，词典内容升级不变；日历版本 + sha256_prefix 不进 TokenizerId，仅供 §12.3 一致性 + §3.3 警告。补两条效果说明（内容升级→不变；格式升级→reindex）。 |
| **S2** §9.1 DashMap → std::sync::RwLock | ✅ 改为「`std::sync::RwLock<HashMap<u64, Arc<…>>>`」并注明 v1.1 原由（与黑名单冲突）。 |
| **S3** §9.2 补 reindex_progress/wait + load_dict/version | ✅ 补列 4 个 FFI 函数，注明 ReindexHandle IDL 落实 + M1 词典分发扩展。 |
| Changelog v1.1 条目 | ✅ 顶部标题改 v1.1；Changelog 加 v1.1 条目概述 S1/S2/S3。 |

SPEC 修订严格限定三处，未擅自扩大。

---

## 二、计划修订落实

### B-1（阻塞）原文持久化 — 新增 00-text-persistence.md

- ✅ 新增 `modules/00-text-persistence.md`（L0 前置，02/06 依赖）。
- ✅ 扩展 stored.bin 布局：`docid(8 LE) | text_len(4 LE) | text_bytes | meta_json_len(4 LE) | meta_json_bytes`。format_version 保持 1（补全 spec'd 格式，无发布数据故无迁移，corpus_compat 重新生成）。
- ✅ `SegmentWriter` 新增 `set_text`（不改 `add_doc` 冻结签名）；`SegmentReader` 新增 `text(local_docid) -> Option<&str>`；`stored_json` 语义不变。
- ✅ api flush 接入：`add_doc` 后调 `set_text`。
- ✅ corpus_compat 更新：验证原文 roundtrip + 文档化格式扩展。
- ✅ TDD：原文 roundtrip + reindex 前置可用性测试。
- ✅ 02 merge 改 posting remap（不重新分词）+ 复用原文（`SegmentReader::text` 读出写入新段）。
- ✅ 06 reindex 读原文用新分词器重新 tokenize（`InvertedIndexBuilder::add_document`，非 posting remap）。

### B-2（阻塞）04-wal WAL truncate 逻辑修复

- ✅ Architecture：flush 不 truncate；仅 compact/merge 成功 + manifest 切换后 truncate。
- ✅ Task 2：truncate 注明「compact-only，flush 不调」。
- ✅ Task 5：flush 测试改为「不 truncate，AddSegment 保留」；compact 测试「truncate」。
- ✅ Task 5b（新）：崩溃恢复回归 `flush→delete→flush→崩溃→reopen` tombstone 不丢。
- ✅ 验收标准更新。

### R-3（阻塞）05-jieba-lite TokenizerId — 推翻方案 A

- ✅ Architecture 补「TokenizerId（R-3）」段：`builtin_dict_version(Jieba)` = `b"jieba-fmt-v1"` 编译期常量，无二次哈希。
- ✅ 签名说明重写：推翻方案 A，`JiebaTokenizer::id()` 直接用 `compute_tokenizer_id(Jieba, user_dict)`。
- ✅ Task 7 删除二次哈希逻辑，改实装 `builtin_dict_version(Jieba)=b"jieba-fmt-v1"` + 新增「id 不依赖词典日历版本」测试。
- ✅ id.rs 注释修正（实装时改「日历版本」→「格式版本」）写入涉及文件。
- ✅ README §05 契约文本统一。
- ✅ jieba-rs dev-dep 传递依赖验证写入验收标准（含降级 fixture 方案）。

### R-4/R-6 M1 全串行搜索

- ✅ 01-hnsw：删 `thread::scope`/`cfg(not(target_arch="wasm32"))`，明确全串行无 cfg。
- ✅ README：加「已知阶段性偏离」节（M1 串行，Executor+rayon 延后 M2；11-bench 实测 >50ms 则补 Executor trait）。
- ✅ 02 MergeTask：M1 同步执行，切片粒度留 M2。
- ✅ 不变量矩阵 I-5 更新（M1 全串行无 thread::scope/cfg）。

### M-2 MergeTask::new 补 tokenizer

- ✅ README §02 契约 `MergeTask::new` 加 `tokenizer: Arc<dyn Tokenizer>` 参数。
- ✅ 02 Task 3/4 测试代码传 tokenizer。
- ✅ 06 Consumes from 02 签名统一；注明 reindex 传新 tokenizer（重新分词）、compact 传当前 tokenizer（posting remap）。

### M-3 02 Task 2 删 sync_tombstones

- ✅ Task 2 重写：删 `col.sync_tombstones()`，改 WAL 路径验证（delete 即时 append AddTombstone，reopen 经 WAL recover 重放）。

### M1 12 Task 2 删 unimplemented!()

- ✅ Task 2 重写为真实测试：`search_brute_baseline` 实装 + `assert_eq!(baseline.len(), 10)` 断言。

### Q-5 01 HnswReader 缺失 fallback brute

- ✅ 01 Architecture + Task 5 + README 契约：`HnswReader::open` 缺失 hnsw.bin 返回 Err，api catch 后 fallback `brute_search`。新增 `m0_corpus_without_hnsw_bin_falls_back_to_brute` 测试。`hnsw_readers` 改 `Vec<Option<Arc<HnswReader>>>`。

### Q-6 06 Rebuilding E_BUSY 注明

- ✅ 06 Architecture + 验收标准 + README 契约注明「M1 选择 Rebuilding 期 E_BUSY（比 SPEC §7.4 更严格），SPEC 允许未来放宽」。

### Q-7 02 MergeTask 重建段 scalar

- ✅ 02 Task 3 补 `set_scalar`（从源段 ScalarReader 读，重映射 docid 写新段）。README §02 Consumes 补 `set_scalar`。

### Q-8 10 nDCG fixture 来源明确

- ✅ 10 新增 Task 7：离线生成中文维基 500 篇 + 50 查询 fixture（提交仓库），CI 跑 nDCG；降级方案文档化（合成语料 + 门禁降级）。

### 额外 minor

- ✅ 01 Task 6 tautological 测试注明「占位，02 Task 7 补 delete 不动图强断言」。
- ✅ 06 Task 3 注明 standard 不消费 user_dict 的测试前提弱问题（jieba 场景留 10-ci-m1 job）。

---

## 三、自审结果

| 检查项 | 结果 |
|---|---|
| placeholder 扫描（`TBD\|TODO\|适当处理\|unimplemented!`） | ✅ 0 命中（grep exit=1） |
| SPEC 修订严格三处（S1/S2/S3） | ✅ 未扩大 |
| 依赖图含 00-text-persistence（L0） | ✅ 00 → 02/06；L0 批次 4 路并行（00/01/05/09） |
| README Global Interface Contracts 含 00 契约 | ✅ `set_text`/`text` + stored.bin 布局 |
| MergeTask::new 签名跨计划一致（README/02/06） | ✅ 含 `tokenizer: Arc<dyn Tokenizer>` |
| HnswReader 缺失 fallback（01 + README） | ✅ |
| 04 flush 不 truncate（Architecture/Task/验收） | ✅ |
| 05 无二次哈希（Architecture/Task7/README） | ✅ |
| 06 reindex 读原文（00 前置）+ E_BUSY 注明 | ✅ |
| M0 冻结签名不破坏（add_doc/open/stored_json/brute_search 等） | ✅ 00 仅新增 `set_text`/`text`；reindex→ReindexHandle 已批准 |
| 黑名单依赖（dashmap/parking_lot/rayon） | ✅ 全部为否定语境引用（「非 dashmap」「无 thread::scope」「不引 rayon」） |
| 不变量矩阵含 00（I-1） | ✅ |
| M1 不引入新 cfg（全串行） | ✅ 01/02/README 均注明零 cfg |
| Won't-have 不触碰 | ✅ 未改 |

---

## 四、剩余疑点

1. **posting remap 实现细节**（02）：M0 `InvertedIndexReader` 当前 pub API 主要是 `search`，是否暴露 raw postings 迭代供 docid 重映射尚需实装时核实。若不暴露，02 需在 M1 新增 postings 迭代方法（非 M0 冻结签名破坏，属扩展）。计划层面已描述清晰，实装时落实。
2. **nDCG 维基 fixture 体积**（10 Task 7）：500 篇维基 + 50 查询 fixture 提交仓库，体积待实测（预计 <5MB JSONL）。若超仓库承载，改 git-lfs 或 CI 生成缓存。降级方案已文档化。
3. **stored.bin format_version 保持 1**（00）：SPEC §6.2 始终要求 stored.bin 含「原文/JSON meta」，M0 实现占位不完整，00 补全属非破坏性完善（无发布数据）。仓库无历史 golden fixture（corpus_compat 注释明确「fresh repo」），corpus 兼容测试重新生成。首个正式发布后须冻结 golden fixture（§13.3）。
4. **HnswReader 缓存 Option 化**（01 Task 5）：`hnsw_readers: Vec<Option<Arc<HnswReader>>>` 是 M1 新增字段，类比 M0 `inverted_readers`，属 CollectionInner 内部扩展（pub(crate)），非 M0 pub API 破坏。
5. **06 reindex 标量重写**：06 Task 3 已补 `set_scalar` 标量重写步骤（与 02 merge 一致，Q-7 机制复用）。已关闭。

---

## 五、修订/新增文件列表

**修订**：
- `docs/SPEC.md`（v1.0→v1.1，S1/S2/S3 + changelog）
- `docs/plans/m1/README.md`（契约/依赖图/不变量矩阵/阶段性偏离）
- `docs/plans/m1/modules/01-hnsw.md`（R-4/R-6, Q-5, Task 6）
- `docs/plans/m1/modules/02-tombstone-merge.md`（B-1, M-2, M-3, Q-7, R-4）
- `docs/plans/m1/modules/04-wal.md`（B-2）
- `docs/plans/m1/modules/05-jieba-lite.md`（R-3）
- `docs/plans/m1/modules/06-userdict-reindex.md`（B-1, M-2, Q-6）
- `docs/plans/m1/modules/10-ci-m1.md`（Q-8）
- `docs/plans/m1/modules/12-recall-regression.md`（M1）

**新增**：
- `docs/plans/m1/modules/00-text-persistence.md`（B-1 前置计划）

未改动：03/07/08/09/11（审查无阻塞/Major 涉及，契约已由 README 统一）。
