//! M2-06 共享召回回归模块（simd/scalar 两变体复用）。
//!
//! 复用 M1 `recall_regression.rs` 方法论（五档选择率 0.1%/1%/10%/50%/99% ×
//! 三模式 vector/text/hybrid，HNSW vs 暴力双路+RRF 基线，recall@10≥0.95），
//! 在 wasm32 上以 `wasm-bindgen-test`（node）运行。
//!
//! I-8 不变量：测试逻辑在 core 方法论，wasm 产物仅作运行载体。
//! 本模块被 `recall_regression_simd.rs` / `recall_regression_scalar.rs` 各 `mod common;`
//! 引入，两测试二进制同源——变体区分由 CI 构建时 RUSTFLAGS（+simd128 / 默认）决定。
//!
//! Jaccard 探针：`recall_jaccard_probe` 测试对固定查询集产出 topK id JSON 行
//! （`JACCARD_PROBE <json>`），由 CI 编排脚本捕获两变体输出后跨变体比对 Jaccard≥0.99。

#![allow(dead_code)]

#[path = "../../../vane-core/tests/recall_fixture.rs"]
mod recall_fixture;

use recall_fixture::{
    build_recall_fixture, recall_at_10, tier_filter, N_QUERIES, RECALL_THRESHOLD, SELECTIVITY_TIERS,
};
use vane_core::api::{FusionSpec, SearchMode, SearchQuery};

// wasm32 console.log（无 web-sys dep）。
#[wasm_bindgen::prelude::wasm_bindgen]
extern "C" {
    #[wasm_bindgen::prelude::wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
}

use wasm_bindgen_test::*;

// ---------- 五档 × 三模式 recall@10 ≥ 0.95（同 M1 方法论） ----------

fn vector_recall(
    col: &vane_core::api::Collection,
    qv: &[f32],
    filter: vane_core::api::Filter,
) -> f32 {
    let q = SearchQuery {
        text: None,
        vector: Some(qv.to_vec()),
        top_k: 10,
        mode: SearchMode::Vector,
        fusion: FusionSpec::Rrf,
        filter: Some(filter.clone()),
        candidate_multiplier: 3,
    };
    let baseline = col.search_brute_baseline(&q).unwrap();
    let hnsw = col.search(&q).unwrap();
    recall_at_10(&hnsw, &baseline)
}

fn text_recall(
    col: &vane_core::api::Collection,
    term: &str,
    filter: vane_core::api::Filter,
) -> f32 {
    let q = SearchQuery {
        text: Some(term.into()),
        vector: None,
        top_k: 10,
        mode: SearchMode::Text,
        fusion: FusionSpec::Rrf,
        filter: Some(filter.clone()),
        candidate_multiplier: 3,
    };
    let baseline = col.search_brute_baseline(&q).unwrap();
    let hnsw = col.search(&q).unwrap();
    recall_at_10(&hnsw, &baseline)
}

fn hybrid_recall(
    col: &vane_core::api::Collection,
    qv: &[f32],
    term: &str,
    filter: vane_core::api::Filter,
) -> f32 {
    let q = SearchQuery {
        text: Some(term.into()),
        vector: Some(qv.to_vec()),
        top_k: 10,
        mode: SearchMode::Hybrid,
        fusion: FusionSpec::Rrf,
        filter: Some(filter.clone()),
        candidate_multiplier: 3,
    };
    let baseline = col.search_brute_baseline(&q).unwrap();
    let hnsw = col.search(&q).unwrap();
    recall_at_10(&hnsw, &baseline)
}

#[wasm_bindgen_test]
fn wasm_recall_vector_5_selectivity_tiers() {
    let (_vfs, _db, col, queries) = build_recall_fixture();
    for &tier in &SELECTIVITY_TIERS {
        let filter = tier_filter(tier);
        let mut min_recall = 1.0_f32;
        for qv in &queries {
            let r = vector_recall(&col, qv, filter.clone());
            min_recall = min_recall.min(r);
            assert!(
                r >= RECALL_THRESHOLD,
                "vector recall@10 {} < {} at tier {}",
                r,
                RECALL_THRESHOLD,
                tier
            );
        }
        log(&format!(
            "[wasm vector] tier={} min_recall={:.3}",
            tier, min_recall
        ));
    }
}

#[wasm_bindgen_test]
fn wasm_recall_text_5_selectivity_tiers() {
    let (_vfs, _db, col, _queries) = build_recall_fixture();
    let terms: Vec<String> = (0..N_QUERIES).map(|i| format!("term{}", i % 50)).collect();
    for &tier in &SELECTIVITY_TIERS {
        let filter = tier_filter(tier);
        let mut min_recall = 1.0_f32;
        for term in &terms {
            let r = text_recall(&col, term, filter.clone());
            min_recall = min_recall.min(r);
            assert!(
                r >= RECALL_THRESHOLD,
                "text recall@10 {} < {} at tier {} (term {})",
                r,
                RECALL_THRESHOLD,
                tier,
                term
            );
        }
        log(&format!(
            "[wasm text] tier={} min_recall={:.3}",
            tier, min_recall
        ));
    }
}

