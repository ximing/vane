# M1 计划集可实现性 / TDD / M0 兼容性审查

> 审查者：feasibility reviewer
> 审查日期：2026-08-09
> 审查对象：`docs/plans/m1/README.md` + `modules/01..12-*.md`（12 份）+ `plan-split-report.md`
> 核对基线：`docs/SPEC.md` v1.0、`docs/REQUIREMENTS.md` v1.1、`docs/plans/m0/README.md`、M0 git HEAD 实际代码（`crates/vane-core/src/` 全部 + vane-node + vane-ffi 逐文件 Read）
> 红线：只读审查，未改任何文件。

---

## 0. Verdict

**CHANGES_REQUESTED**

存在 2 项阻塞项（B1/B2）与 2 项重大问题（M1/M2），在补齐前不应开工。其余 8 份计划（03/04/07/08/09/10/11/12 主体）可实现性可接受，仅需 minor 修正。R-1/R-2/R-5 裁决合理无异议；R-3 存在 README 与计划自相矛盾（M3）；R-4/R-6 倾向串行方案合理。

---

## 1. 阻塞项（按严重度）

### B1【M0 API 错配 / SPEC 偏离】原始文本未持久化 → 06 reindex 不可实现，02 merge "重新分词"不可行

**证据（逐文件核对 M0 实际代码）**：

- `docs/SPEC.md` 第 205 行：`stored.bin // 原文/JSON meta（zstd 块压缩）`——SPEC 明确要求 stored.bin 含"原文"。
- `crates/vane-core/src/api/collection.rs` flush（第 196-212 行）：`stored_json` 仅由 `doc.meta` 序列化而来，**`doc.text` 原文从未写入 stored.bin**，仅被 `tokenizer.tokenize()` 后喂给 `inv_builder` 即丢弃。
- `crates/vane-core/src/segment/mod.rs` SegmentReader（第 183-308 行）：仅暴露 `vectors()/stored_json(local_docid)/external_id(docid)/meta()`，**无任何方法返回原始 text**。
- `crates/vane-core/src/bm25.rs` InvertedIndexReader：posting 存的是 tokenized 后的 term + docid + tf，**无法还原原文**。

**对 06 的影响（阻塞）**：06 计划 Task 3 "reindex 重建倒排（新分词身份）"明确写 "对每段：MergeTask 用新 tokenizer 重新分词 → InvertedIndexBuilder 重建倒排"。但原文已丢弃，无法重新分词。SPEC §7.4 reindex 的本质就是换分词器重建倒排，缺原文 = 不可实现。

**对 02 的影响（可修复）**：02 Task 3 MergeTask 同样写 "InvertedIndexBuilder::add_document（重新分词——复用 collection tokenizer）"。但 merge 时分词器未变，**正确做法是 posting remap**：从源段 InvertedIndexReader 读出每个 term 的 postings，按新 docid（docid 重映射表）重写 posting.docid，重组 InvertedData 后 write_inverted。无需原文。计划当前描述错误，需改为 posting remap 路径。

**修复建议**：
1. 06 reindex 必须先持久化原文。两种路线：
   - (a) M1 在 SegmentWriter::add_doc 增加 text 写入（新增 `text.bin` 或扩展 stored.bin 含 text 字段）——但这改 M0 段格式，需评估 corpus 兼容测试（`tests/corpus_compat.rs`）是否断链；
   - (b) 06 reindex 改为"仅当段内 text 可恢复时支持"——但 M0 不可恢复，等于 reindex 推 M2。
   编排者需裁决：是否在 M1 增加原文持久化（影响 stored.bin 格式 / 新增 text.bin），或将 reindex 降级为"M2 交付"。
2. 02 MergeTask 改为 posting remap 设计（不改段格式，不依赖原文），重写 Task 3/4 的"最小实现"描述。

**严重度**：阻塞 06；02 可修复但需重设计。这是全计划集最大的可实现性缺口。

### B2【契约不一致】MergeTask::new 签名缺 tokenizer 参数

