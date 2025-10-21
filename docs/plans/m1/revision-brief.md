# M1 计划修订简报（plan-splitter 第二轮）

> 你是 plan-splitter（第二轮）。第一轮产出 12 计划 + README 经双视角 reviewer 审查（报告 `review-spec.md` / `review-feasibility.md`），发现 2 阻塞 + 若干 Major/Minor。用户已批准 SPEC 修订 + M1 全串行策略。本任务：应用 SPEC 修订 + 按审查反馈修订全部计划，产出无阻塞版本。
> 工作目录：`/Users/ximing/project/mygithub/vane`（main，HEAD=14db692）。全程中文。

## 一、SPEC 修订（用户已批准，应用到 `docs/SPEC.md` v1.0 → v1.1）

在 SPEC 顶部 changelog 加 v1.1 条目，并落实以下三处：

### S1. §5.4 TokenizerId — builtin_dict_version 语义澄清
- 现状：`TokenizerId := sha256( algorithm_version ‖ builtin_dict_version ‖ user_dict_bytes )`，未定义 builtin_dict_version 语义。与 REQUIREMENTS §3.3「词典升级打开老库仅警告不强制重建」存在张力（若 builtin_dict_version=运行时日历版本，词典升级→TokenizerId 变→E_TOKENIZER_MISMATCH→实质强制重建）。
- 修订：明确 `builtin_dict_version` = **编译期词典格式 spec 版本**（如 `b"jieba-fmt-v1"`），仅当 DAT 结构/HMM 参数**格式**变更时递增；词典**内容**升级（增删词条、日历版本变）**不改变** builtin_dict_version。词典运行时日历版本（`2026.08`）+ sha256_prefix 仅供 §12.3 三渠道一致性校验 + §3.3 升级警告，**不进 TokenizerId**。
- 补一句：词典内容升级→TokenizerId 不变→旧段可继续查询（§3.3 仅警告）；词典格式升级→TokenizerId 变→reindex 触发（合理，格式不兼容需重建）。

### S2. §9.1 FFI 句柄注册表 — DashMap → std::sync::RwLock
- 现状：§9.1 原文「全局注册表 `DashMap<u64, Arc<…>>`」与依赖黑名单（dashmap）直接冲突；M0 已用 `std::sync::RwLock<HashMap>`。
- 修订：改为「全局注册表 `std::sync::RwLock<HashMap<u64, Arc<…>>>`」。

### S3. §9.2 函数面 — 补 ReindexHandle + 词典分发 FFI
- 现状：§9.2 缺 ReindexHandle 必需的 progress/wait（§4.1 ReindexHandle 有 progress()/wait()）；M1 词典分发需 load_dict/version。
- 修订：§9.2 补列：`vane_reindex_progress(h, out_progress*) -> i32`、`vane_reindex_wait(h) -> i32`；并注明 M1 词典分发扩展：`vane_load_dict(h, dict_ptr, dict_len) -> i32`、`vane_dict_version(out_ptr, out_len*) -> i32`。

## 二、计划修订（按审查反馈，逐项落实）

### B-1（阻塞）原文持久化 — 新增前置计划
- 问题：M0 `api/collection.rs:195-223` flush 中 stored_json 仅由 doc.meta 构造，doc.text 仅 tokenize 进倒排后丢弃，**原文不入 stored.bin/任何段文件**。违反 SPEC §6.2（stored.bin 应含"原文/JSON meta"）。→ 06 reindex（换分词器重建倒排）不可实现。
- 修订：**新增 `modules/00-text-persistence.md`**（L0 前置，02/06 依赖）：
  - 扩展 stored.bin 按 §6.2 同时存原文 text + meta（或新增 text.bin——你判断，但 SPEC §6.2 措辞"stored.bin 含原文/JSON meta"倾向存 stored.bin）。format_version 保持 1（补全 spec'd 格式，非破坏；无发布数据故无迁移负担）。
  - SegmentWriter 扩展（不改 add_doc 签名，新增 text 写入路径或 add_doc 增 text 参数——你判断，倾向新增 `set_text` 或 add_doc 加 `text: Option<&str>` 参数；注意 M0 add_doc 签名是 `(external_id, vector, stored_json)`，若改 add_doc 参数属 M0 签名变更，须评估；更安全：stored_json 由 api 层构造时把 text 打包进去，不改 SegmentWriter 签名）。
  - SegmentReader 新增 `text(local_docid) -> Option<&str>`。
  - api/collection.rs flush：把 doc.text 一并写入 stored。
  - **更新 `tests/corpus_compat.rs`**：corpus 现在含原文，reopen 后验证 text 可读；文档化格式扩展。
  - TDD：写原文 roundtrip 测试 + reindex 前置可用性测试。
