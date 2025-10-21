# 12-recall-regression：recall@10≥0.95 五档选择率回归 job

> SPEC 引用：§13.2-1（hybrid recall@10 ≥0.95，相对暴力双路+RRF 基线，五档选择率 0.1%/1%/10%/50%/99%）、§8.4（召回回归覆盖）。
> 前置依赖：01-hnsw（HNSW 搜索）；03-pre-filter（filter 五档选择率）。
> M1 README 契约：`crates/vane-core/tests/recall_regression.rs`。

## Goal

真实回归 job：HNSW 搜索结果相对"暴力双路+RRF"基线的 recall@10 ≥0.95，覆盖 0.1%/1%/10%/50%/99% 五档过滤选择率。替换 M0 `tests/recall.rs`（trivially recall=1.0）。

## Architecture

- **基线**：暴力双路+RRF = M0 `brute_search` + `InvertedIndexReader::search` + `rrf_fuse`（无 HNSW，100% 召回）。
- **被测**：HNSW 搜索（api search vector 路 + text 路 + hybrid 融合）。
- **recall 口径**：`recall@10 = |HNSW_top10 ∩ Baseline_top10| / 10`（按 external_id 比对）。
- **五档选择率**：filter 位图基数占总文档的 0.1%/1%/10%/50%/99%。用 `ScalarValue::Keyword` 随机分布字段构造各档。
- **三模式**：vector / text / hybrid 各跑五档 = 15 组。hybrid 用 RRF（k=60）。
- **fixture**：确定性 1000 文档 × 128 维（小规模保证 CI 快），向量用确定性伪随机。大规模（10 万）在 11-cold-start-bench 的 bench 中跑（非门禁）。

## 涉及文件

- **Create**：
  - `crates/vane-core/tests/recall_regression.rs`
  - `crates/vane-core/tests/recall_fixture.rs`（fixture 生成辅助）
- **Modify**：
  - `crates/vane-core/tests/recall.rs`（M0 trivially 1.0——保留或删除；**裁决**：保留 M0 recall.rs 作冒烟，新增 recall_regression.rs 作真实门禁，10-ci-m1 跑后者）
  - `.github/workflows/ci.yml`（recall job 改跑 `--test recall_regression`，M0 recall.rs 保留作冒烟或删）

## Interfaces

### Consumes from M0 + 01/03

```rust
// M0
pub fn brute_search(vectors, dim, query, metric, topk, filter, docid_base) -> Vec<ScoredDoc>;
impl InvertedIndexReader { pub fn search(&self, query_tokens, topk, filter: Option<&RoaringBitmap>) -> Vec<ScoredDoc>; }
pub fn rrf_fuse(paths: &[Vec<FusionCandidate>], k: u32) -> Vec<ScoredDoc>;
impl Collection { pub fn search(&self, query: &SearchQuery) -> Result<Vec<Hit>>; }
// 01 HnswReader::search（经 api search 间接用）
// 03 Filter/compile_filter（五档选择率）
```

## TDD 任务清单

### Task 1：fixture 生成（1000 文档 × 128 维 + 五档标量）