**证据**：
- README §02 契约（第 169-174 行）：`MergeTask::new(sources: Vec<String>, target_docid_base: u64, tokenizer_id: TokenizerId, schema: Schema) -> Self`——**无 tokenizer 实例**。
- 02 Task 3 最小实现（第 178 行）："MergeTask 持 `Box<dyn Tokenizer>`，由调用方传入"。
- 06 §Consumes from 02（第 81 行）："02 的 MergeTask 持 tokenizer，reindex 传入新 tokenizer。02 计划已注明 'MergeTask 持 Box<dyn Tokenizer>'。"

**问题**：README 是"类型签名单一事实源"，但其 MergeTask::new 不含 tokenizer 参数；02/06 计划又假设它持 tokenizer。reindex（06）必须传新 tokenizer 实例才能重新分词，但契约签名无法传入。若 06 改为 posting remap（不重新分词），则 merge/reindex 都不需要 tokenizer 实例——但 reindex 本质需要重新分词（B1），绕不开。

**修复**：README §02 契约的 `MergeTask::new` 签名需扩展为含 `tokenizer: Box<dyn Tokenizer>`（或 `Arc<dyn Tokenizer>`）。这是 M1 新增类型，非 M0 冻结破坏，但契约与计划必须对齐。

---

## 2. 重大问题

### M1【TDD 真实性】12 Task 2 测试体含 `unimplemented!()`

**证据**：`modules/12-recall-regression.md` Task 2（第 89-101 行）测试函数 `baseline_top10` 函数体为：
```rust
unimplemented!()
```
随后用 prose "裁决：基线计算需访问段内部数据……在 api 增 `pub fn search_brute_baseline`" 收尾。

plan-split-report §6 称 "grep TBD/TODO/适当处理 → 0 命中"。但 `unimplemented!()` 在语义上就是 placeholder——这个测试根本无法"失败→通过"，它直接 panic。Task 2 不是真实 TDD 任务，是"先占位再裁决"。

**修复**：将 Task 2 重写为：先定义 `search_brute_baseline` 签名 + 写真实调用断言（`assert_eq!(baseline.len(), 10)` 等），再实现。`unimplemented!()` 必须移除。

### M2【README 与计划自相矛盾 / R-3】TokenizerId 词典版本注入方式描述不一致

**证据**：
- README §05（第 334 行）："Consumes from M0：`compute_tokenizer_id`（需扩展 `builtin_dict_version(Jieba)` 填入词典日历版本 + sha256_prefix——这是 id.rs 内部函数扩展，不改 `compute_tokenizer_id` 公开签名）"。
- `modules/05-jieba-lite.md`（第 66-70 行）+ plan-split-report R-3（第 92-94 行）："采用方案 A。`id.rs::builtin_dict_version(Jieba)` 保持返回 `b""`（M0 不变），词典版本经 JiebaTokenizer 内部叠加。"
- M0 实际 `id.rs` 第 24 行：`fn builtin_dict_version(kind: BuiltinTokenizer) -> &'static [u8]`——返回 `&'static [u8]`，要"填入词典日历版本"（动态 String "2026.08"）必须改签名为 `-> Cow<[u8]>` 或 `-> String`，这并非"不改公开签名"能解决的内部扩展。

**结论**：05 计划的方案 A（二次哈希）正确且可实现；README §05 的描述（"扩展 builtin_dict_version 填入版本"）是陈旧/错误文本，与方案 A 矛盾，且误导实现者去改一个返回 `&'static` 的函数。需统一 README 描述为方案 A。

**R-3 裁决建议**：批准方案 A（已与 05 计划一致）；README §05 文本需修正。

---

## 3. 各计划可实现性评级

### 01-hnsw —— 中（风险：recall 达标 + 自研图正确性）

- **M0 兼容性 ✅**：Consumes 的 `Metric/ScoredDoc/Vfs/brute_search/SegmentReader/SegmentWriter` 签名逐项核对一致（types.rs / vfs/mod.rs / vector/mod.rs / segment/mod.rs）。
- **可实现性**：自研 HNSW ~800 行（分层插入 + ef 搜索 + filter + 序列化）工作量合理。距离转换描述正确（cosine 距离=1-cos、L2 距离=|a-b|、dot 距离=-dot，与 M0 brute_search "越大越相似"语义一致）。
- **TDD ⚠️**：Task 6（I-3 不变性）测试 tautological——读同一 `hnsw.bin` 两次比对字节相等，且不调用 delete。真正 I-3 测试在 02 Task 7（调 delete 后比 size）。建议删除 01 Task 6 或改为有意义的断言。
- **TDD ⚠️**：Task 5 api 接入测试只断言 `hnsw_hits.len()==10`，不断言 recall（注释推给 12）。可接受但偏弱。
- **风险**：M=16/ef_construction=200 在 500 文档小规模 recall≥0.95 应可达；10 万规模五档在 12 验证。std::thread::scope wasm32 降级（R-6 倾向串行）合理。