#[wasm_bindgen_test]
fn wasm_recall_hybrid_5_selectivity_tiers() {
    let (_vfs, _db, col, queries) = build_recall_fixture();
    let terms: Vec<String> = (0..N_QUERIES).map(|i| format!("term{}", i % 50)).collect();
    for &tier in &SELECTIVITY_TIERS {
        let filter = tier_filter(tier);
        let mut min_recall = 1.0_f32;
        for (qv, term) in queries.iter().zip(terms.iter()) {
            let r = hybrid_recall(&col, qv, term, filter.clone());
            min_recall = min_recall.min(r);
            assert!(
                r >= RECALL_THRESHOLD,
                "hybrid recall@10 {} < {} at tier {} (term {})",
                r,
                RECALL_THRESHOLD,
                tier,
                term
            );
        }
        log(&format!(
            "[wasm hybrid] tier={} min_recall={:.3}",
            tier, min_recall
        ));
    }
}

#[wasm_bindgen_test]
fn wasm_recall_low_selectivity_brute_fallback() {
    // §8.1：0.1% 档位图基数 ~1 < 2*topK=20 → 暴力回退 → recall=1.0。
    let (_vfs, _db, col, queries) = build_recall_fixture();
    let filter = tier_filter(0.001);
    let qv = &queries[0];
    let q = SearchQuery {
        text: None,
        vector: Some(qv.clone()),
        top_k: 10,
        mode: SearchMode::Vector,
        fusion: FusionSpec::Rrf,
        filter: Some(filter.clone()),
        candidate_multiplier: 3,
    };
    let baseline = col.search_brute_baseline(&q).unwrap();
    let hnsw = col.search(&q).unwrap();
    let r = recall_at_10(&hnsw, &baseline);
    assert_eq!(
        r, 1.0,
        "0.1% 档应触发暴力回退，recall 应 = 1.0（got {}）",
        r
    );
}

// ---------- Jaccard 探针：固定查询集 topK id JSON ----------
//
// 输出 `JACCARD_PROBE <json>` 一行（console.log），由 CI 编排脚本捕获后跨变体比对。
// JSON 格式：[{"q":0,"mode":"vector","tier":0.1,"topk":["d5","d12",...]},...]
// 固定查询集 = 5 查询 × 3 模式 × 2 档（10%/50%），共 30 个 topK 集——足够检测变体分歧。

#[wasm_bindgen_test]
fn recall_jaccard_probe() {
    let (_vfs, _db, col, queries) = build_recall_fixture();
    let terms: Vec<String> = (0..N_QUERIES).map(|i| format!("term{}", i % 50)).collect();
    let probe_tiers: [f32; 2] = [0.1, 0.5];
    let mut entries: Vec<String> = Vec::new();

    for &tier in &probe_tiers {
        let filter = tier_filter(tier);
        for i in 0..5usize {
            let qv = &queries[i];
            let term = &terms[i];

            // vector
            let qv_q = SearchQuery {
                text: None,
                vector: Some(qv.clone()),
                top_k: 10,
                mode: SearchMode::Vector,
                fusion: FusionSpec::Rrf,
                filter: Some(filter.clone()),
                candidate_multiplier: 3,
            };
            let v_hits = col.search(&qv_q).unwrap();
            let v_ids: Vec<&str> = v_hits.iter().take(10).map(|h| h.id.as_str()).collect();
            entries.push(format!(
                r#"{{"q":{},"mode":"vector","tier":{},"topk":[{}]}}"#,
                i,
                tier,
                v_ids
                    .iter()
                    .map(|s| format!("\"{}\"", s))
                    .collect::<Vec<_>>()
                    .join(",")
            ));

            // text
            let qt_q = SearchQuery {
                text: Some(term.clone()),
                vector: None,
                top_k: 10,
                mode: SearchMode::Text,
                fusion: FusionSpec::Rrf,
                filter: Some(filter.clone()),
                candidate_multiplier: 3,
            };
            let t_hits = col.search(&qt_q).unwrap();
            let t_ids: Vec<&str> = t_hits.iter().take(10).map(|h| h.id.as_str()).collect();
            entries.push(format!(
                r#"{{"q":{},"mode":"text","tier":{},"topk":[{}]}}"#,
                i,
                tier,
                t_ids
                    .iter()
                    .map(|s| format!("\"{}\"", s))
                    .collect::<Vec<_>>()
                    .join(",")
            ));

            // hybrid
            let qh_q = SearchQuery {
                text: Some(term.clone()),
                vector: Some(qv.clone()),
                top_k: 10,
                mode: SearchMode::Hybrid,
                fusion: FusionSpec::Rrf,
                filter: Some(filter.clone()),
                candidate_multiplier: 3,
            };
            let h_hits = col.search(&qh_q).unwrap();
            let h_ids: Vec<&str> = h_hits.iter().take(10).map(|h| h.id.as_str()).collect();
            entries.push(format!(
                r#"{{"q":{},"mode":"hybrid","tier":{},"topk":[{}]}}"#,
                i,
                tier,
                h_ids
                    .iter()
                    .map(|s| format!("\"{}\"", s))
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        }
    }

    let json = format!("[{}]", entries.join(","));
    log(&format!("JACCARD_PROBE {}", json));
}