**测试**（`crates/vane-core/tests/recall_regression.rs`）：
```rust
use vane_core::api::*;
use vane_core::types::*;
use vane_core::vfs::memory::MemoryVfs;
use std::sync::Arc;

fn build_recall_fixture() -> (Arc<MemoryVfs>, Db, Vec<Vec<f32>>) {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs.clone(), "recall", OpenOptions::default()).unwrap();
    let schema = Schema::new(vec![
        ("body".into(), FieldDef::Text),
        ("cat".into(), FieldDef::Scalar { kind: ScalarKind::Keyword }),  // 五档选择率用
        ("v".into(), FieldDef::Vector { dim: 128, metric: Metric::Cosine }),
    ]).unwrap();
    let col = db.collection("docs", schema, CollectionOptions::default()).unwrap();
    let mut queries = Vec::new();
    // 1000 文档，确定性向量 + cat 分布
    let docs: Vec<Doc> = (0..1000).map(|i| {
        let vector: Vec<f32> = (0..128).map(|j| ((i * 31 + j * 7) as u32).wrapping_mul(2654435761) as f32 / u32::MAX as f32).collect();
        Doc {
            id: format!("d{}", i),
            text: Some(format!("doc {} term{}", i, i % 50)),
            vector: Some(vector.clone()),
            meta: Some(std::collections::HashMap::from([("cat".into(), ScalarValue::Keyword(format!("c{}", i % 1000)))])),
        }
    }).collect();
    col.add(&docs).unwrap();
    col.flush().unwrap();
    // 10 个查询向量
    for i in 0..10 {
        queries.push((0..128).map(|j| ((i * 13 + j * 3) as u32).wrapping_mul(40503) as f32 / u32::MAX as f32).collect());
    }
    (vfs, db, queries)
}
```
commit：`recall: add 1000-doc fixture with 5-tier scalar distribution`。

### Task 2：基线计算（暴力双路+RRF）— search_brute_baseline 实装

**测试**：
```rust
#[test]
fn search_brute_baseline_returns_topk_without_hnsw() {
    let (vfs, db, queries) = build_recall_fixture();
    let col = db.collection("docs", test_schema(), CollectionOptions::default()).unwrap();
    let qv = &queries[0];
    // 基线 = 暴力双路 + RRF，绕过 HNSW
    let baseline = col.search_brute_baseline(&SearchQuery {
        vector: Some(qv.clone()), text: Some("term0".into()),
        top_k: 10, mode: SearchMode::Hybrid,
        ..Default::default()
    }).unwrap();
    assert_eq!(baseline.len(), 10, "baseline must return topK=10");
    // 基线结果按 RRF 分排序
    assert!(baseline[0].score >= baseline[1].score || baseline[0].score.is_nan());
    // 与 api search（HNSW）对比：recall 应 ≥0.95（在 Task 3 五档断言）
}
```
**裁决**（M1 修订：删除占位 panic，改为真实测试）：`Collection` 增 `pub fn search_brute_baseline(&self, query: &SearchQuery) -> Result<Vec<Hit>>`（测试 + bench 辅助，非 IDL 对外，标注 `#[doc(hidden)]`）。内部直接调 `brute_search` + `InvertedIndexReader::search` + `rrf_fuse`，**跳过 HnswReader**（绕过段快照的 hnsw_readers，直接读 vectors/inverted）。访问 `CollectionInner` 段快照（`pub(crate)`，同 crate 内可访问）。
最小实现：
- `api/collection.rs` 增 `#[doc(hidden)] pub fn search_brute_baseline(&self, query: &SearchQuery) -> Result<Vec<Hit>>`。
- 逻辑：复用 `search` 的 mode 推断 + dim 校验 + 回填 Hit 逻辑，但 vector 路强制 `brute_search`（不走 HnswReader），text 路用 `InvertedIndexReader::search`，hybrid 用 `rrf_fuse`。
- filter 参数透传（03 计划实装 compile_filter 后接入；本 Task 先传 None）。
commit：`api: add search_brute_baseline test helper (real impl, no unimplemented)`。

### Task 3：五档选择率 recall 断言

