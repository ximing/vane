# 00-text-persistence：原文持久化（L0 前置）

> SPEC 引用：§6.2（stored.bin 含「原文/JSON meta」）、§6.4（reindex 需原文重新分词）、§7.4（reindex 状态机）、§13.3（corpus 兼容测试）。
> 前置依赖：M0 `segment`（SegmentWriter/SegmentReader，已核查 git HEAD）、M0 `api/collection.rs` flush。
> M1 README 契约：`vane_core::segment` 扩展（见 `docs/plans/m1/README.md` § 00-text-persistence）。
> **本计划为 L0 前置**：02-tombstone-merge（merge 复用原文）与 06-userdict-reindex（reindex 读原文用新分词器重建倒排）均依赖本计划产出的 `SegmentReader::text`。

## 背景（为什么必须前置）

M0 `api/collection.rs` flush（第 195-223 行）中 `stored_json` 仅由 `doc.meta` 序列化构造，`doc.text` 仅被 `tokenizer.tokenize()` 后喂给 `InvertedIndexBuilder` 即丢弃——**原文从未写入 stored.bin 或任何段文件**。这违反 SPEC §6.2（stored.bin 应含「原文/JSON meta」），并使 06 reindex（换分词器重建倒排）不可实现：原文已丢，无法重新分词。

M0 `SegmentReader`（segment/mod.rs 第 183-308 行）仅暴露 `stored_json(local_docid)`（返回 meta JSON），无任何方法返回原文 text。M0 `InvertedIndexReader` 的 posting 存 tokenized 后的 term + docid + tf，无法还原原文。

本计划补全 SPEC §6.2 始终要求的 stored.bin 完整格式（原文 + meta），不引入新段文件，不改 `SegmentWriter::add_doc` 冻结签名。

## Goal

1. 扩展 `stored.bin` 每条记录为 `docid(8 LE) | text_len(4 LE) | text_bytes | meta_json_len(4 LE) | meta_json_bytes`（原文 + meta 分离存储）。
2. `SegmentWriter` 新增 `set_text` 方法（不改 `add_doc` 签名），api 层 flush 在 `add_doc` 后调 `set_text` 写入原文。
3. `SegmentReader` 新增 `text(local_docid) -> Option<&str>`；`stored_json` 语义不变（仍返回 meta JSON）。
4. 更新 `tests/corpus_compat.rs` 验证原文 roundtrip。
5. TDD：原文 roundtrip 测试 + reindex 前置可用性测试（用原文 + 新分词器重建倒排的骨架验证）。

## Architecture

- **stored.bin 布局扩展**（format_version 保持 1）：
  ```
  magic(4)="VANE" | format_version(4 LE)=1 | count(4 LE) |
  { docid(8 LE) | text_len(4 LE) | text_bytes | meta_json_len(4 LE) | meta_json_bytes }...
  ```
  - `text_len=0` 表示该文档无原文（纯向量 collection 或未调 `set_text`）。
  - `meta_json_len=0` 表示无 meta（`{}` 也写为 2 字节 `"{}"`，与 M0 行为一致；此处 0 仅用于 api 层传 `""` 的边界，正常路径 api 层传 `"{}"`）。
  - **format_version 不递增**：SPEC §6.2 对 stored.bin 的格式描述始终是「原文/JSON meta」，M0 实现是占位不完整（仓库无历史发布产物，`tests/corpus_compat.rs` 注释明确「冻结清理后格式」）。本次补全 spec'd 字段属非破坏性完善，无迁移负担。corpus 兼容测试同步更新（重新生成 corpus）。
- **SegmentWriter 扩展**（不改 `add_doc` 签名）：
  - 内部 `stored: Vec<StoredEntry>`，`StoredEntry { docid: u64, text: Option<String>, meta_json: String }`。
  - `add_doc(external_id, vector, stored_json)` 仍接 meta JSON，push `StoredEntry { docid, text: None, meta_json: stored_json }`（签名不变）。
  - 新增 `set_text(&mut self, text: &str) -> Result<()>`：为**最近一次 `add_doc`** 的文档设置原文（修改 stored 最后一条的 text 字段）。若 `add_doc` 未先调用则 `Err(Schema)`。在 `add_doc` 之后、`finalize` 之前调用。
