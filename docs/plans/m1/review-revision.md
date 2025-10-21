# M1 计划集第二轮修订复审报告

> 复审者：代码审查者（聚焦修订/新增部分）
> 复审日期：2026-08-09
> 审查对象：revision-report.md + SPEC v1.1（§5.4/§9.1/§9.2）+ 新增/修订计划文件（00/01/02/04/05/06 + README）
> 基线：review-spec.md（opus）+ review-feasibility.md（sonnet）原审查发现 + M0 README 契约 + M0 实际代码（`crates/vane-core/src/` 逐文件核对）
> 红线：只读审查，不改文件。全程中文。

---

## 0. Verdict

**APPROVED_WITH_MINOR** — 可进入阶段二（L0 开工）。

第一轮 2 阻塞项（B-1 原文持久化、B-2 WAL truncate）+ R-3 推翻方案 A + R-4/R-6 全串行 + 全部 Major/Minor（M-2/M-3/M1/Q-5/Q-6/Q-7/Q-8）均已正确闭环，未引入新阻塞或契约错配。仅余 2 项 Minor（测试代码 + 陈旧注释），可在实装时修正，不阻断计划批准。

---

## 1. B-1 闭环（00-text-persistence + 02/06 消费） — ✅

| 子项 | 结论 | 证据 |
|---|---|---|
| stored.bin 新布局 | ✅ | 00 Architecture + README §00：`magic(4) \| format_version(4 LE)=1 \| count(4 LE) \| { docid(8 LE) \| text_len(4 LE) \| text_bytes \| meta_json_len(4 LE) \| meta_json_bytes }...`。format_version 保持 1（补全 spec'd 字段，无发布数据故无迁移，合理） |
| `set_text`/`text` 不改 M0 冻结签名 | ✅ | 00 Produces：仅新增 `set_text(&mut self, text: &str) -> Result<()>` + `text(&self, local_docid: u64) -> Option<&str>`。M0 README 契约 `add_doc`/`finalize`/`new`/`open`/`stored_json` 签名均未改（已核对 M0 README §04-segment-format 第 282-316 行） |
| api flush 接入 | ✅ | 00 Task 2：`add_doc` 后 `writer.set_text(doc.text.as_deref().unwrap_or(""))`；text 为 None 传空串（text_len=0） |
| corpus_compat 覆盖原文 | ✅ | 00 Task 3：读 stored.bin 字节校验 `text_len > 0` 且 text_bytes 等于 `corpus_docs()[0].text` UTF-8 字节 |
| 02 merge 改 posting remap + 复用原文 | ✅ | 02 Architecture + Task 3：倒排用 posting remap（不重新分词），原文从 `SegmentReader::text` 读出 → `set_text` 写新段。描述清晰 |
| 06 reindex 读原文用新分词器重建倒排 | ✅ | 06 Architecture 步骤 3：从旧段 `SegmentReader::text` 读原文 → 新分词器 tokenize → `InvertedIndexBuilder::add_document`（**非 posting remap**）。与 02 merge 路径区分正确（reindex 分词器变 → 重新分词；merge 分词器不变 → posting remap） |

### 重点核查：M0 TokenizerId API

00 Task 1 测试代码使用 `TokenizerId::from_bytes([0u8; 32])`。经核对 M0 实际代码 `crates/vane-core/src/types.rs` 第 109-124 行：

```rust
pub struct TokenizerId(pub [u8; 32]);
impl TokenizerId {
    pub fn as_bytes(&self) -> &[u8; 32] { ... }
    pub fn to_hex(&self) -> String { ... }
    pub fn from_hex(s: &str) -> Result<Self> { ... }
}
```

M0 TokenizerId **无 `from_bytes` 方法**，仅有 `as_bytes`/`to_hex`/`from_hex` + 公开元组字段 `pub [u8; 32]`。

**结论**：⚠️ Minor（测试代码）。`TokenizerId::from_bytes([0u8; 32])` 编译会失败，developer 实装时应改为 `TokenizerId([0u8; 32])`（直接元组构造，字段 pub 可用）。非计划阻塞，按任务指令标记为 minor。

