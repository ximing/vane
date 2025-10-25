// 03-pre-filter 单元测试（Task 1/2/3）。

use super::*;
use crate::api::{Filter, FilterCond, ScalarValue};
use crate::segment::{ScalarReader, SegmentWriter};
use crate::types::{FieldDef, Metric, ScalarKind, Schema, TokenizerId};
use crate::vfs::memory::MemoryVfs;
use crate::vfs::Vfs;
use std::sync::Arc;

fn test_tid() -> TokenizerId {
    TokenizerId([0u8; 32])
}

fn schema() -> Schema {
    Schema::new(vec![
        (
            "lang".into(),
            FieldDef::Scalar {
                kind: ScalarKind::Keyword,
            },
        ),
        (
            "year".into(),
            FieldDef::Scalar {
                kind: ScalarKind::Int,
            },
        ),
        (
            "score".into(),
            FieldDef::Scalar {
                kind: ScalarKind::Float,
            },
        ),
        (
            "active".into(),
            FieldDef::Scalar {
                kind: ScalarKind::Bool,
            },
        ),
        (
            "v".into(),
            FieldDef::Vector {
                dim: 2,
                metric: Metric::Cosine,
            },
        ),
    ])
    .unwrap()
}

/// 构建一段含 2 文档的标量语料：d0 lang=zh year=2024 score=9.5 active=true；
/// d1 lang=en year=2023 score=8.0 active=false。
#[allow(clippy::type_complexity)]
fn setup_filter_corpus() -> (
    Schema,
    Vec<Arc<crate::segment::SegmentReader>>,
    Vec<Arc<ScalarReader>>,
    Vec<Arc<RoaringBitmap>>,
) {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    let s = schema();
    let mut w = SegmentWriter::new(vfs.clone(), "db/segments", &s, &test_tid(), 0).unwrap();
    w.add_doc("d1", Some(&[1.0, 0.0]), "{}").unwrap();
    w.set_scalar("lang", ScalarValue::Keyword("zh".into()))
        .unwrap();
    w.set_scalar("year", ScalarValue::Int(2024)).unwrap();
    w.set_scalar("score", ScalarValue::Float(9.5)).unwrap();
    w.set_scalar("active", ScalarValue::Bool(true)).unwrap();
    w.add_doc("d2", Some(&[0.0, 1.0]), "{}").unwrap();
    w.set_scalar("lang", ScalarValue::Keyword("en".into()))
        .unwrap();
    w.set_scalar("year", ScalarValue::Int(2023)).unwrap();
    w.set_scalar("score", ScalarValue::Float(8.0)).unwrap();
    w.set_scalar("active", ScalarValue::Bool(false)).unwrap();
    let meta = w.finalize().unwrap();
    let seg_dir = format!("db/segments/seg_{}", meta.ulid);
    let reader = Arc::new(crate::segment::SegmentReader::open(&vfs, &seg_dir).unwrap());
    let sr = Arc::new(ScalarReader::open(&vfs, &seg_dir).unwrap());
    let tomb = Arc::new(RoaringBitmap::new());
    (s, vec![reader], vec![sr], vec![tomb])
}

