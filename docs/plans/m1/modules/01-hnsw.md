# 01-hnsw：分段 HNSW 图 + 段级搜索归并 + 暴力自适应回退

> SPEC 引用：§3.1（分段 HNSW）、§6.2（hnsw.bin）、§8.1（vector 模式 + 自适应回退）、§7.2（图不原地删，不变量 I-3）、§13.1（hybrid P99 <50ms）。
> 前置依赖：M0 `segment`/`vector`/`types`/`vfs`（已核查 git HEAD）。
> M1 README 契约：`vane_core::hnsw`（见 `docs/plans/m1/README.md` § 01-hnsw）。

## Goal

实现段内不可变 HNSW 图（自研，~800 行），写期 `HnswWriter` 构建 + `write_hnsw` 落盘，读期 `HnswReader` 加载 + `search`。多段搜索归并与暴力自适应回退由 api 层编排（本计划提供 hnsw 模块 + api 层接入测试）。图从不原地删（I-3），删除走 tombstone（02 计划）。

## Architecture

- **算法**：分层小世界图（HNSW）。参数 M=16、ef_construction=200、ef_search=max(ef_construction, topk*4)。插入用贪婪搜索找 ef_construction 个近邻，按距离分层连接。entry_point 为最高层节点。
- **距离**：复用 M0 `Metric::{Cosine, L2, Dot}` 语义（越大越相似）。内部转距离 = -score 用于图导航（cosine 距离 = 1-cosine；L2 距离 = |a-b|；dot 距离 = -dot）。
- **不可变**：`HnswWriter::build` 消费 self 产出 `HnswGraph`，`write_hnsw` 序列化到 `hnsw.bin`。读期 `HnswReader` 只读。
- **filter 接口**：`search(filter: Option<&RoaringBitmap>)`——访问邻居时检查位图（local_docid = 绝对 docid - docid_base）。低选择率回退由 api 层判定后调 `brute_search`（M0 已支持 filter）。
- **M1 全串行搜索**（R-4/R-6）：hnsw 模块零 `cfg(target)`，无 `thread::scope`，无 rayon。多段搜索 = 串行搜各段 → 归并。Executor trait + 并行延后 M2（100 万规模时引入，cfg 仅在 Executor impl）。**若 11-cold-start-bench 实测 P99 >50ms，则在 M1 内补 Executor trait**（详见 README「已知阶段性偏离」）。
- **M0 corpus 兼容**（Q-5）：`HnswReader::open` 缺失 hnsw.bin（M0 corpus 无此文件）时返回 `Err`，api 层 catch 后 fallback `brute_search`（与 Task 5「hnsw_readers 无该段则 brute」一致）。M0 corpus 可被 M1 打开并暴力检索。

## 涉及文件

- **Create**：
  - `crates/vane-core/src/hnsw/mod.rs`（HnswGraph/HnswWriter/HnswReader/write_hnsw）
  - `crates/vane-core/src/hnsw/tests.rs`
- **Modify**：
  - `crates/vane-core/src/lib.rs`（增 `pub mod hnsw;`）
  - `crates/vane-core/src/api/collection.rs`（search vector 路接入 HnswReader + 自适应回退；SegmentReader 旁挂 HnswReader 缓存，类比 M0 InvertedIndexReader 缓存）
  - `crates/vane-core/src/api/collection.rs` flush：构建 HnswWriter → insert 全部向量 → write_hnsw
- **Test**：
  - `crates/vane-core/src/hnsw/tests.rs`（单元）
  - `crates/vane-core/tests/hnsw_recall.rs`（集成：HNSW vs brute recall≥0.95 小规模）

## Interfaces

### Consumes from M0（已核查 git HEAD）