### 02-tombstone-merge —— 中低（B1/B2 阻塞 + posting remap 需重设计）

- **M0 兼容性 ✅**：`SegmentMeta.tombstones: RoaringBitmap`（segment/mod.rs 第 18 行）确实存在且 header.bin 已序列化（header.rs 第 23-27 行）；`ManifestStore::add_segment/save_atomic/load`（persistence/mod.rs）签名一致；`encode_header/decode_header`（header.rs）含 tombstone 序列化。
- **M0 兼容性 ⚠️**：02/06 测试使用 `col.segment_count()`、`col.segment_ulids()`、`col.snapshot_readers()`、`col.set_state_for_test()`——M0 Collection 无这些方法（collection.rs 第 127-454 行仅 add/flush/search/delete/compact/reindex）。属新增 pub 方法，非冻结破坏，但未列入 README 契约。
- **可实现性 ❌（B1）**：Task 3 "重新分词"不可行（无原文）。需改为 posting remap。
- **TDD ✅**：Task 1（delete 隐藏文档）、Task 3（merge 物理 清除 tombstone）、Task 7（I-3 delete 不改 hnsw.bin）测试真实，断言有意义。
- **依赖 ⚠️**：Task 2（tombstone reopen）依赖 04-wal，但 04 Task 5 又依赖 02 的 delete/compact——循环依赖。02 已处理（Task 2 标 blockedBy 04，先跳过 reopen），可接受。

### 03-pre-filter —— 高

- **M0 兼容性 ✅**：`Filter/FilterCond/ScalarValue`（api/types.rs 第 58-77 行）、`ScalarKind/FieldDef::Scalar`（types.rs 第 154-168 行）、`brute_search(filter)` / `InvertedIndexReader::search(filter)` 均 已支持 filter 参数（bm25.rs 第 515-520 行、vector/mod.rs 第 101-108 行）——plan-split-report §3.1 此项核查无误。
- **M0 兼容性 ✅**：M0 finalize 已写 scalars.col 空 stub（segment/mod.rs 第 149-158 行，magic+version+0u32），03 扩展为真实数据不改头格式。
- **可实现性 ✅**：compile_filter（eq/in/gte/lte + AND）+ should_fallback_brute 逻辑直接，~200 行。
- **TDD ✅**：Task 1（scalars.col roundtrip）、Task 2（filter 编译）、Task 3（回退判定）、Task 4（api 接入）测试真实。Task 4 第二个测试 `assert!(hits.iter().all(|h| /* year>=2030 */))` 注释占位需补真实断言（minor）。

### 04-wal —— 高

- **M0 兼容性 ✅**：`Vfs::append/read_at/sync/delete/list`（vfs/mod.rs 第 5-14 行）签名一致；`Manifest/ManifestStore`（persistence/mod.rs）一致。
- **可实现性 ✅**：薄 WAL（JSON 行 + append/sync/truncate）简单。
- **TDD ✅**：Task 1-4 测试真实。
- **设计 ⚠️**：`recover(vfs, db_path, &manifest)` 是 free function，但需"注入对应 CollectionInner.tombstones"——CollectionInner 是 `pub(crate)`，recover 作为 `vane_core::wal` 模块函数无法直接写 CollectionInner.tombstones。需 recover 返回 `HashMap<(collection, ulid), RoaringBitmap>` 供 Db::open 在 restore 后注入，或把 recover 逻辑移入 api 层。计划描述模糊（minor-design）。

### 05-jieba-lite —— 中低（DAT+HMM 正确性风险高）