// Task 1: scalars.col 写读 roundtrip
#[test]
fn scalars_col_roundtrip_int_keyword() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    let schema = Schema::new(vec![
        (
            "lang".into(),
            FieldDef::Scalar {
                kind: ScalarKind::Keyword,
            },
        ),
        (
            "year".into(),
            FieldDef::Scalar {
                kind: ScalarKind::Int,
            },
        ),
        (
            "v".into(),
            FieldDef::Vector {
                dim: 2,
                metric: Metric::Cosine,
            },
        ),
    ])
    .unwrap();
    let mut w = SegmentWriter::new(vfs.clone(), "db/segments", &schema, &test_tid(), 0).unwrap();
    w.add_doc("d1", Some(&[1.0, 0.0]), "{}").unwrap();
    w.set_scalar("lang", ScalarValue::Keyword("zh".into()))
        .unwrap();
    w.set_scalar("year", ScalarValue::Int(2024)).unwrap();
    w.add_doc("d2", Some(&[0.0, 1.0]), "{}").unwrap();
    w.set_scalar("lang", ScalarValue::Keyword("en".into()))
        .unwrap();
    w.set_scalar("year", ScalarValue::Int(2023)).unwrap();
    let meta = w.finalize().unwrap();
    let sr = ScalarReader::open(&vfs, &format!("db/segments/seg_{}", meta.ulid)).unwrap();
    assert_eq!(sr.get("lang", 0), Some(ScalarValue::Keyword("zh".into())));
    assert_eq!(sr.get("year", 1), Some(ScalarValue::Int(2023)));
    // 未设值的字段返回 None（active 不在 schema，has_field=false）。
    assert_eq!(sr.get("active", 0), None);
    // docid 越界。
    assert_eq!(sr.get("year", 99), None);
}

// Task 1: set_scalar 在 add_doc 前调用报错
#[test]
fn set_scalar_before_add_doc_errors() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    let schema = Schema::new(vec![
        (
            "year".into(),
            FieldDef::Scalar {
                kind: ScalarKind::Int,
            },
        ),
        (
            "v".into(),
            FieldDef::Vector {
                dim: 2,
                metric: Metric::Cosine,
            },
        ),
    ])
    .unwrap();
    let mut w = SegmentWriter::new(vfs, "db/segments", &schema, &test_tid(), 0).unwrap();
    let err = w.set_scalar("year", ScalarValue::Int(2024)).unwrap_err();
    assert!(matches!(err, crate::types::VaneError::Schema(_)));
}

// Task 1: set_scalar 字段不存在 / kind 不匹配报错
#[test]
fn set_scalar_wrong_field_or_kind_errors() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    let schema = Schema::new(vec![
        (
            "year".into(),
            FieldDef::Scalar {
                kind: ScalarKind::Int,
            },
        ),
        (
            "v".into(),
            FieldDef::Vector {
                dim: 2,
                metric: Metric::Cosine,
            },
        ),
    ])
    .unwrap();
    let mut w = SegmentWriter::new(vfs, "db/segments", &schema, &test_tid(), 0).unwrap();
    w.add_doc("d1", Some(&[1.0, 0.0]), "{}").unwrap();
    // 字段不存在。
    assert!(matches!(
        w.set_scalar("nope", ScalarValue::Int(1)).unwrap_err(),
        crate::types::VaneError::Schema(_)
    ));
    // 字段非 Scalar（v 是 Vector）。
    assert!(matches!(
        w.set_scalar("v", ScalarValue::Int(1)).unwrap_err(),
        crate::types::VaneError::Schema(_)
    ));
    // kind 不匹配（year 是 Int，传 Keyword）。
    assert!(matches!(
        w.set_scalar("year", ScalarValue::Keyword("x".into()))
            .unwrap_err(),
        crate::types::VaneError::Schema(_)
    ));
}

// Task 1: 未调 set_scalar 的 docid 该字段为 None
#[test]
fn scalars_col_sparse_missing_doc() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    let schema = Schema::new(vec![
        (
            "year".into(),
            FieldDef::Scalar {
                kind: ScalarKind::Int,
            },
        ),
        (
            "v".into(),
            FieldDef::Vector {
                dim: 2,
                metric: Metric::Cosine,
            },
        ),
    ])
    .unwrap();
    let mut w = SegmentWriter::new(vfs.clone(), "db/segments", &schema, &test_tid(), 0).unwrap();
    w.add_doc("d1", Some(&[1.0, 0.0]), "{}").unwrap();
    // d1 不设 year
    w.add_doc("d2", Some(&[0.0, 1.0]), "{}").unwrap();
    w.set_scalar("year", ScalarValue::Int(2024)).unwrap(); // 仅 d2
    let meta = w.finalize().unwrap();
    let sr = ScalarReader::open(&vfs, &format!("db/segments/seg_{}", meta.ulid)).unwrap();
    assert_eq!(sr.get("year", 0), None); // d1 未设
    assert_eq!(sr.get("year", 1), Some(ScalarValue::Int(2024)));
}

