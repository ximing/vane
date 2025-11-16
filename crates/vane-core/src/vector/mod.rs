//! 暴力向量检索（SPEC §8.1 vector 模式，M0）。
//!
//! 纯函数式：无状态、无 IO。消费 ScoredDoc / Metric，产出 brute_search。

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use crate::types::{Metric, ScoredDoc, DIM_MAX};

// M2-09：SQ8 标量量化（feature `sq8`，纯算术，wasm32 可编译）。
#[cfg(feature = "sq8")]
pub mod sq8;

/// f32 的全序包装：NaN 视为 -∞（最小），保证 BinaryHeap 可用。
/// 这是 score 排序的唯一真相源，避免 f32 无 Ord 导致堆污染。
///
/// M2-09：`pub(crate)` 供 `vector::sq8` 模块复用同一排序逻辑（避免重复实现 NaN 处理）。
#[derive(Debug, Clone, Copy)]
pub(crate) struct Keyf32(f32);

impl Keyf32 {
    pub(crate) fn val(self) -> f32 {
        self.0
    }
}

impl PartialEq for Keyf32 {
    fn eq(&self, other: &Self) -> bool {
        self.0.to_bits() == other.0.to_bits()
    }
}
impl Eq for Keyf32 {}

impl PartialOrd for Keyf32 {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Keyf32 {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // NaN 视为最小
        match (self.0.is_nan(), other.0.is_nan()) {
            (true, true) => std::cmp::Ordering::Equal,
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            (false, false) => self.0.total_cmp(&other.0),
        }
    }
}

/// cosine 相似度 = (a·b) / (|a|·|b|)。零向量返回 0.0。
///
/// 维度校验：debug_assert a.len() == b.len()（上层保证；本层防御性）。
fn cosine_score(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len(), "cosine_score: dim mismatch");
    let mut dot = 0.0_f32;
    let mut na = 0.0_f32;
    let mut nb = 0.0_f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    let denom = na.sqrt() * nb.sqrt();
    if denom == 0.0_f32 || !denom.is_finite() {
        return 0.0_f32; // 零向量或溢出，无信息
    }
    dot / denom
}

/// L2 score = -|a-b|（负欧氏距离，越大越相似）。
fn l2_score(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len(), "l2_score: dim mismatch");
    let mut sum_sq = 0.0_f32;
    for i in 0..a.len() {
        let d = a[i] - b[i];
        sum_sq += d * d;
    }
    -sum_sq.sqrt()
}

/// dot score = a·b（未归一化点积）。
fn dot_score(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len(), "dot_score: dim mismatch");
    let mut s = 0.0_f32;
    for i in 0..a.len() {
        s += a[i] * b[i];
    }
    s
}