- **M0 兼容性 ✅**：`Tokenizer` trait / `Token` / `UserDictEntry` / `compute_tokenizer_id` / `build_tokenizer`（tokenizer/mod.rs + id.rs）签名一致；M0 `build_tokenizer(Jieba)` 确实返回 `DictUnavailable`（mod.rs 第 64 行）。
- **M0 兼容性 ⚠️**：计划称"复用 M0 `cjk_bigram.rs` 的 `is_cjk` + run 切分逻辑"——但 `is_cjk` 在 cjk_bigram.rs 第 97 行是**私有 `fn`（无 pub）**，jieba 子模块无法访问。需改为 `pub(crate) fn is_cjk` 或在 jieba 内重实现。计划未标注此可见性变更。
- **可实现性 ⚠️**：DAT 双数组（~150 行）+ DAG 最大概率 + HMM Viterbi + 中英混排，工作量 ~800-1000 行。"算法与 jieba-rs 完全一致"是高门槛——HMM 转移矩阵 4x4 + 发射概率须从 jieba 原版提取且 bit-identical，否则 200 句 100% 一致会失败。Task 4 HMM 测试自身只断言 `!toks.is_empty()`（弱），真正验收推给 CI 200 句 job。
- **依赖 ✅**：ruzstd（feature jieba，optional）不在 deny.toml 黑名单；纯 Rust，wasm32 安全；core 默认不启用 → wasm32 check 不带 `--features jieba`，不污染体积。jieba-rs 作 dev-dependency 在 crates.io，非黑名单，可行。
- **TDD ✅**：Task 1-3/5-8 测试真实（DAT 查询、DAG 切分、混排 position、用户词优先级）。Task 4 HMM 测试弱（见上）。
- **R-3**：见 M2，方案 A 正确，README 文本需修正。

### 06-userdict-reindex —— 低（B1 阻塞 + 测试前提弱）

- **M0 兼容性 ✅**：`Collection::reindex() -> Result<()>`（collection.rs 第 451 行）确实是 M0 占位；M0 README 第 454 行标注 "ReindexHandle 留 M1"。R-2 签名变更（`Result<()>` → `Result<ReindexHandle>`）回归 SPEC §4.1 第 87 行冻结 IDL，合理。
- **可实现性 ❌（B1）**：reindex 需重新分词，原文未持久化 → 不可实现。
- **TDD ⚠️**：Task 3 用 `setup_col_with_docs` + standard tokenizer 测 reindex。但 M0 `StandardTokenizer::new(user_dict)`（standard.rs 第 16-22 行）明确**不消费 user_dict 做切分**（仅影响 TokenizerId）。即 standard + userDict 变更只改 id 不改切分，reindex 后 tokenization 不变，测试 `hits_after.len()>=1` 虽过但无意义。reindex 真正有意义的场景是 jieba + userDict，但 jieba feature 默认关。计划第 260 行已注明"若 jieba 未启用，reindex 仍可用于 standard/cjk_bigram 的 userDict 变更"——但 standard 不消费 user_dict，此说法不成立。cjk_bigram 是否消费 user_dict 需核实（未在本次审查范围内，但 cjk_bigram.rs 第 18 行 `new(user_dict)` 同样可能仅用于 id）。
- **TDD ✅（其余）**：Task 1（DictState 状态机）、Task 4（Rebuilding E_BUSY）、Task 5（原子切换）、Task 6（I-4）测试真实。
- **修复**：B1 解决后，Task 3 应改为 jieba feature 启用场景或明确 reindex 对 standard 的"仅 id 变更"语义。

### 07-dict-distribution-node —— 高

- **M0 兼容性 ✅**：消费 05 的 `JiebaDict::load` / `build_jieba_tokenizer`（M1 新增）。
- **设计 ⚠️**：Task 2 裁决"DbInner 增 `jieba_dict: Option<Arc<JiebaDict>>`"——DbInner 是 `pub(crate)`（db.rs 第 17 行），扩展字段非 M0 冻结破坏，合理。但 `build_tokenizer(Jieba)` 仍返回 DictUnavailable，需 CollectionInner 构造时改走 `build_jieba_tokenizer`——这要求 create_new/restore_from_manifest 能访问 DbInner.jieba_dict。当前 `create_new(db: &DbInner, ...)` 已有 db 引用，可行。
- **TDD ✅**：Task 1（crate 骨架）、Task 3（降级）、Task 4（体积门禁）、Task 5（冷加载 bench）真实。
- **fixture ⚠️**：Task 4 完整词典生成依赖 jieba 开源词表（~350k 词）剪枝 + DAT 构建 + zstd，离线脚本可行但工作量在 05 DAT 代码复用范围内。

