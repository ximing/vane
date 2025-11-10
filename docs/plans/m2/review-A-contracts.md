# Reviewer A 评审报告 — SPEC 契约 + M0/M1 接口对接

> 评审范围：`docs/plans/m2/README.md` + `modules/M2-01~14-*.md`（14 份）。
> 视角：接口契约对接（vs 实际代码）、冻结签名破坏、M2-07/08 dim 协同、跨计划契约一致性、SPEC v1.2 一致性、不变量覆盖。
> 方法：只读 grep/Read 核查 `crates/vane-core/src/`、`crates/vane-ffi/src/lib.rs`、`crates/vane-node/src/db.rs`，对照 M1 README Global Interface Contracts。
> 分级：**阻塞（B）** / **重要（I）** / **次要（M）**。

---

## 状态：PASS_WITH_FINDINGS

- 阻塞（B）：0
- 重要（I）：5
- 次要（M）：4

---

## 重要发现（I）

### I-1. M2-01 `Db::open` 签名漏 `vfs` 参数；`Collection::{export,close}` 方法不存在

**计划**：`modules/M2-01-wasm-cdylib-size.md` §3 Consumes from。
**证据**：
- 计划写：`Db::open(path: &str, opts: OpenOptions) -> Result<Db>`、`Collection::{add,flush,search,delete,compact,reindex,export,close}`。
- 实际代码 `crates/vane-core/src/api/db.rs:35`：`pub fn open(vfs: Arc<dyn Vfs>, path: &str, opts: OpenOptions) -> Result<Self>` —— 首参 `vfs: Arc<dyn Vfs>` 被漏掉。
- `export`/`close` 在 `Db` 上（`api/db.rs:164`、`api/db.rs:168`），`Collection` 上无此二方法（`api/collection.rs` grep 全文仅 add/flush/search/delete/compact/reindex/set_user_dict/dict_state/segment_count 等，无 export/close）。

**影响**：vane-wasm 胶水层若按计划签名实现，会漏传 Vfs（Db 无法构造）；且误以为 Collection 有 export/close 会导致绑定层调错对象。
**建议**：订正为 `Db::open(vfs: Arc<dyn Vfs>, path: &str, opts: OpenOptions)`；export/close 归到 Db 而非 Collection。

### I-2. M2-08 引用不存在的文件 `segment/scalars.rs`

**计划**：`modules/M2-08-stored-zstd.md` §2 涉及文件。
**证据**：
- 计划写：`Modify crates/vane-core/src/segment/scalars.rs（ScalarReader::decode_scalars）：FORMAT_VERSION → SCALARS_FORMAT_V1`。
- 实际 `crates/vane-core/src/segment/` 仅含 `header.rs / mod.rs / tests.rs / ulid.rs`（`ls` 确认），无 `scalars.rs`。`ScalarReader`/`decode_scalars` 定义在 `segment/mod.rs:583`/`mod.rs:652`。
- 同计划 §2 另有 `crates/vane-core/src/hnsw/mod.rs:533` 引用 FORMAT_VERSION（实际 hnsw 的 FORMAT_VERSION 校验在 `hnsw/mod.rs` ~533 区间，大致正确）。

**影响**：实现者按计划找不到 `scalars.rs`，可能误创建新文件而非改 mod.rs，破坏模块结构。
**建议**：订正为 `segment/mod.rs`（decode_scalars 在 mod.rs:652）。

### I-3. M2-09 `brute_search_sq8` 签名缺 `metric` 与 `docid_base`

**计划**：`modules/M2-09-sq8.md` §3 Produces for + README M2-09 Global Interface Contracts 节。
**证据**：
- 计划/README 写：`pub fn brute_search_sq8(sq8: &[u8], dim: u32, query: &[f32], topk: usize, filter: Option<&RoaringBitmap>) -> Vec<ScoredDoc>`。
- 实际 `crates/vane-core/src/vector/mod.rs:101` `brute_search` 签名：`(vectors: &[f32], dim: u32, query: &[f32], metric: Metric, topk: usize, filter: Option<&roaring::RoaringBitmap>, docid_base: u64) -> Vec<ScoredDoc>`。
- M2-09 提议签名缺 `metric: Metric`（SQ8 距离依赖度量类型，cosine/L2/dot 量化方式不同）与 `docid_base: u64`（结果 docid 需映射回绝对空间，与 HnswReader::search/brute_search 一致）。
- M2-09 §2 调用点 `api/collection.rs:765,776`（实际 brute_search 调用在 ~769/782）传入 metric+base，新签名无法对接。

