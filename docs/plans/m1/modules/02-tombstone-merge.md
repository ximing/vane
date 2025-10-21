# 02-tombstone-merge：delete tombstone + 段合并 + compact() 实装

> SPEC 引用：§7.2（删除 tombstone）、§7.3（段合并可切片增量）、§3.3（段数硬上限 10）、§6.3（tombstone 存 header.bin）、§4.1（delete/compact 动词）。
> 前置依赖：M0 `segment`/`persistence`/`api`（已核查 git HEAD）；01-hnsw（合并重建图）；**00-text-persistence**（`SegmentReader::text` 读原文写入新段，供未来 reindex）。
> M1 README 契约：`vane_core::merge` + api 扩展。

## Goal

实装 `Collection::delete`（追加 tombstone，即时进 WAL）+ `Collection::compact`（手动触发段合并）。段合并为可切片增量任务（MergeTask），物理清除 tombstone 文档，新段从零重建 HNSW 图（I-3）。段数超 10 强制合并，小段（<1 万文档）优先。

## Architecture

- **tombstone**：M0 `SegmentMeta.tombstones: RoaringBitmap` 已存在（header.bin 已含 tombstone 字段，M0 写空）。M1 实装 delete 时更新**内存段级 tombstone**（不修改段文件——I-1 段不可变），tombstone 持久化经 WAL（04 计划）+ 新段合并时物理清除。
- **delete 编排**：`delete(ids)` 查 external_id → 定位段 + local_docid → 追加到该段内存 tombstone（`CollectionInner` 增 `tombstones: RwLock<HashMap<String /*ulid*/, RoaringBitmap>>`）→ WAL append（04 计划）→ 返回 count。查询期 `brute_search`/`InvertedIndexReader::search`/`HnswReader::search` 的 filter 参数已支持位图，delete 后查询自动过滤（03 计划把 tombstone 并入 filter）。
- **MergeTask**（B-1 修订）：可切片增量。每 `step()` 处理一个源段：
  - 读 vectors/idmap/stored/**原文（`SegmentReader::text`，来自 00）** → 跳过 tombstone → `SegmentWriter::add_doc` + `set_text`（原文复用）+ `set_scalar`（Q-7 标量重写，从源段 ScalarReader 读重映射 docid）写入新段。
  - **倒排用 posting remap，不重新分词**（B-1）：merge 时分词器不变，从源段 `InvertedIndexReader` 读每个 term 的 postings，按新 docid（docid 重映射表）重写 posting.docid，重组 `InvertedData` 后 `write_inverted`。无需重新分词，不依赖 tokenizer 实例做切分（但 MergeTask 仍持 `Arc<dyn Tokenizer>` 以备 06 reindex 复用管线——compact 传当前 tokenizer，reindex 传新 tokenizer 且走重新分词路径，见 06）。
  - `HnswWriter` 从零重建图（vectors 重写，I-3）。
  - 全部源段处理完后 `finalize_merge` 落盘 + manifest 切换 + 旧段标记删除。
- **compact()**：手动触发 `pick_merge_candidates`（全部段或按策略）→ `MergeTask` → **M1 同步执行（全串行，无 Executor，R-4/R-6）** → E_BUSY 若 reindex 进行中。切片粒度（每步 N 个 posting 块/图节点）留 M2 细化——M1 `step()` 处理一个源段全部数据，粒度粗于 SPEC §7.3 但同步执行可接受。
- **段数上限**：flush 后检查 `snapshot.len() > SEGMENT_MAX` → 自动触发合并小段。

## 涉及文件

- **Create**：
  - `crates/vane-core/src/merge/mod.rs`（pick_merge_candidates / MergeTask / finalize_merge）
  - `crates/vane-core/src/merge/tests.rs`
- **Modify**：
  - `crates/vane-core/src/lib.rs`（增 `pub mod merge;`）
  - `crates/vane-core/src/api/collection.rs`（实装 delete/compact；增 tombstones 字段；flush 后检查段数上限）
  - `crates/vane-core/src/api/collection.rs` restore_from_manifest：加载段 header 中的 tombstones
- **Test**：
  - `crates/vane-core/src/merge/tests.rs`
  - `crates/vane-core/tests/tombstone_merge.rs`（集成）

## Interfaces

### Consumes from M0（已核查 git HEAD）

```rust
// crates/vane-core/src/segment/mod.rs
pub struct SegmentMeta {
    pub ulid: String, pub doc_count: u32, pub docid_base: u64,
    pub tokenizer_id: TokenizerId, pub tombstones: roaring::RoaringBitmap,
}
impl SegmentReader {
    pub fn open(vfs: &Arc<dyn Vfs>, segment_dir: &str) -> Result<Self>;
    pub fn meta(&self) -> &SegmentMeta;  // .tombstones 可读
    pub fn vectors(&self) -> &[f32]; pub fn dim(&self) -> u32;
    pub fn external_id(&self, docid: u64) -> Option<&str>;
    pub fn stored_json(&self, local_docid: u64) -> Option<&str>;
    pub fn segment_dir(&self) -> &str; pub fn vfs(&self) -> &Arc<dyn Vfs>;
}
impl SegmentWriter {
    pub fn new(vfs, segments_dir, schema, tokenizer_id, docid_base) -> Result<Self>;
    pub fn add_doc(&mut self, external_id: &str, vector: Option<&[f32]>, stored_json: &str) -> Result<u64>;
    pub fn finalize(self) -> Result<SegmentMeta>;
}
// crates/vane-core/src/segment/header.rs
pub fn encode_header(meta: &SegmentMeta) -> Result<Vec<u8>>;  // 已含 tombstone 序列化
pub fn decode_header(buf: &[u8]) -> Result<SegmentMeta>;

// crates/vane-core/src/bm25.rs
pub struct InvertedIndexBuilder { ... }
impl InvertedIndexBuilder {
    pub fn new(doc_count_hint: usize) -> Self;
    pub fn add_document(&mut self, docid: u64, tokens: &[Token], field_length: u32);
    pub fn build(self) -> InvertedData;
}
pub fn write_inverted(vfs: &dyn Vfs, segment_dir: &str, data: &InvertedData) -> Result<()>;

// crates/vane-core/src/persistence/mod.rs
pub struct ManifestStore { ... }
impl ManifestStore {
    pub fn add_segment(&self, collection: &str, ulid: &str) -> Result<()>;
    pub fn save_atomic(&self, manifest: &Manifest) -> Result<()>;
    pub fn load(&self) -> Result<Option<Manifest>>;
}
pub struct CollectionMeta { pub schema, tokenizer_kind, tokenizer_id, user_dict, segment_ulids: Vec<String> }
```

### Consumes from 01-hnsw

```rust
pub struct HnswWriter { ... }
impl HnswWriter {
    pub fn new(dim: u32, metric: Metric, m: u32, ef_construction: u32) -> Self;
    pub fn insert(&mut self, local_docid: u32, vector: &[f32]);
    pub fn build(self) -> HnswGraph;
}
pub fn write_hnsw(vfs: &dyn Vfs, segment_dir: &str, graph: &HnswGraph) -> Result<()>;
```

### Consumes from 00-text-persistence

```rust
impl SegmentReader {
    pub fn text(&self, local_docid: u64) -> Option<&str>;  // 原文复用（写入新段）
}
impl SegmentWriter {
    pub fn set_text(&mut self, text: &str) -> Result<()>;  // 写原文到新段
}
```

### Produces（见 README § 02-tombstone-merge 契约）

## TDD 任务清单

### Task 1：delete 追加 tombstone（内存 + 查询过滤）

**测试**（`crates/vane-core/tests/tombstone_merge.rs`）：
```rust
use vane_core::api::*;
use vane_core::types::*;
use vane_core::vfs::memory::MemoryVfs;
use std::sync::Arc;

#[test]
fn delete_hides_doc_from_search() {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let schema = Schema::new(vec![
        ("body".into(), FieldDef::Text),
        ("v".into(), FieldDef::Vector { dim: 4, metric: Metric::Cosine }),
    ]).unwrap();
    let col = db.collection("c", schema, CollectionOptions::default()).unwrap();
    col.add(&[Doc { id: "d1".into(), text: Some("hello".into()), vector: Some(vec![1.0,0.0,0.0,0.0]), meta: None }]).unwrap();
    col.add(&[Doc { id: "d2".into(), text: Some("hello".into()), vector: Some(vec![1.0,0.0,0.0,0.0]), meta: None }]).unwrap();
    col.flush().unwrap();
    let hits = col.search(&SearchQuery { text: Some("hello".into()), top_k: 10, mode: SearchMode::Text, ..Default::default() }).unwrap();
    assert_eq!(hits.len(), 2);
    // delete d1
    let n = col.delete(&["d1".into()]).unwrap();
    assert_eq!(n, 1);
    let hits2 = col.search(&SearchQuery { text: Some("hello".into()), top_k: 10, mode: SearchMode::Text, ..Default::default() }).unwrap();
    assert_eq!(hits2.len(), 1);
    assert_eq!(hits2[0].id, "d2");
}
```
验证失败：`delete` 返回 E_UNSUPPORTED（M0 占位）。
最小实现：`CollectionInner` 增 `tombstones: RwLock<HashMap<String, RoaringBitmap>>`（ulid → 绝对 docid 位图）。`delete(ids)`：遍历 snapshot 段查 external_id → 定位 ulid + docid → 插入位图。search 时把 tombstone 位图并入 filter（本 Task 先在 search 内手动合并 tombstone 到 filter 参数；03 计划正式 compile_filter 统一）。
commit：`api: implement delete with in-memory tombstone`。

### Task 2：tombstone 持久化经 WAL（reopen 后保留）

**测试**：
```rust
#[test]
fn tombstone_survives_reopen_via_wal() {
    let vfs = Arc::new(MemoryVfs::new());
    {
        let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
        let col = setup_col(&db);
        col.add(&docs()).unwrap(); col.flush().unwrap();
        col.delete(&["d1".into()]).unwrap();
        // tombstone 经 WAL 持久化（04 计划）：delete 即时 append AddTombstone 到 wal.log。
        // 不调 sync_tombstones（M-3 修订：header.bin 不改，tombstone 运行期仅存 WAL+内存）。
        // 不调 close（模拟崩溃），WAL 未 truncate。
    }
    let db2 = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col2 = db2.collection("c", schema(), CollectionOptions::default()).unwrap();
    let hits = col2.search(&SearchQuery { text: Some("hello".into()), top_k: 10, mode: SearchMode::Text, ..Default::default() }).unwrap();
    assert!(!hits.iter().any(|h| h.id == "d1"), "tombstone must be replayed from WAL");
}
```
最小实现：delete 调 `Wal::append(AddTombstone)`（04 计划产出）。reopen 时 `wal::recover` 重放 AddTombstone → 注入 `CollectionInner.tombstones`。**header.bin 不改**（段不可变 I-1，tombstone 运行期仅存 WAL+内存；header.bin 的 tombstone 字段仅在段合并时物理写入新段为空）。**依赖 04-wal**——本 Task 标记 blockedBy 04，先跳过 reopen 测试，仅保留内存 delete（Task 1）。
commit：`api: persist tombstone via WAL (no header.bin mutation, M-3)`。

### Task 3：MergeTask 单段合并（物理清除 tombstone + 重建图 + posting remap + 原文/标量复用）

**测试**（`crates/vane-core/src/merge/tests.rs`）：
```rust
use super::*;
use crate::segment::SegmentReader;
use std::sync::Arc;
use crate::vfs::memory::MemoryVfs;

#[test]
fn merge_single_segment_drops_tombstoned_docs() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn crate::vfs::Vfs>;
    // 准备：1 段 5 文档，tombstone 含 docid 1,3
    let (meta, tombstones) = setup_segment_with_tombstone(&vfs, 5, &[1, 3]);
    let tok = std::sync::Arc::new(crate::tokenizer::build_tokenizer(
        crate::tokenizer::BuiltinTokenizer::Standard, &[]).unwrap()) as std::sync::Arc<dyn crate::tokenizer::Tokenizer>;
    let mut task = MergeTask::new(
        vec![meta.ulid.clone()], 0, meta.tokenizer_id.clone(), test_schema(), tok);
    let ctx = MergeContext { vfs: &vfs, db_path: "db", segments_dir: "db/segments" };
    // 执行合并
    while !task.step(&ctx).unwrap() {}
    let new_meta = finalize_merge(task, &ctx).unwrap();
    // 新段 doc_count = 5 - 2 = 3
    let reader = SegmentReader::open(&vfs, &format!("db/segments/seg_{}", new_meta.ulid)).unwrap();
    assert_eq!(reader.doc_count(), 3);
    assert!(new_meta.tombstones.is_empty());  // tombstone 物理清除
    // 原文复用（B-1/00）：新段原文可读
    assert!(reader.text(0).is_some());
    // 标量复用（Q-7）：新段标量可读
    // （set_scalar 已在 03 计划实装；本测试若 03 未完成可后置标量断言）
}
```
最小实现：`MergeTask::step` 处理一个源段：
- open `SegmentReader` + `InvertedIndexReader` + `ScalarReader`（03 计划，若未完成标量部分后置）。
- 遍历 docid 跳过 tombstone → `SegmentWriter::add_doc`（重写 external_id/vector/stored_json）+ `set_text`（**原文从 `SegmentReader::text` 读，B-1/00**）+ `set_scalar`（**标量从源段 ScalarReader 读，重映射 docid，Q-7**）+ `HnswWriter::insert`。
- **倒排用 posting remap**（B-1）：从源段 `InvertedIndexReader` 读每个 term 的 postings，按新 docid（重映射表）重写 posting.docid，重组 `InvertedData` 后 `write_inverted`。**不重新分词**（分词器不变）。
- `finalize_merge`：writer.finalize + write_inverted + write_hnsw + manifest 切换。
commit：`merge: single-segment merge with posting remap + text/scalar reuse (B-1/Q-7)`。

### Task 4：多段合并 + docid 重映射

**测试**：
```rust
#[test]
fn merge_multi_segments_remaps_docid_contiguous() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn crate::vfs::Vfs>;
    // 2 段：seg_a (docid_base=0, 3 docs), seg_b (docid_base=100, 3 docs)
    setup_two_segments(&vfs);
    let tok = std::sync::Arc::new(crate::tokenizer::build_tokenizer(
        crate::tokenizer::BuiltinTokenizer::Standard, &[]).unwrap()) as std::sync::Arc<dyn crate::tokenizer::Tokenizer>;
    let mut task = MergeTask::new(vec!["seg_a".into(), "seg_b".into()], 0, test_tokenizer_id(), test_schema(), tok);
    let ctx = MergeContext { vfs: &vfs, db_path: "db", segments_dir: "db/segments" };
    while !task.step(&ctx).unwrap() {}
    let new_meta = finalize_merge(task, &ctx).unwrap();
    let reader = SegmentReader::open(&vfs, &format!("db/segments/seg_{}", new_meta.ulid)).unwrap();
    assert_eq!(reader.doc_count(), 6);
    // 新 docid 连续从 0 起
    assert_eq!(reader.meta().docid_base, 0);
}
```
最小实现：MergeTask 维护 `target_docid` 计数器，每写入一文档分配新连续 docid；InvertedIndexBuilder 用新 docid；HnswWriter 用新 local_docid。
commit：`merge: multi-segment merge with contiguous docid remap`。

### Task 5：compact() 实装 + 旧段删除

**测试**：
```rust
#[test]
fn compact_merges_all_segments_and_removes_old() {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
    let col = setup_col(&db);
    // 多次 flush 造多段
    for batch in 0..3 {
        col.add(&docs_batch(batch)).unwrap();
        col.flush().unwrap();
    }
    assert_eq!(col.segment_count(), 3);
    col.compact().unwrap();
    assert_eq!(col.segment_count(), 1);
    // 旧段目录已删
    assert!(col.search(&SearchQuery { text: Some("hello".into()), top_k: 10, mode: SearchMode::Text, ..Default::default() }).unwrap().len() > 0);
}
```
最小实现：`compact()`：pick_merge_candidates(全部段) → MergeTask → finalize → manifest 删除旧 ulid + 新增新 ulid → Vfs::delete 旧段目录 → 更新 snapshot/seg_offsets/inverted_readers/hnsw_readers 缓存。E_BUSY 若 reindex 进行中（用 `CollectionInner` 增 `state: RwLock<DictState>` 检查）。
commit：`api: implement compact with old segment cleanup`。

### Task 6：段数超 10 自动合并

**测试**：
```rust
#[test]
fn flush_auto_merges_when_exceeding_segment_max() {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col = setup_col(&db);
    for batch in 0..11 {
        col.add(&docs_batch(batch)).unwrap();
        col.flush().unwrap();
    }
    // SEGMENT_MAX=10，第 11 次 flush 后自动合并
    assert!(col.segment_count() <= 10);
}
```
最小实现：flush 末尾 `if snapshot.len() > SEGMENT_MAX { trigger_auto_merge }`。auto_merge 选最小两段合并（pick_merge_candidates 返回最小段）。
commit：`api: auto-merge on exceeding SEGMENT_MAX`。

### Task 7：不变量 I-3（图重建仅段合并）

**测试**：
```rust
#[test]
fn graph_rebuilt_only_during_merge() {
    // delete 后 hnsw.bin 字节不变；compact 后新段有新 hnsw.bin
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
    let col = setup_col(&db);
    col.add(&docs()).unwrap(); col.flush().unwrap();
    let hnsw_path = format!("db/segments/seg_{}/hnsw.bin", first_ulid(&col));
    let size_before = file_size(&vfs, &hnsw_path);
    col.delete(&["d1".into()]).unwrap();
    let size_after = file_size(&vfs, &hnsw_path);
    assert_eq!(size_before, size_after, "hnsw.bin must not change on delete (I-3)");
    col.compact().unwrap();
    // compact 后旧段 hnsw.bin 不存在（已删），新段有新 hnsw.bin
}
```
commit：`merge: assert graph immutability on delete (I-3)`。

## 验收标准

- **SPEC §7.2**：delete = 追加 tombstone（即时进 WAL，flush 后随段生效）；tombstone 比例进可观测指标；compact() 物理清除。
- **SPEC §7.3**：合并为可切片增量任务（MergeTask::step/progress）；合并不阻塞读（快照不可变）；段数硬上限 10。
- **SPEC §3.3**：超限强制合并，小段（<1 万文档）优先。
- **SPEC §6.3**：tombstone 存 header.bin（合并后新段空）；查询期过滤。
- **不变量 I-3**：图不原地删，图重建仅段合并（Task 7）。
- **不变量 I-2**：合并后向量+倒排+图同快照出现。
- **M0 占位对接**：delete/compact 实装完成（M0 返回 E_UNSUPPORTED 消除）。

## 前置依赖

- M0 segment/persistence/api（已合并）。
- 01-hnsw（HnswWriter/write_hnsw，合并重建图）。
- **00-text-persistence**（`SegmentReader::text` 读原文 + `SegmentWriter::set_text` 写原文到新段，B-1 前置）。
- 04-wal（tombstone 持久化经 WAL，Task 2 reopen 测试依赖——若 04 未完成，Task 2 reopen 部分可后置，Task 1/3-7 不依赖）。

## Global Constraints

core 禁 std::fs；并发原语 std::sync；段不可变（I-1，header.bin tombstone 经 WAL 不改段数据文件）；图不原地删（I-3）；manifest 原子切换（I-6）；**MergeTask M1 全串行同步执行（无 Executor/cfg，R-4/R-6），切片粒度留 M2**；**倒排用 posting remap 不重新分词（B-1）**；原文从 00 的 `SegmentReader::text` 复用。