### 08-dict-distribution-go —— 高

- **M0 兼容性 ✅**：消费 05 dict.bin 格式 + 09 C ABI。
- **可实现性 ✅**：go:embed + build tag + DictVersion 常规。
- **TDD ✅**：Task 1-5 测试真实。
- **依赖**：09 可后移，08 的 C ABI 对接部分顺延，但 embed + 体积门禁可先行——合理。

### 09-go-cgo-binding —— 中（cbindgen + zig cc 可行，FFI 注册表常规）

- **M0 兼容性 ✅**：Consumes 的 `Db/Collection` 全部 pub API（db.rs / collection.rs）签名一致；`StdFsVfs::new()`（vfs/std_fs.rs 第 19 行）存在。
- **可实现性 ✅**：句柄注册表 `RwLock<HashMap>` + AtomicU64 + thread-local last_error 是标准 FFI 模式，~600 行。cbindgen 生成 vane.h 常规。
- **TDD ✅**：Task 1-4 Rust 侧 FFI 测试真实（extern "C" + unsafe 调用）。Task 5-6 Go 侧测试真实。
- **CI ⚠️**：10 计划的 zig cc 矩阵（第 156-162 行）target 格式 `x86_64-unknown-linux-gnu` 需转 zig triple `x86_64-linux-gnu`；windows-msvc 行 `cc: cl` 但脚本统一写 `CC_target="zig cc..."`——不一致，需 CI 脚本分支处理。非阻塞。

### 10-ci-m1 —— 高

- **可实现性 ✅**：6 个 CI job 均为标准 GitHub Actions 模式。
- **门禁可达性**：wasm ≤800KB（M0 无 jieba，应远低于）；dict ≤1.5MB（20 万词 zstd 可达，但需实测）；Go embed <2MB（同源）；recall≥0.95（依赖 01/12）；冷启动 <1s（依赖 11，可能走降级）。
- **TDD ✅**：每个 job 有对应脚本/命令验证。

### 11-cold-start-bench —— 中

- **M0 兼容性 ✅**：`Db::open` / `SegmentReader::open` / `StdFsVfs` 签名一致。
- **可实现性 ⚠️**：fixture 生成 100 批 × 1000 文档 = 100 次 flush。M0 flush 无 auto-merge（02 才有），若 11 在 02 前跑会产 100 段（超 SEGMENT_MAX=10）。11 前置依赖列了 01+02，OK，但 fixture 脚本注释"100 段 → 触发 auto-merge 到 ≤10 段"依赖 02 实装。
- **性能风险**：M0 SegmentReader::open 一次性全加载 vectors/inverted/stored/idmap 到内存（无懒加载，segment/mod.rs 第 211-262 行）。10 万×384 维 vectors ≈ 154MB，open 全加载很可能 >1s。计划已预判降级路径（metadata <1s + 首次查询 <3s），合理。Task 3 分级断言逻辑 `if open_ms > 1000 { assert!(open_ms < 1000 || query_ms < 3000) }` 有逻辑瑕疵——`if open_ms>1000` 内又断言 `open_ms<1000` 恒 false，实际只靠 `query_ms<3000`。应改为 `assert!(query_ms < 3000)` 或重构条件。Minor。
- **TDD ✅**：Task 1-3 真实（fixture 脚本 + criterion bench + 分级断言）。

### 12-recall-regression —— 中（M1 阻塞）

- **M0 兼容性 ✅**：`brute_search` / `InvertedIndexReader::search` / `rrf_fuse` / `Collection::search` 签名一致。
- **TDD ❌（M1）**：Task 2 含 `unimplemented!()`，非真实测试。
- **设计 ⚠️**：`search_brute_baseline` 需访问 CollectionInner 段快照（pub(crate)），计划裁决新增 `pub fn search_brute_baseline` + `#[doc(hidden)]`——可行，但是新增 pub API（非 IDL），需列入契约。
- **fixture ✅**：1000 文档 × 128 维 + cat 字段（i%1000 → 1000 distinct cat），五档选择率（0.1%=1 cat、99%=990 cat）可确定性构造。`pick_cats_for_tier` 可实现。
- **TDD ✅（其余）**：Task 3（五档 recall）、Task 4（0.1% 暴力回退 recall=1.0）断言有意义。