**测试**：
```rust
#[test]
fn recall_hybrid_5_selectivity_tiers() {
    let (vfs, db, queries) = build_recall_fixture();
    let col = db.collection("docs", test_schema(), CollectionOptions::default()).unwrap();
    let tiers = [0.001, 0.01, 0.1, 0.5, 0.99];  // 0.1%/1%/10%/50%/99%
    for &tier in &tiers {
        // 构造 filter 使选择率≈tier
        let allowed_cats = pick_cats_for_tier(tier);
        let filter = Filter { fields: vec![("cat".into(), FilterCond::In(
            allowed_cats.iter().map(|c| ScalarValue::Keyword(c.clone())).collect()))] };
        for qv in &queries {
            let baseline = col.search_brute_baseline(&SearchQuery {
                vector: Some(qv.clone()), text: Some("term".into()),
                top_k: 10, mode: SearchMode::Hybrid,
                filter: Some(filter.clone()), ..Default::default()
            }).unwrap();
            let hnsw = col.search(&SearchQuery {
                vector: Some(qv.clone()), text: Some("term".into()),
                top_k: 10, mode: SearchMode::Hybrid,
                filter: Some(filter.clone()), ..Default::default()
            }).unwrap();
            let recall = recall_at_10(&hnsw, &baseline);
            assert!(recall >= 0.95,
                "recall {} <0.95 at tier {} for query {:?}", recall, tier, qv);
        }
    }
}

fn recall_at_10(hnsw: &[Hit], baseline: &[Hit]) -> f32 {
    let base_ids: std::collections::HashSet<_> = baseline.iter().take(10).map(|h| h.id.clone()).collect();
    let hit = hnsw.iter().take(10).filter(|h| base_ids.contains(&h.id)).count();
    hit as f32 / 10.0
}
```
最小实现：`pick_cats_for_tier` 按 tier 从 1000 个 cat 选 round(1000*tier) 个。`search_brute_baseline` 实装。vector/text/hybrid 三模式各跑一遍（三个 #[test]）。
commit：`recall: add 5-tier selectivity regression (vector/text/hybrid)`。

### Task 4：低选择率暴力回退验证（0.1% 档）

**测试**：
```rust
#[test]
fn recall_low_selectivity_uses_brute_fallback() {
    // 0.1% 档：位图基数 ~1 < 2*topK=20 → api 应走暴力回退 → recall=1.0
    let (vfs, db, queries) = build_recall_fixture();
    let col = db.collection("docs", test_schema(), CollectionOptions::default()).unwrap();
    let filter = Filter { fields: vec![("cat".into(), FilterCond::Eq(ScalarValue::Keyword("c0".into())))] };
    let qv = &queries[0];
    let baseline = col.search_brute_baseline(&SearchQuery {
        vector: Some(qv.clone()), top_k: 10, mode: SearchMode::Vector,
        filter: Some(filter.clone()), ..Default::default()
    }).unwrap();
    let hnsw = col.search(&SearchQuery {
        vector: Some(qv.clone()), top_k: 10, mode: SearchMode::Vector,
        filter: Some(filter.clone()), ..Default::default()
    }).unwrap();
    // 0.1% 档候选<2*topK → 暴力回退 → recall 应=1.0
    assert_eq!(recall_at_10(&hnsw, &baseline), 1.0);
}
```
commit：`recall: assert brute fallback at 0.1% selectivity (§8.3)`。

## 验收标准

- **SPEC §13.2-1**：hybrid recall@10 ≥0.95，五档选择率（0.1%/1%/10%/50%/99%），CI 硬门禁。
- **SPEC §8.4**：召回回归覆盖五档。
- **SPEC §8.1**：低选择率（<2×topK）暴力回退 100% 召回（Task 4）。
- **基线口径**：相对"暴力双路+RRF"基线（M0 brute_search + InvertedIndexReader::search + rrf_fuse）。
- **三模式**：vector/text/hybrid 各五档。

## 前置依赖

- 01-hnsw（HNSW 搜索被测）。
- 03-pre-filter（filter 五档选择率）。
- M0 brute_search/InvertedIndexReader::search/rrf_fuse（基线）。

## Global Constraints

recall 门禁硬卡（<0.95 fail）；fixture 确定性（无随机种子依赖）；`search_brute_baseline` 标注 `#[doc(hidden)]` 非对外 IDL；大规模（10 万）在 11-cold-start-bench 跑非门禁。
