# 03-pre-filter：metadata 过滤位图进 HNSW+WAND + 低选择率暴力回退 + scalars.col 写读

> SPEC 引用：§8.3（过滤 pre-filter）、§3.1（scalar 字段）、§6.2（scalars.col 列式块）、§8.1（位图基数 <2×topK 暴力回退）。
> 前置依赖：M0 `segment`/`bm25`/`vector`/`api::types`；01-hnsw；02-tombstone-merge（tombstone 并入 filter）。
> M1 README 契约：`vane_core::filter` + segment 扩展。

## Goal

实装 `Filter` 编译为 roaring 位图，传入 HNSW 遍历（01 已支持 filter 参数）+ WAND 推进（M0 InvertedIndexReader::search 已支持 filter）+ brute_search（M0 已支持）。低选择率（位图基数 <2×topK）向量路自动切暴力精确扫描。scalars.col 列式块写读（M0 写空 stub）。

## Architecture

- **scalars.col 写期**：`SegmentWriter` 新增 `set_scalar(field, value)` 方法（不改 M0 add_doc 签名）。flush 时 finalize 写 scalars.col：magic+version+num_fields+{field_name+kind+column_data}。
- **scalars.col 读期**：`ScalarReader::open` 加载列式块；`get(field, local_docid) -> Option<ScalarValue>`。
- **Filter 编译**：`compile_filter(filter, schema, segments, scalars, tombstones) -> RoaringBitmap`。遍历 filter.fields，每字段按条件（eq/in/gte/lte）扫列式块，结果 AND 合并；最后 AND NOT tombstones（tombstone 文档排除）。
- **低选择率回退**：`should_fallback_brute(bitmap, topk) = bitmap.cardinality() < 2*topk`。api search vector 路：若 fallback → `brute_search(filter=Some(&bitmap))`（100% 召回）；否则 HnswReader::search(filter)。
- **SearchQuery.filter 解禁**：M0 search 对 filter 返回 InvalidArg；M1 移除此 reject，改为编译 filter 传入各路。

## 涉及文件

- **Create**：
  - `crates/vane-core/src/filter/mod.rs`（compile_filter / should_fallback_brute）
  - `crates/vane-core/src/filter/tests.rs`
- **Modify**：
  - `crates/vane-core/src/segment/mod.rs`（ScalarReader 类型；SegmentWriter::set_scalar；finalize 写 scalars.col 真实数据）
  - `crates/vane-core/src/api/collection.rs`（search 接入 filter 编译 + 回退判定；移除 M0 filter reject）
  - `crates/vane-core/src/api/collection.rs` restore_from_manifest：加载 ScalarReader 缓存（类比 inverted_readers）
  - `crates/vane-core/src/api/collection.rs` flush：把 BufferedDoc.meta 的 scalar 字段经 set_scalar 写入
- **Test**：
  - `crates/vane-core/src/filter/tests.rs`
  - `crates/vane-core/tests/pre_filter.rs`（集成）

## Interfaces

### Consumes from M0（已核查 git HEAD）

```rust
// crates/vane-core/src/api/types.rs
pub enum ScalarValue { Int(i64), Float(f64), Bool(bool), Keyword(String) }
pub enum FilterCond { Eq(ScalarValue), In(Vec<ScalarValue>), Gte(ScalarValue), Lte(ScalarValue) }
pub struct Filter { pub fields: Vec<(String, FilterCond)> }
// crates/vane-core/src/types.rs
pub enum ScalarKind { Int, Float, Bool, Keyword }
pub enum FieldDef { Text, Vector { dim, metric }, Scalar { kind: ScalarKind } }
pub struct Schema { pub fields: Vec<(String, FieldDef)> }
// crates/vane-core/src/bm25.rs（M0 InvertedIndexReader::search 已支持 filter）
pub fn search(&self, query_tokens: &[Token], topk: usize, filter: Option<&roaring::RoaringBitmap>) -> Vec<ScoredDoc>;
// crates/vane-core/src/vector/mod.rs（M0 brute_search 已支持 filter）
pub fn brute_search(vectors, dim, query, metric, topk, filter: Option<&roaring::RoaringBitmap>, docid_base) -> Vec<ScoredDoc>;
// crates/vane-core/src/segment/mod.rs（M0 finalize 写 scalars.col 空 stub：magic+version+0u32）
```

### Consumes from 01-hnsw

```rust
impl HnswReader {
    pub fn search(&self, query: &[f32], topk: usize, ef_search: usize,
                  filter: Option<&roaring::RoaringBitmap>, docid_base: u64) -> Vec<ScoredDoc>;
}
```

### Consumes from 02-tombstone-merge

`CollectionInner.tombstones: RwLock<HashMap<String, RoaringBitmap>>`（delete 产出的段级 tombstone）。

### Produces（见 README § 03-pre-filter 契约）

## TDD 任务清单

### Task 1：scalars.col 写期（SegmentWriter::set_scalar）

