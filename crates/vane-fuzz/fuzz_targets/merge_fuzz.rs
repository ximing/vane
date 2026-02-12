//! Fuzz target：merge 不丢文档（除 tombstone）/ docid 连续。
//!
//! 输入 decode：ByteCursor → dim（1..=8）+ n_flushes（1..=4）+ docs_per_flush（1..=5）
//!   + n_delete 选择（cursor 驱动）+ 各 doc 的 vector + query 向量。
//! 流程：多轮 add+flush（多段）→ 按字节选择 delete（tombstone）→ compact（merge 全段）
//!   → search top_k=1000。
//! 不变量：tombstoned id 不在 hits；hit id 全已知（无 phantom）；hits 无重复 id
//!   （docid 连续）；live id 全可见（不丢文档）。
//!
//! 设计 §3.2 target 表第 4 行。compact() 合并全段 + 物理清 tombstone（collection.rs:1076）。

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
    let n_flushes = (c.u8() as usize % 4) + 1; // 1..=4
    let docs_per_flush = (c.u8() as usize % 5) + 1; // 1..=5

    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    let schema = build_schema(false, dim);
    let db = Db::open(vfs, "db", OpenOptions::default()).expect("Db::open");
    let col = db
        .collection("c", schema, CollectionOptions::default())
        .expect("collection create");

    let mut added_ids: Vec<String> = Vec::new();
    let mut deleted_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    // 多轮 add+flush → 多段结构。
    for flush_idx in 0..n_flushes {
        let mut docs = Vec::with_capacity(docs_per_flush);
        for j in 0..docs_per_flush {
            let id = format!("f{flush_idx}_d{j}");
            added_ids.push(id.clone());
            docs.push(Doc {
                id,
                text: None,
                vector: Some(c.f32_vec(dim as usize)),
                meta: None,
            });
        }
        let _ = col.add(&docs);
        let _ = col.flush();
    }

    // 按字节选择 delete（tombstone）：删 n_delete % (added+1) 个 id。
    let total_added = added_ids.len();
    let n_delete = (c.u8() as usize) % (total_added + 1);
    for _ in 0..n_delete {
        if added_ids.is_empty() {
            break;
        }
        let idx = (c.u8() as usize) % added_ids.len();
        let id = added_ids[idx].clone();
        let _ = col.delete(&[id.clone()]);
        deleted_ids.insert(id);
    }

    let live_ids: std::collections::HashSet<&String> = added_ids
        .iter()
        .filter(|id| !deleted_ids.contains(*id))
        .collect();

    // compact = merge 全段 + 物理 tombstone 清除。
    let _ = col.compact();

    // search top_k=TOPK_MAX=1000：live docs 应全可见。
    let query = SearchQuery {
        text: None,
        vector: Some(c.f32_vec(dim as usize)),
        top_k: 1000,
        mode: SearchMode::Vector,
        fusion: FusionSpec::Rrf,
        filter: None,
        candidate_multiplier: 3,
    };
    let hits = col.search(&query).unwrap_or_default();
    let hit_ids: std::collections::HashSet<&String> = hits.iter().map(|h| &h.id).collect();

    // 不变量 1：tombstoned id 不可见。
    for did in &deleted_ids {
        assert!(
            !hit_ids.contains(did),
            "tombstoned id visible after compact: {}",
            did
        );
    }
    // 不变量 2：hit id 全已知（无 phantom）。
    for hid in &hit_ids {
        assert!(added_ids.contains(hid), "unknown id after compact: {}", hid);
    }
    // 不变量 3：无重复 id（docid 连续——compact 后段内无重复）。
    assert_eq!(hits.len(), hit_ids.len(), "duplicate ids after compact");
    // 不变量 4：live docs 全可见（不丢文档）。
    //    top_k=1000 > total live（≤20）→ search 应返回所有 live docs。
    assert_eq!(
        hit_ids.len(),
        live_ids.len(),
        "live doc count mismatch after compact: hits={} live={} (deleted={})",
        hit_ids.len(),
        live_ids.len(),
        deleted_ids.len()
    );
    for live_id in &live_ids {
        assert!(
            hit_ids.contains(live_id),
            "live id missing after compact: {}",
            live_id
        );
    }
});
