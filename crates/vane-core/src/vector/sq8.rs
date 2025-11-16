//! SQ8 标量量化（SPEC §13.1，M2-09）。
//!
//! 每维 1 字节（min/max + 256 级量化），内存降 4 倍。
//! feature `sq8` 可选；纯算术，无新依赖，wasm32 可编译。
//!
//! 设计：
//! - `Sq8Bundle{data,min,max}`：编码产物。`data` 为 `doc_count×dim` 字节量化数据；
//!   `min`/`max` 为 per-dim 标量边界（`Vec<f32>` 长度 = dim）。
//! - 距离计算不解码整段为 Vec<f32>，而是逐字节 on-the-fly dequantize（无分配）。
//! - query 量化一次复用全段扫描（reviewer B-M5）。
//! - 仅用于暴力回退路径（HNSW 导航仍用 f32，精度优先，SPEC §13.2-1）。
//!
//! 不变量 I-5：`cfg(feature="sq8")` 在 segment/vector 编解码处；core 算法零 `cfg(target)`。
//! 不变量 I-1：SQ8 是内存缓存，不写段文件（vectors.bin 仍 f32 落盘）。

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use crate::types::{Metric, ScoredDoc, DIM_MAX};
use crate::vector::Keyf32;

/// SQ8 编码产物（SPEC §13.1，M2-09）。
///
/// - `data`：量化字节，`doc_count × dim` 连续排布，doc i 的向量 = `data[i*dim..(i+1)*dim]`。
/// - `min`/`max`：per-dim 标量边界（长度 = dim），用于 dequantize：`v = min + (q/255)*(max-min)`。
///
/// 当 `max == min`（该维恒定）时，量化级为 0，dequantize 恒为 min（避免除零）。
#[derive(Debug, Clone)]
pub struct Sq8Bundle {
    pub data: Vec<u8>,
    pub min: Vec<f32>,
    pub max: Vec<f32>,
}

/// 将一段 f32 向量编码为 SQ8 量化字节（SPEC §13.1）。
///
/// - `vectors`：扁平 f32 数组，doc i 的向量 = `vectors[i*dim..(i+1)*dim]`。
/// - `dim`：单向量维度。
///
/// 编码方式：per-dim 计算 min/max，`q = round((v - min) / (max - min) * 255)`，clamp 到 [0,255]。
/// `max == min` 时该维量化为 0。
///
/// 非法输入（dim=0 / dim>DIM_MAX / vectors.len()%dim!=0）返回空 bundle。
pub fn encode_sq8(vectors: &[f32], dim: u32) -> Sq8Bundle {
    if dim == 0 || dim > DIM_MAX {
        return Sq8Bundle {
            data: Vec::new(),
            min: Vec::new(),
            max: Vec::new(),
        };
    }
    let dim = dim as usize;
    if !vectors.len().is_multiple_of(dim) {
        return Sq8Bundle {
            data: Vec::new(),
            min: Vec::new(),
            max: Vec::new(),
        };
    }
    let doc_count = vectors.len() / dim;
    if doc_count == 0 {
        return Sq8Bundle {
            data: Vec::new(),
            min: vec![0.0; dim],
            max: vec![0.0; dim],
        };
    }

    // per-dim min/max
    let mut min = vec![f32::INFINITY; dim];
    let mut max = vec![f32::NEG_INFINITY; dim];
    for i in 0..doc_count {
        let v = &vectors[i * dim..(i + 1) * dim];
        for d in 0..dim {
            if v[d] < min[d] {
                min[d] = v[d];
            }
            if v[d] > max[d] {
                max[d] = v[d];
            }
        }
    }

    // 量化
    let mut data = vec![0u8; doc_count * dim];
    for i in 0..doc_count {
        let v = &vectors[i * dim..(i + 1) * dim];
        for d in 0..dim {
            let range = max[d] - min[d];
            if range <= 0.0 || !range.is_finite() {
                data[i * dim + d] = 0;
            } else {
                let t = (v[d] - min[d]) / range;
                let q = (t * 255.0).round();
                // clamp to [0, 255]
                let q = q.clamp(0.0, 255.0) as u8;
                data[i * dim + d] = q;
            }
        }
    }

    Sq8Bundle { data, min, max }
}

/// dequantize 单字节：`v = min + (q/255)*(max-min)`。
#[inline]
fn dequant(q: u8, min: f32, max: f32) -> f32 {
    let range = max - min;
    if range <= 0.0 || !range.is_finite() {
        min
    } else {
        min + (q as f32 / 255.0) * range
    }
}