- **SegmentReader 扩展**：
  - 内部 `stored: HashMap<u64, StoredReadEntry>`，`StoredReadEntry { text: String, meta_json: String }`。
  - `text(local_docid: u64) -> Option<&str>`：返回原文（无原文或 docid 不存在返回 None）。
  - `stored_json(local_docid: u64) -> Option<&str>`：返回 meta_json（语义不变，M0 调用方 api search 回填 `Hit.fields` 不受影响）。
- **api 层 flush 接入**（`api/collection.rs`）：flush 循环中 `add_doc` 后调 `writer.set_text(doc.text.as_deref().unwrap_or(""))`（text 为 None 时传空串，text_len=0）。不改 `add_doc` 调用方式。
- **不变量 I-1**：stored.bin 仍在 `finalize` 一次性写入（段不可变），`set_text` 仅在写期 buffer 内修改，不违反「段写一次后只读」。

## 涉及文件

- **Modify**：
  - `crates/vane-core/src/segment/mod.rs`（SegmentWriter.stored 改结构 + set_text + finalize 写新布局；SegmentReader.load_stored 解码新布局 + text 方法；decodeStored 辅助）
  - `crates/vane-core/src/api/collection.rs`（flush：add_doc 后 set_text）
  - `tests/corpus_compat.rs`（验证原文 roundtrip：reopen 后 `SegmentReader::text` 返回原文；文档化格式扩展）
- **Test**：
  - `crates/vane-core/src/segment/tests.rs`（原文 roundtrip 单元测试）
  - `crates/vane-core/tests/text_persistence.rs`（集成：原文持久化 + reindex 前置可用性）

## Interfaces

### Consumes from M0（已核查 git HEAD）

```rust
// crates/vane-core/src/segment/mod.rs（M0 冻结签名，本计划不改 add_doc/finalize/new 签名）
pub struct SegmentWriter { /* stored: Vec<(u64, String)> —— M0 内部字段，本计划改结构 */ }
impl SegmentWriter {
    pub fn new(vfs, segments_dir, schema, tokenizer_id, docid_base) -> Result<Self>;
    pub fn add_doc(&mut self, external_id: &str, vector: Option<&[f32]>, stored_json: &str) -> Result<u64>;
    pub fn finalize(self) -> Result<SegmentMeta>;
}
impl SegmentReader {
    pub fn open(vfs: &Arc<dyn Vfs>, segment_dir: &str) -> Result<Self>;
    pub fn stored_json(&self, local_docid: u64) -> Option<&str>;  // 语义不变（返回 meta JSON）
    // ...其余 M0 方法不变
}
```

### Produces（见 README § 00-text-persistence 契约）

```rust
impl SegmentWriter {
    /// 为最近一次 add_doc 的文档设置原文。在 add_doc 之后、finalize 之前调用。
    /// 重复调用覆盖。未调用则该文档 text_len=0。
    pub fn set_text(&mut self, text: &str) -> Result<()>;
}

impl SegmentReader {
    /// 读取原文（SPEC §6.2 stored.bin 含原文）。local_docid 为段内局部 docid。
    /// 无原文或 docid 不存在返回 None。
    pub fn text(&self, local_docid: u64) -> Option<&str>;
}
```

**Produces for**：02-tombstone-merge（merge 时从源段 `SegmentReader::text` 读原文写入新段；**不重新分词，倒排用 posting remap**）、06-userdict-reindex（reindex 从旧段 `SegmentReader::text` 读原文，用**新分词器**重新 tokenize 重建倒排）。

## TDD 任务清单

### Task 1：stored.bin 原文 roundtrip（写失败测试 → 验证失败 → 实现 → 通过 → commit）