/// 暴力向量扫描（SPEC §8.1 vector 模式；M0 无 HNSW）。
///
/// - `vectors`: 扁平 f32 数组，doc i 的向量 = `vectors[i*dim .. (i+1)*dim]`
/// - `dim`: 单向量维度；必须 `query.len() == dim` 且 `vectors.len() % dim == 0`
/// - `query`: 查询向量
/// - `metric`: 距离度量（cosine / l2 / dot）
/// - `topk`: 返回前 topk 个；实际返回数 `min(topk, 命中文档数)`
/// - `filter`: `Some(bitmap)` 时只扫描位图中的 docid；
///   `local_index = docid - docid_base`，越界项静默跳过
/// - `docid_base`: 段内起始 docid，结果 docid = `docid_base + local_index`
///
/// 返回：按 score 降序，同分按 docid 升序。空输入返回空 Vec。
///
/// 非法输入（dim=0 / dim>DIM_MAX / query.len()!=dim / vectors.len()%dim!=0）
/// 返回空 Vec，不 panic。严格错误码由上层 SearchQuery 校验产出。
pub fn brute_search(
    vectors: &[f32],
    dim: u32,
    query: &[f32],
    metric: Metric,
    topk: usize,
    filter: Option<&roaring::RoaringBitmap>,
    docid_base: u64,
) -> Vec<ScoredDoc> {
    // ---- 维度校验（不 panic，非法输入返回空 Vec）----
    if dim == 0 || dim > DIM_MAX {
        return Vec::new();
    }
    let dim = dim as usize;
    if query.len() != dim || !vectors.len().is_multiple_of(dim) {
        return Vec::new();
    }
    if topk == 0 {
        return Vec::new();
    }
    let doc_count = vectors.len() / dim;
    if doc_count == 0 {
        return Vec::new();
    }

    // ---- score 分派 ----
    let score_fn: fn(&[f32], &[f32]) -> f32 = match metric {
        Metric::Cosine => cosine_score,
        Metric::L2 => l2_score,
        Metric::Dot => dot_score,
    };

    // ---- 最小堆保留 topK ----
    // 堆元素 Reverse<(Keyf32, u64)>：BinaryHeap 是最大堆，Reverse 后堆顶=最小 score。
    // 堆满（size > topk）时弹出最小，保留 topK 个最大。
    let mut heap: BinaryHeap<Reverse<(Keyf32, u64)>> = BinaryHeap::with_capacity(topk + 1);

    match filter {
        None => {
            for i in 0..doc_count {
                let v = &vectors[i * dim..(i + 1) * dim];
                let s = score_fn(v, query);
                let key = if s.is_finite() {
                    Keyf32(s)
                } else {
                    Keyf32(f32::NEG_INFINITY)
                };
                heap.push(Reverse((key, docid_base + i as u64)));
                if heap.len() > topk {
                    heap.pop();
                }
            }
        }
        Some(bm) => {
            // 只扫描位图中的 docid；local_index = docid - docid_base
            // roaring 迭代器产出 u32，这里转 u64 与 docid_base 对齐
            for docid_u32 in bm.iter() {
                let docid = docid_u32 as u64;
                // 越界静默跳过（防御性：调用方可能传跨段合并位图）
                if docid < docid_base {
                    continue;
                }
                let local = (docid - docid_base) as usize;
                if local >= doc_count {
                    continue;
                }
                let v = &vectors[local * dim..(local + 1) * dim];
                let s = score_fn(v, query);
                let key = if s.is_finite() {
                    Keyf32(s)
                } else {
                    Keyf32(f32::NEG_INFINITY)
                };
                heap.push(Reverse((key, docid)));
                if heap.len() > topk {
                    heap.pop();
                }
            }
        }
    }

    // ---- 堆 -> 有序 Vec（降序，同分 docid 升序）----
    let mut out: Vec<ScoredDoc> = Vec::with_capacity(heap.len());
    while let Some(Reverse((key, docid))) = heap.pop() {
        out.push(ScoredDoc {
            docid,
            score: key.val(),
        });
    }
    // pop 出来是升序（最小堆），反转得降序
    out.reverse();

    // 同分按 docid 升序：显式 sort_by 保证确定性，不依赖入堆顺序的隐式不变量。
    out.sort_by(|a, b| {
        // score 降序（b.score vs a.score）；同分 docid 升序（a.docid vs b.docid）
        match Keyf32(b.score).cmp(&Keyf32(a.score)) {
            std::cmp::Ordering::Equal => a.docid.cmp(&b.docid),
            other => other,
        }
    });

    out
}

// ---------------------------------------------------------------------------
// M2-09：暴力搜索分发层（I-5 B-1 fix：cfg(feature="sq8") 下沉到 vector 模块）
// ---------------------------------------------------------------------------

