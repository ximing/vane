//! 12-recall-regression：HNSW 搜索 vs 暴力双路+RRF 基线，recall@10 ≥ 0.95 真实回归门禁。
//!
//! SPEC §13.2-1（hybrid recall@10 ≥0.95，五档选择率 0.1%/1%/10%/50%/99%）/
//! §8.4（召回回归覆盖五档）/§8.1（低选择率 <2×topK 暴力回退 100% 召回）。
//!
//! - **基线**：`Collection::search_brute_baseline` = `brute_search`（vector 路）+
//!   `InvertedIndexReader::search`（text 路）+ `rrf_fuse`（融合），绕过 HnswReader。
//! - **被测**：`Collection::search`（内部 HnswReader + 自适应回退）。
//! - **recall 口径**：`|hnsw_top10 ∩ baseline_top10| / min(10, |baseline_top10|)`
//!   （按 external_id 比对；分母取 min 保证低选择率档有意义，见 `recall_fixture::recall_at_10`）。
//! - **三模式 × 五档**：vector / text / hybrid 各跑五档选择率，断言 recall@10 ≥ 0.95。
//!
//! 替换 M0 `tests/recall.rs`（trivially recall=1.0）为真实回归门禁。

mod recall_fixture;

use recall_fixture::{
    build_recall_fixture, recall_at_10, tier_filter, N_QUERIES, RECALL_THRESHOLD, SELECTIVITY_TIERS,
};
use vane_core::api::{FusionSpec, SearchMode, SearchQuery};

// ---------- Task 2：基线（暴力双路+RRF）实装验证 ----------

#[test]
fn search_brute_baseline_returns_topk_without_hnsw() {
    let (_vfs, _db, col, queries) = build_recall_fixture();
    let qv = &queries[0];
    // 基线 = 暴力双路 + RRF，绕过 HNSW。
    let baseline = col
        .search_brute_baseline(&SearchQuery {
            vector: Some(qv.clone()),
            text: Some("term0".into()),
            top_k: 10,
            mode: SearchMode::Hybrid,
            fusion: FusionSpec::Rrf,
            filter: None,
            candidate_multiplier: 3,
        })
        .unwrap();
    assert_eq!(baseline.len(), 10, "baseline must return topK=10");
    // 基线结果按 RRF 分降序（允许 NaN 兜底，此处 RRF 分恒有限）。
    for w in baseline.windows(2) {
        assert!(
            w[0].score >= w[1].score,
            "baseline not desc: {} vs {}",
            w[0].score,
            w[1].score
        );
    }
}

#[test]
fn search_brute_baseline_matches_search_when_no_hnsw_segment() {
    // 无 filter 时，基线与 search 在 text-only 模式下应完全一致
    //（text 路两边都用 InvertedIndexReader::search，不涉及 HNSW）。
    let (_vfs, _db, col, queries) = build_recall_fixture();
    let q = SearchQuery {
        text: Some("term0".into()),
        vector: Some(queries[0].clone()),
        top_k: 10,
        mode: SearchMode::Text,
        fusion: FusionSpec::Rrf,
        filter: None,
        candidate_multiplier: 3,
    };
    let baseline = col.search_brute_baseline(&q).unwrap();
    let hnsw = col.search(&q).unwrap();
    // text 路恒等 → recall 应为 1.0。
    let recall = recall_at_10(&hnsw, &baseline);
    assert_eq!(
        recall, 1.0,
        "text-mode baseline must equal search (recall=1.0)"
    );
}

// ---------- Task 3：五档选择率 × 三模式 recall@10 ≥ 0.95 ----------

/// 对单档单查询计算 recall@10（vector 模式）。
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

/// 对单档单查询计算 recall@10（text 模式）。
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

/// 对单档单查询计算 recall@10（hybrid 模式）。
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

#[test]
fn recall_vector_5_selectivity_tiers() {
    let (_vfs, _db, col, queries) = build_recall_fixture();
    for &tier in &SELECTIVITY_TIERS {
        let filter = tier_filter(tier);
        let mut min_recall = 1.0_f32;
        for qv in &queries {
            let r = vector_recall(&col, qv, filter.clone());
            min_recall = min_recall.min(r);
            assert!(
                r >= RECALL_THRESHOLD,
                "vector recall@10 {} < {} at tier {} (query vec)",
                r,
                RECALL_THRESHOLD,
                tier
            );
        }
        eprintln!("[vector] tier={} min_recall={:.3}", tier, min_recall);
    }
}

#[test]
fn recall_text_5_selectivity_tiers() {
    let (_vfs, _db, col, _queries) = build_recall_fixture();
    // 用 50 个 term 轮换作为查询词（term{i%50} 分布）。
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
        eprintln!("[text] tier={} min_recall={:.3}", tier, min_recall);
    }
}

#[test]
fn recall_hybrid_5_selectivity_tiers() {
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
        eprintln!("[hybrid] tier={} min_recall={:.3}", tier, min_recall);
    }
}

// ---------- Task 4：低选择率（0.1%）暴力回退 → recall=1.0（§8.1） ----------

#[test]
fn recall_low_selectivity_uses_brute_fallback() {
    // 0.1% 档：位图基数 ~1 < 2*topK=20 → api search 走暴力回退 → recall 应 = 1.0。
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

// ---------- 兼容：保留 M0 recall.rs 冒烟语义的等价校验 ----------
//
// `tests/recall.rs` 仍保留作 M0 冒烟（trivially 1.0）。本文件为 M1 真实门禁。
// 10-ci-m1 的 recall job 改跑 `--test recall_regression`（见 ci.yml）。

#[test]
fn fixture_has_expected_scale() {
    let (_vfs, _db, col, queries) = build_recall_fixture();
    assert_eq!(queries.len(), N_QUERIES);
    assert_eq!(col.segment_count(), 1, "fixture 应为单段（一次 flush）");
}
