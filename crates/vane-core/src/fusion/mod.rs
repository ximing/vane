//! SPEC §8.2 融合算法：RRF(k=60) + linear(minmax)。
//!
//! 纯函数模块，无状态、无 IO、无 cfg。仅依赖 `std` 与 `vane_core::types::ScoredDoc`。

use crate::types::ScoredDoc;
use std::collections::HashMap;

/// 单路候选（rank 从 0 开始，按 score 降序）。
pub struct FusionCandidate {
    pub docid: u64,
    pub rank: u32,
    pub score: f32,
}

/// linear 归一化输入。
pub struct LinearInput {
    pub docid: u64,
    pub score: f32,
}

/// RRF 融合（SPEC §8.2）。
///
/// `score(d) = Σ_path 1/(k + rank_path(d))`
///
/// - `paths`：每路候选，`rank` 由调用方按 `score` 降序从 0 起编号。
/// - `k`：RRF 平滑常数，SPEC 冻结为 60；调用方应传 [`crate::types::RRF_K`]。
/// - 返回值按 `score` 降序，同分按 `docid` 升序。
/// - 不在任何路出现的 `docid` 不会出现在结果中。
pub fn rrf_fuse(paths: &[Vec<FusionCandidate>], k: u32) -> Vec<ScoredDoc> {
    let mut acc: HashMap<u64, f32> = HashMap::new();
    for path in paths {
        for c in path {
            // k 为 u32，rank 为 u32，相加不会溢出（u32::MAX 远超任何合理候选规模）
            let contrib = 1.0f32 / (k as f32 + c.rank as f32);
            *acc.entry(c.docid).or_insert(0.0) += contrib;
        }
    }
    let mut out: Vec<ScoredDoc> = acc
        .into_iter()
        .map(|(docid, score)| ScoredDoc { docid, score })
        .collect();
    // 降序 score，升序 docid（稳定 tie-break）
    out.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.docid.cmp(&b.docid))
    });
    out
}

/// minmax 归一化（SPEC §8.2；按当次候选集）。
///
/// - `norm(s) = (s - min) / (max - min)`，min/max 取自输入候选集。
/// - 候选集为空 -> 返回空 vec。
/// - `max == min`（单元素或全相同分数）-> 所有归一化值记 `0.0`。
/// - 输出顺序与输入一致；不排序。
pub fn minmax_normalize(scored: &[ScoredDoc]) -> Vec<LinearInput> {
    if scored.is_empty() {
        return Vec::new();
    }
    let mut min = scored[0].score;
    let mut max = scored[0].score;
    for d in &scored[1..] {
        if d.score < min {
            min = d.score;
        }
        if d.score > max {
            max = d.score;
        }
    }
    let range = max - min;
    scored
        .iter()
        .map(|d| LinearInput {
            docid: d.docid,
            score: if range == 0.0 || range.is_nan() {
                0.0
            } else {
                (d.score - min) / range
            },
        })
        .collect()
}

/// linear 融合（SPEC §8.2）。
///
/// `fused(d) = alpha × vec_score(d) + (1 - alpha) × text_score(d)`
///
/// - 两路取 `docid` 并集；缺路记 `score = 0.0`。
/// - `alpha` 由调用方传入，本函数不提供默认值（SPEC §8.2：API 默认路径不出现 alpha）。
/// - 结果按 `score` 降序，同分按 `docid` 升序。
/// - `alpha` 范围校验由上层负责（本函数不校验，行为可预测的线性外推）。
pub fn linear_fuse(
    vec_scores: &[LinearInput],
    text_scores: &[LinearInput],
    alpha: f32,
) -> Vec<ScoredDoc> {
    let mut acc: HashMap<u64, (f32, f32)> = HashMap::new();
    for v in vec_scores {
        acc.entry(v.docid).or_insert((0.0, 0.0)).0 = v.score;
    }
    for t in text_scores {
        acc.entry(t.docid).or_insert((0.0, 0.0)).1 = t.score;
    }
    let mut out: Vec<ScoredDoc> = acc
        .into_iter()
        .map(|(docid, (v, t))| ScoredDoc {
            docid,
            score: alpha * v + (1.0 - alpha) * t,
        })
        .collect();
    out.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.docid.cmp(&b.docid))
    });
    out
}

#[cfg(test)]
mod tests;