**影响**：跨计划契约（README M2-09）与实际 brute_search 契约不对齐；实现时要么签名缺参无法工作，要么需临时改签名 → 与 README 分歧。
**建议**：补 `metric: Metric` 与 `docid_base: u64`，与 brute_search 对齐；同步更新 README M2-09 节。

### I-4. M2-11 `Db::open(path, opts)` 签名漏 `vfs` 参数

**计划**：`modules/M2-11-go-cgo-binding.md` §3 Consumes from。
**证据**：
- 计划写：`Db::open(path, opts)`。
- 实际 `api/db.rs:35`：`Db::open(vfs: Arc<dyn Vfs>, path: &str, opts: OpenOptions)`。
- vane-ffi C ABI `vane_open` 需在内部构造 Vfs（StdFsVfs）再传 Db::open，漏 vfs 会导致实现者忽略 Vfs 构造。

**影响**：与 I-1 同类；FFI 实装需 Vfs 参数。
**建议**：订正为 `Db::open(vfs: Arc<dyn Vfs>, path, opts)`。

### I-5. README 约束「wasm32 永不启用 jieba feature」与 M2-04/M2-14 浏览器 jieba 设计冲突

**计划/约束**：
- README §全局约束表：`M2：vane-wasm default features 不启 jieba/dict-zh | wasm32 构建永不启用 jieba feature | §4.1/红线`（沿自 M1 README §05「wasm32 构建永不启用 jieba feature（词典永不进 wasm，红线）」）。
- README §M2 实现：`WASM 词典：CDN URL fetch → sha256 校验 → OPFS 缓存；dictData 内联注入；fetch 失败降级 bigram`。
- M2-04 §3 Consumes from：`M1 vane_core::tokenizer::jieba::{JiebaDict, JiebaTokenizer}（tokenizer/jieba/dict.rs:46 JiebaDict::load、tokenizer/jieba/mod.rs:41 JiebaTokenizer::new）—— 词典加载后注入 collection（feature-gated，wasm 默认不启 jieba feature；但词典数据可运行时注入）`。
- M2-14 §4 测试 3/7/8：浏览器内 jieba 中文搜索 + CDN fetch 词典。

**证据**：`jieba` feature 在 `crates/vane-core/Cargo.toml:26` 定义为 `jieba = ["ruzstd"]`，门控 `tokenizer/jieba/` 整个模块（`JiebaTokenizer`/`JiebaDict`）。若 wasm32 构建永不启用 jieba feature，则 `JiebaTokenizer` 代码不编译进 wasm，M2-04 即使运行时 fetch 到 dict.bin 也无法 `JiebaDict::load` + `JiebaTokenizer::new`（类型不存在）。M2-04「wasm 默认不启 jieba feature；但词典数据可运行时注入」的表述误解了 Rust feature 语义——feature 门控的是编译期代码，非运行时数据。

**影响**：M2-04/M2-14 的浏览器 jieba 检索核心功能无法在「永不启用 jieba feature」约束下实现。M1 约束的初衷是「词典数据不进 wasm」（由 dict-zh feature 控制 vane-dict-zh 数据包），但措辞门控了 jieba 算法代码本身。
**建议**：编排者裁决——将约束放宽为「wasm32 永不启用 **dict-zh** feature（词典数据不编译进 wasm）；jieba feature 可在 vane-wasm 非 default 构建启用（算法代码进 wasm，词典运行时 fetch/内联）」。需在 README 约束表显式修订并注明 M2 放宽 M1 约束的理由。

---

## 次要发现（M）

### M-1. M2-10 / README `compile_filter` 行号 `filter/mod.rs:43` 实为 32

