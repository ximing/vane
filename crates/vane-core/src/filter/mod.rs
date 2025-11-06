//! 03-pre-filter：metadata 过滤编译为 roaring 位图（SPEC §8.3）。
//!
//! - `compile_filter`：遍历 `Filter.fields`，每字段按条件（eq/in/gte/lte）扫
//!   `ScalarReader` 列式块，命中 docid 入位图；多字段 AND（交集）。
//!   末尾对每段 `and_not` 排除 tombstone（02 产物消费）。
//! - `should_fallback_brute`：位图基数 < 2*topK → 向量路切暴力精确扫描（SPEC §8.1）。
//!
//! 不支持 OR/NOT（SPEC §8.3 M0-M2 限制）。core 禁 std::fs，零 cfg（I-5）。

#[cfg(test)]
mod tests;

use crate::api::{Filter, FilterCond, ScalarValue};
use crate::segment::ScalarReader;
use crate::types::{FieldDef, Result, Schema, VaneError};
use crate::vfs::Vfs;
use roaring::RoaringBitmap;
use std::sync::Arc;

/// 编译 Filter 为 roaring 位图（SPEC §8.3）。
///
/// `segments` / `scalars` / `tombstones` 三者按段对齐（同一下标的元素属同一段）。
/// 返回位图存绝对 docid（u32 空间）。多字段 AND；末尾对每段 `and_not` 排除 tombstone。
///
/// **schema 校验（M2 parked minor 2.1.3）**：入口校验每个 filter 字段在 `schema`
/// 中存在且为 `FieldDef::Scalar`；字段不存在或为 Text/Vector → `Err(InvalidArg)`
/// （SPEC §10 E_INVALID_ARG：filter 作用于非标量字段）。此前字段不存在时静默产
/// 空位图（不报错），现改为显式报错——调用方须确保 filter 字段在 schema 中。
///
/// 无 filter 字段时返回全量 alive 位图（所有段全部 docid 减 tombstone），
/// 供无 filter 的 search 路径统一走 filter 通道排除 tombstone（Task 5）。
pub fn compile_filter(
    filter: &Filter,
    schema: &Schema,
    segments: &[Arc<crate::segment::SegmentReader>],
    scalars: &[Arc<ScalarReader>],
    tombstones: &[Arc<RoaringBitmap>],
) -> Result<RoaringBitmap> {
    // 段对齐校验。
    if segments.len() != scalars.len() || segments.len() != tombstones.len() {
        return Err(VaneError::InvalidArg(format!(
            "compile_filter: segments/scalars/tombstones length mismatch: {}/{}/{}",
            segments.len(),
            scalars.len(),
            tombstones.len()
        )));
    }

    // 2.1.3：schema 校验——每个 filter 字段必须存在于 schema 且为 Scalar。
    // 字段不存在或为 Text/Vector → Err(InvalidArg)（SPEC §10 E_INVALID_ARG）。
    // 此前字段不存在时静默 continue（位图空），现改为显式报错。
    for (field, _) in &filter.fields {
        match schema.fields.iter().find(|(name, _)| name == field) {
            None => {
                return Err(VaneError::InvalidArg(format!(
                    "compile_filter: field '{}' not in schema",
                    field
                )));
            }
            Some((_, def)) if !matches!(def, FieldDef::Scalar { .. }) => {
                return Err(VaneError::InvalidArg(format!(
                    "compile_filter: field '{}' is not a scalar field (got {:?})",
                    field, def
                )));
            }
            _ => {}
        }
    }

    let mut acc: Option<RoaringBitmap> = None;
    for (field, cond) in &filter.fields {
        let mut field_bm: RoaringBitmap = RoaringBitmap::new();
        for (i, reader) in segments.iter().enumerate() {
            let sr = &scalars[i];
            let base = reader.meta().docid_base;
            let count = reader.doc_count();
            // 字段在该段无列 → 该段无 doc 命中此条件（AND 后整体为空）。
            if !sr.has_field(field) {
                continue;
            }
            for local in 0..count {
                let v = match sr.get(field, local) {
                    Some(v) => v,
                    None => continue, // 该 docid 未设值 → 不命中
                };
                if matches_cond(&v, cond) {
                    let abs = base + local as u64;
                    if abs <= u32::MAX as u64 {
                        field_bm.insert(abs as u32);
                    }
                }
            }
        }
        acc = Some(match acc {
            None => field_bm,
            Some(prev) => prev & field_bm,
        });
    }

    let mut bm = acc.unwrap_or_default();

    // 末尾排除各段 tombstone（绝对 docid 空间，与 bm 同空间）。
    for tb in tombstones {
        bm -= tb.as_ref();
    }
    Ok(bm)
}

