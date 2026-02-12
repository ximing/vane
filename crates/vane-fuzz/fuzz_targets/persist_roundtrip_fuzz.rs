//! Fuzz target：persist round-trip 数据一致 / external_id 全回填。
//!
//! 输入 decode：ByteCursor → dim（1..=8）+ n_docs（1..=9）+ query 向量
//!   + 各 doc 的 text+vector。
//! 流程：open → add → flush → search（基线）→ close → reopen → search（对照）。
//! 不变量：reopen 后 topK 合法、score 非 NaN、hit id 全在原 id 集合（external_id
//!   回填后可读）、reopen 前后 id 集合相同（round-trip 一致）。
//!
//! 设计 §3.2 target 表第 3 行。MemoryVfs 保跨 open 调用数据持久（虚拟持久化）。

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

    let dim = (c.u8() as u32).max(1).min(8);
    let n_docs = (c.u8() as usize % 9) + 1; // 1..=9
                                            // 先捕获 query 向量（reopen 后复用同一向量 → round-trip 可比）。
    let query_vec = c.f32_vec(dim as usize);

    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    let schema = build_schema(true, dim);
    let original_ids: Vec<String> = (0..n_docs).map(|i| format!("d{i}")).collect();

    // Phase 1：open → add → flush → search（基线）→ close。
    let baseline_id_set = {
        let db = Db::open(vfs.clone(), "db", OpenOptions::default()).expect("Db::open");
        let col = db
            .collection("c", schema.clone(), CollectionOptions::default())
            .expect("collection create");
        let docs: Vec<Doc> = (0..n_docs)
            .map(|i| Doc {
                id: format!("d{i}"),
                text: Some(c.small_string()),
                vector: Some(c.f32_vec(dim as usize)),
                meta: None,
            })
            .collect();
        let _ = col.add(&docs);
        let _ = col.flush();
        let query = SearchQuery {
            text: None,
            vector: Some(query_vec.clone()),
            top_k: n_docs as u32,
            mode: SearchMode::Vector,
            fusion: FusionSpec::Rrf,
            filter: None,
            candidate_multiplier: 3,
        };
        let hits = col.search(&query).unwrap_or_default();
        for h in &hits {
            assert!(!h.score.is_nan(), "baseline NaN score");
            assert!(
                original_ids.contains(&h.id),
                "baseline unknown id: {}",
                h.id
            );
        }
        let id_set: std::collections::HashSet<String> = hits.iter().map(|h| h.id.clone()).collect();
        let _ = db.close();
        id_set
    };

    // Phase 2：reopen → search（对照）→ close。
    {
        let db = Db::open(vfs.clone(), "db", OpenOptions::default()).expect("Db::reopen");
        let col = db
            .collection("c", schema, CollectionOptions::default())
            .expect("collection reopen");
        let query = SearchQuery {
            text: None,
            vector: Some(query_vec.clone()),
            top_k: n_docs as u32,
            mode: SearchMode::Vector,
            fusion: FusionSpec::Rrf,
            filter: None,
            candidate_multiplier: 3,
        };
        let hits = col.search(&query).unwrap_or_default();
        // 不变量 1：topK 合法。
        assert!(hits.len() <= n_docs, "reopen topK overflow");
        // 不变量 2：score 非 NaN。
        for h in &hits {
            assert!(!h.score.is_nan(), "reopen NaN score");
            // 不变量 3：external_id 全回填 —— hit id 必在原 id 集合。
            assert!(original_ids.contains(&h.id), "reopen unknown id: {}", h.id);
        }
        // 不变量 4：round-trip id 集合一致（关闭前后 search 返回同一 id 集）。
        let reopened_id_set: std::collections::HashSet<String> =
            hits.iter().map(|h| h.id.clone()).collect();
        assert_eq!(
            baseline_id_set, reopened_id_set,
            "round-trip id set mismatch: baseline={:?} reopened={:?}",
            baseline_id_set, reopened_id_set
        );
        let _ = db.close();
    }
});