- 02-tombstone-merge 的 MergeTask 重建段时**复用原文**（从旧段 SegmentReader.text 读原文，新段写入）；**不再"重新分词"**——merge 分词器不变，倒排用 posting remap（可行性 reviewer 建议：从旧 InvertedIndexReader 读 postings 做 docid 重映射）。在 02 计划明确此点。
- 06-reindex：从旧段 `SegmentReader::text` 读原文，用**新分词器**重新 tokenize 重建倒排（向量/hnsw 复制不变）。明确 reindex 依赖 00 的原文持久化。

### B-2（阻塞）04-wal WAL truncate 逻辑修复
- 问题：`truncate` 清空整个 WAL → `flush→delete→flush→崩溃` 后 AddTombstone 丢失 → 已删除文档复活（数据损坏）。tombstone 运行期仅存 WAL（02 不改 header.bin），flush 的 truncate 会清除未消费的 AddTombstone。
- 修订（采用 Q-1 倾向方案）：**flush 不 truncate；仅 compact/merge 的 manifest 切换后 truncate WAL**。WAL 累积 AddSegment 记录直到 compact（ULID 字符串体积可忽略）。在 04 计划 Task 明确：`Wal::truncate` 仅由 compact 成功后调用；flush 路径不调 truncate。补崩溃恢复测试覆盖 `flush→delete→flush→崩溃→reopen` tombstone 不丢。

### R-3（阻塞）05-jieba-lite TokenizerId — 推翻方案 A
- 修订：`builtin_dict_version(Jieba)` 改为编译期格式常量 `b"jieba-fmt-v1"`（M0 `id.rs` 当前返回 `b""`，05 计划实装时改为常量）。`JiebaTokenizer::id()` **直接用 `compute_tokenizer_id(Jieba, user_dict)`，无二次哈希**。
- 删除 05 Task 7 的二次哈希逻辑（`sha256(compute_tokenizer_id(...).as_bytes() || dict.version() || sha256_prefix)`）。
- 修正 M0 `crates/vane-core/src/tokenizer/id.rs` 第 23 行注释（"日历版本"→"格式版本"）+ 模块文档注释第 4-5 行（05 计划实装时一并修，非现在改 M0）。
- README §05 契约文本统一（删除"扩展 builtin_dict_version 填词典版本"的矛盾表述，改为"builtin_dict_version(Jieba)=编译期格式常量"）。
- 词典日历版本 + sha256_prefix 仍存 dict.bin 头 + CollectionMeta，供 §12.3 三渠道一致性 + §3.3 升级警告，**不进 TokenizerId**。

### R-4/R-6（用户已确认）M1 全串行搜索
- 01-hnsw：**删除所有 `thread::scope` / `cfg(not(target_arch="wasm32"))` 表述**，明确「M1 全串行搜索，无 cfg（I-5 干净），Executor+并行延后 M2」。多段归并仍做（串行搜各段→归并）。
- README：加「已知阶段性偏离：M1 串行搜索，Executor trait + rayon/并行延后 M2（100万规模时引入）；若 11-cold-start-bench 实测 P99 >50ms 则在 M1 内补 Executor trait（cfg 仅在 Executor impl）」。
- 02-tombstone-merge MergeTask：M1 同步执行（step 串行），切片粒度留 M2 细化（注明）。

### M-2 02/06 MergeTask::new 签名补 tokenizer
- README §02 契约 `MergeTask::new` 补 `tokenizer` 参数：`new(sources, target_docid_base, tokenizer_id, schema, tokenizer: Box<dyn Tokenizer>)`（或 `with_tokenizer` builder——你判断）。02/06 正文与 README 统一。reindex 传新 tokenizer；compact 传当前 tokenizer。