// Task 1: float / bool 列 roundtrip
#[test]
fn scalars_col_roundtrip_float_bool() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    let s = schema();
    let mut w = SegmentWriter::new(vfs.clone(), "db/segments", &s, &test_tid(), 0).unwrap();
    w.add_doc("d1", Some(&[1.0, 0.0]), "{}").unwrap();
    w.set_scalar("score", ScalarValue::Float(9.5)).unwrap();
    w.set_scalar("active", ScalarValue::Bool(true)).unwrap();
    w.add_doc("d2", Some(&[0.0, 1.0]), "{}").unwrap();
    w.set_scalar("score", ScalarValue::Float(-1.0)).unwrap();
    w.set_scalar("active", ScalarValue::Bool(false)).unwrap();
    let meta = w.finalize().unwrap();
    let sr = ScalarReader::open(&vfs, &format!("db/segments/seg_{}", meta.ulid)).unwrap();
    assert_eq!(sr.get("score", 0), Some(ScalarValue::Float(9.5)));
    assert_eq!(sr.get("score", 1), Some(ScalarValue::Float(-1.0)));
    assert_eq!(sr.get("active", 0), Some(ScalarValue::Bool(true)));
    assert_eq!(sr.get("active", 1), Some(ScalarValue::Bool(false)));
}

// Task 1: M0 空段（无 set_scalar）scalars.col 仍可读（空 reader）
#[test]
fn scalars_col_empty_segment_readable() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    let s = schema();
    let w = SegmentWriter::new(vfs.clone(), "db/segments", &s, &test_tid(), 0).unwrap();
    let meta = w.finalize().unwrap();
    let sr = ScalarReader::open(&vfs, &format!("db/segments/seg_{}", meta.ulid)).unwrap();
    assert!(!sr.has_field("lang"));
    assert_eq!(sr.get("lang", 0), None);
}

// Task 2: eq keyword
#[test]
fn compile_filter_eq_keyword() {
    let (s, segments, scalars, tombstones) = setup_filter_corpus();
    let filter = Filter {
        fields: vec![(
            "lang".into(),
            FilterCond::Eq(ScalarValue::Keyword("zh".into())),
        )],
    };
    let bm = compile_filter(&filter, &s, &segments, &scalars, &tombstones).unwrap();
    assert!(bm.contains(0)); // d1 lang=zh
    assert!(!bm.contains(1)); // d2 lang=en
}

// Task 2: gte int AND keyword in
#[test]
fn compile_filter_gte_int_and_keyword_in() {
    let (s, segments, scalars, tombstones) = setup_filter_corpus();
    let filter = Filter {
        fields: vec![
            ("year".into(), FilterCond::Gte(ScalarValue::Int(2024))),
            (
                "lang".into(),
                FilterCond::In(vec![
                    ScalarValue::Keyword("zh".into()),
                    ScalarValue::Keyword("ja".into()),
                ]),
            ),
        ],
    };
    let bm = compile_filter(&filter, &s, &segments, &scalars, &tombstones).unwrap();
    assert!(bm.contains(0)); // d1 year=2024 lang=zh
    assert!(!bm.contains(1)); // d2 year=2023
}

// Task 2: lte float
#[test]
fn compile_filter_lte_float() {
    let (s, segments, scalars, tombstones) = setup_filter_corpus();
    let filter = Filter {
        fields: vec![("score".into(), FilterCond::Lte(ScalarValue::Float(8.5)))],
    };
    let bm = compile_filter(&filter, &s, &segments, &scalars, &tombstones).unwrap();
    assert!(!bm.contains(0)); // d1 score=9.5 > 8.5
    assert!(bm.contains(1)); // d2 score=8.0 <= 8.5
}

