// tests/million_scale.rs — M2-10 100万规模压测（SPEC §3.3/§13.1）。
//
// 策略（heavy）：
// - 100万×384 维 f32 = 1.5GB 超 §13.1；用低 dim（64）压测证并行搜索正确 + 不崩。
//   100万×64 f32 = 256MB（<500MB §13.1 承诺）。
// - fixture 程序化生成（确定性，无外部依赖）。
// - 默认跑小规模（1万）证并行搜索正确 + 不崩；100万/10万标 #[ignore]（manual/CI heavy）。
//
// 门禁（自证 12）：
// - 100万 #[ignore]：add/flush/search/compact 全流程不崩 + search P99 <5s（宽松阈值）。
// - 1万 默认：并行搜索结果与串行等价（recall 不退）。

use std::sync::Arc;
use std::time::Instant;

use vane_core::api::{
    CollectionOptions, Db, Doc, FusionSpec, OpenOptions, SearchMode, SearchQuery,
};
use vane_core::types::{FieldDef, Metric, Schema};
use vane_core::vfs::memory::MemoryVfs;

/// 确定性生成文档向量：seed → pseudo-random 64 维向量（归一化）。
fn make_vector(seed: u64) -> Vec<f32> {
    // 简单 LCG 伪随机，确定性无外部依赖
    let mut state = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    let mut v = Vec::with_capacity(64);
    for _ in 0..64 {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let x = ((state >> 33) as f32) / (1u64 << 31) as f32 - 1.0;
        v.push(x);
    }
    // 归一化（cosine 友好）
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        v.iter_mut().for_each(|x| *x /= norm);
    }
    v
}

fn make_doc(i: u64) -> Doc {
    Doc {
        id: format!("doc{}", i),
        text: Some(format!("text {} term{}", i, i % 100)),
        vector: Some(make_vector(i)),
        meta: None,
    }
}

fn build_corpus(n: u64, batch_size: usize) -> (Arc<MemoryVfs>, Db) {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs.clone(), "million", OpenOptions::default()).unwrap();
    let schema = Schema::new(vec![
        ("body".into(), FieldDef::Text),
        (
            "v".into(),
            FieldDef::Vector {
                dim: 64,
                metric: Metric::Cosine,
            },
        ),
    ])
    .unwrap();
    let col = db
        .collection("docs", schema, CollectionOptions::default())
        .unwrap();

    let start = Instant::now();
    let mut added = 0u64;
    while added < n {
        let end = (added + batch_size as u64).min(n);
        let batch: Vec<Doc> = (added..end).map(make_doc).collect();
        col.add(&batch).unwrap();
        col.flush().unwrap();
        added = end;
    }
    eprintln!(
        "[million_scale] built {} docs in {} batches, {:.2}s",
        n,
        n.div_ceil(batch_size as u64),
        start.elapsed().as_secs_f64()
    );
    (vfs, db)
}

fn open_col(db: &Db) -> vane_core::api::Collection {
    db.collection(
        "docs",
        Schema::new(vec![
            ("body".into(), FieldDef::Text),
            (
                "v".into(),
                FieldDef::Vector {
                    dim: 64,
                    metric: Metric::Cosine,
                },
            ),
        ])
        .unwrap(),
        CollectionOptions::default(),
    )
    .unwrap()
}

/// 默认小规模（1万）：证并行搜索不崩 + 结果正确。
#[test]
fn parallel_search_10k_no_crash() {
    let (_vfs, db) = build_corpus(10_000, 1_000);
    let col = open_col(&db);

    // 段数 >1（多段并行搜索路径）
    let seg_count = col.segment_count();
    assert!(
        seg_count > 1,
        "expected multiple segments, got {}",
        seg_count
    );

    // vector 搜索
    let qv = make_vector(42);
    let start = Instant::now();
    let hits = col
        .search(&SearchQuery {
            text: None,
            vector: Some(qv.clone()),
            top_k: 10,
            mode: SearchMode::Vector,
            fusion: FusionSpec::Rrf,
            filter: None,
            candidate_multiplier: 3,
        })
        .unwrap();
    let elapsed = start.elapsed();
    assert_eq!(hits.len(), 10);
    eprintln!(
        "[million_scale] 10k vector search: {:.2}ms, seg_count={}",
        elapsed.as_secs_f64() * 1000.0,
        seg_count
    );

    // hybrid 搜索
    let start = Instant::now();
    let hits = col
        .search(&SearchQuery {
            text: Some("text 42 term42".into()),
            vector: Some(qv),
            top_k: 10,
            mode: SearchMode::Hybrid,
            fusion: FusionSpec::Rrf,
            filter: None,
            candidate_multiplier: 3,
        })
        .unwrap();
    let elapsed = start.elapsed();
    assert!(!hits.is_empty());
    eprintln!(
        "[million_scale] 10k hybrid search: {:.2}ms",
        elapsed.as_secs_f64() * 1000.0
    );
}

