use std::collections::HashMap;

use crate::search::SearchHit;

/// Reciprocal rank fusion. Rank is 1-based. Ties break by `id` ascending.
pub fn rrf_merge(lists: Vec<Vec<SearchHit>>, k: u32, top_k: usize) -> Vec<SearchHit> {
    if top_k == 0 {
        return Vec::new();
    }
    let k = k as f32;
    let mut acc: HashMap<String, SearchHit> = HashMap::new();
    for list in lists {
        for (rank0, mut hit) in list.into_iter().enumerate() {
            let add = 1.0 / (k + (rank0 + 1) as f32);
            let id = hit.id.clone();
            let degraded = hit.degraded;
            if let Some(existing) = acc.get_mut(&id) {
                existing.score += add;
                existing.degraded |= degraded;
            } else {
                hit.score = add;
                acc.insert(id, hit);
            }
        }
    }
    let mut out: Vec<SearchHit> = acc.into_values().collect();
    out.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });
    out.truncate(top_k);
    out
}
