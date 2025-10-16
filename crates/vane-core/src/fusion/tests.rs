// fusion/tests.rs — 03-fusion 单元测试
use super::*;
use crate::types::RRF_K;

fn fc(docid: u64, rank: u32, score: f32) -> FusionCandidate {
    FusionCandidate { docid, rank, score }
}

fn sd(docid: u64, score: f32) -> ScoredDoc {
    ScoredDoc { docid, score }
}

fn li(docid: u64, score: f32) -> LinearInput {
    LinearInput { docid, score }
}

fn scored_eq(a: &[ScoredDoc], b: &[(u64, f32)]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b.iter())
            .all(|(x, y)| x.docid == y.0 && (x.score - y.1).abs() < 1e-6)
}

fn li_eq(a: &[LinearInput], b: &[(u64, f32)]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b.iter())
            .all(|(x, y)| x.docid == y.0 && (x.score - y.1).abs() < 1e-6)
}

// ===== rrf_fuse =====

#[test]
fn rrf_two_paths_basic() {
    let path_a = vec![fc(0, 0, 9.0), fc(1, 1, 8.0)];
    let path_b = vec![fc(1, 0, 7.0), fc(0, 1, 6.0)];
    let out = rrf_fuse(&[path_a, path_b], 60);
    let s0 = 1.0 / 60.0 + 1.0 / 61.0;
    let s1 = s0;
    assert!(scored_eq(&out, &[(0, s0), (1, s1)]));
}

#[test]
fn rrf_single_path() {
    let path = vec![fc(10, 0, 5.0), fc(20, 1, 4.0), fc(30, 2, 3.0)];
    let out = rrf_fuse(&[path], 60);
    let s10 = 1.0 / 60.0;
    let s20 = 1.0 / 61.0;
    let s30 = 1.0 / 62.0;
    assert!(scored_eq(&out, &[(10, s10), (20, s20), (30, s30)]));
}

#[test]
fn rrf_doc_absent_in_one_path() {
    let path_a = vec![fc(0, 0, 9.0)];
    let path_b = vec![fc(1, 0, 7.0)];
    let out = rrf_fuse(&[path_a, path_b], 60);
    let s0 = 1.0 / 60.0;
    let s1 = 1.0 / 60.0;
    assert!(scored_eq(&out, &[(0, s0), (1, s1)]));
}

#[test]
fn rrf_doc_absent_in_all_paths_excluded() {
    let path_a = vec![fc(0, 0, 9.0)];
    let path_b = vec![fc(1, 0, 7.0)];
    let out = rrf_fuse(&[path_a, path_b], 60);
    assert!(out.iter().all(|d| d.docid == 0 || d.docid == 1));
    assert_eq!(out.len(), 2);
}

#[test]
fn rrf_empty_paths() {
    let out = rrf_fuse(&[], 60);
    assert!(out.is_empty());
}

#[test]
fn rrf_empty_vecs_in_paths() {
    let out = rrf_fuse(&[vec![], vec![]], 60);
    assert!(out.is_empty());
}

#[test]
fn rrf_result_sorted_desc() {
    let path_a = vec![fc(0, 0, 9.0), fc(1, 1, 8.0), fc(2, 2, 7.0)];
    let path_b = vec![fc(2, 0, 7.0), fc(1, 1, 6.0), fc(0, 2, 5.0)];
    let out = rrf_fuse(&[path_a, path_b], 60);
    let s0 = 1.0 / 60.0 + 1.0 / 62.0;
    let s1 = 2.0 / 61.0;
    let s2 = s0;
    assert!(scored_eq(&out, &[(0, s0), (2, s2), (1, s1)]));
}

#[test]
fn rrf_k_is_60_frozen() {
    assert_eq!(RRF_K, 60);
}

#[test]
fn rrf_duplicate_docid_within_single_path() {
    let path = vec![fc(0, 0, 5.0), fc(0, 1, 4.0)];
    let out = rrf_fuse(&[path], 60);
    let s = 1.0 / 60.0 + 1.0 / 61.0;
    assert_eq!(out.len(), 1);
    assert!(scored_eq(&out, &[(0, s)]));
}

// ===== minmax_normalize =====

#[test]
fn minmax_basic() {
    let scored = vec![sd(0, 10.0), sd(1, 5.0), sd(2, 0.0)];
    let out = minmax_normalize(&scored);
    assert!(li_eq(&out, &[(0, 1.0), (1, 0.5), (2, 0.0)]));
}

#[test]
fn minmax_preserves_input_order() {
    let scored = vec![sd(7, 1.0), sd(3, 5.0), sd(9, 3.0)];
    let out = minmax_normalize(&scored);
    assert!(li_eq(&out, &[(7, 0.0), (3, 1.0), (9, 0.5)]));
}

#[test]
fn minmax_empty() {
    let out = minmax_normalize(&[]);
    assert!(out.is_empty());
}

