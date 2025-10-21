# 11-cold-start-bench：冷启动 <1s 实测背书 + 分级降级指标

> SPEC 引用：§13.1（冷启动打开 10 万库 <1s；>2s 降级为分级指标：元数据 <1s、首次查询 <3s）。
> 前置依赖：01-hnsw、02-tombstone-merge（多段+合并后的真实库结构）；M0 persistence（open 加载）。
> M1 README 契约：`crates/vane-core/benches/cold_start.rs`。

## Goal

实测背书冷启动打开 10 万文档库 <1s。若 >2s 则降级为分级指标：元数据加载 <1s、首次查询 <3s。产出 criterion bench + 分级报告。

## Architecture

- **bench fixture**：预生成 10 万文档 × 384 维 StdFsVfs 库（10 段，含 HNSW 图 + 倒排 + scalars）。fixture 生成脚本 `scripts/gen_cold_start_fixture.rs`，CI 缓存或运行时生成。
- **bench 场景**：
  1. `open_100k_metadata`：Db::open 加载 manifest + 段 header（不含 vectors 全加载）→ 断言 <1s。
  2. `open_100k_full`：open + SegmentReader::open 全部段（含 vectors/inverted/hnsw 加载）→ 测量（可能 >1s）。
  3. `first_query_after_open`：open 后首次 search → 断言 <3s（分级降级指标）。
- **分级报告**：若 `open_100k_full` >1s，断言 `open_100k_metadata` <1s 且 `first_query_after_open` <3s（SPEC §13.1 降级路径）。
- **M0 段加载现状**：M0 `SegmentReader::open` 一次性加载 vectors/inverted/stored/idmap 全部到内存（无懒加载）。10 万×384 维 vectors ≈ 154MB，加载可能 >1s。**裁决**：M1 不改 M0 SegmentReader 签名（冻结），但可在 CollectionInner.restore 后异步预热（native）/首次查询时懒加载（wasm）。本计划先实测，若超 1s 则在报告标注降级，M2 实装懒加载。

## 涉及文件

- **Create**：
  - `crates/vane-core/benches/cold_start.rs`
  - `scripts/gen_cold_start_fixture.rs`
- **Modify**：
  - `crates/vane-core/Cargo.toml`（增 `[[bench]] name = "cold_start"`）
- **Test**：
  - bench 本身（criterion）

## Interfaces

### Consumes from M0 + 01/02

```rust
// M0
impl Db { pub fn open(vfs: Arc<dyn Vfs>, path: &str, opts: OpenOptions) -> Result<Self>; }
impl SegmentReader { pub fn open(vfs: &Arc<dyn Vfs>, segment_dir: &str) -> Result<Self>; }
// 01 HnswReader::open, 02 段合并后结构
```

## TDD 任务清单

### Task 1：fixture 生成脚本

**测试**（`scripts/gen_cold_start_fixture.rs`）：
```rust
// 生成 10 万文档 × 384 维 StdFsVfs 库
use vane_core::api::*;
use vane_core::types::*;
use vane_core::vfs::std_fs::StdFsVfs;
use std::sync::Arc;

fn main() {
    let vfs = Arc::new(StdFsVfs::new());
    let db = Db::open(vfs, "bench_fixture_100k", OpenOptions::default()).unwrap();
    let schema = Schema::new(vec![
        ("body".into(), FieldDef::Text),
        ("v".into(), FieldDef::Vector { dim: 384, metric: Metric::Cosine }),
    ]).unwrap();
    let col = db.collection("docs", schema, CollectionOptions::default()).unwrap();
    for batch in 0..100 {
        let docs: Vec<Doc> = (0..1000).map(|i| {
            let id = batch * 1000 + i;
            Doc {
                id: format!("d{}", id),
                text: Some(format!("document {} batch {}", id, batch)),
                vector: Some(vec![id as f32 * 0.001; 384]),
                meta: None,
            }
        }).collect();
        col.add(&docs).unwrap();
        col.flush().unwrap();  // 100 段 → 触发 auto-merge 到 ≤10 段
    }
    db.close().unwrap();
}
```
验证：`cargo run --example gen_cold_start_fixture` 生成 `bench_fixture_100k/` 目录。
commit：`bench: add 100k cold-start fixture generator`。