### M-3 02 Task 2 测试代码
- 删除 `col.sync_tombstones()` 调用（正文已改用 WAL），测试代码同步更新为 WAL 路径验证。

### M1（可行性）12-recall Task 2 placeholder
- 删除 `unimplemented!()`，改为真实失败测试（或先写一个真能 fail 的 recall 断言骨架）。

### Q-5 01-hnsw M0 corpus 兼容（无 hnsw.bin）
- 01 计划明确：`HnswReader::open` 缺失 hnsw.bin 时返回 Err，api 层 catch 后 fallback `brute_search`（与 01 Task 5「hnsw_readers 无该段则 brute」一致）。M0 corpus（无 hnsw.bin）可被 M1 打开并暴力检索。

### Q-6 06 Rebuilding 期 E_BUSY
- 06 计划注明：「M1 选择 Rebuilding 期写路径返回 E_BUSY（保守，比 SPEC §7.4 更严格）；SPEC 允许未来放宽为旧身份写入」。

### Q-7 02 MergeTask 重建段 scalar 字段
- 02 Task 3 补：MergeTask 重建段时调 `SegmentWriter::set_scalar` 重写标量数据（从旧段 ScalarReader 读，重映射 docid 写新段）。

### Q-8 10-ci-m1 nDCG 维基 fixture 来源
- 10 计划明确 §13.2-2 ② 的 fixture 生成脚本与 CI 执行方式（离线生成中文维基 500 篇 + 50 查询的 fixture，CI 跑 nDCG 对比；若维基语料获取困难，文档化替代合成语料方案并标注降级）。

### 额外（可行性 reviewer minor）
- jieba-rs dev-dependency：05 计划注明须验证 jieba-rs 传递依赖不含黑名单 crate（regex/ndarray 等），若含则改用对照 fixture 而非 jieba-rs 运行时。
- 01 Task 6 测试 tautological（只读两次比字节）→ 改为有意义的 I-3 测试（如 delete 后验证 hnsw.bin 字节不变，需 02 delete 实装后补；M1 内 01 先保留字节稳定测试，02 补 delete 不动图测试）。

## 三、修订后自审

- 重新跑 placeholder 扫描（`grep -rn "TBD\|TODO\|适当处理\|unimplemented!" docs/plans/m1/modules/ docs/plans/m1/README.md`）→ 0 命中。
- 跨计划契约一致性：README Global Interface Contracts 与各计划 Produces/Consumes 对得上（重点核查 00-text-persistence 新契约 + MergeTask::new 新签名 + HnswReader 缺失 fallback）。
- 依赖图更新：00-text-persistence 加入 L0（或合适批次），02/06 依赖它。
- M0 API 对接：00 计划的 SegmentWriter/SegmentReader 扩展不破坏 M0 冻结签名（add_doc 签名若改须评估——倾向不改，api 层把 text 打包进 stored_json）。
- 不变量矩阵更新：00 补 I-1（段不可变，stored 扩展仍写一次）。

## 四、输出

- 修订 `docs/SPEC.md`（v1.0→v1.1，三处修订 + changelog）。
- 修订 `docs/plans/m1/README.md`（契约/依赖图/不变量矩阵/阶段性偏离注明）。
- 修订 `docs/plans/m1/modules/01..12-*.md` + 新增 `modules/00-text-persistence.md`。
- 报告写入 `docs/plans/m1/revision-report.md`：每项修订落实情况、自审结果、剩余疑点。
- 最终回复只返回：状态、修订文件列表、依赖拓扑一句话、是否无阻塞、剩余疑点。

## 红线（不变）
- MoSCoW 即合同；Won't-have 不碰；不引入黑名单依赖/dashmap/parking_lot/rayon（M1 全串行）。
- 词典永不进 wasm；jieba 算法不动只裁词典；core 禁 std::fs；cfg 只在 VFS（M1 不引入 Executor，故无新 cfg）。
- 不改 M0 已冻结 pub API（Vfs/Schema/brute_search/SegmentReader/SegmentWriter::add_doc 等）；reindex→ReindexHandle 是 SPEC IDL 落实（已批准）。
- SPEC 修订严格按本简报三处，不擅自扩大 SPEC 变更。