// Task 2: eq bool
#[test]
fn compile_filter_eq_bool() {
    let (s, segments, scalars, tombstones) = setup_filter_corpus();
    let filter = Filter {
        fields: vec![("active".into(), FilterCond::Eq(ScalarValue::Bool(true)))],
    };
    let bm = compile_filter(&filter, &s, &segments, &scalars, &tombstones).unwrap();
    assert!(bm.contains(0)); // d1 active=true
    assert!(!bm.contains(1));
}

// Task 2: 多字段 AND 交集为空
#[test]
fn compile_filter_and_yields_empty() {
    let (s, segments, scalars, tombstones) = setup_filter_corpus();
    let filter = Filter {
        fields: vec![
            ("year".into(), FilterCond::Eq(ScalarValue::Int(2024))),
            (
                "lang".into(),
                FilterCond::Eq(ScalarValue::Keyword("en".into())),
            ),
        ],
    };
    let bm = compile_filter(&filter, &s, &segments, &scalars, &tombstones).unwrap();
    assert!(bm.is_empty());
}

// Task 2: 字段在该段无列 → 无命中
#[test]
fn compile_filter_field_missing_in_segment() {
    let (s, segments, scalars, tombstones) = setup_filter_corpus();
    let filter = Filter {
        fields: vec![("nonexistent".into(), FilterCond::Eq(ScalarValue::Int(1)))],
    };
    let bm = compile_filter(&filter, &s, &segments, &scalars, &tombstones).unwrap();
    assert!(bm.is_empty());
}

// Task 2: tombstone 排除
#[test]
fn compile_filter_excludes_tombstone() {
    let (s, segments, scalars, _tombstones) = setup_filter_corpus();
    // d1 (docid 0) 被 tombstone。
    let mut tb = RoaringBitmap::new();
    tb.insert(0);
    let tombstones = vec![Arc::new(tb)];
    let filter = Filter {
        fields: vec![(
            "lang".into(),
            FilterCond::In(vec![
                ScalarValue::Keyword("zh".into()),
                ScalarValue::Keyword("en".into()),
            ]),
        )],
    };
    let bm = compile_filter(&filter, &s, &segments, &scalars, &tombstones).unwrap();
    assert!(!bm.contains(0)); // d1 被 tombstone 排除
    assert!(bm.contains(1)); // d2 仍在
}

// Task 3: 低选择率回退判定
#[test]
fn should_fallback_brute_when_bitmap_small() {
    let mut bm = RoaringBitmap::new();
    bm.insert(1);
    bm.insert(2); // cardinality=2
    assert!(should_fallback_brute(&bm, 10)); // 2 < 2*10
    let mut big = RoaringBitmap::new();
    for i in 0..100 {
        big.insert(i);
    }
    assert!(!should_fallback_brute(&big, 10)); // 100 >= 20
}

// Task 3: 边界——基数恰等于 2*topK 不回退
#[test]
fn should_fallback_brute_boundary() {
    let mut bm = RoaringBitmap::new();
    for i in 0..20 {
        bm.insert(i);
    }
    assert!(!should_fallback_brute(&bm, 10)); // 20 == 2*10, 不 < 故不回退
    bm.remove(19);
    assert!(should_fallback_brute(&bm, 10)); // 19 < 20
}

// alive_bitmap：无 filter 时全量减 tombstone
#[test]
fn alive_bitmap_excludes_tombstone() {
    let (s, segments, scalars, _tombstones) = setup_filter_corpus();
    let _ = s;
    let _ = scalars;
    let mut tb = RoaringBitmap::new();
    tb.insert(0);
    let tombstones = vec![Arc::new(tb)];
    let bm = alive_bitmap(&segments, &tombstones).unwrap();
    assert!(!bm.contains(0));
    assert!(bm.contains(1));
}