---

## 2. B-2 闭环（04-wal） — ✅

| 子项 | 结论 | 证据 |
|---|---|---|
| flush 不调 truncate | ✅ | 04 Task 5 `flush_appends_add_segment_does_not_truncate`：断言 flush 后 WAL `read_all()` 含 AddSegment（"flush must NOT truncate WAL (B-2)"）。Architecture 明确「flush 路径不调 `Wal::truncate`」 |
| compact 唯一 truncate 点 | ✅ | 04 Task 5 `compact_truncates_wal_after_manifest_switch`：compact + manifest 切换后 WAL 清空。Task 5 最小实现注明「compact 是唯一 truncate 调用点」 |
| Task 5b 回归测试覆盖 flush→delete→flush→崩溃 | ✅ | 04 Task 5b `crash_after_flush_delete_flush_keeps_tombstone`：构造 flush1→delete→flush2→崩溃序列，reopen 后断言 `!hits.iter().any(|h| h.id == "d0")`（tombstone 存活）+ d1 仍可见。精确覆盖 B-2 场景 |
| 无遗漏 truncate 调用路径 | ✅ | grep 全计划：`truncate` 仅出现在 04 Task 2（实现）/Task 5（compact 调用）/Task 5b（注释）/README §04 契约注释。flush 路径仅 append AddSegment，delete 路径仅 append AddTombstone。无第三处 truncate 调用 |

**核心修复机制正确**：tombstone 仅存 WAL（02 不改 header.bin，段不可变 I-1），flush 不 truncate 保证 AddTombstone 不丢；compact 时 AddTombstone 随旧段物理清除才 truncate。逻辑闭环。

---

## 3. R-3 闭环（05-jieba-lite + SPEC §5.4） — ✅

| 子项 | 结论 | 证据 |
|---|---|---|
| 05 删除二次哈希 | ✅ | 05 Architecture「TokenizerId（R-3，推翻方案 A）」+ Produces 签名说明：明确「`JiebaTokenizer::id()` **直接用 `compute_tokenizer_id(Jieba, user_dict)`，无二次哈希**」。旧方案 A（`sha256(compute_tokenizer_id(...).as_bytes() \|\| dict.version() \|\| sha256_prefix)`）被否决 |
| builtin_dict_version 改编译期常量 | ✅ | 05 Task 7 最小实现：「`id.rs::builtin_dict_version(Jieba)` 改为 `b"jieba-fmt-v1"`（编译期常量，R-3）」。返回类型仍 `&'static [u8]`（M0 签名不变） |
| 05 Task 7 新增「id 不依赖词典日历版本」测试 | ✅ | `jieba_tokenizer_id_independent_of_dict_calendar_version`：两个不同内容词典（同格式）→ 同一 TokenizerId。直接验证 REQUIREMENTS §3.3 |
| SPEC §5.4 语义与计划一致 | ✅ | SPEC §5.4 v1.1 第 169-173 行：`builtin_dict_version` = 编译期词典格式 spec 版本常量，仅 DAT/HMM 格式变更递增，内容升级不变。与 05 Architecture 完全一致 |
| id.rs 注释修正写入涉及文件 | ✅ | 05 涉及文件 Modify 列：`id.rs`（builtin_dict_version 从 `b""` 改 `b"jieba-fmt-v1"`；修正第 23 行注释「日历版本」→「格式版本」+ 模块文档注释第 4-5 行「jieba 词典版本」→「jieba 词典**格式**版本」）。已核对 M0 id.rs 第 4-5 行 + 第 23 行注释确实存在且需修正 |

### 新发现 Minor

**06 line 68 陈旧注释**：`// JiebaTokenizer::id() 含词典版本 + sha256_prefix` — 与 R-3 修订（id() 直接用 compute_tokenizer_id，**不**含词典版本/sha256_prefix）矛盾。这是 06「Consumes from 05」块的代码注释，函数签名本身无误。developer 实装时应改为「`JiebaTokenizer::id()` 直接用 `compute_tokenizer_id`，无二次哈希（R-3）」。⚠️ Minor（陈旧注释，非阻塞）。