---

## 4. 依赖与门禁可行性

| 项 | 结论 | 证据 |
|---|---|---|
| ruzstd（feature jieba） | ✅ 可用 | deny.toml `[bans] deny = [...]` 不含 ruzstd；纯 Rust，wasm32 安全；core 默认不启用，wasm32 check 不带 `--features jieba` |
| jieba-rs（dev-dep） | ✅ 可用 | crates.io 注册（sources.allow-registry 含 crates.io-index）；非黑名单；dev-dependency 不进 core 运行时 |
| cbindgen | ✅ 可用 | build-dependency 常规 |
| zig cc 交叉 | ✅ 可行 | 6 平台矩阵标准模式；target triple 转换 + windows msvc 分支需脚本修正（minor） |
| wasm ≤800KB | ✅ 可达 | M0 无 jieba，core 依赖极简（roaring/sha2/serde/serde_json/unicode-segmentation/rust-stemmers/ulid），release+wasm-opt 应远 <800KB |
| dict ≤1.5MB | ⚠️ 需实测 | 20 万词 DAT + HMM zstd 压缩，理论可达但依赖词表剪枝策略；07 Task 4 门禁会卡 |
| Go embed <2MB | ✅ 可达 | 同源 dict，gzip <1.5MB 满足 <2MB |
| recall≥0.95 | ⚠️ 依赖 01 HNSW 正确性 + 12 search_brute_baseline 实装 | 五档中 0.1% 走暴力回退 recall=1.0；高选择率（99%）HNSW 近似暴力应 ≥0.95；中档（1%/10%）是风险点 |

---

## 5. 测试 fixture 可行性

| fixture | 计划 | 可行性 |
|---|---|---|
| 200 句 jieba 对照 | 05/10 | ✅ 可离线生成（jieba-rs 原版切分结果固化入 `tests/fixtures/jieba_200.txt`）；风险在 HMM 参数 bit-identical |
| 500 篇维基 nDCG | 05/10 | ⚠️ "离线生成"较乐观——需爬取/采样 500 篇中文维基 + 50 查询 + 标注或 jieba-rs 基线 nDCG；工作量大，建议降为"可选非门禁"或缩小规模 |
| 20 生造词 | 05 | ✅ 可手工构造 |
| 五档选择率回归 | 12 | ✅ 确定性 1000 文档可生成 |
| 10 万文档冷启动 | 11 | ✅ 脚本生成可行（154MB vectors），但 CI 缓存或运行时生成需时间；依赖 02 auto-merge |

---

## 6. R-item 裁决复核

| R-item | 编排者裁决 | 审查意见 |
|---|---|---|
| R-1 export→M2 | 批准 | ✅ 同意。SPEC §15 明确 export 列 M2；M1 不覆盖 |
| R-2 reindex→ReindexHandle | 批准 | ✅ 同意。M0 README 标注 "ReindexHandle 留 M1"；SPEC §4.1 第 87 行冻结 IDL 为 `Result<ReindexHandle>`；非 M0 冻结签名破坏 |
| R-3 TokenizerId 方案A | 待确认 | ⚠️ 方案 A 正确（05 计划已采用），但 README §05 文本仍描述"扩展 builtin_dict_version"，与方案 A 矛盾（M2）。需统一 README。`builtin_dict_version` 返回 `&'static [u8]`，方案 A 绕开此限制，正确 |
| R-4 不引 rayon 用 thread::scope | 待确认 | ✅ 同意。deny.toml 未黑名单 rayon，但 M1 用 std::thread::scope 避免依赖；SPEC §11 "rayon" 是实现路线描述非硬约束 |
| R-5 stored zstd→M2 | 批准 | ✅ 同意。core 加 zstd 编码器会撑爆 800KB 红线；ruzstd 仅解码不够（压缩需编码器） |
| R-6 HNSW 并行 cfg | 待确认 | ✅ 同意倾向串行。M1 先串行搜索（10万×384 HNSW <50ms 可达），避免 `cfg(target_arch="wasm32")` 污染 I-5；01 Task 5 验收时若性能达标则保持串行。若必须并行，cfg 仅限 Executor 抽象处 |

