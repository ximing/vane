// tests/pre_filter.rs — 03-pre-filter 集成测试（Task 4/5 + Q-7）。
//
// 验证 SPEC §8.3（filter 编译进 HNSW/WAND/brute）/§8.1（低选择率暴力回退）/
// tombstone 并入 filter 端到端。

use std::collections::HashMap;
use std::sync::Arc;

use vane_core::api::{
    Db, Doc, Filter, FilterCond, FusionSpec, OpenOptions, ScalarValue, SearchMode, SearchQuery,
};
use vane_core::types::{FieldDef, Metric, ScalarKind, Schema};
use vane_core::vfs::memory::MemoryVfs;

fn schema_with_scalars() -> Schema {
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
            "v".into(),
            FieldDef::Vector {
                dim: 2,
                metric: Metric::Cosine,
            },
        ),
    ])
    .unwrap()
}

fn doc(id: &str, vector: &[f32], lang: &str, year: i64) -> Doc {
    let mut meta = HashMap::new();
    meta.insert("lang".into(), ScalarValue::Keyword(lang.into()));
    meta.insert("year".into(), ScalarValue::Int(year));
    Doc {
        id: id.into(),
        text: None,
        vector: Some(vector.to_vec()),
        meta: Some(meta),
    }
}

fn vec_query(filter: Option<Filter>) -> SearchQuery {
    SearchQuery {
        text: None,
        vector: Some(vec![1.0, 0.0]),
        top_k: 10,
        mode: SearchMode::Vector,
        fusion: FusionSpec::Rrf,
        filter,
        candidate_multiplier: 3,
    }
}

// Task 4: filter 只返回匹配文档
#[test]
fn search_with_filter_returns_only_matching() {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col = db
        .collection("c", schema_with_scalars(), Default::default())
        .unwrap();
    col.add(&[
        doc("d1", &[1.0, 0.0], "zh", 2024),
        doc("d2", &[1.0, 0.0], "en", 2023),
    ])
    .unwrap();
    col.flush().unwrap();
    let hits = col
        .search(&vec_query(Some(Filter {
            fields: vec![(
                "lang".into(),
                FilterCond::Eq(ScalarValue::Keyword("zh".into())),
            )],
        })))
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "d1");
}

// Task 4: gte int filter
#[test]
fn search_with_filter_gte_year() {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col = db
        .collection("c", schema_with_scalars(), Default::default())
        .unwrap();
    col.add(&[
        doc("d1", &[1.0, 0.0], "zh", 2024),
        doc("d2", &[1.0, 0.0], "en", 2023),
        doc("d3", &[1.0, 0.0], "ja", 2025),
    ])
    .unwrap();
    col.flush().unwrap();
    let hits = col
        .search(&vec_query(Some(Filter {
            fields: vec![("year".into(), FilterCond::Gte(ScalarValue::Int(2024)))],
        })))
        .unwrap();
    let ids: Vec<&str> = hits.iter().map(|h| h.id.as_str()).collect();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&"d1"));
    assert!(ids.contains(&"d3"));
    assert!(!ids.contains(&"d2"));
}

// Task 4: 低选择率回退（filter 只匹配少数文档 → 暴力精确扫描，100% 召回）
#[test]
fn search_filter_low_selectivity_uses_brute() {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col = db
        .collection("c", schema_with_scalars(), Default::default())
        .unwrap();
    // 10 文档，只有 1 个 year >= 2029。
    let mut docs = Vec::new();
    for i in 0..10 {
        let year = 2020 + i;
        let v = vec![1.0, 0.0];
        docs.push(doc(&format!("d{}", i), &v, "zh", year));
    }
    col.add(&docs).unwrap();
    col.flush().unwrap();
    // filter 匹配 1 个文档（year >= 2029 → d9）。
    let hits = col
        .search(&vec_query(Some(Filter {
            fields: vec![("year".into(), FilterCond::Gte(ScalarValue::Int(2029)))],
        })))
        .unwrap();
    // 位图基数 1 < 2*10=20 → 暴力回退；仍应只返回匹配的 1 个文档。
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "d9");
}

// Task 4: 多字段 AND
#[test]
fn search_filter_multi_field_and() {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col = db
        .collection("c", schema_with_scalars(), Default::default())
        .unwrap();
    col.add(&[
        doc("d1", &[1.0, 0.0], "zh", 2024),
        doc("d2", &[1.0, 0.0], "zh", 2023),
        doc("d3", &[1.0, 0.0], "en", 2024),
    ])
    .unwrap();
    col.flush().unwrap();
    let hits = col
        .search(&vec_query(Some(Filter {
            fields: vec![
                (
                    "lang".into(),
                    FilterCond::Eq(ScalarValue::Keyword("zh".into())),
                ),
                ("year".into(), FilterCond::Gte(ScalarValue::Int(2024))),
            ],
        })))
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "d1");
}