---

## 4. R-4/R-6 闭环（01-hnsw） — ✅

| 子项 | 结论 | 证据 |
|---|---|---|
| 01 删除 thread::scope/cfg 表述 | ✅ | grep 全计划：`thread::scope` 仅出现在否定语境（「无 `thread::scope`」「不引 rayon/std::thread::scope」）。01 Architecture 第 17 行明确「hnsw 模块零 `cfg(target)`，无 `thread::scope`，无 rayon」。无残留矛盾表述 |
| 明确全串行 | ✅ | 01 Architecture + Global Constraints + README §01 契约 + README「已知阶段性偏离」第 1 项均注明 M1 全串行，Executor+rayon 延后 M2。11-bench 实测 >50ms 则补 Executor trait（cfg 仅在 Executor impl） |
| Q-5 HnswReader 缺失 fallback brute | ✅ | 01 Architecture 第 18 行 + Task 5 `m0_corpus_without_hnsw_bin_falls_back_to_brute` 测试 + README §01 契约注释：`HnswReader::open` 缺失 hnsw.bin 返回 Err，api catch 后 fallback `brute_search`。`hnsw_readers` 改 `Vec<Option<Arc<HnswReader>>>`（类比 M0 inverted_readers） |

---

## 5. M-2/M-3/M1/Q-6/Q-7/Q-8 逐项核对

| # | revision-report 声明 | 实际文件 | 结论 |
|---|---|---|---|
| M-2 | README §02 契约 `MergeTask::new` 加 `tokenizer: Arc<dyn Tokenizer>` | README §02 第 202-209 行 + 02 Task 3/4 测试传 `tok` + 06 Consumes from 02 签名含 `tokenizer: Arc<dyn Tokenizer>` | ✅ 一致 |
| M-3 | 02 Task 2 删 `sync_tombstones()`，改 WAL 路径 | 02 Task 2 测试注释「不调 sync_tombstones（M-3 修订：header.bin 不改，tombstone 运行期仅存 WAL+内存）」+ 改验证 WAL 重放 | ✅ |
| M1 | 12 Task 2 删 `unimplemented!()`，改真实测试 | 12 Task 2 `search_brute_baseline_returns_topk_without_hnsw`：实装 `search_brute_baseline` + `assert_eq!(baseline.len(), 10)`。grep 确认无 `unimplemented!` 残留 | ✅ |
| Q-6 | 06 Rebuilding E_BUSY 注明 | 06 Architecture 第 24 行 + 验收标准 + README §06 契约均注明「M1 选择 Rebuilding 期 E_BUSY（比 SPEC §7.4 更严格），SPEC 允许未来放宽」 | ✅ |
| Q-7 | 02 Task 3 补 `set_scalar` | 02 Task 3 最小实现：`set_scalar`（标量从源段 ScalarReader 读，重映射 docid）+ README §02 Consumes 补 `set_scalar` | ✅ |
| Q-8 | 10 新增 Task 7 nDCG fixture | 10 Task 7：离线生成 500 篇 + 50 查询 fixture（`scripts/ndcg-fixture/`），产物提交仓库 `tests/fixtures/ndcg_wiki_500.jsonl` + `ndcg_queries_50.json`，CI 跑 nDCG；降级方案（合成语料 + 门禁降级）文档化 | ✅ |

---

## 6. 跨计划契约一致性 — ✅