**测试**（`crates/vane-core/src/filter/tests.rs`）：
```rust
use vane_core::segment::{SegmentWriter, ScalarReader};
use vane_core::types::*;
use vane_core::api::ScalarValue;
use vane_core::vfs::memory::MemoryVfs;
use std::sync::Arc;

#[test]
fn scalars_col_roundtrip_int_keyword() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn vane_core::vfs::Vfs>;
    let schema = Schema::new(vec![
        ("lang".into(), FieldDef::Scalar { kind: ScalarKind::Keyword }),
        ("year".into(), FieldDef::Scalar { kind: ScalarKind::Int }),
        ("v".into(), FieldDef::Vector { dim: 2, metric: Metric::Cosine }),
    ]).unwrap();
    let mut w = SegmentWriter::new(vfs.clone(), "db/segments", &schema, &test_tid(), 0).unwrap();
    w.add_doc("d1", Some(&[1.0, 0.0]), "{}").unwrap();
    w.set_scalar("lang", ScalarValue::Keyword("zh".into())).unwrap();
    w.set_scalar("year", ScalarValue::Int(2024)).unwrap();
    w.add_doc("d2", Some(&[0.0, 1.0]), "{}").unwrap();
    w.set_scalar("lang", ScalarValue::Keyword("en".into())).unwrap();
    w.set_scalar("year", ScalarValue::Int(2023)).unwrap();
    let meta = w.finalize().unwrap();
    let sr = ScalarReader::open(&vfs, &format!("db/segments/seg_{}", meta.ulid)).unwrap();
    assert_eq!(sr.get("lang", 0), Some(ScalarValue::Keyword("zh".into())));
    assert_eq!(sr.get("year", 1), Some(ScalarValue::Int(2023)));
}
```
验证失败：`set_scalar` 方法不存在；scalars.col 仍为空 stub。
最小实现：`SegmentWriter` 增 `scalars: HashMap<String, (ScalarKind, Vec<ScalarValue>)>`；`set_scalar` 校验字段在 schema 且为 Scalar 类型 + kind 匹配，追加到列。finalize 写 scalars.col：magic+version+num_fields+{name_len+name+kind(1)+count(4 LE)+values}。Int 列写 i64 LE；Keyword 列写 len+bytes。
commit：`segment: implement scalars.col write/read with ScalarReader`。

### Task 2：Filter 编译（eq/in/gte/lte + AND）

**测试**：
```rust
#[test]
fn compile_filter_eq_keyword() {
    let (segments, scalars, tombstones) = setup_filter_corpus();
    let filter = Filter { fields: vec![("lang".into(), FilterCond::Eq(ScalarValue::Keyword("zh".into())))] };
    let bm = vane_core::filter::compile_filter(&filter, &schema(), &segments, &scalars, &tombstones).unwrap();
    // 仅 lang=zh 的 docid 在位图
    assert!(bm.contains(0));  // d1 lang=zh
    assert!(!bm.contains(1)); // d2 lang=en
}

#[test]
fn compile_filter_gte_int_and_keyword_in() {
    let (segments, scalars, tombstones) = setup_filter_corpus();
    let filter = Filter { fields: vec![
        ("year".into(), FilterCond::Gte(ScalarValue::Int(2024))),
        ("lang".into(), FilterCond::In(vec![ScalarValue::Keyword("zh".into()), ScalarValue::Keyword("ja".into())])),
    ]};
    let bm = vane_core::filter::compile_filter(&filter, &schema(), &segments, &scalars, &tombstones).unwrap();
    // year>=2024 AND lang in (zh,ja)
    assert!(bm.contains(0));  // d1 year=2024 lang=zh
    assert!(!bm.contains(1)); // d2 year=2023
}
```
最小实现：`compile_filter` 遍历 filter.fields，每字段读 ScalarReader 列，按 FilterCond 匹配 local_docid → 绝对 docid（+docid_base）入位图；多字段 AND（交集）。最后 AND NOT 各段 tombstone（tombstone 文档排除）。
commit：`filter: implement compile_filter with eq/in/gte/lte and tombstone exclusion`。

### Task 3：低选择率暴力回退

**测试**：
```rust
#[test]
fn should_fallback_brute_when_bitmap_small() {
    let mut bm = roaring::RoaringBitmap::new();
    bm.insert(1); bm.insert(2);  // cardinality=2
    assert!(vane_core::filter::should_fallback_brute(&bm, 10));  // 2 < 2*10
    let mut big = roaring::RoaringBitmap::new();
    for i in 0..100 { big.insert(i); }
    assert!(!vane_core::filter::should_fallback_brute(&big, 10));  // 100 >= 20
}
```
最小实现：`should_fallback_brute(bm, topk) = bm.cardinality() < (2 * topk as u64)`。
commit：`filter: add low-selectivity brute fallback predicate`。

### Task 4：api search 接入 filter + 回退