/// 构造全量 alive 位图（无 filter 时用）：所有段全部 docid 减 tombstone。
/// 供 search 无 filter 路径统一排除 tombstone（Task 5）。
pub fn alive_bitmap(
    segments: &[Arc<crate::segment::SegmentReader>],
    tombstones: &[Arc<RoaringBitmap>],
) -> Result<RoaringBitmap> {
    if segments.len() != tombstones.len() {
        return Err(VaneError::InvalidArg(format!(
            "alive_bitmap: segments/tombstones length mismatch: {}/{}",
            segments.len(),
            tombstones.len()
        )));
    }
    let mut bm = RoaringBitmap::new();
    for (i, reader) in segments.iter().enumerate() {
        let base = reader.meta().docid_base;
        let count = reader.doc_count() as u64;
        let start = base as u32;
        let end = base + count;
        if end > u64::from(u32::MAX) {
            // 超 u32 部分 roaring 存不下，截断到 u32::MAX（防御性，与 search 一致）。
            if (start as u64) <= u64::from(u32::MAX) {
                bm.insert_range(start..=u32::MAX);
            }
        } else if count > 0 {
            bm.insert_range(start..(end as u32));
        }
        bm -= tombstones[i].as_ref();
    }
    Ok(bm)
}

/// 判断标量值是否满足条件（SPEC §8.3 eq/in/gte/lte）。
fn matches_cond(v: &ScalarValue, cond: &FilterCond) -> bool {
    match cond {
        FilterCond::Eq(target) => scalar_eq(v, target),
        FilterCond::In(targets) => targets.iter().any(|t| scalar_eq(v, t)),
        FilterCond::Gte(threshold) => scalar_cmp(v, threshold).is_some_and(|o| o.is_ge()),
        FilterCond::Lte(threshold) => scalar_cmp(v, threshold).is_some_and(|o| o.is_le()),
    }
}

/// 标量相等比较（同类型才相等）。
fn scalar_eq(a: &ScalarValue, b: &ScalarValue) -> bool {
    match (a, b) {
        (ScalarValue::Int(x), ScalarValue::Int(y)) => x == y,
        (ScalarValue::Float(x), ScalarValue::Float(y)) => x == y,
        (ScalarValue::Bool(x), ScalarValue::Bool(y)) => x == y,
        (ScalarValue::Keyword(x), ScalarValue::Keyword(y)) => x == y,
        _ => false,
    }
}

/// 标量有序比较（仅同类型；跨类型返回 None，视为不命中）。
/// Float 比较用 total_cmp 保证 NaN 确定性。
fn scalar_cmp(a: &ScalarValue, b: &ScalarValue) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (ScalarValue::Int(x), ScalarValue::Int(y)) => Some(x.cmp(y)),
        (ScalarValue::Float(x), ScalarValue::Float(y)) => Some(x.total_cmp(y)),
        (ScalarValue::Bool(x), ScalarValue::Bool(y)) => Some(x.cmp(y)),
        (ScalarValue::Keyword(x), ScalarValue::Keyword(y)) => Some(x.cmp(y)),
        _ => None,
    }
}

/// 低选择率判定（SPEC §8.1）：位图基数 < 2*topK → 向量路切暴力精确扫描。
pub fn should_fallback_brute(bitmap: &RoaringBitmap, topk: usize) -> bool {
    bitmap.len() < 2 * topk as u64
}

/// 从段目录加载 ScalarReader 的便捷封装（api 层 restore/flush 用）。
pub fn load_scalar_reader(vfs: &Arc<dyn Vfs>, segment_dir: &str) -> Result<Arc<ScalarReader>> {
    Ok(Arc::new(ScalarReader::open(vfs, segment_dir)?))
}