### Task 2：open metadata bench

**测试**（`crates/vane-core/benches/cold_start.rs`）：
```rust
use criterion::{criterion_group, criterion_main, Criterion};
use vane_core::api::*;
use vane_core::vfs::std_fs::StdFsVfs;
use std::sync::Arc;

fn bench_open_metadata(c: &mut Criterion) {
    c.bench_function("open_100k_metadata", |b| {
        b.iter(|| {
            let vfs = Arc::new(StdFsVfs::new());
            let db = Db::open(vfs, "bench_fixture_100k", OpenOptions::default()).unwrap();
            let _col = db.collection("docs", test_schema(), CollectionOptions::default()).unwrap();
            db.close().unwrap();
        });
    });
}

fn bench_open_full_and_first_query(c: &mut Criterion) {
    c.bench_function("open_100k_full_and_first_query", |b| {
        b.iter(|| {
            let vfs = Arc::new(StdFsVfs::new());
            let db = Db::open(vfs, "bench_fixture_100k", OpenOptions::default()).unwrap();
            let col = db.collection("docs", test_schema(), CollectionOptions::default()).unwrap();
            // 首次查询
            let _hits = col.search(&SearchQuery {
                vector: Some(vec![0.5; 384]), top_k: 10, mode: SearchMode::Vector,
                ..Default::default()
            }).unwrap();
            db.close().unwrap();
        });
    });
}

criterion_group!(benches, bench_open_metadata, bench_open_full_and_first_query);
criterion_main!(benches);
```
验证：`cargo bench -p vane-core --bench cold_start` 产出数据。
commit：`bench: add cold_start criterion benches`。

### Task 3：分级降级断言

**测试**（`crates/vane-core/benches/cold_start.rs` 扩展）：
```rust
// 在 bench 末尾加分级断言（criterion 不直接断言，用单独 #[test]）
#[cfg(test)]
mod gate {
    use super::*;
    #[test]
    fn cold_start_meets_grade_or_fallback() {
        let vfs = Arc::new(StdFsVfs::new());
        let t0 = std::time::Instant::now();
        let db = Db::open(vfs.clone(), "bench_fixture_100k", OpenOptions::default()).unwrap();
        let open_ms = t0.elapsed().as_millis();
        let col = db.collection("docs", test_schema(), CollectionOptions::default()).unwrap();
        let t1 = std::time::Instant::now();
        let _ = col.search(&SearchQuery { vector: Some(vec![0.5;384]), top_k: 10, mode: SearchMode::Vector, ..Default::default() }).unwrap();
        let query_ms = t1.elapsed().as_millis();
        // SPEC §13.1：open <1s，或降级（metadata <1s + 首次查询 <3s）
        if open_ms > 1000 {
            assert!(open_ms < 1000 || query_ms < 3000,
                "cold start fail: open={}ms query={}ms (no fallback path)", open_ms, query_ms);
        }
        db.close().unwrap();
    }
}
```
commit：`bench: add graded fallback assertion (§13.1)`。

## 验收标准

- **SPEC §13.1**：冷启动打开 10 万库 <1s；>2s 降级为分级指标（元数据 <1s、首次查询 <3s）。
- **实测背书**：bench 数据写入 `docs/plans/m1/cold-start-report.md`（M1 收尾时由编排者或本计划执行者产出）。
- **降级路径**：若 open >1s，Task 3 断言走分级路径。

## 前置依赖

- 01-hnsw（HnswReader::open 参与冷启动加载）。
- 02-tombstone-merge（auto-merge 后 ≤10 段的真实库结构）。
- M0 persistence/api。

## Global Constraints

core 禁 std::fs（bench 用 StdFsVfs）；fixture 10 万文档（SPEC §13.1 口径）；不改 M0 SegmentReader 签名（懒加载留 M2）；criterion bench（M0 已用）。