#[test]
fn minmax_single_element() {
    let scored = vec![sd(42, 2.5)];
    let out = minmax_normalize(&scored);
    assert!(li_eq(&out, &[(42, 0.0)]));
}

#[test]
fn minmax_all_equal_scores() {
    let scored = vec![sd(0, 2.5), sd(1, 2.5), sd(2, 2.5)];
    let out = minmax_normalize(&scored);
    assert!(li_eq(&out, &[(0, 0.0), (1, 0.0), (2, 0.0)]));
}

#[test]
fn minmax_negative_scores() {
    let scored = vec![sd(0, -1.0), sd(1, -5.0)];
    let out = minmax_normalize(&scored);
    assert!(li_eq(&out, &[(0, 1.0), (1, 0.0)]));
}

#[test]
fn minmax_nan_input_does_not_panic() {
    // P3 命名澄清：minmax 对 NaN 输入不 panic（range==0.0 或 NaN 分支记 0.0），
    // 但调用方契约不含 NaN——此测试仅验证不 panic，不代表 NaN 是合法输入。
    let scored = vec![sd(0, f32::NAN)];
    let _ = minmax_normalize(&scored);
}

// ===== linear_fuse =====

#[test]
fn linear_basic_overlap() {
    let vec_scores = vec![li(0, 1.0), li(1, 0.5)];
    let text_scores = vec![li(0, 0.2), li(1, 0.8)];
    let out = linear_fuse(&vec_scores, &text_scores, 0.5);
    assert!(scored_eq(&out, &[(1, 0.65), (0, 0.6)]));
}

#[test]
fn linear_alpha_one_ignores_text() {
    let vec_scores = vec![li(0, 0.9), li(1, 0.1)];
    let text_scores = vec![li(0, 1.0), li(1, 1.0)];
    let out = linear_fuse(&vec_scores, &text_scores, 1.0);
    assert!(scored_eq(&out, &[(0, 0.9), (1, 0.1)]));
}

#[test]
fn linear_alpha_zero_ignores_vec() {
    let vec_scores = vec![li(0, 1.0), li(1, 1.0)];
    let text_scores = vec![li(0, 0.3), li(1, 0.7)];
    let out = linear_fuse(&vec_scores, &text_scores, 0.0);
    assert!(scored_eq(&out, &[(1, 0.7), (0, 0.3)]));
}

#[test]
fn linear_disjoint_docids_union() {
    let vec_scores = vec![li(0, 1.0)];
    let text_scores = vec![li(1, 1.0)];
    let out = linear_fuse(&vec_scores, &text_scores, 0.5);
    assert!(scored_eq(&out, &[(0, 0.5), (1, 0.5)]));
}

#[test]
fn linear_partial_overlap() {
    let vec_scores = vec![li(0, 1.0), li(1, 0.5), li(2, 0.0)];
    let text_scores = vec![li(1, 1.0), li(2, 0.5), li(3, 0.0)];
    let out = linear_fuse(&vec_scores, &text_scores, 0.5);
    assert!(scored_eq(&out, &[(1, 0.75), (0, 0.5), (2, 0.25), (3, 0.0)]));
}

#[test]
fn linear_both_empty() {
    let out = linear_fuse(&[], &[], 0.5);
    assert!(out.is_empty());
}

#[test]
fn linear_vec_empty() {
    let text_scores = vec![li(0, 0.4), li(1, 0.6)];
    let out = linear_fuse(&[], &text_scores, 0.5);
    assert!(scored_eq(&out, &[(1, 0.3), (0, 0.2)]));
}

#[test]
fn linear_text_empty() {
    let vec_scores = vec![li(0, 0.4), li(1, 0.6)];
    let out = linear_fuse(&vec_scores, &[], 0.5);
    assert!(scored_eq(&out, &[(1, 0.3), (0, 0.2)]));
}

#[test]
fn linear_tie_break_docid_asc() {
    let vec_scores = vec![li(5, 0.2), li(3, 0.2)];
    let text_scores = vec![li(5, 0.2), li(3, 0.2)];
    let out = linear_fuse(&vec_scores, &text_scores, 0.5);
    assert!(scored_eq(&out, &[(3, 0.2), (5, 0.2)]));
}

#[test]
fn linear_does_not_provide_alpha_default() {
    let _: fn(&[LinearInput], &[LinearInput], f32) -> Vec<ScoredDoc> = linear_fuse;
}

#[test]
fn linear_duplicate_docid_in_same_path_last_wins() {
    let vec_scores = vec![li(0, 1.0), li(0, 0.2)];
    let text_scores = vec![li(0, 0.0)];
    let out = linear_fuse(&vec_scores, &text_scores, 1.0);
    assert_eq!(out.len(), 1);
    assert!(scored_eq(&out, &[(0, 0.2)]));
}
