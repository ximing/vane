//! criterion bench：hybrid search（10k 文档，topK=10）。
//!
//! DoD 要求 benchmark CI 产生基线数据（hybrid P99 延迟）。
//! criterion 默认输出统计含 percentile，P99 在 criterion 报告中可见。
//! benchmark.yml 夜间跑此 bench 与 main 基线对比，回退 >10% 报警（SPEC §13.2）。

use std::sync::Arc;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use vane_core::api::{
    CollectionOptions, Db, Doc, FusionSpec, OpenOptions, SearchMode, SearchQuery,
};
use vane_core::types::{FieldDef, Metric, Schema};
use vane_core::vfs::memory::MemoryVfs;

const DOC_COUNT: usize = 10_000;
const DIM: usize = 384;
const TOP_K: u32 = 10;

fn build_corpus() -> (Db, Vec<f32>) {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs.clone(), "bench_hybrid", OpenOptions::default()).unwrap();
    let schema = Schema::new(vec![
        ("body".into(), FieldDef::Text),
        (
            "v".into(),
            FieldDef::Vector {
                dim: DIM as u32,
                metric: Metric::Cosine,
            },
        ),
    ])
    .unwrap();
    let col = db
        .collection("docs", schema, CollectionOptions::default())
        .unwrap();

    // 确定性伪随机向量（不依赖 rand crate，避免触碰黑名单/增加依赖）
    let mut docs: Vec<Doc> = Vec::with_capacity(DOC_COUNT);
    let mut query_vec = vec![0.0f32; DIM];
    for i in 0..DOC_COUNT {
        let mut v = vec![0.0f32; DIM];
        for (d, slot) in v.iter_mut().enumerate() {
            // 简单哈希：i 与 d 派生的伪随机值，落在 [-1, 1]
            let h = ((i as u32).wrapping_mul(2654435761).wrapping_add(d as u32)) as f32;
            *slot = ((h % 1000.0) / 500.0) - 1.0;
        }
        // 让 query 向量接近第 0 个文档，确保 topK 命中稳定
        if i == 0 {
            query_vec = v.clone();
        }
        docs.push(Doc {
            id: format!("doc{}", i),
            text: Some(format!("term{} common token{}", i, i % 16)),
            vector: Some(v),
            meta: None,
        });
    }
    col.add(&docs).unwrap();
    col.flush().unwrap();

    (db, query_vec)
}

fn bench_hybrid_search(c: &mut Criterion) {
    let (db, query_vec) = build_corpus();
    // build_corpus 已创建 "docs" collection；collection() 幂等返回已有句柄。
    // 此前冗余调用 db.collections() + 重复构造 schema，删除（死代码）。
    let schema = Schema::new(vec![
        ("body".into(), FieldDef::Text),
        (
            "v".into(),
            FieldDef::Vector {
                dim: DIM as u32,
                metric: Metric::Cosine,
            },
        ),
    ])
    .unwrap();
    let col = db
        .collection("docs", schema, CollectionOptions::default())
        .unwrap();

    let query = SearchQuery {
        text: Some("common token0".into()),
        vector: Some(query_vec.clone()),
        top_k: TOP_K,
        mode: SearchMode::Hybrid,
        fusion: FusionSpec::Rrf,
        filter: None,
        candidate_multiplier: 3,
    };

    c.bench_function("hybrid_search_10k_topk10", |b| {
        b.iter(|| {
            let hits = col.search(black_box(&query)).unwrap();
            black_box(hits.len());
        });
    });
}

criterion_group!(benches, bench_hybrid_search);
criterion_main!(benches);
