// tests/proptest_invariants.rs — M4 阶段一 b：proptest property-based 不变量测试
//
// 设计 §3.3：3 不变量（检索排序稳定合法 / persist round-trip 一致 / merge 不丢文档）。
// proptest! 宏将测试体包入传给 TestRunner::run 的闭包，rustc dead_code 分析
// 无法穿越闭包追踪 helper 调用 → 文件级 allow(dead_code) 消除假告警（helper 实际
// 被闭包内的测试体调用）。不影响 clippy 其他门禁。
#![allow(dead_code)]
// proptest 默认 256 cases，失败 seed 持久化到 proptest-regressions/ 确保 CI 复现。
//
// proptest 是 dev-dep，不进 wasm/native 生产构建（wasm32 check 不含 dev-deps）。
// 传递依赖无黑名单项（regex/tokio/prost/tonic/openssl/lindera/ndarray/
// wee_alloc/dashmap/parking_lot），cargo deny check 守护。
//
// Strategy 设计（§3.3 骨架 + 零 regex 路径）：
// - arb_letter/arb_word/arb_text：a-z 字符生成，绕开 proptest string_regex 的
//   regex-syntax 可选依赖路径（默认 features 不启用 regex feature）。
// - arb_finite_f32/arb_vector：f32 NaN/Inf 过滤 + 非全零（避 cosine 0/0 退化 NaN score）。
// - Strategy 返回 Debug 可格式化元组（Doc/SearchQuery 未 derive Debug，proptest! 宏
//   需值类型实现 Debug 以打印失败输入），测试体内构造 API 类型。
//
// 不变量 3 用 search_brute_baseline（非 HNSW 近似）验证活文档全集，避假红/绿
// （1a merge_fuzz review M2 建议）。

use proptest::prelude::*;
use std::sync::Arc;

use vane_core::api::{
    CollectionOptions, Db, Doc, FusionSpec, OpenOptions, ScalarValue, SearchMode, SearchQuery,
};
use vane_core::types::{FieldDef, Metric, ScalarKind, Schema};
use vane_core::vfs::memory::MemoryVfs;

/// 维度——小 dim 保 CI 速度（256 cases × 多 mode × round-trip + merge）。
const DIM: usize = 4;
/// 单批次最大文档数——小规模保 proptest 速度（round-trip 每例 close+reopen，
/// 256 cases × MAX_DOCS=8 约 50s），足够覆盖排序/round-trip/merge 不变量。
const MAX_DOCS: usize = 8;

// ---------------------------------------------------------------------------
// Strategy 设计
// ---------------------------------------------------------------------------

/// 生成 a-z 随机字符（零 regex 依赖——绕开 proptest string_regex 的 regex-syntax 路径）。
fn arb_letter() -> impl Strategy<Value = char> {
    (0u8..26u8).prop_map(|i| char::from(b'a' + i))
}

/// 生成 1..max_len 长度的 a-z 字符串。
fn arb_word(max_len: usize) -> impl Strategy<Value = String> {
    prop::collection::vec(arb_letter(), 1..max_len).prop_map(|cs| cs.into_iter().collect())
}

/// 生成空格分隔的 a-z 文本（1..max_words 个词，每词 1..max_word_len 字符）。
fn arb_text(max_words: usize, max_word_len: usize) -> impl Strategy<Value = String> {
    prop::collection::vec(arb_word(max_word_len), 1..max_words).prop_map(|ws| ws.join(" "))
}

/// 有限 f32（过滤 NaN/Inf，避 score 退化与排序异常）。
fn arb_finite_f32() -> impl Strategy<Value = f32> {
    prop::num::f32::ANY.prop_filter("finite", |x| x.is_finite())
}

/// 随机向量（dim 维，有限值，非全零避 cosine 0/0 退化 NaN score）。
fn arb_vector(dim: usize) -> impl Strategy<Value = Vec<f32>> {
    prop::collection::vec(arb_finite_f32(), dim..=dim)
        .prop_filter("not_all_zero", |v| v.iter().any(|x| *x != 0.0))
}