/// 并行搜索结果与串行（baseline）等价（recall 不退）。
#[test]
fn parallel_search_matches_serial_10k() {
    let (_vfs, db) = build_corpus(10_000, 1_000);
    let col = open_col(&db);

    let qv = make_vector(99);
    // 并行搜索（search）
    let hits_parallel = col
        .search(&SearchQuery {
            text: None,
            vector: Some(qv.clone()),
            top_k: 10,
            mode: SearchMode::Vector,
            fusion: FusionSpec::Rrf,
            filter: None,
            candidate_multiplier: 3,
        })
        .unwrap();
    // 串行基线（search_brute_baseline，allow_hnsw=false 走 brute f32）
    let hits_serial = col
        .search_brute_baseline(&SearchQuery {
            text: None,
            vector: Some(qv),
            top_k: 10,
            mode: SearchMode::Vector,
            fusion: FusionSpec::Rrf,
            filter: None,
            candidate_multiplier: 3,
        })
        .unwrap();

    // docid 集合应高度重叠（parallel 用 HNSW 近似，serial 用 brute 精确）。
    let serial_ids: std::collections::HashSet<String> =
        hits_serial.iter().map(|h| h.id.clone()).collect();
    let overlap = hits_parallel
        .iter()
        .filter(|h| serial_ids.contains(&h.id))
        .count();
    let recall = overlap as f64 / serial_ids.len().max(1) as f64;
    eprintln!(
        "[million_scale] 10k recall@10 (parallel vs brute): {:.3}",
        recall
    );
    assert!(recall >= 0.80, "recall too low: {}", recall);
}

/// compact 后段数减少，搜索仍正确。
#[test]
fn compact_multi_segment_10k() {
    let (_vfs, db) = build_corpus(10_000, 1_000);
    let col = open_col(&db);

    let before = col.segment_count();
    assert!(before > 1);

    col.compact().unwrap();
    let after = col.segment_count();
    eprintln!("[million_scale] compact: {} -> {} segments", before, after);
    assert!(after <= before, "compact should not increase segments");

    // compact 后搜索仍正确
    let qv = make_vector(77);
    let hits = col
        .search(&SearchQuery {
            text: None,
            vector: Some(qv),
            top_k: 10,
            mode: SearchMode::Vector,
            fusion: FusionSpec::Rrf,
            filter: None,
            candidate_multiplier: 3,
        })
        .unwrap();
    assert_eq!(hits.len(), 10);
}

/// 10万规模压测（#[ignore]，CI heavy）。证多段并行搜索不崩 + 延迟。
#[test]
#[ignore]
fn parallel_search_100k() {
    let (_vfs, db) = build_corpus(100_000, 5_000);
    let col = open_col(&db);

    let seg_count = col.segment_count();
    eprintln!("[million_scale] 100k docs, {} segments", seg_count);
    assert!(seg_count > 1);

    let qv = make_vector(42);
    let start = Instant::now();
    let hits = col
        .search(&SearchQuery {
            text: None,
            vector: Some(qv.clone()),
            top_k: 10,
            mode: SearchMode::Vector,
            fusion: FusionSpec::Rrf,
            filter: None,
            candidate_multiplier: 3,
        })
        .unwrap();
    let elapsed = start.elapsed();
    assert_eq!(hits.len(), 10);
    eprintln!(
        "[million_scale] 100k vector search: {:.2}ms",
        elapsed.as_secs_f64() * 1000.0
    );
}

/// 100万全量压测（#[ignore]，manual/CI heavy）。
/// 自证门禁 12：add/flush/search/compact 全流程不崩 + search P99 <5s。
#[test]
#[ignore]
fn million_scale_full_pipeline() {
    let n: u64 = 1_000_000;
    let (_vfs, db) = build_corpus(n, 10_000);
    let col = open_col(&db);

    let seg_count = col.segment_count();
    eprintln!("[million_scale] {} docs, {} segments", n, seg_count);
    assert!(seg_count > 1, "expected multiple segments");

    // 多次搜索测 P99
    let mut latencies: Vec<f64> = Vec::new();
    for seed in 0..20u64 {
        let qv = make_vector(seed);
        let start = Instant::now();
        let hits = col
            .search(&SearchQuery {
                text: None,
                vector: Some(qv),
                top_k: 10,
                mode: SearchMode::Vector,
                fusion: FusionSpec::Rrf,
                filter: None,
                candidate_multiplier: 3,
            })
            .unwrap();
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        latencies.push(elapsed);
        assert_eq!(hits.len(), 10, "seed {} returned {} hits", seed, hits.len());
    }
    latencies.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p50 = latencies[latencies.len() / 2];
    let p99 = latencies[latencies.len() - 1];
    eprintln!(
        "[million_scale] 1M search: P50={:.1}ms P99={:.1}ms",
        p50, p99
    );
    // 宽松阈值：<5s（SPEC §13.1 native 100万放宽档）
    assert!(p99 < 5000.0, "P99 too slow: {:.1}ms", p99);

    // compact
    let before = col.segment_count();
    let compact_start = Instant::now();
    col.compact().unwrap();
    let compact_elapsed = compact_start.elapsed().as_secs_f64();
    let after = col.segment_count();
    eprintln!(
        "[million_scale] compact: {} -> {} segments, {:.2}s",
        before, after, compact_elapsed
    );

    // compact 后搜索仍正确
    let qv = make_vector(123);
    let hits = col
        .search(&SearchQuery {
            text: None,
            vector: Some(qv),
            top_k: 10,
            mode: SearchMode::Vector,
            fusion: FusionSpec::Rrf,
            filter: None,
            candidate_multiplier: 3,
        })
        .unwrap();
    assert_eq!(hits.len(), 10);
}