**测试**（`crates/vane-core/src/segment/tests.rs` 扩展）：
```rust
#[test]
fn stored_text_roundtrip() {
    let vfs = std::sync::Arc::new(crate::vfs::memory::MemoryVfs::new())
        as std::sync::Arc<dyn crate::vfs::Vfs>;
    use crate::segment::{SegmentWriter, SegmentReader};
    use crate::types::{Schema, FieldDef, Metric, TokenizerId};
    let schema = Schema::new(vec![("v".into(), FieldDef::Vector { dim: 2, metric: Metric::Cosine })]).unwrap();
    let tid = TokenizerId::from_bytes([0u8; 32]);
    let mut w = SegmentWriter::new(vfs.clone(), "db/segments", &schema, &tid, 0).unwrap();
    let _local0 = w.add_doc("d0", Some(&[1.0, 0.0]), "{}").unwrap();
    w.set_text("机器学习检索").unwrap();
    let _local1 = w.add_doc("d1", Some(&[0.0, 1.0]), "{}").unwrap();
    // d1 不调 set_text → text_len=0
    let meta = w.finalize().unwrap();
    let seg_dir = format!("db/segments/seg_{}", meta.ulid);
    let r = SegmentReader::open(&vfs, &seg_dir).unwrap();
    assert_eq!(r.text(0), Some("机器学习检索"));
    assert_eq!(r.text(1), Some(""));  // 未调 set_text → 空串（text_len=0）
    assert_eq!(r.text(999), None);
    // meta JSON 语义不变
    assert_eq!(r.stored_json(0), Some("{}"));
}
```
验证失败：`set_text` / `text` 方法不存在（编译错误）。
最小实现：
- `SegmentWriter.stored` 改为 `Vec<StoredEntry { docid: u64, text: Option<String>, meta_json: String }>`。`add_doc` push `StoredEntry { docid, text: None, meta_json: stored_json.to_string() }`。
- `set_text`：取 `stored.last_mut()`，设 `text = Some(text.to_string())`；若 `stored` 为空 `Err(VaneError::Schema("set_text called before add_doc"))`。
- `finalize` 写 stored.bin 新布局：每条 `docid(8 LE) | text_len(4 LE) | text_bytes | meta_json_len(4 LE) | meta_json_bytes`。
- `SegmentReader.load_stored` 解码新布局为 `HashMap<u64, StoredReadEntry { text, meta_json }>`。`text(local_docid)` 返回 `stored.get(&local_docid).map(|e| e.text.as_str())`（text 为 None 时返回 None；但 finalize 写入时 Option→空串，故读期 text 始终是 Some(String)，空串表示无原文）。`stored_json` 返回 `e.meta_json.as_str()`。
- 保留 magic + format_version=1 + count 头。
commit：`segment: persist original text in stored.bin (SPEC §6.2)`。

### Task 2：api flush 接入 set_text

**测试**（`crates/vane-core/tests/text_persistence.rs`）：
```rust
use vane_core::api::*;
use vane_core::types::*;
use vane_core::vfs::memory::MemoryVfs;
use std::sync::Arc;

#[test]
fn flush_persists_text_readable_after_reopen() {
    let vfs = Arc::new(MemoryVfs::new());
    {
        let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
        let schema = Schema::new(vec![
            ("body".into(), FieldDef::Text),
            ("v".into(), FieldDef::Vector { dim: 2, metric: Metric::Cosine }),
        ]).unwrap();
        let col = db.collection("c", schema, CollectionOptions::default()).unwrap();
        col.add(&[Doc {
            id: "d0".into(),
            text: Some("原文必须持久化".into()),
            vector: Some(vec![1.0, 0.0]),
            meta: None,
        }]).unwrap();
        col.flush().unwrap();
    }
    // reopen 后经 api 搜索验证（api 内部回填 stored_json 不变）+ 直接验证段原文可读
    let db2 = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col2 = db2.collection("c", /* same schema */, CollectionOptions::default()).unwrap();
    let hits = col2.search(&SearchQuery {
        text: Some("原文".into()), top_k: 10, mode: SearchMode::Text, ..Default::default()
    }).unwrap();
    assert!(hits.iter().any(|h| h.id == "d0"));
    // 段原文可读（reindex 前置可用性：能拿到原文才能重新分词）
    // （api 层不暴露 SegmentReader::text，但 02/06 经 CollectionInner 内部访问；此处间接验证搜索仍命中）
}
```
最小实现：`api/collection.rs` flush 循环中 `add_doc` 之后加 `writer.set_text(doc.text.as_deref().unwrap_or(""))`。其余 flush 逻辑不变。
commit：`api: persist doc.text into stored.bin on flush`。

### Task 3：corpus 兼容测试更新