/// 将 SQ8 bundle 解码回 f32 向量（主要用于测试/验证）。
///
/// 返回扁平 Vec<f32>，doc i 的向量 = `result[i*dim..(i+1)*dim]`。
pub fn decode_sq8(bundle: &Sq8Bundle) -> Vec<f32> {
    let dim = bundle.min.len();
    if dim == 0 {
        return Vec::new();
    }
    let doc_count = bundle.data.len() / dim;
    let mut out = Vec::with_capacity(doc_count * dim);
    for i in 0..doc_count {
        for d in 0..dim {
            out.push(dequant(
                bundle.data[i * dim + d],
                bundle.min[d],
                bundle.max[d],
            ));
        }
    }
    out
}

/// 将单个 f32 query 向量量化为 SQ8 字节（复用 bundle 的 min/max）。
///
/// 用于 `sq8_query_distance`：query 量化一次，全段复用（reviewer B-M5）。
fn quantize_query(query: &[f32], min: &[f32], max: &[f32]) -> Vec<u8> {
    let dim = min.len();
    debug_assert_eq!(query.len(), dim);
    debug_assert_eq!(max.len(), dim);
    let mut q = vec![0u8; dim];
    for d in 0..dim {
        let range = max[d] - min[d];
        if range <= 0.0 || !range.is_finite() {
            q[d] = 0;
        } else {
            let t = (query[d] - min[d]) / range;
            let v = (t * 255.0).round().clamp(0.0, 255.0) as u8;
            q[d] = v;
        }
    }
    q
}

/// 近似 cosine score（dequantize 后计算）。
fn sq8_cosine_score(a: &[u8], b: &[u8], min: &[f32], max: &[f32]) -> f32 {
    let mut dot = 0.0_f32;
    let mut na = 0.0_f32;
    let mut nb = 0.0_f32;
    for d in 0..a.len() {
        let va = dequant(a[d], min[d], max[d]);
        let vb = dequant(b[d], min[d], max[d]);
        dot += va * vb;
        na += va * va;
        nb += vb * vb;
    }
    let denom = na.sqrt() * nb.sqrt();
    if denom == 0.0 || !denom.is_finite() {
        return 0.0;
    }
    dot / denom
}

/// 近似 L2 score = -|a-b|（dequantize 后计算）。
fn sq8_l2_score(a: &[u8], b: &[u8], min: &[f32], max: &[f32]) -> f32 {
    let mut sum_sq = 0.0_f32;
    for d in 0..a.len() {
        let va = dequant(a[d], min[d], max[d]);
        let vb = dequant(b[d], min[d], max[d]);
        let diff = va - vb;
        sum_sq += diff * diff;
    }
    -sum_sq.sqrt()
}

/// 近似 dot score = a·b（dequantize 后计算）。
fn sq8_dot_score(a: &[u8], b: &[u8], min: &[f32], max: &[f32]) -> f32 {
    let mut s = 0.0_f32;
    for d in 0..a.len() {
        s += dequant(a[d], min[d], max[d]) * dequant(b[d], min[d], max[d]);
    }
    s
}

/// 两个 SQ8 量化向量间的近似距离（SPEC §13.1，M2-09）。
///
/// - `sq8_a`/`sq8_b`：量化字节（长度 = dim），来自同一 bundle 的 min/max 体系。
/// - `min`/`max`：per-dim 标量边界（编码时计算，dequantize 用）。
/// - `metric`：cosine / L2 / dot，覆盖三种 metric（reviewer A-I3/B-I2）。
///
/// 不解码整段为 Vec<f32>；逐字节 on-the-fly dequantize（无分配，快速）。
pub fn sq8_distance(sq8_a: &[u8], sq8_b: &[u8], min: &[f32], max: &[f32], metric: Metric) -> f32 {
    debug_assert_eq!(sq8_a.len(), sq8_b.len());
    debug_assert_eq!(sq8_a.len(), min.len());
    debug_assert_eq!(min.len(), max.len());
    match metric {
        Metric::Cosine => sq8_cosine_score(sq8_a, sq8_b, min, max),
        Metric::L2 => sq8_l2_score(sq8_a, sq8_b, min, max),
        Metric::Dot => sq8_dot_score(sq8_a, sq8_b, min, max),
    }
}