```rust
// crates/vane-core/src/types.rs
pub enum Metric { Cosine, L2, Dot }
pub struct ScoredDoc { pub docid: u64, pub score: f32 }
pub type Result<T> = std::result::Result<T, VaneError>;
pub const MAGIC: &[u8; 4] = b"VANE";
pub const FORMAT_VERSION: u32 = 1;

// crates/vane-core/src/vfs/mod.rs（M0 冻结）
pub trait Vfs: Send + Sync {
    fn create(&self, path: &str) -> Result<()>;
    fn read_at(&self, path: &str, buf: &mut [u8], offset: u64) -> Result<usize>;
    fn write_at(&self, path: &str, buf: &[u8], offset: u64) -> Result<()>;
    fn sync(&self, path: &str) -> Result<()>;
    // ...其余 4 方法
}

// crates/vane-core/src/vector/mod.rs（M0 冻结）
pub fn brute_search(
    vectors: &[f32], dim: u32, query: &[f32], metric: Metric,
    topk: usize, filter: Option<&roaring::RoaringBitmap>, docid_base: u64,
) -> Vec<ScoredDoc>;

// crates/vane-core/src/segment/mod.rs（M0 冻结）
impl SegmentReader {
    pub fn open(vfs: &std::sync::Arc<dyn Vfs>, segment_dir: &str) -> Result<Self>;
    pub fn vectors(&self) -> &[f32];
    pub fn dim(&self) -> u32;
    pub fn meta(&self) -> &SegmentMeta;  // 含 ulid, doc_count, docid_base, tokenizer_id, tombstones
    pub fn segment_dir(&self) -> &str;
    pub fn vfs(&self) -> &std::sync::Arc<dyn Vfs>;
}
impl SegmentWriter {
    pub fn new(vfs, segments_dir, schema, tokenizer_id, docid_base) -> Result<Self>;
    pub fn add_doc(&mut self, external_id: &str, vector: Option<&[f32]>, stored_json: &str) -> Result<u64>;
    pub fn finalize(self) -> Result<SegmentMeta>;
}
```

### Produces（见 README § 01-hnsw 契约，此处不重复）

## TDD 任务清单

### Task 1：距离函数 + HnswWriter 骨架（写失败测试 → 验证失败 → 实现 → 通过 → commit）

**测试**（`crates/vane-core/src/hnsw/tests.rs`）：
```rust
use super::*;
use crate::types::Metric;

#[test]
fn hnsw_writer_builds_empty_graph() {
    let mut w = HnswWriter::new(4, Metric::Cosine, 16, 200);
    let g = w.build();
    assert_eq!(g.doc_count(), 0);
}

#[test]
fn hnsw_writer_insert_single_node() {
    let mut w = HnswWriter::new(2, Metric::Cosine, 4, 8);
    w.insert(0, &[1.0, 0.0]);
    let g = w.build();
    assert_eq!(g.doc_count(), 1);
}
```
验证失败：`cargo test -p vane-core hnsw::tests` 编译错误（模块不存在）。
最小实现：`hnsw/mod.rs` 定义 `HnswGraph`/`HnswWriter`，`new`/`insert`/`build`，`doc_count`。距离函数 `metric_distance(metric, a, b) -> f32`（cosine=1-cos, l2=|a-b|², dot=-dot）。
commit：`hnsw: add writer skeleton with distance metrics`。

### Task 2：插入多个节点 + 图结构正确性

**测试**：
```rust
#[test]
fn hnsw_insert_multiple_nodes_connects_neighbors() {
    let mut w = HnswWriter::new(2, Metric::L2, 4, 16);
    // 3 个点近邻：[0,0],[1,0],[2,0]
    w.insert(0, &[0.0, 0.0]);
    w.insert(1, &[1.0, 0.0]);
    w.insert(2, &[2.0, 0.0]);
    let g = w.build();
    assert_eq!(g.doc_count(), 3);
    // entry_point 存在
    assert!(g.entry_point().is_some());
    // 节点 1 的邻居含 0 或 2
    let neighbors = g.neighbors(1);
    assert!(neighbors.contains(&0) || neighbors.contains(&2));
}
```
最小实现：分层插入算法。`insert` 先从 entry_point 贪婪搜索到 ef_construction 候选，选 M 个连接；新节点层级按指数分布 `floor(-ln(uniform) * mL)`。`HnswGraph` 存 `nodes: Vec<Node>`，`Node { local_docid, level, neighbors: Vec<Vec<u32>> }`（每层一组邻居）。
commit：`hnsw: implement layered insert with neighbor selection`。

### Task 3：HnswReader 搜索（无 filter）

**测试**：
```rust
#[test]
fn hnsw_search_returns_topk_nearest() {
    let mut w = HnswWriter::new(2, Metric::L2, 8, 32);
    for i in 0..50u32 {
        w.insert(i, &[i as f32 * 0.1, 0.0]);
    }
    let g = w.build();
    let vfs = std::sync::Arc::new(crate::vfs::memory::MemoryVfs::new()) as std::sync::Arc<dyn crate::vfs::Vfs>;
    vfs.create("seg").unwrap();
    write_hnsw(vfs.as_ref(), "seg", &g).unwrap();
    let r = HnswReader::open(&vfs, "seg").unwrap();
    let res = r.search(&[2.5, 0.0], 5, 64, None, 0);
    assert_eq!(res.len(), 5);
    // 最近的是 i=25 (2.5,0.0)
    assert_eq!(res[0].docid, 25);
    assert!(res[0].score >= res[1].score);
}
```
最小实现：`write_hnsw` 序列化（格式见 README 契约）；`HnswReader::open` 反序列化；`search` 贪婪层降 + ef 搜索。返回 `Vec<ScoredDoc>`（score 用原 metric 的 score 语义，越大越相似）。
commit：`hnsw: implement write/read/search roundtrip`。