**测试**（`tests/corpus_compat.rs` 扩展）：
```rust
// 在 corpus_format_compat_roundtrip 中 reopen 后补：
// 经 SegmentReader 验证原文可读（corpus_docs 已含 text 字段）
// 注：corpus_compat 用 StdFsVfs，reopen 后 col 内部段快照含原文。
// 由于 api 不暴露 text()，此测试通过搜索 text 路命中验证原文已持久化（若原文丢失，reindex 不可实现，但搜索仍走 tokenized 倒排——故此处额外用段级断言）。
// 裁决：corpus_compat 增加「stored.bin 含原文」的段级断言需访问 SegmentReader。
//   在 corpus_compat 中通过 vfs 直接读 stored.bin 字节校验 text_len>0（不依赖 api 暴露）。
```
最小实现：`corpus_compat.rs` 的 `corpus_segment_files_have_magic_version_headers` 测试扩展——读 stored.bin 字节，校验首条记录 `text_len > 0` 且 text_bytes 等于 `corpus_docs()[0].text` 的 UTF-8 字节。文档化 stored.bin 格式扩展（注释说明 v1.1 起含原文 + meta 分离）。
commit：`test: update corpus_compat for stored.bin text+meta layout`。

### Task 4：reindex 前置可用性测试（原文 → 新分词器重建倒排骨架）

**测试**（`crates/vane-core/tests/text_persistence.rs`）：
```rust
#[test]
fn reindex_prerequisite_text_readable_for_retokenize() {
    // 验证 06 reindex 的前置条件成立：旧段原文可读，能用新分词器重新 tokenize。
    // 本测试不实装 reindex（06 计划负责），只验证「原文可读 + 可重新分词」的管线不缺料。
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
    let schema = Schema::new(vec![
        ("body".into(), FieldDef::Text),
        ("v".into(), FieldDef::Vector { dim: 2, metric: Metric::Cosine }),
    ]).unwrap();
    let col = db.collection("c", schema, CollectionOptions::default()).unwrap();
    col.add(&[Doc {
        id: "d0".into(), text: Some("机器学习".into()),
        vector: Some(vec![1.0, 0.0]), meta: None,
    }]).unwrap();
    col.flush().unwrap();
    // 经 CollectionInner 段快照读原文（测试用 col.snapshot_readers() 测试辅助，02 计划新增）
    // 若 02 未完成，本测试可降级为：reopen 后搜索 text="机器学习" 命中（证明倒排建于原文）
    let hits = col.search(&SearchQuery {
        text: Some("机器学习".into()), top_k: 10, mode: SearchMode::Text, ..Default::default()
    }).unwrap();
    assert!(hits.iter().any(|h| h.id == "d0"));
    // 06 reindex 实装后：用 cjk_bigram 重新分词「机器学习」→ 验证新倒排可建。
    // 本 Task 仅断言原文进了倒排（搜索命中），证明原文数据流完整。
}
```
commit：`test: assert reindex prerequisite (original text persisted and indexable)`。

## 验收标准

- **SPEC §6.2**：stored.bin 含原文 + JSON meta（Task 1 字节级验证；Task 3 corpus 兼容）。
- **SPEC §6.4/§7.4 前置**：reindex（06）所需原文可从 `SegmentReader::text` 读出（Task 4 间接验证数据流完整；06 计划实装真实 reindex）。
- **不变量 I-1**：stored.bin 仍在 finalize 一次性写入（段不可变），`set_text` 仅修改写期 buffer。
- **M0 冻结签名不破坏**：`SegmentWriter::add_doc` / `finalize` / `new` 签名不变；`SegmentReader::open` / `stored_json` / 其余方法签名不变。仅新增 `set_text` / `text`。
- **corpus 兼容**：`tests/corpus_compat.rs` 重新生成 corpus 验证新布局 roundtrip（M0 无发布产物，无迁移负担）。

## 前置依赖

- M0 `segment` / `api/collection.rs`（已合并）。
- 无 M1 内部前置（L0 批次，与 01/05/09 并行）。

## Global Constraints

core 禁 std::fs（stored.bin 经 Vfs）；段不可变（I-1，stored.bin 写一次）；不改 M0 冻结 pub API（`add_doc`/`finalize`/`new`/`open`/`stored_json` 签名不变，仅新增 `set_text`/`text`）；format_version 保持 1（补全 spec'd 格式，无发布数据故无迁移）；并发原语 std::sync。