**测试**（`crates/vane-core/tests/pre_filter.rs`）：
```rust
use vane_core::api::*;
use vane_core::types::*;
use vane_core::vfs::memory::MemoryVfs;
use std::sync::Arc;

#[test]
fn search_with_filter_returns_only_matching() {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let schema = Schema::new(vec![
        ("lang".into(), FieldDef::Scalar { kind: ScalarKind::Keyword }),
        ("v".into(), FieldDef::Vector { dim: 2, metric: Metric::Cosine }),
    ]).unwrap();
    let col = db.collection("c", schema, CollectionOptions::default()).unwrap();
    col.add(&[
        Doc { id: "d1".into(), text: None, vector: Some(vec![1.0, 0.0]), meta: Some(hashmap![("lang".into(), ScalarValue::Keyword("zh".into()))]) },
        Doc { id: "d2".into(), text: None, vector: Some(vec![1.0, 0.0]), meta: Some(hashmap![("lang".into(), ScalarValue::Keyword("en".into()))]) },
    ]).unwrap();
    col.flush().unwrap();
    let hits = col.search(&SearchQuery {
        vector: Some(vec![1.0, 0.0]), top_k: 10, mode: SearchMode::Vector,
        filter: Some(Filter { fields: vec![("lang".into(), FilterCond::Eq(ScalarValue::Keyword("zh".into())))] }),
        ..Default::default()
    }).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "d1");
}

#[test]
fn search_filter_low_selectivity_uses_brute() {
    // 位图基数 < 2*topK → 暴力回退（100% 召回）
    // 构造 10 文档，filter 只匹配 1 个
    let (col, _db) = setup_10_docs();
    let hits = col.search(&SearchQuery {
        vector: Some(vec![1.0, 0.0]), top_k: 10, mode: SearchMode::Vector,
        filter: Some(Filter { fields: vec![("year".into(), FilterCond::Gte(ScalarValue::Int(2030)))] }),
        ..Default::default()
    }).unwrap();
    assert!(hits.iter().all(|h| /* year>=2030 */));
}
```
最小实现：`Collection::search` 移除 M0 的 `filter.is_some() → InvalidArg`；改为若 `query.filter.is_some()` → `compile_filter` 产出位图 → 传给 vector/text 路的 search 调用。vector 路：若 `should_fallback_brute` → `brute_search(filter=Some(&bm))`；否则 `HnswReader::search(filter=Some(&bm))`。BufferedDoc.meta 的 scalar 字段在 flush 时经 `SegmentWriter::set_scalar` 写入（collection.rs flush 已有 meta → stored_json，新增 meta → set_scalar）。
commit：`api: integrate pre-filter with adaptive brute fallback`。

### Task 5：tombstone 并入 filter（消费 02 产物）

**测试**：
```rust
#[test]
fn filter_excludes_tombstoned_docs() {
    let (col, _db) = setup_with_docs();
    col.delete(&["d2".into()]).unwrap();
    // search with filter that would match d2，验证 d2 不出现
    let hits = col.search(&SearchQuery {
        vector: Some(vec![1.0, 0.0]), top_k: 10, mode: SearchMode::Vector,
        filter: Some(Filter { fields: vec![("lang".into(), FilterCond::In(vec![
            ScalarValue::Keyword("zh".into()), ScalarValue::Keyword("en".into())]))] }),
        ..Default::default()
    }).unwrap();
    assert!(!hits.iter().any(|h| h.id == "d2"));
}
```
最小实现：`compile_filter` 末尾对每段 `bm.and_not(&segment_tombstones)`。无 filter 时 search 也要应用 tombstone（构造全量位图 AND NOT tombstone，或各 search 调用传 tombstone 作为 filter——M0 brute_search/InvertedIndexReader::search/HnswReader::search 都接受 filter 参数，传 `Some(&tombstone_bm)` 即可）。
commit：`filter: exclude tombstoned docs from search results`。

## 验收标准

- **SPEC §8.3**：filter 编译为 roaring 位图，传入 HNSW 遍历（01 HnswReader::search filter）+ WAND 推进（M0 InvertedIndexReader::search filter）；位图基数 <2×topK 向量路切暴力。
- **SPEC §3.1**：标量条件 eq/in/gte/lte，字段间 AND，不支持 OR/NOT（M0-M2）。
- **SPEC §6.2**：scalars.col 列式块按字段分区。
- **M0 占位对接**：SearchQuery.filter 从 M0 reject（InvalidArg）变为实装。
- **风险 #7**（低选择率过滤召回崩溃）：pre-filter 位图进遍历 + 暴力回退双保险。

## 前置依赖

- M0 segment/bm25/vector/api（已合并）。
- 01-hnsw（HnswReader::search filter 参数）。
- 02-tombstone-merge（tombstone 并入 filter，Task 5）。

## Global Constraints

core 禁 std::fs；不动 M0 冻结签名（SegmentWriter::add_doc 不改，新增 set_scalar 扩展）；scalars.col 格式 magic+version 头（与 M0 一致）；filter 编译在 core（binding 薄壳）；位图用 roaring（M0 已依赖）。