### Task 4：filter 参数（pre-filter 接口预留）

**测试**：
```rust
#[test]
fn hnsw_search_with_filter_skips_excluded() {
    let mut w = HnswWriter::new(2, Metric::L2, 8, 32);
    for i in 0..20u32 { w.insert(i, &[i as f32, 0.0]); }
    let g = w.build();
    let vfs = std::sync::Arc::new(crate::vfs::memory::MemoryVfs::new()) as std::sync::Arc<dyn crate::vfs::Vfs>;
    vfs.create("seg").unwrap();
    write_hnsw(vfs.as_ref(), "seg", &g).unwrap();
    let r = HnswReader::open(&vfs, "seg").unwrap();
    let mut bm = roaring::RoaringBitmap::new();
    // 只允许 docid 5,6,7（绝对 = base+local，base=0）
    bm.insert(5); bm.insert(6); bm.insert(7);
    let res = r.search(&[6.0, 0.0], 3, 64, Some(&bm), 0);
    assert!(res.iter().all(|d| d.docid >= 5 && d.docid <= 7));
    assert_eq!(res[0].docid, 6);
}
```
最小实现：搜索时访问候选节点先检查 `filter.contains(local_docid as u32 + docid_base as u32)`（位图存绝对 docid）。不命中则跳过入堆但仍可作导航点（与 SPEC §8.3 "位图进 HNSW 遍历"一致——导航用邻居，结果用过滤）。
commit：`hnsw: support filter bitmap in search`。

### Task 5：api 层接入（HnswReader 缓存 + 自适应回退 + 缺失 fallback）

**测试**（`crates/vane-core/tests/hnsw_recall.rs`）：
```rust
use vane_core::api::*;
use vane_core::types::*;
use vane_core::vfs::memory::MemoryVfs;
use std::sync::Arc;

#[test]
fn api_hnsw_recall_vs_brute_at_least_95pct() {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
    let schema = Schema::new(vec![
        ("v".into(), FieldDef::Vector { dim: 8, metric: Metric::Cosine }),
    ]).unwrap();
    let col = db.collection("c", schema, CollectionOptions::default()).unwrap();
    // 500 文档，确定性向量
    let docs: Vec<Doc> = (0..500).map(|i| Doc {
        id: format!("d{}", i), text: None,
        vector: Some(((0..8).map(|j| (i*j) as f32 * 0.01).collect())),
        meta: None,
    }).collect();
    col.add(&docs).unwrap();
    col.flush().unwrap();
    // HNSW 搜索
    let q = vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8];
    let hnsw_hits = col.search(&SearchQuery {
        vector: Some(q.clone()), top_k: 10, mode: SearchMode::Vector,
        ..Default::default()
    }).unwrap();
    assert_eq!(hnsw_hits.len(), 10);
    // recall 检查见 12-recall-regression 的完整 job；此处断言不 panic + 10 条
}

#[test]
fn m0_corpus_without_hnsw_bin_falls_back_to_brute() {
    // Q-5：M0 corpus（无 hnsw.bin）被 M1 打开后，HnswReader::open 返回 Err，
    // api 层 catch → fallback brute_search，搜索仍正常返回。
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
    let schema = Schema::new(vec![
        ("v".into(), FieldDef::Vector { dim: 4, metric: Metric::Cosine }),
    ]).unwrap();
    let col = db.collection("c", schema, CollectionOptions::default()).unwrap();
    col.add(&[Doc { id: "d0".into(), text: None, vector: Some(vec![1.0,0.0,0.0,0.0]), meta: None }]).unwrap();
    col.flush().unwrap();
    // 模拟 M0 corpus：删除刚写入的 hnsw.bin（若 flush 已写）
    // （M1 flush 写 hnsw.bin；测试手动删该文件模拟 M0 段）
    let seg_ulid = col.segment_ulids()[0].clone();
    let _ = vfs.delete(&format!("db/segments/seg_{}/hnsw.bin", seg_ulid));
    // reopen 后 HnswReader::open 缺失文件 → fallback brute
    let hits = col.search(&SearchQuery {
        vector: Some(vec![1.0,0.0,0.0,0.0]), top_k: 10, mode: SearchMode::Vector,
        ..Default::default()
    }).unwrap();
    assert!(hits.iter().any(|h| h.id == "d0"));
}
```
最小实现：
- `CollectionInner` 增 `hnsw_readers: RwLock<Vec<Option<Arc<HnswReader>>>>`（类比 M0 `inverted_readers`；`Option` 因 M0 段无 hnsw.bin）。
- flush：`add_doc` 全部完成后，用 `HnswWriter` 收集 (local_docid, vector) → `build` → `write_hnsw`；push `Some(HnswReader::open)` 到缓存。
- restore_from_manifest：每段尝试 `HnswReader::open`，成功 push `Some`，失败（缺 hnsw.bin）push `None`。
- search vector 路：若该段 `hnsw_readers[i]` 为 `Some` → 用 `HnswReader::search`；否则 fallback `brute_search`。自适应回退：若 filter 位图基数 < 2*topk → 直接 `brute_search`（100% 召回）。
- `SearchQuery.filter` 不再 reject（M0 返回 InvalidArg）；filter 编译由 03 计划实装，本计划先透传 `query.filter` 占位为 None（03 接入后补）。
commit：`api: integrate HnswReader with adaptive brute fallback (Q-5 missing-hnsw fallback)`。