---

## 7. 需编排者裁决的疑点（新增）

1. **B1 原文持久化**：reindex（06）需要原文重新分词，但 M0 stored.bin 不含 text。裁决：(a) M1 增加原文持久化（改 stored.bin 格式 / 新增 text.bin，评估 corpus 兼容）；或 (b) reindex 降级 M2。此裁决影响 06 是否进 M1。
2. **B2 MergeTask 签名**：README §02 契约 `MergeTask::new` 是否扩展含 `tokenizer` 参数？需与 02/06 计划对齐。
3. **02 MergeTask 改 posting remap**：02 Task 3/4 的"重新分词"改为"posting remap"（不依赖原文），编排者需确认此设计变更。
4. **M2 README §05 文本修正**：统一为方案 A（JiebaTokenizer 内部二次哈希，`builtin_dict_version(Jieba)` 保持 `b""`）。

---

## 8. Minor 清单（不阻塞，实现时修正）

- m1：05 计划 `is_cjk` 复用——cjk_bigram.rs 第 97 行 `fn is_cjk` 私有，需改 `pub(crate)` 或 jieba 内重实现。
- m2：01 Task 6 I-3 测试 tautological（读同文件两次比字节，不调 delete）——删除或改为有意义断言（02 Task 7 已覆盖真 I-3）。
- m3：02/06 测试用 `segment_count()/segment_ulids()/snapshot_readers()/set_state_for_test()` 等 M0 不存在方法——新增 pub 方法，需列入 README 契约。
- m4：04 `recover` free function 无法直接写 CollectionInner.tombstones——改为返回 tombstone map 供 Db::open 注入，或移入 api 层。
- m5：06 Task 3 用 standard tokenizer 测 reindex——standard 不消费 user_dict（standard.rs 第 17 行注释明确），测试前提弱；改用 jieba feature 场景或明确"仅 id 变更"语义。
- m6：10 CI zig cc target triple 格式（`x86_64-unknown-linux-gnu` → `x86_64-linux-gnu`）+ windows-msvc `cc: cl` 分支未在脚本处理。
- m7：11 Task 3 分级断言逻辑 `if open_ms>1000 { assert!(open_ms<1000 || query_ms<3000) }`——`open_ms<1000` 恒 false，应改为 `assert!(query_ms < 3000)`。
- m8：12 Task 2 `unimplemented!()`（M1）——重写为真实测试。
- m9：03 Task 4 第二测试 `assert!(hits.iter().all(|h| /* year>=2030 */))` 注释占位——补真实断言。
- m10：02 Task 2 ↔ 04 Task 5 循环依赖——已部分处理（02 Task 2 标 blockedBy 04），编排者确认排期。

---

## 9. 结论

M1 计划集在 M0 pub API 兼容性上**总体核查无误**（SegmentReader/SegmentWriter/brute_search/InvertedIndexReader::search(filter)/ManifestStore/CollectionMeta 签名均与 git HEAD 一致），plan-split-report §3 的对接核查基本可信。但存在 2 项阻塞：

1. **B1 原文未持久化**——这是计划集未发现的 M0 实现与 SPEC §6.2 偏离（SPEC 要求 stored.bin 含原文，M0 只存 meta），直接阻塞 06 reindex，并使 02 merge 的"重新分词"路径不可行（需改 posting remap）。
2. **B2 MergeTask 签名不一致**——README 契约与 02/06 计划描述脱节。

加上 12 Task 2 的 `unimplemented!()` placeholder（M1）和 README §05 的 R-3 文本矛盾（M2），需编排者裁决 4 项疑点后方可开工。其余计划可实现性中到高，TDD 任务多数真实有效。

建议：编排者先裁决 B1（原文持久化 vs reindex 降级 M2）+ B2（MergeTask 签名），再由 plan-splitter 修正 02/06 的 MergeTask 设计与 12 Task 2，然后进入 L0 批次。
