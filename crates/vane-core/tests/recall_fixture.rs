//! 12-recall-regression 共享 fixture：1000 文档 × 128 维 + cat 标量五档选择率分布。
//!
//! SPEC §13.2-1 / §8.4 召回回归基线口径 fixture：
//! - 1000 文档（CI 秒级可接受；大规模 10 万在 11-cold-start-bench 跑非门禁）。
//! - 128 维 cosine 向量，确定性伪随机（Knuth 乘法哈希，无种子依赖，结果可复现）。
//! - `cat` 标量字段每文档唯一值（`c0..c999`），便于 `FilterCond::In` 精确构造
//!   0.1%/1%/10%/50%/99% 五档选择率。
//! - `body` 文本字段含 `term{i%50}` 分布，使 text 路返回有意义 topK。
//!
//! 本文件作为 `recall_regression.rs` 的子模块（`mod recall_fixture;`），
//! 不被 Cargo 编译为独立 test 二进制。

use std::collections::HashMap;
use std::sync::Arc;

use vane_core::api::{Collection, Db, Doc, Filter, FilterCond, OpenOptions, ScalarValue};
use vane_core::types::{FieldDef, Metric, ScalarKind, Schema};
use vane_core::vfs::memory::MemoryVfs;

/// 文档数（1000：CI 秒级，足够体现 HNSW 召回）。
pub const DOC_COUNT: usize = 1000;
/// 向量维度。
pub const DIM: u32 = 128;
/// 查询向量数。
pub const N_QUERIES: usize = 10;
/// recall@10 硬门禁（SPEC §13.2-1）。
pub const RECALL_THRESHOLD: f32 = 0.95;
/// 五档选择率（SPEC §13.2-1：0.1%/1%/10%/50%/99%）。
pub const SELECTIVITY_TIERS: [f32; 5] = [0.001, 0.01, 0.1, 0.5, 0.99];

/// 回归 fixture schema：body(text) + cat(scalar keyword) + v(vector 128 cosine)。
pub fn recall_schema() -> Schema {
    Schema::new(vec![
        ("body".into(), FieldDef::Text),
        (
            "cat".into(),
            FieldDef::Scalar {
                kind: ScalarKind::Keyword,
            },
        ),
        (
            "v".into(),
            FieldDef::Vector {
                dim: DIM,
                metric: Metric::Cosine,
            },
        ),
    ])
    .unwrap()
}

/// 确定性伪随机向量（Knuth 乘法哈希，无 RNG 种子依赖）。
///
/// `seed_a` / `seed_b` 组合区分文档向量与查询向量，避免线性相关。
/// 输出落在 [0, 1]，保证可复现。
pub fn deterministic_vector(seed_a: u32, seed_b: u32) -> Vec<f32> {
    (0..DIM)
        .map(|j| {
            let h = seed_a
                .wrapping_add(j.wrapping_mul(7))
                .wrapping_mul(2654435761)
                .wrapping_add(seed_b.wrapping_mul(40503));
            h as f32 / u32::MAX as f32
        })
        .collect()
}

/// 构造回归 fixture：1000 文档 + 10 查询向量。
///
/// 返回 `(vfs, db, col, queries)`。`db`/`col` 供测试调用 `search` / `search_brute_baseline`。
pub fn build_recall_fixture() -> (Arc<MemoryVfs>, Db, Collection, Vec<Vec<f32>>) {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs.clone(), "recall", OpenOptions::default()).unwrap();
    let col = db
        .collection("docs", recall_schema(), Default::default())
        .unwrap();
    let docs: Vec<Doc> = (0..DOC_COUNT)
        .map(|i| {
            let vector = deterministic_vector(i as u32 * 31, 0);
            let mut meta = HashMap::new();
            // cat 每文档唯一值，便于 In 精确选档。
            meta.insert("cat".into(), ScalarValue::Keyword(format!("c{}", i)));
            Doc {
                id: format!("d{}", i),
                text: Some(format!("doc {} term{}", i, i % 50)),
                vector: Some(vector),
                meta: Some(meta),
            }
        })
        .collect();
    col.add(&docs).unwrap();
    col.flush().unwrap();
    let queries: Vec<Vec<f32>> = (0..N_QUERIES)
        .map(|i| deterministic_vector(i as u32 * 13 + 7, 1))
        .collect();
    (vfs, db, col, queries)
}

/// 按 tier 选择 `cat` 值列表（tier ∈ {0.001, 0.01, 0.1, 0.5, 0.99}）。
///
/// `n = round(DOC_COUNT * tier)`，至少 1，至多 DOC_COUNT；从 `c0` 起连续取。
/// 例：tier=0.001 → `["c0"]`（1 doc，0.1%）；tier=0.5 → `["c0".."c499"]`（500 doc，50%）。
pub fn cats_for_tier(tier: f32) -> Vec<String> {
    let n = ((DOC_COUNT as f32) * tier).round() as usize;
    let n = n.clamp(1, DOC_COUNT);
    (0..n).map(|i| format!("c{}", i)).collect()
}

/// 构造某档选择率的 `Filter`（cat In [values]）。
pub fn tier_filter(tier: f32) -> Filter {
    let cats = cats_for_tier(tier);
    Filter {
        fields: vec![(
            "cat".into(),
            FilterCond::In(cats.into_iter().map(ScalarValue::Keyword).collect()),
        )],
    }
}

/// 计算 recall@10 = |hnsw_top10 ∩ baseline_top10| / min(10, |baseline_top10|)。
///
/// 按 external_id 比对。分母取 `min(10, baseline.len())`（标准 IR recall@k 口径）：
/// 当基线相关文档数 < 10（如 0.1% 档仅 1 doc）时，除以 10 会使 recall 恒 ≤0.1，
/// 与 SPEC §8.1「低选择率暴力回退 100% 召回」矛盾。基线为空（无相关文档）时
/// 记 recall=1.0（vacuously perfect）。
pub fn recall_at_10(hnsw: &[vane_core::api::Hit], baseline: &[vane_core::api::Hit]) -> f32 {
    let base_ids: std::collections::HashSet<&str> =
        baseline.iter().take(10).map(|h| h.id.as_str()).collect();
    let hit = hnsw
        .iter()
        .take(10)
        .filter(|h| base_ids.contains(h.id.as_str()))
        .count();
    let denom = base_ids.len();
    if denom == 0 {
        1.0
    } else {
        hit as f32 / denom as f32
    }
}