/// SQ8 暴力扫描（SPEC §13.1，M2-09）。
///
/// - `sq8_vectors`：量化字节（`doc_count × dim` 连续排布）。
/// - `min`/`max`：per-dim 标量边界。
/// - `query`：f32 查询向量（量化一次复用全段，reviewer B-M5）。
/// - 其余参数与 `brute_search` 对齐（`metric` + `docid_base`，reviewer A-I3/B-I2）。
///
/// 返回：按 score 降序，同分按 docid 升序（与 `brute_search` 一致）。
#[allow(clippy::too_many_arguments)]
pub fn sq8_query_distance(
    sq8_vectors: &[u8],
    min: &[f32],
    max: &[f32],
    dim: u32,
    query: &[f32],
    metric: Metric,
    topk: usize,
    filter: Option<&roaring::RoaringBitmap>,
    docid_base: u64,
) -> Vec<ScoredDoc> {
    if dim == 0 || dim > DIM_MAX {
        return Vec::new();
    }
    let dim_usize = dim as usize;
    if query.len() != dim_usize
        || min.len() != dim_usize
        || max.len() != dim_usize
        || !sq8_vectors.len().is_multiple_of(dim_usize)
    {
        return Vec::new();
    }
    if topk == 0 {
        return Vec::new();
    }
    let doc_count = sq8_vectors.len() / dim_usize;
    if doc_count == 0 {
        return Vec::new();
    }

    // query 量化一次，全段复用（避免每向量解码回 f32）
    let sq8_q = quantize_query(query, min, max);

    type ScoreFn = fn(&[u8], &[u8], &[f32], &[f32]) -> f32;
    let score_fn: ScoreFn = match metric {
        Metric::Cosine => sq8_cosine_score,
        Metric::L2 => sq8_l2_score,
        Metric::Dot => sq8_dot_score,
    };

    let mut heap: BinaryHeap<Reverse<(Keyf32, u64)>> = BinaryHeap::with_capacity(topk + 1);

    match filter {
        None => {
            for i in 0..doc_count {
                let v = &sq8_vectors[i * dim_usize..(i + 1) * dim_usize];
                let s = score_fn(v, &sq8_q, min, max);
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
            for docid_u32 in bm.iter() {
                let docid = docid_u32 as u64;
                if docid < docid_base {
                    continue;
                }
                let local = (docid - docid_base) as usize;
                if local >= doc_count {
                    continue;
                }
                let v = &sq8_vectors[local * dim_usize..(local + 1) * dim_usize];
                let s = score_fn(v, &sq8_q, min, max);
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

    let mut out: Vec<ScoredDoc> = Vec::with_capacity(heap.len());
    while let Some(Reverse((key, docid))) = heap.pop() {
        out.push(ScoredDoc {
            docid,
            score: key.val(),
        });
    }
    out.reverse();
    out.sort_by(|a, b| match Keyf32(b.score).cmp(&Keyf32(a.score)) {
        std::cmp::Ordering::Equal => a.docid.cmp(&b.docid),
        other => other,
    });
    out
}

/// brute_search_sq8：SQ8 暴力搜索（与 `brute_search` 签名对齐，reviewer A-I3/B-I2）。
///
/// 与 `brute_search` 的区别：第一参数为 `&Sq8Bundle`（含 data + min + max），
/// 内部调 `sq8_query_distance`。`brute_search` 原签名不变。
pub fn brute_search_sq8(
    bundle: &Sq8Bundle,
    dim: u32,
    query: &[f32],
    metric: Metric,
    topk: usize,
    filter: Option<&roaring::RoaringBitmap>,
    docid_base: u64,
) -> Vec<ScoredDoc> {
    sq8_query_distance(
        &bundle.data,
        &bundle.min,
        &bundle.max,
        dim,
        query,
        metric,
        topk,
        filter,
        docid_base,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Metric;
    use roaring::RoaringBitmap;

    fn approx_eq(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() < eps
    }

    // ---- 测试 1: encode/decode roundtrip 误差 ----

    #[test]
    fn encode_decode_roundtrip_small_error() {
        let dim = 4_u32;
        let vectors: Vec<f32> = vec![
            0.1, 0.5, -0.3, 0.8, 0.2, 0.6, -0.1, 0.9, -0.5, 0.0, 0.4, 0.3,
        ];
        let bundle = encode_sq8(&vectors, dim);
        let decoded = decode_sq8(&bundle);
        assert_eq!(decoded.len(), vectors.len());
        // 每维误差 < 1/(255) * (max-min)（SQ8 量化精度）
        for d in 0..dim as usize {
            let range = bundle.max[d] - bundle.min[d];
            let eps = range / 255.0 + 1e-6;
            for i in 0..3 {
                let orig = vectors[i * dim as usize + d];
                let dec = decoded[i * dim as usize + d];
                assert!(
                    approx_eq(orig, dec, eps),
                    "dim {d} doc {i}: orig={orig} dec={dec} eps={eps}"
                );
            }
        }
    }

    #[test]
    fn encode_decode_roundtrip_dim_384() {
        let dim = 384_u32;
        let vectors: Vec<f32> = (0..(10 * dim as usize))
            .map(|i| ((i as u32).wrapping_mul(2654435761) as f32) / (u32::MAX as f32) - 0.5)
            .collect();
        let bundle = encode_sq8(&vectors, dim);
        let decoded = decode_sq8(&bundle);
        assert_eq!(decoded.len(), vectors.len());
        // 最大误差 < max_range/255
        let max_range: f32 = (0..dim as usize)
            .map(|d| bundle.max[d] - bundle.min[d])
            .fold(0.0_f32, f32::max);
        let eps = max_range / 255.0 + 1e-5;
        for (i, (o, d)) in vectors.iter().zip(decoded.iter()).enumerate() {
            assert!(
                approx_eq(*o, *d, eps),
                "idx {i}: orig={o} dec={d} eps={eps}"
            );
        }
    }

    // ---- 测试 2: encode 内存缩减 ----

    #[test]
    fn encode_memory_reduction_4x() {
        let dim = 384_u32;
        let doc_count = 1000_usize;
        let vectors: Vec<f32> = vec![0.5; doc_count * dim as usize];
        let bundle = encode_sq8(&vectors, dim);
        // f32 = 4 bytes/dim, sq8 = 1 byte/dim → 4 倍降
        let f32_bytes = doc_count * dim as usize * 4;
        let sq8_bytes = bundle.data.len();
        assert_eq!(sq8_bytes, doc_count * dim as usize);
        assert_eq!(f32_bytes / sq8_bytes, 4);
    }

    // ---- 测试 3: sq8_distance vs f32 distance（三 metric 覆盖）----

    #[test]
    fn sq8_distance_vs_f32_cosine() {
        let dim = 8_u32;
        let vectors: Vec<f32> = (0..(4 * dim as usize))
            .map(|i| ((i as u32).wrapping_mul(2654435761) as f32) / (u32::MAX as f32) - 0.5)
            .collect();
        let bundle = encode_sq8(&vectors, dim);
        let a = &bundle.data[0..dim as usize];
        let b = &bundle.data[dim as usize..2 * dim as usize];
        let sq8_d = sq8_distance(a, b, &bundle.min, &bundle.max, Metric::Cosine);
        // f32 精确 cosine
        let va = &vectors[0..dim as usize];
        let vb = &vectors[dim as usize..2 * dim as usize];
        let mut dot = 0.0_f32;
        let mut na = 0.0_f32;
        let mut nb = 0.0_f32;
        for d in 0..dim as usize {
            dot += va[d] * vb[d];
            na += va[d] * va[d];
            nb += vb[d] * vb[d];
        }
        let f32_d = dot / (na.sqrt() * nb.sqrt());
        // 误差 < 1%
        let err = ((sq8_d - f32_d).abs() / f32_d.abs().max(1e-6)) * 100.0;
        assert!(err < 1.0, "cosine err {err}%: sq8={sq8_d} f32={f32_d}");
    }

    #[test]
    fn sq8_distance_vs_f32_l2() {
        let dim = 8_u32;
        let vectors: Vec<f32> = (0..(4 * dim as usize))
            .map(|i| ((i as u32).wrapping_mul(2654435761) as f32) / (u32::MAX as f32) - 0.5)
            .collect();
        let bundle = encode_sq8(&vectors, dim);
        let a = &bundle.data[0..dim as usize];
        let b = &bundle.data[dim as usize..2 * dim as usize];
        let sq8_d = sq8_distance(a, b, &bundle.min, &bundle.max, Metric::L2);
        let va = &vectors[0..dim as usize];
        let vb = &vectors[dim as usize..2 * dim as usize];
        let mut sum_sq = 0.0_f32;
        for d in 0..dim as usize {
            let diff = va[d] - vb[d];
            sum_sq += diff * diff;
        }
        let f32_d = -sum_sq.sqrt();
        let err = ((sq8_d - f32_d).abs() / f32_d.abs().max(1e-6)) * 100.0;
        assert!(err < 1.0, "l2 err {err}%: sq8={sq8_d} f32={f32_d}");
    }

    #[test]
    fn sq8_distance_vs_f32_dot() {
        let dim = 8_u32;
        let vectors: Vec<f32> = (0..(4 * dim as usize))
            .map(|i| ((i as u32).wrapping_mul(2654435761) as f32) / (u32::MAX as f32) - 0.5)
            .collect();
        let bundle = encode_sq8(&vectors, dim);
        let a = &bundle.data[0..dim as usize];
        let b = &bundle.data[dim as usize..2 * dim as usize];
        let sq8_d = sq8_distance(a, b, &bundle.min, &bundle.max, Metric::Dot);
        let va = &vectors[0..dim as usize];
        let vb = &vectors[dim as usize..2 * dim as usize];
        let f32_d: f32 = (0..dim as usize).map(|d| va[d] * vb[d]).sum();
        let err = ((sq8_d - f32_d).abs() / f32_d.abs().max(1e-6)) * 100.0;
        assert!(err < 1.0, "dot err {err}%: sq8={sq8_d} f32={f32_d}");
    }

    // ---- 测试 4: brute_search_sq8 召回 ----

    #[test]
    fn brute_search_sq8_recall_vs_f32() {
        let dim = 128_u32;
        let doc_count = 1000_usize;
        // 确定性伪随机向量（值域 [-1, 1]，范围足够大使 SQ8 量化精度充足）。
        let vectors: Vec<f32> = (0..(doc_count * dim as usize))
            .map(|i| {
                let h = (i as u32).wrapping_mul(2654435761);
                (h as f32) / (u32::MAX as f32) * 2.0 - 1.0
            })
            .collect();
        // query = data[0] 的微小扰动（保证 doc 0 是明确最近邻，三 metric 一致）。
        // 扰动幅度 0.01，远小于 SQ8 量化步长（~2/255≈0.0078）的数倍，但足以使
        // f32 下 doc 0 遥遥领先，量化后排名不翻转。
        let query: Vec<f32> = (0..dim as usize)
            .map(|d| vectors[d] + 0.001 * (d as f32))
            .collect();
        let bundle = encode_sq8(&vectors, dim);
        let topk = 10;
        for metric in [Metric::Cosine, Metric::L2, Metric::Dot] {
            let f32_res = crate::vector::brute_search(&vectors, dim, &query, metric, topk, None, 0);
            let sq8_res = brute_search_sq8(&bundle, dim, &query, metric, topk, None, 0);
            assert_eq!(f32_res.len(), topk);
            assert_eq!(sq8_res.len(), topk);
            let f32_ids: std::collections::HashSet<u64> = f32_res.iter().map(|d| d.docid).collect();
            let sq8_ids: std::collections::HashSet<u64> = sq8_res.iter().map(|d| d.docid).collect();
            let inter = f32_ids.intersection(&sq8_ids).count();
            let union = f32_ids.union(&sq8_ids).count();
            let jaccard = inter as f64 / union as f64;
            assert!(
                jaccard >= 0.95,
                "metric {:?}: jaccard={:.4} < 0.95\n  f32={:?}\n  sq8={:?}",
                metric,
                jaccard,
                f32_res.iter().map(|d| d.docid).collect::<Vec<_>>(),
                sq8_res.iter().map(|d| d.docid).collect::<Vec<_>>(),
            );
        }
    }

    // ---- 测试 5/6: sq8_vectors 懒加载（在 segment tests 中覆盖，这里测 bundle 编码）----

    #[test]
    fn encode_empty_vectors_returns_empty() {
        let bundle = encode_sq8(&[], 4);
        assert!(bundle.data.is_empty());
    }

    #[test]
    fn encode_dim_zero_returns_empty() {
        let bundle = encode_sq8(&[1.0, 2.0], 0);
        assert!(bundle.data.is_empty());
    }

    #[test]
    fn encode_dim_exceeds_max_returns_empty() {
        let dim = 4097_u32;
        let bundle = encode_sq8(&[0.0; 4097], dim);
        assert!(bundle.data.is_empty());
    }

    #[test]
    fn encode_constant_dim_quantizes_to_zero() {
        // 该维恒定（min==max）→ 量化为 0
        let vectors: Vec<f32> = vec![5.0, 5.0, 5.0, 5.0];
        let bundle = encode_sq8(&vectors, 2);
        assert_eq!(bundle.min, vec![5.0, 5.0]);
        assert_eq!(bundle.max, vec![5.0, 5.0]);
        assert!(bundle.data.iter().all(|&b| b == 0));
        // decode 回 5.0
        let decoded = decode_sq8(&bundle);
        assert!(decoded.iter().all(|&v| approx_eq(v, 5.0, 1e-6)));
    }

    // ---- 测试: brute_search_sq8 边界 ----

    #[test]
    fn brute_search_sq8_empty_returns_empty() {
        let bundle = encode_sq8(&[], 4);
        let query = [1.0, 0.0, 0.0, 0.0];
        let res = brute_search_sq8(&bundle, 4, &query, Metric::Cosine, 10, None, 0);
        assert!(res.is_empty());
    }

    #[test]
    fn brute_search_sq8_topk_zero_returns_empty() {
        let vectors: Vec<f32> = vec![1.0, 0.0, 0.0, 1.0];
        let bundle = encode_sq8(&vectors, 2);
        let query = [1.0, 0.0];
        let res = brute_search_sq8(&bundle, 2, &query, Metric::Cosine, 0, None, 0);
        assert!(res.is_empty());
    }

    #[test]
    fn brute_search_sq8_filter_bitmap() {
        let vectors: Vec<f32> = vec![1.0, 0.0, 0.0, 1.0, 1.0, 1.0, -1.0, -1.0];
        let bundle = encode_sq8(&vectors, 2);
        let query = [1.0, 0.0];
        let mut bm = RoaringBitmap::new();
        bm.insert(1001); // docid_base=1000 → local 1
        let res = brute_search_sq8(&bundle, 2, &query, Metric::Cosine, 2, Some(&bm), 1000);
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].docid, 1001);
    }

    #[test]
    fn brute_search_sq8_docid_base_offset() {
        let vectors: Vec<f32> = vec![1.0, 0.0, 1.0, 0.0];
        let bundle = encode_sq8(&vectors, 2);
        let query = [1.0, 0.0];
        let res = brute_search_sq8(&bundle, 2, &query, Metric::Cosine, 2, None, 42);
        assert_eq!(res[0].docid, 42);
        assert_eq!(res[1].docid, 43);
    }

    #[test]
    fn brute_search_sq8_results_sorted_desc() {
        let vectors: Vec<f32> = vec![0.1, 0.1, 1.0, 0.0, 0.5, 0.0, -1.0, 0.0];
        let bundle = encode_sq8(&vectors, 2);
        let query = [1.0, 0.0];
        let res = brute_search_sq8(&bundle, 2, &query, Metric::Cosine, 4, None, 0);
        assert_eq!(res.len(), 4);
        for w in res.windows(2) {
            assert!(
                w[0].score >= w[1].score,
                "not desc: {:?} vs {:?}",
                w[0],
                w[1]
            );
        }
    }

    #[test]
    fn brute_search_sq8_all_three_metrics() {
        let vectors: Vec<f32> = vec![1.0, 0.0, 0.0, 1.0, 1.0, 1.0, -1.0, -1.0];
        let bundle = encode_sq8(&vectors, 2);
        let query = [1.0, 1.0];
        for metric in [Metric::Cosine, Metric::L2, Metric::Dot] {
            let res = brute_search_sq8(&bundle, 2, &query, metric, 2, None, 0);
            assert_eq!(res.len(), 2, "metric {:?} wrong len", metric);
            assert!(res[0].score >= res[1].score, "metric {:?} not desc", metric);
        }
    }

    // ---- 测试: 内存估算 ----

    #[test]
    fn memory_estimate_100k_384_under_200mb() {
        let dim = 384_usize;
        let doc_count = 100_000_usize;
        // SQ8 量化数据 = doc_count × dim × 1 byte
        let sq8_bytes = doc_count * dim;
        // min/max = 2 × dim × 4 bytes（可忽略）
        let minmax_bytes = 2 * dim * 4;
        let total_mb = (sq8_bytes + minmax_bytes) as f64 / (1024.0 * 1024.0);
        eprintln!(
            "SQ8 100k×384: data={:.1}MB minmax={:.0}B total={:.2}MB (vs f32 {:.1}MB)",
            sq8_bytes as f64 / (1024.0 * 1024.0),
            minmax_bytes,
            total_mb,
            (doc_count * dim * 4) as f64 / (1024.0 * 1024.0),
        );
        assert!(total_mb < 200.0, "SQ8 total {total_mb}MB >= 200MB");
        // 验证 4 倍降
        assert!(total_mb < 50.0, "SQ8 should be ~38MB, got {total_mb}MB");
    }
}