// Task 4: text 模式 filter 也生效
#[test]
fn search_text_mode_filter_works() {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let schema = Schema::new(vec![
        ("body".into(), FieldDef::Text),
        (
            "lang".into(),
            FieldDef::Scalar {
                kind: ScalarKind::Keyword,
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
    let col = db.collection("c", schema, Default::default()).unwrap();
    let mk = |id: &str, text: &str, lang: &str| Doc {
        id: id.into(),
        text: Some(text.into()),
        vector: Some(vec![1.0, 0.0]),
        meta: Some(HashMap::from([(
            "lang".into(),
            ScalarValue::Keyword(lang.into()),
        )])),
    };
    col.add(&[mk("d1", "hello world", "zh"), mk("d2", "hello world", "en")])
        .unwrap();
    col.flush().unwrap();
    let q = SearchQuery {
        text: Some("hello".into()),
        vector: None,
        top_k: 10,
        mode: SearchMode::Text,
        fusion: FusionSpec::Rrf,
        filter: Some(Filter {
            fields: vec![(
                "lang".into(),
                FilterCond::Eq(ScalarValue::Keyword("zh".into())),
            )],
        }),
        candidate_multiplier: 3,
    };
    let hits = col.search(&q).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "d1");
}

// Task 5: tombstone 并入 filter——已删文档不出现
#[test]
fn filter_excludes_tombstoned_docs() {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col = db
        .collection("c", schema_with_scalars(), Default::default())
        .unwrap();
    col.add(&[
        doc("d1", &[1.0, 0.0], "zh", 2024),
        doc("d2", &[1.0, 0.0], "en", 2023),
    ])
    .unwrap();
    col.flush().unwrap();
    col.delete(&["d2".into()]).unwrap();
    // filter 匹配 zh 和 en，但 d2 被 tombstone → 只剩 d1。
    let hits = col
        .search(&vec_query(Some(Filter {
            fields: vec![(
                "lang".into(),
                FilterCond::In(vec![
                    ScalarValue::Keyword("zh".into()),
                    ScalarValue::Keyword("en".into()),
                ]),
            )],
        })))
        .unwrap();
    assert!(!hits.iter().any(|h| h.id == "d2"));
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "d1");
}

// Task 5: 无 filter 时 tombstone 仍排除（alive_bitmap 路径）
#[test]
fn no_filter_still_excludes_tombstone() {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col = db
        .collection("c", schema_with_scalars(), Default::default())
        .unwrap();
    col.add(&[
        doc("d1", &[1.0, 0.0], "zh", 2024),
        doc("d2", &[1.0, 0.0], "en", 2023),
    ])
    .unwrap();
    col.flush().unwrap();
    col.delete(&["d2".into()]).unwrap();
    let hits = col.search(&vec_query(None)).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "d1");
}

// Task 4: flush 多段后 filter 跨段生效
#[test]
fn filter_across_multiple_segments() {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col = db
        .collection("c", schema_with_scalars(), Default::default())
        .unwrap();
    col.add(&[doc("d1", &[1.0, 0.0], "zh", 2024)]).unwrap();
    col.flush().unwrap();
    col.add(&[doc("d2", &[1.0, 0.0], "en", 2023)]).unwrap();
    col.flush().unwrap();
    assert_eq!(col.segment_count(), 2);
    let hits = col
        .search(&vec_query(Some(Filter {
            fields: vec![(
                "lang".into(),
                FilterCond::Eq(ScalarValue::Keyword("zh".into())),
            )],
        })))
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "d1");
}

// Q-7: compact 后标量仍可过滤（MergeTask 标量重写）
#[test]
fn compact_preserves_scalars_for_filter() {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col = db
        .collection("c", schema_with_scalars(), Default::default())
        .unwrap();
    col.add(&[
        doc("d1", &[1.0, 0.0], "zh", 2024),
        doc("d2", &[1.0, 0.0], "en", 2023),
        doc("d3", &[1.0, 0.0], "ja", 2025),
    ])
    .unwrap();
    col.flush().unwrap();
    // compact 合并为单段，标量应重写到新段。
    col.compact().unwrap();
    assert_eq!(col.segment_count(), 1);
    let hits = col
        .search(&vec_query(Some(Filter {
            fields: vec![(
                "lang".into(),
                FilterCond::Eq(ScalarValue::Keyword("ja".into())),
            )],
        })))
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "d3");
    // year filter 也应生效（验证 int 列重写）。
    let hits2 = col
        .search(&vec_query(Some(Filter {
            fields: vec![("year".into(), FilterCond::Gte(ScalarValue::Int(2024)))],
        })))
        .unwrap();
    let ids: Vec<&str> = hits2.iter().map(|h| h.id.as_str()).collect();
    assert!(ids.contains(&"d1"));
    assert!(ids.contains(&"d3"));
    assert!(!ids.contains(&"d2"));
}

// Q-7: compact 物理清除 tombstone 后 filter 不受影响
#[test]
fn compact_then_filter_after_delete() {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col = db
        .collection("c", schema_with_scalars(), Default::default())
        .unwrap();
    col.add(&[
        doc("d1", &[1.0, 0.0], "zh", 2024),
        doc("d2", &[1.0, 0.0], "zh", 2023),
    ])
    .unwrap();
    col.flush().unwrap();
    col.delete(&["d2".into()]).unwrap();
    col.compact().unwrap();
    // compact 后新段无 tombstone；filter lang=zh 应只返回 d1。
    let hits = col
        .search(&vec_query(Some(Filter {
            fields: vec![(
                "lang".into(),
                FilterCond::Eq(ScalarValue::Keyword("zh".into())),
            )],
        })))
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "d1");
}

// reopen 后 ScalarReader 仍可加载（restore_from_manifest 路径）
#[test]
fn reopen_loads_scalar_reader() {
    let vfs = Arc::new(MemoryVfs::new());
    {
        let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
        let col = db
            .collection("c", schema_with_scalars(), Default::default())
            .unwrap();
        col.add(&[doc("d1", &[1.0, 0.0], "zh", 2024)]).unwrap();
        col.flush().unwrap();
    }
    let db2 = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col2 = db2
        .collection("c", schema_with_scalars(), Default::default())
        .unwrap();
    let hits = col2
        .search(&vec_query(Some(Filter {
            fields: vec![(
                "lang".into(),
                FilterCond::Eq(ScalarValue::Keyword("zh".into())),
            )],
        })))
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "d1");
}