/// 随机查询组件（text + vector + topK + mode）。返回 Debug 元组，测试体内构造 SearchQuery。
fn arb_query_components(dim: usize) -> impl Strategy<Value = (String, Vec<f32>, u32, SearchMode)> {
    (
        arb_text(4, 8),
        arb_vector(dim),
        1u32..=8u32,
        prop_oneof![
            Just(SearchMode::Hybrid),
            Just(SearchMode::Vector),
            Just(SearchMode::Text),
        ],
    )
}

/// 随机文档体组件批次（text + vector + tag char）。返回 Debug 元组 Vec，
/// 测试体内构造 Doc（顺序 id d0..d{n-1}，保 id 唯一；含 meta tag scalar 供 stored_json round-trip）。
fn arb_doc_bodies(
    dim: usize,
    max_docs: usize,
) -> impl Strategy<Value = Vec<(String, Vec<f32>, char)>> {
    prop::collection::vec((arb_text(8, 8), arb_vector(dim), arb_letter()), 1..max_docs)
}

/// merge 场景组件批次（text + vector + tag + delete_flag）。返回 Debug 元组 Vec，
/// 测试体内构造 (Vec<Doc>, Vec<bool>)——并行删除标志位（同长度，一一对应）。
fn arb_merge_bodies(
    dim: usize,
    max_docs: usize,
) -> impl Strategy<Value = Vec<(String, Vec<f32>, char, bool)>> {
    prop::collection::vec(
        (
            arb_text(8, 8),
            arb_vector(dim),
            arb_letter(),
            prop::bool::ANY,
        ),
        1..max_docs,
    )
}

/// 从组件批次构造 Doc Vec（顺序 id 保唯一 + meta tag scalar）。
fn build_docs(bodies: &[(String, Vec<f32>, char)]) -> Vec<Doc> {
    bodies
        .iter()
        .enumerate()
        .map(|(i, (text, vec, tag))| {
            let mut meta = std::collections::HashMap::new();
            meta.insert("tag".to_string(), ScalarValue::Keyword(tag.to_string()));
            Doc {
                id: format!("d{}", i),
                text: Some(text.clone()),
                vector: Some(vec.clone()),
                meta: Some(meta),
            }
        })
        .collect()
}

/// 从 merge 组件批次构造 (Vec<Doc>, Vec<bool>)。
fn build_merge_scenario(bodies: &[(String, Vec<f32>, char, bool)]) -> (Vec<Doc>, Vec<bool>) {
    let docs: Vec<Doc> = bodies
        .iter()
        .enumerate()
        .map(|(i, (text, vec, tag, _))| {
            let mut meta = std::collections::HashMap::new();
            meta.insert("tag".to_string(), ScalarValue::Keyword(tag.to_string()));
            Doc {
                id: format!("d{}", i),
                text: Some(text.clone()),
                vector: Some(vec.clone()),
                meta: Some(meta),
            }
        })
        .collect();
    let delete_flags: Vec<bool> = bodies.iter().map(|(_, _, _, del)| *del).collect();
    (docs, delete_flags)
}

/// 从组件构造 SearchQuery。
fn build_query((text, vector, top_k, mode): (String, Vec<f32>, u32, SearchMode)) -> SearchQuery {
    SearchQuery {
        text: Some(text),
        vector: Some(vector),
        top_k,
        mode,
        fusion: FusionSpec::Rrf,
        filter: None,
        candidate_multiplier: 3,
    }
}

fn build_schema(dim: usize) -> Schema {
    Schema::new(vec![
        ("body".into(), FieldDef::Text),
        (
            "v".into(),
            FieldDef::Vector {
                dim: dim as u32,
                metric: Metric::Cosine,
            },
        ),
        (
            "tag".into(),
            FieldDef::Scalar {
                kind: ScalarKind::Keyword,
            },
        ),
    ])
    .unwrap()
}

