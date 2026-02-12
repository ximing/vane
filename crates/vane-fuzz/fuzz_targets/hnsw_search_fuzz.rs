//! Fuzz target：HNSW build+search 不 panic / score 非 NaN / hit id 全已知。
//!
//! 输入 decode：ByteCursor → dim（2..=8）+ n_docs（1..=20）+ top_k（1..=5）
//!   + 各 doc 的 vector + query 的 vector。
//! 不变量：search（HNSW 路径）不 panic；hits.len() ≤ top_k；每 hit.score 非 NaN；
//!   每 hit.id ∈ 已添加 doc id 集合（无 phantom id）。
//!
//! 设计 §3.2 target 表第 2 行。recall 与暴力一致性由 proptest §3.3 覆盖；
//! 本 target 不做严格 recall 断言（HNSW 近似，随机小图 recall 未必 100%，
//! 严格断言易误报）。仅验 HNSW 路径不 crash + 结构合法。
//! 双重路径：search_brute_baseline 也跑一次，保证 brute 不 panic（基线对照）。

#![no_main]

mod common;

use std::sync::Arc;

use libfuzzer::fuzz_target;

use common::{build_schema, ByteCursor};
use vane_core::api::{
    CollectionOptions, Db, Doc, FusionSpec, OpenOptions, SearchMode, SearchQuery,
};
use vane_core::vfs::memory::MemoryVfs;
use vane_core::vfs::Vfs;

fuzz_target!(|data: &[u8]| {
    let mut c = ByteCursor::new(data);

    let dim = (c.u8() as u32).max(2).min(8);
    let n_docs = (c.u8() as usize % 20) + 1; // 1..=20
    let top_k = (c.u8() as u32).max(1).min(5);

    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    // hnsw 路径只需 Vector 字段（无 Text 字段）。
    let schema = build_schema(false, dim);
    let db = Db::open(vfs, "db", OpenOptions::default()).expect("Db::open on MemoryVfs");
    let col = db
        .collection("c", schema, CollectionOptions::default())
        .expect("collection create");

    let known_ids: std::collections::HashSet<String> =
        (0..n_docs).map(|i| format!("d{i}")).collect();
    let mut docs = Vec::with_capacity(n_docs);
    for i in 0..n_docs {
        docs.push(Doc {
            id: format!("d{i}"),
            text: None,
            vector: Some(c.f32_vec(dim as usize)),
            meta: None,
        });
    }
    let _ = col.add(&docs);
    let _ = col.flush();

    let query = SearchQuery {
        text: None,
        vector: Some(c.f32_vec(dim as usize)),
        top_k,
        mode: SearchMode::Vector,
        fusion: FusionSpec::Rrf,
        filter: None,
        candidate_multiplier: 3,
    };

    // HNSW 路径（search 允 HNSW；若 HNSW 缺失自动 brute 回退，不 panic）。
    let hnsw_hits = match col.search(&query) {
        Ok(h) => h,
        Err(_) => return,
    };
    // 不变量：topK 合法、score 非 NaN、id 全已知（无 phantom）。
    assert!(hnsw_hits.len() <= top_k as usize, "HNSW topK overflow");
    for h in &hnsw_hits {
        assert!(!h.score.is_nan(), "HNSW NaN score");
        assert!(known_ids.contains(&h.id), "HNSW phantom id: {}", h.id);
    }

    // Brute 基线对照（不 panic + 结构合法）。
    let brute_hits = match col.search_brute_baseline(&query) {
        Ok(h) => h,
        Err(_) => return,
    };
    assert!(brute_hits.len() <= top_k as usize, "brute topK overflow");
    for h in &brute_hits {
        assert!(!h.score.is_nan(), "brute NaN score");
        assert!(known_ids.contains(&h.id), "brute phantom id: {}", h.id);
    }
});