**计划**：M2-10 §3 Consumes from + README M2-10 Global Interface Contracts 节。
**证据**：`crates/vane-core/src/filter/mod.rs:32` 定义 `pub fn compile_filter(`（非 43）。签名本身与 M1 README §03 一致，仅行号漂移。不影响契约。

### M-2. M2-08 `header.rs:21,40` 行号 40 不准

**计划**：M2-08 §2。
**证据**：`segment/header.rs:21` 用 `FORMAT_VERSION.to_le_bytes()`（encode，正确）；decode 的 `version != FORMAT_VERSION` 在 `header.rs:46`（非 40，40 行是 `if buf.len() < 8`）。仅行号漂移。

### M-3. M2-09 调用点行号 `api/collection.rs:765,776` 实为 ~769/782

**计划**：M2-09 §2。
**证据**：`api/collection.rs` 中 brute_search 调用在 ~769（HNSW fallback 分支）与 ~782（force_brute 分支）。行号小漂移，不影响实现。

### M-4. M2-10 §2 称 `mod.rs:740-790` 实指 `api/collection.rs:740-790`

**计划**：M2-10 §2 涉及文件。
**证据**：search 路径在 `api/collection.rs`（非 segment/mod.rs）。文件名笔误，不影响契约。

---

## 重点核查项结论

### 1. 接口契约对接（M0/M1 pub API）
- `SegmentReader::open/vectors/dim/stored_json/text`（`segment/mod.rs:344/417/420/442/450`）：M2-07 引用签名与实际一致，OnceLock 改造保持 `&self` 签名不变。✓
- `HnswReader::search`（`hnsw/mod.rs:624`）：M2-09/M2-10 引用行号与签名一致（`(query, topk, ef_search, filter, docid_base, vectors)`）。✓
- `compile_filter`（`filter/mod.rs:32`）：M2-10 引用签名一致（行号漂移见 M-1）。✓
- `JiebaDict::load`（`dict.rs:46`）/`JiebaTokenizer::new`（`mod.rs:41`）：M2-04/M2-11/M2-13 引用一致。✓
- `ReindexHandle::{progress,wait}`（`api/reindex.rs:64,69`）：M2-11 引用一致。✓
- `Db::export`（`api/db.rs:164`）：M2-12 引用一致（占位 `Err(VaneError::Unsupported)`）。✓
- `vane-ffi` stub（`lib.rs:1`）：M2-11 引用一致。✓
- `vane-node ExportTask`（`db.rs:110`）：M2-12 引用一致。✓
- **问题**：见 I-1/I-4（Db::open 漏 vfs）、I-3（brute_search_sq8 缺参）。

### 2. 冻结签名破坏（两个 ⚠️ 修订点）
- **M2-05 SIMD128**：首选方案明确「core 不引入手写 SIMD intrinsics，依赖 LLVM 自动向量化」，core 零新增 `cfg(target_feature)`。✓ 未违反 I-5。计划已明确标注若评估发现必须手写 intrinsics 则停下标「⚠️ 需 SPEC 修订」，首选方案存疑时已有兜底。✓
- **M2-09 SQ8**：首选方案明确「HNSW 导航仍用 `vectors()` f32，SQ8 仅暴力回退路径」，`HnswReader::search` 签名不变（`hnsw/mod.rs:624` 不改）。✓ 未破坏 M1 冻结签名。计划已标注若 HNSW 必须用 SQ8 则停下标「⚠️ 需 SPEC 修订」。✓
- 两处首选方案均真正避免了签名破坏。

### 3. M2-07/M2-08 dim 协同
- M2-07 读 vectors.bin v2 头 dim（v1 回退 `payload_len/doc_count/4`，与 `segment/mod.rs:373-377` 现有逻辑一致）；M2-08 写 v2 头 `magic|version=2|dim(4 LE)|payload`。格式对齐。✓
- **无死锁**：README M2-07 节明确「M2-07 与 M2-08 同批 L0 推进：M2-07 测试可用 stub v2 header；M2-08 落实 finalize 写 v2 后回归」。M2-07 §6 前置依赖写「M2-08 协同（可先 stub，M2-08 落实后回归）」。两者可独立落地 + 回归。✓
- 版本/格式对齐：v2 头 12 字节（magic4+version4+dim4），v1 头 8 字节，两计划描述一致。✓