/// 全量 vector 查询（topK=n，取回所有文档）。
fn vector_query_all(n: usize) -> SearchQuery {
    SearchQuery {
        text: None,
        vector: Some(vec![1.0; DIM]),
        top_k: n as u32,
        mode: SearchMode::Vector,
        fusion: FusionSpec::Rrf,
        filter: None,
        candidate_multiplier: 3,
    }
}

/// 捕获 (id, score, tag) 三元组用于一致性比对（tag 来自 stored.bin meta JSON）。
fn capture(hits: &[vane_core::api::Hit]) -> Vec<(String, f32, Option<String>)> {
    hits.iter()
        .map(|h| {
            let tag = h.fields.as_ref().and_then(|f| f.get("tag")).cloned();
            (h.id.clone(), h.score, tag)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// 不变量 1：检索排序稳定合法
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn search_returns_stable_topk(
        bodies in arb_doc_bodies(DIM, MAX_DOCS),
        q_components in arb_query_components(DIM),
    ) {
        let docs = build_docs(&bodies);
        let q = build_query(q_components);

        let vfs = Arc::new(MemoryVfs::new());
        let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
        let col = db
            .collection("docs", build_schema(DIM), CollectionOptions::default())
            .unwrap();
        col.add(&docs).unwrap();
        col.flush().unwrap();

        let hits1 = col.search(&q).unwrap();
        let hits2 = col.search(&q).unwrap();

        // 不变量 1a：结果数 ≤ min(topK, total_docs)。
        let upper = (q.top_k as usize).min(docs.len());
        prop_assert!(
            hits1.len() <= upper,
            "hits1.len() {} exceeds min(topK={}, total={})",
            hits1.len(), q.top_k, docs.len()
        );
        prop_assert_eq!(hits1.len(), hits2.len(), "same query must return same count");

        // 不变量 1b：score 单调非递增，且全部有限。
        for w in hits1.windows(2) {
            prop_assert!(
                w[0].score >= w[1].score,
                "scores not monotonically non-increasing: {} then {}",
                w[0].score, w[1].score
            );
        }
        for h in &hits1 {
            prop_assert!(
                h.score.is_finite(),
                "score not finite: id={} score={}", h.id, h.score
            );
        }

        // 不变量 1c：同 query 二次检索 (id, score, tag) 完全一致。
        let cap1 = capture(&hits1);
        let cap2 = capture(&hits2);
        prop_assert_eq!(cap1, cap2, "same query must yield identical (id, score, tag) sequence");
    }
}

// ---------------------------------------------------------------------------
// 不变量 2：persist round-trip 一致
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn persist_roundtrip_consistent(
        bodies in arb_doc_bodies(DIM, MAX_DOCS),
    ) {
        let docs = build_docs(&bodies);
        let total = docs.len();
        let vfs = Arc::new(MemoryVfs::new());

        // 第一次 open：建库 + 灌数据 + flush + 基线搜索 + close。
        let baseline = {
            let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
            let col = db
                .collection("docs", build_schema(DIM), CollectionOptions::default())
                .unwrap();
            col.add(&docs).unwrap();
            col.flush().unwrap();

            // 全量基线：vector 模式 topK=total，取回所有文档。
            let q = vector_query_all(total);
            let hits = col.search(&q).unwrap();
            // 期望全部文档可见（无 delete）。
            prop_assert_eq!(
                hits.len(), total,
                "baseline must return all {} docs, got {}", total, hits.len()
            );
            let baseline = capture(&hits);
            db.close().unwrap();
            baseline
        };

        // 第二次 open：同 vfs，验证 manifest/segment/WAL 恢复。
        let db2 = Db::open(vfs, "db", OpenOptions::default()).unwrap();
        prop_assert!(
            db2.collections().iter().any(|c| c == "docs"),
            "collection 'docs' not restored after reopen"
        );
        let col2 = db2
            .collection("docs", build_schema(DIM), CollectionOptions::default())
            .unwrap();
        let q = vector_query_all(total);
        let hits2 = col2.search(&q).unwrap();

        // 不变量 2a：external_id 全回填——结果数 == total，且 id 集合等于原文档 id 集合。
        prop_assert_eq!(
            hits2.len(), total,
            "reopen must return all {} docs, got {}", total, hits2.len()
        );
        let expected_ids: std::collections::HashSet<String> =
            docs.iter().map(|d| d.id.clone()).collect();
        let got_ids: std::collections::HashSet<String> =
            hits2.iter().map(|h| h.id.clone()).collect();
        prop_assert_eq!(got_ids, expected_ids, "external_id set mismatch after reopen");

        // 不变量 2b：stored tag 一致——每条 hit 的 tag 字段非空且为合法 JSON 字符串
        // （stored.bin meta JSON round-trip；单字符 tag 回填为 "\"x\"" 3 字符）。
        for h in &hits2 {
            let tag = h.fields.as_ref().and_then(|f| f.get("tag"));
            prop_assert!(tag.is_some(), "stored tag missing for id={} after reopen", h.id);
            let t = tag.unwrap();
            prop_assert!(
                t.len() >= 3 && t.starts_with('"') && t.ends_with('"'),
                "stored tag not a JSON string: {}", t
            );
        }

        // 不变量 2c：search 结果集 (id, score, tag) 与基线完全一致。
        let after = capture(&hits2);
        prop_assert_eq!(after, baseline, "round-trip (id, score, tag) differs from baseline");

        db2.close().unwrap();
    }
}

// ---------------------------------------------------------------------------
// 不变量 3：merge 不丢文档
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn merge_preserves_live_docs(
        bodies in arb_merge_bodies(DIM, MAX_DOCS),
        chunk_size in 1u32..=4u32,
    ) {
        let (docs, delete_flags) = build_merge_scenario(&bodies);
        let total = docs.len();

        let vfs = Arc::new(MemoryVfs::new());
        let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
        let col = db
            .collection("docs", build_schema(DIM), CollectionOptions::default())
            .unwrap();

        // 多段灌入：按 chunk_size 分批 add+flush，制造多个段。
        for chunk_docs in docs.chunks(chunk_size as usize) {
            col.add(chunk_docs).unwrap();
            col.flush().unwrap();
        }

        // 删除标志位对应的文档。
        let delete_ids: Vec<String> = docs
            .iter()
            .zip(delete_flags.iter())
            .filter(|(_, &del)| del)
            .map(|(d, _)| d.id.clone())
            .collect();
        if !delete_ids.is_empty() {
            col.delete(&delete_ids).unwrap();
        }

        // 期望活文档集合（未被 delete 的）。
        let expected_live: std::collections::HashSet<String> = docs
            .iter()
            .zip(delete_flags.iter())
            .filter(|(_, &del)| !del)
            .map(|(d, _)| d.id.clone())
            .collect();

        // compact 合并所有段 + 物理清 tombstone。
        col.compact().unwrap();

        // 用 brute baseline（Vector 模式，topK=total）验证活文档全集——
        // 绕过 HNSW 近似，确保 docid 不重叠/不丢失的确定性验证。
        let q = vector_query_all(total);
        let hits = col.search_brute_baseline(&q).unwrap();
        let hit_ids: std::collections::HashSet<&String> =
            hits.iter().map(|h| &h.id).collect();

        // 不变量 3a：活文档全可见——结果数 == 期望活文档数，且 id 集合相等。
        prop_assert_eq!(
            hits.len(),
            expected_live.len(),
            "merge lost docs: got {} hits, expected {} live (deleted={}, total={})",
            hits.len(), expected_live.len(), delete_ids.len(), total
        );
        for id in &expected_live {
            prop_assert!(
                hit_ids.contains(id),
                "live doc {} not visible after merge+compact", id
            );
        }

        // 不变量 3b：tombstoned 文档不可见。
        for id in &delete_ids {
            prop_assert!(
                !hit_ids.contains(id),
                "tombstoned doc {} visible after merge+compact", id
            );
        }

        // 不变量 3c：无重复 docid——hits.len() == unique id count。
        prop_assert_eq!(
            hits.len(),
            hit_ids.len(),
            "duplicate docids: {} hits, {} unique ids", hits.len(), hit_ids.len()
        );

        db.close().unwrap();
    }
}