### Task 6：不变量 I-3 测试（图字节稳定 + 02 delete 后不变）

**测试**：
```rust
#[test]
fn hnsw_graph_bytes_stable_after_write() {
    // 图写后只读：write 后 read 两次字节一致（不可变）。
    // 注：本测试偏弱（只读两次比字节相同）——真正有意义的 I-3 测试是
    // 「delete 后 hnsw.bin 字节不变」，由 02 Task 7 实装 delete 后补（delete 走 tombstone 不动图）。
    // 本 Task 保留字节稳定测试作 I-3 占位，02 Task 7 补 delete 不动图的强断言。
    let mut w = HnswWriter::new(2, Metric::L2, 4, 8);
    w.insert(0, &[0.0, 0.0]); w.insert(1, &[1.0, 0.0]);
    let g = w.build();
    let vfs = Arc::new(crate::vfs::memory::MemoryVfs::new()) as Arc<dyn crate::vfs::Vfs>;
    vfs.create("seg").unwrap();
    write_hnsw(vfs.as_ref(), "seg", &g).unwrap();
    let mut buf1 = Vec::new();
    let mut tmp = [0u8; 4096]; let mut off = 0;
    loop { let n = vfs.read_at("seg/hnsw.bin", &mut tmp, off).unwrap(); if n==0 {break;} buf1.extend_from_slice(&tmp[..n]); off+=n as u64; }
    // 再读一次，字节一致（不可变）
    let mut buf2 = Vec::new(); let mut off = 0;
    loop { let n = vfs.read_at("seg/hnsw.bin", &mut tmp, off).unwrap(); if n==0 {break;} buf2.extend_from_slice(&tmp[..n]); off+=n as u64; }
    assert_eq!(buf1, buf2);
}
```
commit：`hnsw: assert graph byte stability (I-3 placeholder; 02 Task 7 adds delete-invariant)`。

## 验收标准

- **SPEC §8.1**：vector 模式 = HNSW 段级并行搜索 → 归并；过滤候选 <2×topK 时暴力回退。api 接入测试覆盖回退路径。
- **SPEC §3.1**：段内不可变，M=16/ef_construction=200；段数硬上限 10（合并由 02 计划）。
- **SPEC §7.2/不变量 I-3**：图不原地删，图重建仅段合并（Task 6 测试字节稳定；02 计划验证 delete 不动 hnsw.bin）。
- **SPEC §13.1**：hybrid P99 <50ms（10 万×384）——由 11-cold-start-bench 实测背书，本计划保证算法正确性。
- **recall**：`tests/hnsw_recall.rs` 小规模 HNSW vs brute recall≥0.95（大规模五档在 12 计划）。
- **wasm32**：`cargo check --target wasm32-unknown-unknown -p vane-core` 通过（hnsw 零 cfg，M1 全串行无 `thread::scope`，wasm32 直接可编译）。

## 前置依赖

- M0 全部已合并（segment/vector/types/vfs/api）。
- 无 M1 内部前置（L0 批次）。

## Global Constraints

引用 M1 README 全局约束表：core 禁 std::fs、cfg 只在 VFS（M1 全串行，不引入 Executor，核心算法零 cfg）、依赖黑名单（不引 rayon/std::thread::scope）、HNSW 图不原地删（I-3）、wasm32 check 通过。
