//! Fuzz target：暴力检索不 panic / topK 合法 / score 非 NaN。
//!
//! 输入 decode：ByteCursor → dim（1..=16）+ n_docs（0..=8）+ top_k（1..=10）
//!   + mode（Vector/Text/Hybrid）+ 各 doc 的 text+vector + query 的 text+vector。
//! 不变量：search_brute_baseline 不 panic；hits.len() ≤ top_k；每 hit.score 非 NaN。
//!
//! 设计 §3.2 target 表第 1 行。recall 质量由 proptest §3.3 覆盖；本 target 只验
//! 暴力路径在随机输入下不 crash。

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

    // dim 1..=16（≤ DIM_MAX=4096，Schema::new 必过）。
    let dim = (c.u8() as u32).max(1).min(16);
    let n_docs = (c.u8() as usize).min(8);
    let top_k = (c.u8() as u32).max(1).min(10);
    let mode = match c.u8() % 3 {
        0 => SearchMode::Vector,
        1 => SearchMode::Text,
        _ => SearchMode::Hybrid,
    };

    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    let schema = build_schema(true, dim);
    let db = Db::open(vfs, "db", OpenOptions::default()).expect("Db::open on MemoryVfs");
    let col = db
        .collection("c", schema, CollectionOptions::default())
        .expect("collection create with valid schema");

    let mut docs = Vec::with_capacity(n_docs);
    for i in 0..n_docs {
        let text = c.small_string();
        docs.push(Doc {
            id: format!("d{i}"),
            text: if text.is_empty() { None } else { Some(text) },
            vector: Some(c.f32_vec(dim as usize)),
            meta: None,
        });
    }
    if !docs.is_empty() {
        let _ = col.add(&docs);
    }
    let _ = col.flush();

    let q_text = if c.bool() {
        Some(c.small_string())
    } else {
        None
    };
    let q_vec = if c.bool() {
        Some(c.f32_vec(dim as usize))
    } else {
        None
    };
    let query = SearchQuery {
        text: q_text,
        vector: q_vec,
        top_k,
        mode,
        fusion: FusionSpec::Rrf,
        filter: None,
        candidate_multiplier: 3,
    };

    // 暴力检索（search_brute_baseline 强制 f32 brute，bypass HNSW/SQ8）。
    let hits = match col.search_brute_baseline(&query) {
        Ok(h) => h,
        Err(_) => return,
    };
    // 不变量 1：topK 合法（≤ top_k）。
    assert!(
        hits.len() <= top_k as usize,
        "topK overflow: {} > {}",
        hits.len(),
        top_k
    );
    // 不变量 2：score 非 NaN。
    for h in &hits {
        assert!(!h.score.is_nan(), "NaN score from brute search");
    }
});