### 4. 跨计划契约一致性（README vs modules）
- README M2-07 节 SegmentReader 字段与 M2-07 计划 §2 一致。✓
- README M2-08 节 per-file 常量与 M2-08 计划 §2 一致。✓
- README M2-09 节 `brute_search_sq8` 签名与 M2-09 计划 §3 一致——**但两者都缺 metric/docid_base**（见 I-3）。即 README 与 module 一致，但契约本身有缺陷。
- README M2-10 节 Executor trait 与 M2-10 计划 §3 一致。✓
- README M2-11 节 C ABI 函数面与 M2-11 计划 §3 一致（与 M1 README §09 逐字对齐）。✓
- README M2-12 节 `Db::export` 签名与 M2-12 计划 §3 一致。✓
- **无同名不同签名分歧**（除 I-3 的契约缺陷）。

### 5. SPEC v1.2 一致性
- §13.1 懒加载承诺（open<1s + 首次查询<3s）：M2-07 §4 测试 2/4 落实。✓
- §6.2 per-file format_version + stored v1/v2 双模：M2-08 §4 测试 1-7 落实，v1 读兼容 + v2 zstd roundtrip。✓
- §14 I-5 释义（cfg(feature) 允许 segment 编解码，cfg(target) 仅 VFS/Executor）：M2-08 §7 测试 13 grep 守护；M2-10 §7 测试 4/5 grep 守护；M2-05 §7 测试 6 守护。✓
- 无与三处修订冲突的计划。

### 6. 不变量覆盖
- I-1 段不可变：M2-08（stored v2 finalize 一次性写，测试 6）、M2-07（懒加载不写回，测试 10/11）、M2-09（SQ8 内存缓存不写段，测试 12）均声明。✓
- I-3 图不原地删：M2-09 首选方案不改 HNSW（测试 8 grep 验证签名未改），SQ8 不触及图删除语义。✓ 未声明 I-3 但实际不触及，可接受。
- I-5 核心零平台分支：M2-05/M2-08/M2-09/M2-10 均声明 grep 守护。✓
- I-6 manifest 原子性：M2-12（export 读一致快照 + 临时文件→rename，测试 14）。✓
- I-7 FFI 内存铁律：M2-11（句柄注销后 E_NOT_FOUND + arena free，测试 1/5/13）、M2-12（vane_export 不分配 arena，测试 11）、M2-04（close 后调用 reject，测试 13）。✓
- I-8 binding 薄壳：M2-01/M2-02/M2-03/M2-04/M2-11/M2-12/M2-14 均声明。✓
- **无遗漏**。

---

## 依赖黑名单核查
- M2-08 引入 `zstd`/`ruzstd`：非黑名单。✓
- M2-10 引入 `rayon`：非黑名单（黑名单：regex/tokio/prost/tonic/openssl/lindera/ndarray/wee_alloc/dashmap/parking_lot）。✓
- M2-11 用 `std::sync::RwLock`（非 dashmap/parking_lot）：合规。✓
- M2-02/03/04 引入 `web-sys`/`js-sys`/`wasm-bindgen`/`wasm-bindgen-futures`：非黑名单。✓
- 无黑名单违反。

---

## 结论

M2 Phase One 14 份计划 + README 整体契约对接良好：冻结签名（SegmentReader/HnswReader/compile_filter/Db::export/C ABI）均未被破坏；M2-05/M2-09 两个 ⚠️ 修订点首选方案真正避免了 I-5 违反与 M1 签名破坏；M2-07/08 dim 协同无死锁；SPEC v1.2 三处修订被妥善遵守；不变量覆盖矩阵无遗漏。

5 条重要发现均为**签名/文件路径/约束表述的文档缺陷**（Db::open 漏 vfs、Collection 误列 export/close、scalars.rs 文件不存在、brute_search_sq8 缺参、jieba feature 约束冲突），可在实现前订正，不阻塞计划推进。其中 I-5（jieba feature 约束）需编排者裁决是否放宽 M1 约束；I-3（brute_search_sq8 签名）需补 metric/docid_base 后同步 README。