| 契约 | 产出方 | 消费方 | 结论 |
|---|---|---|---|
| `set_text`/`text` | 00 | 02（merge 复用原文）、06（reindex 读原文重新分词） | ✅ 02 Task 3 + 06 Architecture 步骤 3 均正确消费 `SegmentReader::text` + `SegmentWriter::set_text` |
| `MergeTask::new` 含 `tokenizer: Arc<dyn Tokenizer>` | 02 | 06（reindex 传新 tokenizer 走重新分词）、02 内部 compact（传当前 tokenizer 走 posting remap） | ✅ README/02/06 三处签名一致；reindex vs compact 路径区分清晰（重新分词 vs posting remap） |
| 依赖图含 00 | README | — | ✅ README 依赖图：`00 --> 02` + `00 --> 06`；L0 批次 4 路并行（00/01/05/09） |
| `HnswReader::open` 缺失 fallback | 01 | README §01 契约 | ✅ 一致 |
| WalRecord 语义 | 04 | 02（delete/compact 产出） | ✅ AddSegment/DeleteSegment/AddTombstone 三变体对齐 |

### 剩余实装期疑点（revision-report 已声明，非阻塞）

1. **02 posting remap 需访问 InvertedIndexReader 私有 terms 字段**：已核对 M0 `bm25.rs` 第 313-324 行 `InvertedIndexReader.terms: Vec<(String, TermEntry)>` 为**私有字段**（无 `pub`），`TermEntry`/`Block`/`Posting` 为 pub struct。02 posting remap 需遍历 term→postings 做 docid 重映射，当前 pub API（`search`/`doc_count`/`avg_field_length`）不暴露 raw postings 迭代。revision-report §四-1 已声明「若不暴露，02 需在 M1 新增 postings 迭代方法（非 M0 冻结签名破坏，属扩展）」。计划层面描述清晰，实装时落实。**可接受**。

---

## 7. 新问题扫描

| 扫描项 | 结论 |
|---|---|
| 新 placeholder（TBD/TODO/unimplemented!） | ✅ grep 全计划 0 命中 |
| 契约错配 | ✅ 无（见 §6） |
| M0 冻结签名破坏 | ✅ 无（00 仅新增 set_text/text；reindex→ReindexHandle 已批准 R-2；MergeTask::new 是 M1 新增类型非 M0 冻结） |
| 黑名单依赖（dashmap/parking_lot/rayon） | ✅ 全部为否定语境引用（「非 dashmap」「无 thread::scope」「不引 rayon」）。09 用 std::sync::RwLock |
| SPEC 修订严格三处（S1/S2/S3） | ✅ 未扩大。SPEC v1.1 changelog 第 455 行明确仅 S1/S2/S3 |
| 新 cfg 引入 | ✅ 01/02/README 均注明零 cfg；jieba 用 `cfg(feature="jieba")` 是 feature cfg 非 target cfg，不违反 I-5 |
| Won't-have 触碰 | ✅ 未改 |

### 新发现 Minor（2 项，均非阻塞）

1. **00 Task 1 测试用 `TokenizerId::from_bytes([0u8; 32])`**：M0 TokenizerId 无此方法（仅有 as_bytes/to_hex/from_hex + pub 元组字段）。developer 实装时改 `TokenizerId([0u8; 32])`。已在 §1 标注。
2. **06 line 68 陈旧注释**：`// JiebaTokenizer::id() 含词典版本 + sha256_prefix` 与 R-3 修订矛盾。developer 实装时改注释。已在 §3 标注。

---

## 8. 总结

第一轮审查发现的全部阻塞项（B-1 原文持久化、B-2 WAL truncate）+ R-3 推翻方案 A + R-4/R-6 全串行 + 全部 Major（M-1/M-2/M-3）+ Q-5/Q-6/Q-7/Q-8 均已在第二轮修订中正确闭环，且经逐项核对 revision-report 声明与实际计划文件一致（无「声明落实但文件未改」的虚报）。SPEC v1.1 三处修订（§5.4/§9.1/§9.2）严格限定未扩大。未引入新阻塞、新契约错配、新 M0 签名破坏或新黑名单依赖。

仅余 2 项 Minor（00 测试代码 from_bytes 误用 + 06 陈旧注释），均为实装期可修正项，不影响计划批准。

**可进入阶段二（L0 批次：00/01/05/09 四路并行）**。