/// 暴力搜索分发——feature `sq8` 时优先 SQ8 量化路径（内存降 4 倍），
/// 否则/空段时 f32 `brute_search`。
///
/// **I-5 fix round 1**：`cfg(feature="sq8")` 仅出现在 vector 模块（编解码处），
/// api/collection.rs search 路径零 cfg 属性——调用本函数时不感知 feature。
///
/// - feature on + reader.sq8_vectors() 返回 Some → `brute_search_sq8`（SQ8 量化路径）
/// - feature off / 空段（None） → `brute_search`（f32 精确路径）
///
/// SQ8 仅用于暴力回退路径（HNSW 导航仍用 f32，精度优先，首选方案）。
/// `brute_search` 原签名不变；本函数是 additive 分发层。
pub fn brute_search_dispatch(
    reader: &crate::segment::SegmentReader,
    qv: &[f32],
    metric: Metric,
    want: usize,
    merged_filter: Option<&roaring::RoaringBitmap>,
    base: u64,
) -> Vec<ScoredDoc> {
    let dim = reader.dim();
    #[cfg(feature = "sq8")]
    if let Some(bundle) = reader.sq8_vectors() {
        return sq8::brute_search_sq8(bundle, dim, qv, metric, want, merged_filter, base);
    }
    brute_search(reader.vectors(), dim, qv, metric, want, merged_filter, base)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Metric, ScoredDoc};
    use roaring::RoaringBitmap;

    fn approx_eq(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    // ---- Task 1: score 函数 ----

    #[test]
    fn cosine_identical_vectors_is_one() {
        let a = [1.0_f32, 0.0, 0.0];
        let b = [1.0_f32, 0.0, 0.0];
        let s = cosine_score(&a, &b);
        assert!((s - 1.0).abs() < 1e-6, "got {s}");
    }

    #[test]
    fn cosine_orthogonal_is_zero() {
        let a = [1.0_f32, 0.0];
        let b = [0.0_f32, 1.0];
        assert!(cosine_score(&a, &b).abs() < 1e-6);
    }

    #[test]
    fn cosine_opposite_is_minus_one() {
        let a = [1.0_f32, 0.0];
        let b = [-1.0_f32, 0.0];
        assert!((cosine_score(&a, &b) - (-1.0)).abs() < 1e-6);
    }

    #[test]
    fn cosine_zero_vector_returns_zero() {
        // 零向量：|a|=0，无法归一化，约定返回 0.0（无信息），不得 NaN
        let a = [0.0_f32, 0.0, 0.0];
        let b = [1.0_f32, 2.0, 3.0];
        assert_eq!(cosine_score(&a, &b), 0.0);
        assert_eq!(cosine_score(&b, &a), 0.0);
        assert_eq!(cosine_score(&a, &a), 0.0);
    }

    #[test]
    fn cosine_dim_mismatch_debug_assert_exists() {
        // debug_assert_eq 在 debug 下校验维度一致；这里用相同维度验证不 panic，
        // 维度不一致的 panic 行为依赖编译模式，不做断言（上层 schema 保证维度）。
        let a = [1.0_f32, 0.0];
        let b = [0.5_f32, 0.5];
        debug_assert_eq!(a.len(), b.len());
        let _ = cosine_score(&a, &b);
    }

    #[test]
    fn l2_identical_is_zero() {
        let a = [1.0_f32, 2.0, 3.0];
        assert_eq!(l2_score(&a, &a), 0.0); // score = -|a-b| = 0
    }

    #[test]
    fn l2_distance_negated() {
        let a = [0.0_f32, 0.0];
        let b = [3.0_f32, 4.0];
        // |a-b| = 5, score = -5
        assert!((l2_score(&a, &b) - (-5.0)).abs() < 1e-6);
    }

    #[test]
    fn l2_larger_distance_lower_score() {
        // 距离越大 score 越小（越负）
        let a = [0.0_f32];
        let near = [1.0_f32];
        let far = [10.0_f32];
        assert!(l2_score(&a, &near) > l2_score(&a, &far));
    }

    #[test]
    fn dot_basic() {
        let a = [1.0_f32, 2.0, 3.0];
        let b = [4.0_f32, 5.0, 6.0];
        // 4+10+18 = 32
        assert!((dot_score(&a, &b) - 32.0).abs() < 1e-5);
    }

    #[test]
    fn dot_orthogonal_is_zero() {
        let a = [1.0_f32, 0.0];
        let b = [0.0_f32, 5.0];
        assert!(dot_score(&a, &b).abs() < 1e-6);
    }

    #[test]
    fn dot_can_be_negative() {
        let a = [1.0_f32, -1.0];
        let b = [-1.0_f32, 1.0];
        // -1 + -1 = -2
        assert!((dot_score(&a, &b) - (-2.0)).abs() < 1e-6);
    }

    #[test]
    fn keyf32_orders_nan_as_min() {
        let nan = Keyf32(f32::NAN);
        let neg = Keyf32(-1.0_f32);
        assert!(nan < neg);
        assert!(neg > nan);
    }

    #[test]
    fn keyf32_eq_bitwise() {
        assert_ne!(Keyf32(0.0_f32), Keyf32(-0.0_f32)); // bits 不同 -> 不等
        assert_eq!(Keyf32(1.5), Keyf32(1.5));
    }

    // ---- Task 2: brute_search topK ----

    #[test]
    fn brute_cosine_topk_basic() {
        // 4 个 2 维向量，query=[1,0]
        // cosine: v0=[1,0]->1.0, v1=[0,1]->0.0, v2=[-1,0]->-1.0, v3=[1,1]->0.7071
        let vectors: Vec<f32> = vec![1.0, 0.0, 0.0, 1.0, -1.0, 0.0, 1.0, 1.0];
        let query = [1.0_f32, 0.0];
        let res = brute_search(&vectors, 2, &query, Metric::Cosine, 2, None, 0);
        assert_eq!(res.len(), 2);
        // 降序：v0 (1.0) > v3 (0.7071)
        assert_eq!(res[0].docid, 0);
        assert!(approx_eq(res[0].score, 1.0));
        assert_eq!(res[1].docid, 3);
        assert!(approx_eq(res[1].score, 1.0_f32 / 2.0_f32.sqrt()));
    }

    #[test]
    fn brute_l2_topk_order() {
        // query=[0,0]，最近的是 v0=[1,0]（dist=1），次近 v1=[2,0]（dist=2）
        let vectors: Vec<f32> = vec![1.0, 0.0, 2.0, 0.0, 5.0, 0.0];
        let query = [0.0_f32, 0.0];
        let res = brute_search(&vectors, 2, &query, Metric::L2, 2, None, 100);
        assert_eq!(res.len(), 2);
        assert_eq!(res[0].docid, 100); // docid_base 偏移
        assert!(approx_eq(res[0].score, -1.0));
        assert_eq!(res[1].docid, 101);
        assert!(approx_eq(res[1].score, -2.0));
    }

    #[test]
    fn brute_dot_topk() {
        let vectors: Vec<f32> = vec![1.0, 1.0, 2.0, 2.0]; // v0 dot q=1, v1 dot q=2
        let query = [1.0_f32, 1.0];
        let res = brute_search(&vectors, 2, &query, Metric::Dot, 2, None, 0);
        assert_eq!(res[0].docid, 1);
        assert!(approx_eq(res[0].score, 4.0));
        assert_eq!(res[1].docid, 0);
        assert!(approx_eq(res[1].score, 2.0));
    }

    #[test]
    fn brute_filter_only_scanned_docs_in_bitmap() {
        let vectors: Vec<f32> = vec![1.0, 0.0, 1.0, 0.0, 1.0, 0.0]; // 3 个相同向量
        let query = [1.0_f32, 0.0];
        let mut bm = RoaringBitmap::new();
        // 位图存绝对 docid；local_index=1 -> docid = 1000+1 = 1001
        bm.insert(1001);
        let res = brute_search(&vectors, 2, &query, Metric::Cosine, 2, Some(&bm), 1000);
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].docid, 1001);
    }

    #[test]
    fn brute_filter_bitmap_out_of_range_skipped() {
        // 位图含 docid 超出段范围（local_index >= doc_count），静默跳过
        let vectors: Vec<f32> = vec![1.0, 0.0]; // 1 个向量
        let query = [1.0_f32, 0.0];
        let mut bm = RoaringBitmap::new();
        bm.insert(0);
        bm.insert(999); // 越界
        let res = brute_search(&vectors, 2, &query, Metric::Cosine, 2, Some(&bm), 0);
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].docid, 0);
    }

    #[test]
    fn brute_docid_base_offset_applied() {
        let vectors: Vec<f32> = vec![1.0, 0.0, 1.0, 0.0];
        let query = [1.0_f32, 0.0];
        let res = brute_search(&vectors, 2, &query, Metric::Cosine, 2, None, 42);
        assert_eq!(res[0].docid, 42);
        assert_eq!(res[1].docid, 43);
    }

    #[test]
    fn brute_topk_full_results_when_eq_doc_count() {
        let vectors: Vec<f32> = vec![1.0, 0.0, 0.0, 1.0];
        let query = [1.0_f32, 0.0];
        let res = brute_search(&vectors, 2, &query, Metric::Cosine, 5, None, 0);
        assert_eq!(res.len(), 2); // 只有 2 个 doc，topK=5 也只返回 2
    }

    #[test]
    fn brute_results_sorted_desc_by_score() {
        // 随机乱序向量，验证输出严格降序（允许同分按 docid 升序）
        // v0=[0.1,0.1] 与 query=[1,0] 不同向（cosine≈0.707），避免与 v1 同分
        let vectors: Vec<f32> = vec![0.1, 0.1, 1.0, 0.0, 0.5, 0.0, -1.0, 0.0];
        let query = [1.0_f32, 0.0];
        let res = brute_search(&vectors, 2, &query, Metric::Cosine, 4, None, 0);
        assert_eq!(res.len(), 4);
        for w in res.windows(2) {
            assert!(
                w[0].score >= w[1].score,
                "not desc: {:?} vs {:?}",
                w[0],
                w[1]
            );
        }
        assert_eq!(res[0].docid, 1); // 最相似
        assert_eq!(res[3].docid, 3); // 最不相似
    }

    #[test]
    fn brute_tie_break_by_docid_ascending() {
        // 两个相同向量，同分，docid 小的排前
        let vectors: Vec<f32> = vec![1.0, 0.0, 1.0, 0.0];
        let query = [1.0_f32, 0.0];
        let res = brute_search(&vectors, 2, &query, Metric::Cosine, 2, None, 10);
        assert_eq!(res[0].docid, 10);
        assert_eq!(res[1].docid, 11);
        assert!(approx_eq(res[0].score, res[1].score));
    }

    // ---- Task 3: 边界与错误用例 ----

    #[test]
    fn brute_empty_vectors_returns_empty() {
        let query = [1.0_f32, 0.0];
        let res = brute_search(&[], 2, &query, Metric::Cosine, 10, None, 0);
        assert!(res.is_empty());
    }

    #[test]
    fn brute_topk_zero_returns_empty() {
        let vectors: Vec<f32> = vec![1.0, 0.0, 0.0, 1.0];
        let query = [1.0_f32, 0.0];
        let res = brute_search(&vectors, 2, &query, Metric::Cosine, 0, None, 0);
        assert!(res.is_empty());
    }

    #[test]
    fn brute_topk_exceeds_doc_count_returns_all() {
        // topK=10 但只有 3 个 doc，返回 3 个（降序）
        let vectors: Vec<f32> = vec![1.0, 0.0, 0.5, 0.0, 0.1, 0.0];
        let query = [1.0_f32, 0.0];
        let res = brute_search(&vectors, 2, &query, Metric::Cosine, 10, None, 0);
        assert_eq!(res.len(), 3);
        // 降序
        assert!(res[0].score >= res[1].score);
        assert!(res[1].score >= res[2].score);
        assert_eq!(res[0].docid, 0);
        assert_eq!(res[1].docid, 1);
        assert_eq!(res[2].docid, 2);
    }

    #[test]
    fn brute_dim_zero_returns_empty() {
        let query: [f32; 0] = [];
        let res = brute_search(&[], 0, &query, Metric::Cosine, 10, None, 0);
        assert!(res.is_empty());
    }

    #[test]
    fn brute_dim_exceeds_max_returns_empty() {
        // DIM_MAX = 4096；dim=4097 应被拒
        let dim = 4097_u32;
        let query = vec![0.0_f32; dim as usize];
        let vectors = vec![0.0_f32; dim as usize];
        let res = brute_search(&vectors, dim, &query, Metric::Cosine, 1, None, 0);
        assert!(res.is_empty(), "dim > DIM_MAX should return empty");
    }

    #[test]
    fn brute_dim_just_at_max_ok() {
        let dim = 4096_u32;
        let query = vec![1.0_f32; dim as usize];
        let vectors = vec![1.0_f32; dim as usize]; // 1 个向量
        let res = brute_search(&vectors, dim, &query, Metric::Cosine, 1, None, 0);
        assert_eq!(res.len(), 1);
        assert!(approx_eq(res[0].score, 1.0));
    }

    #[test]
    fn brute_query_dim_mismatch_returns_empty() {
        // query.len() != dim
        let vectors: Vec<f32> = vec![1.0, 0.0, 0.0, 1.0]; // dim=2, 2 docs
        let query = [1.0_f32, 0.0, 0.0]; // len=3 != 2
        let res = brute_search(&vectors, 2, &query, Metric::Cosine, 2, None, 0);
        assert!(res.is_empty());
    }

    #[test]
    fn brute_vectors_not_multiple_of_dim_returns_empty() {
        // vectors.len()=5 不是 dim=2 的整数倍
        let vectors: Vec<f32> = vec![1.0, 0.0, 0.0, 1.0, 0.5];
        let query = [1.0_f32, 0.0];
        let res = brute_search(&vectors, 2, &query, Metric::Cosine, 2, None, 0);
        assert!(res.is_empty());
    }

    #[test]
    fn brute_filter_empty_bitmap_returns_empty() {
        let vectors: Vec<f32> = vec![1.0, 0.0, 0.0, 1.0];
        let query = [1.0_f32, 0.0];
        let bm = RoaringBitmap::new(); // 空
        let res = brute_search(&vectors, 2, &query, Metric::Cosine, 2, Some(&bm), 0);
        assert!(res.is_empty());
    }

    #[test]
    fn brute_filter_all_out_of_range_returns_empty() {
        let vectors: Vec<f32> = vec![1.0, 0.0]; // 1 doc
        let query = [1.0_f32, 0.0];
        let mut bm = RoaringBitmap::new();
        bm.insert(100);
        bm.insert(200);
        let res = brute_search(&vectors, 2, &query, Metric::Cosine, 2, Some(&bm), 0);
        assert!(res.is_empty());
    }

    #[test]
    fn brute_filter_with_docid_base_offset() {
        // docid_base=50；位图含 docid=51 -> local=1
        let vectors: Vec<f32> = vec![
            0.0, 0.0, 0.0, 0.0, // local 0: 零向量
            1.0, 0.0, 0.0, 0.0, // local 1
        ];
        let query = [1.0_f32, 0.0, 0.0, 0.0];
        let mut bm = RoaringBitmap::new();
        bm.insert(50); // local 0
        bm.insert(51); // local 1
        let res = brute_search(&vectors, 4, &query, Metric::Cosine, 2, Some(&bm), 50);
        assert_eq!(res.len(), 2);
        assert_eq!(res[0].docid, 51); // cosine=1.0
        assert_eq!(res[1].docid, 50); // 零向量 cosine=0.0
    }

    #[test]
    fn brute_filter_below_docid_base_skipped() {
        // 位图含 docid < docid_base，静默跳过
        let vectors: Vec<f32> = vec![1.0, 0.0]; // 1 doc
        let query = [1.0_f32, 0.0];
        let mut bm = RoaringBitmap::new();
        bm.insert(0); // < docid_base=100，跳过
        bm.insert(100); // local 0
        let res = brute_search(&vectors, 2, &query, Metric::Cosine, 2, Some(&bm), 100);
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].docid, 100);
    }

    #[test]
    fn brute_single_vector_returns_one() {
        let vectors: Vec<f32> = vec![1.0, 0.0];
        let query = [1.0_f32, 0.0];
        let res = brute_search(&vectors, 2, &query, Metric::Cosine, 1, None, 0);
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].docid, 0);
        assert!(approx_eq(res[0].score, 1.0));
    }

    #[test]
    fn brute_all_three_metrics_on_same_data() {
        // 同一份数据跑三种 metric，确保都返回 topK 且不 panic
        let vectors: Vec<f32> = vec![1.0, 0.0, 0.0, 1.0, 1.0, 1.0, -1.0, -1.0];
        let query = [1.0_f32, 1.0];
        for metric in [Metric::Cosine, Metric::L2, Metric::Dot] {
            let res = brute_search(&vectors, 2, &query, metric, 2, None, 0);
            assert_eq!(res.len(), 2, "metric {:?} returned wrong len", metric);
            assert!(res[0].score >= res[1].score, "metric {:?} not desc", metric);
        }
    }

    #[test]
    fn brute_nan_in_query_does_not_panic() {
        // 防御性：query 含 NaN（不应发生，但要保证不 panic、不污染堆序）
        let vectors: Vec<f32> = vec![1.0, 0.0, 0.0, 1.0];
        let query = [f32::NAN, 0.0];
        let res = brute_search(&vectors, 2, &query, Metric::Cosine, 2, None, 0);
        // 不 panic 即可；结果数仍为 min(topk, doc_count)
        assert!(res.len() <= 2);
    }

    #[test]
    #[ignore]
    fn perf_100k_384_cosine_top10() {
        let dim = 384_usize;
        let n = 100_000_usize;
        let vectors: Vec<f32> = (0..(n * dim))
            .map(|i| ((i as u32).wrapping_mul(2654435761) as f32) / (u32::MAX as f32))
            .collect();
        let query: Vec<f32> = (0..dim).map(|i| i as f32 / dim as f32).collect();
        let start = std::time::Instant::now();
        let res = brute_search(&vectors, dim as u32, &query, Metric::Cosine, 10, None, 0);
        let elapsed = start.elapsed();
        assert_eq!(res.len(), 10);
        eprintln!("100k x 384 cosine top10: {:?}", elapsed);
        assert!(elapsed.as_millis() < 150, "P99 预算超限: {:?}", elapsed);
    }

    // 显式引用 ScoredDoc 避免未使用 import 告警（测试中通过 brute_search 返回值间接使用）。
    #[test]
    fn scored_doc_type_is_used() {
        let _v: Vec<ScoredDoc> = Vec::new();
    }
}
