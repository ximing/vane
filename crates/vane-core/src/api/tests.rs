use super::db::Db;
use super::types::*;
use crate::persistence::AutoCommitConfig;
use crate::tokenizer::BuiltinTokenizer;
use crate::types::{FieldDef, Metric, Schema, VaneError};
use crate::vfs::memory::MemoryVfs;

#[test]
fn open_options_default() {
    let o = OpenOptions::default();
    assert_eq!(o.page_cache_mb, 32);
    assert!(matches!(
        o.auto_commit,
        crate::persistence::AutoCommitConfig::On { .. }
    ));
}

#[test]
fn search_query_default() {
    let q = SearchQuery::default();
    assert_eq!(q.top_k, 10);
    assert!(matches!(q.mode, SearchMode::Auto));
    assert!(matches!(q.fusion, FusionSpec::Rrf));
    assert_eq!(q.candidate_multiplier, 3);
    assert!(q.filter.is_none());
}

#[test]
fn collection_options_default_tokenizer_standard() {
    let o = CollectionOptions::default();
    assert!(matches!(o.tokenizer, BuiltinTokenizer::Standard));
}

// ---- Task 2: Db ----

#[test]
fn db_open_new_returns_empty_collections() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn crate::vfs::Vfs>;
    let db = Db::open(vfs, "mydb", OpenOptions::default()).unwrap();
    assert!(db.collections().is_empty());
}

#[test]
fn db_collection_creates_and_returns() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn crate::vfs::Vfs>;
    let db = Db::open(vfs, "mydb", OpenOptions::default()).unwrap();
    let schema = Schema::new(vec![
        ("body".into(), FieldDef::Text),
        (
            "v".into(),
            FieldDef::Vector {
                dim: 4,
                metric: Metric::Cosine,
            },
        ),
    ])
    .unwrap();
    let col = db
        .collection("docs", schema, CollectionOptions::default())
        .unwrap();
    let _ = col;
    assert!(db.collections().contains(&"docs".to_string()));
}

#[test]
fn db_collection_idempotent_same_schema() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn crate::vfs::Vfs>;
    let db = Db::open(vfs, "mydb", OpenOptions::default()).unwrap();
    let schema = Schema::new(vec![(
        "v".into(),
        FieldDef::Vector {
            dim: 4,
            metric: Metric::Cosine,
        },
    )])
    .unwrap();
    let _c1 = db
        .collection("docs", schema.clone(), CollectionOptions::default())
        .unwrap();
    let _c2 = db
        .collection("docs", schema, CollectionOptions::default())
        .unwrap();
    assert_eq!(db.collections().len(), 1);
}

#[test]
fn db_collection_idempotent_different_schema_rejected() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn crate::vfs::Vfs>;
    let db = Db::open(vfs, "mydb", OpenOptions::default()).unwrap();
    let schema1 = Schema::new(vec![(
        "v".into(),
        FieldDef::Vector {
            dim: 4,
            metric: Metric::Cosine,
        },
    )])
    .unwrap();
    let _c1 = db
        .collection("docs", schema1, CollectionOptions::default())
        .unwrap();
    let schema2 = Schema::new(vec![(
        "v".into(),
        FieldDef::Vector {
            dim: 8,
            metric: Metric::Cosine,
        },
    )])
    .unwrap();
    let r = db.collection("docs", schema2, CollectionOptions::default());
    assert!(matches!(r, Err(VaneError::Schema(_))));
}

// ---- Task 3: add ----

#[test]
fn collection_add_buffers_docs() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn crate::vfs::Vfs>;
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let schema = Schema::new(vec![
        ("body".into(), FieldDef::Text),
        (
            "v".into(),
            FieldDef::Vector {
                dim: 2,
                metric: Metric::Cosine,
            },
        ),
    ])
    .unwrap();
    let col = db
        .collection("docs", schema, CollectionOptions::default())
        .unwrap();
    let report = col
        .add(&[
            Doc {
                id: "a".into(),
                text: Some("hello world".into()),
                vector: Some(vec![1.0, 0.0]),
                meta: None,
            },
            Doc {
                id: "b".into(),
                text: Some("foo bar".into()),
                vector: Some(vec![0.0, 1.0]),
                meta: None,
            },
        ])
        .unwrap();
    assert_eq!(report.accepted, 2);
    assert!(report.visible_after_flush);
    // 未 flush 不可搜
    let hits = col
        .search(&SearchQuery {
            text: Some("hello".into()),
            vector: None,
            top_k: 10,
            mode: SearchMode::Auto,
            fusion: FusionSpec::Rrf,
            filter: None,
            candidate_multiplier: 3,
        })
        .unwrap();
    assert!(hits.is_empty(), "unflushed data should not be searchable");
}

#[test]
fn collection_auto_commit_off_does_not_trigger_flush() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn crate::vfs::Vfs>;
    let opts = OpenOptions {
        persistence: PersistenceMode::Persistent,
        auto_commit: AutoCommitConfig::Off,
        page_cache_mb: 32,
    };
    let db = Db::open(vfs, "db", opts).unwrap();
    let schema = Schema::new(vec![(
        "v".into(),
        FieldDef::Vector {
            dim: 2,
            metric: Metric::Cosine,
        },
    )])
    .unwrap();
    let col = db
        .collection(
            "docs",
            schema,
            CollectionOptions {
                tokenizer: BuiltinTokenizer::Standard,
                user_dict: vec![],
                auto_commit: AutoCommitConfig::Off,
            },
        )
        .unwrap();
    col.add(&[Doc {
        id: "a".into(),
        text: None,
        vector: Some(vec![1.0, 0.0]),
        meta: None,
    }])
    .unwrap();
    let hits = col
        .search(&SearchQuery {
            text: None,
            vector: Some(vec![1.0, 0.0]),
            top_k: 10,
            mode: SearchMode::Vector,
            fusion: FusionSpec::Rrf,
            filter: None,
            candidate_multiplier: 3,
        })
        .unwrap();
    assert!(hits.is_empty(), "auto_commit=Off should not trigger flush");
}

// ---- Task 4: flush ----

#[test]
fn collection_flush_makes_data_searchable() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn crate::vfs::Vfs>;
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let schema = Schema::new(vec![
        ("body".into(), FieldDef::Text),
        (
            "v".into(),
            FieldDef::Vector {
                dim: 2,
                metric: Metric::Cosine,
            },
        ),
    ])
    .unwrap();
    let col = db
        .collection("docs", schema, CollectionOptions::default())
        .unwrap();
    col.add(&[
        Doc {
            id: "a".into(),
            text: Some("hello world".into()),
            vector: Some(vec![1.0, 0.0]),
            meta: None,
        },
        Doc {
            id: "b".into(),
            text: Some("foo bar".into()),
            vector: Some(vec![0.0, 1.0]),
            meta: None,
        },
    ])
    .unwrap();
    col.flush().unwrap();
    let hits = col
        .search(&SearchQuery {
            text: Some("hello".into()),
            vector: None,
            top_k: 10,
            mode: SearchMode::Auto,
            fusion: FusionSpec::Rrf,
            filter: None,
            candidate_multiplier: 3,
        })
        .unwrap();
    assert!(!hits.is_empty());
    assert_eq!(hits[0].id, "a");
}

#[test]
fn flush_preserves_doc_meta() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn crate::vfs::Vfs>;
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let schema = Schema::new(vec![
        ("body".into(), FieldDef::Text),
        (
            "v".into(),
            FieldDef::Vector {
                dim: 2,
                metric: Metric::Cosine,
            },
        ),
    ])
    .unwrap();
    let col = db
        .collection("docs", schema, CollectionOptions::default())
        .unwrap();
    let mut meta = std::collections::HashMap::new();
    meta.insert(
        "category".to_string(),
        ScalarValue::Keyword("science".to_string()),
    );
    col.add(&[Doc {
        id: "a".into(),
        text: Some("hello world".into()),
        vector: Some(vec![1.0, 0.0]),
        meta: Some(meta),
    }])
    .unwrap();
    col.flush().unwrap();
    let hits = col
        .search(&SearchQuery {
            text: Some("hello".into()),
            vector: None,
            top_k: 10,
            mode: SearchMode::Text,
            fusion: FusionSpec::Rrf,
            filter: None,
            candidate_multiplier: 3,
        })
        .unwrap();
    assert!(!hits.is_empty());
    assert!(hits[0].fields.is_some());
    assert!(hits[0].fields.as_ref().unwrap().contains_key("category"));
}

// ---- Task 5: search ----

#[test]
fn search_hybrid_returns_relevant() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn crate::vfs::Vfs>;
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let schema = Schema::new(vec![
        ("body".into(), FieldDef::Text),
        (
            "v".into(),
            FieldDef::Vector {
                dim: 3,
                metric: Metric::Cosine,
            },
        ),
    ])
    .unwrap();
    let col = db
        .collection("docs", schema, CollectionOptions::default())
        .unwrap();
    col.add(&[
        Doc {
            id: "cat".into(),
            text: Some("the cat sat on the mat".into()),
            vector: Some(vec![1.0, 0.0, 0.0]),
            meta: None,
        },
        Doc {
            id: "dog".into(),
            text: Some("the dog ran in the park".into()),
            vector: Some(vec![0.0, 1.0, 0.0]),
            meta: None,
        },
        Doc {
            id: "fish".into(),
            text: Some("fish swim in water".into()),
            vector: Some(vec![0.0, 0.0, 1.0]),
            meta: None,
        },
    ])
    .unwrap();
    col.flush().unwrap();
    let hits = col
        .search(&SearchQuery {
            text: Some("cat mat".into()),
            vector: Some(vec![1.0, 0.0, 0.0]),
            top_k: 3,
            mode: SearchMode::Hybrid,
            fusion: FusionSpec::Rrf,
            filter: None,
            candidate_multiplier: 3,
        })
        .unwrap();
    assert!(!hits.is_empty());
    assert_eq!(hits[0].id, "cat");
}

#[test]
fn search_vector_only() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn crate::vfs::Vfs>;
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let schema = Schema::new(vec![(
        "v".into(),
        FieldDef::Vector {
            dim: 2,
            metric: Metric::Cosine,
        },
    )])
    .unwrap();
    let col = db
        .collection("docs", schema, CollectionOptions::default())
        .unwrap();
    col.add(&[
        Doc {
            id: "a".into(),
            text: None,
            vector: Some(vec![1.0, 0.0]),
            meta: None,
        },
        Doc {
            id: "b".into(),
            text: None,
            vector: Some(vec![0.0, 1.0]),
            meta: None,
        },
    ])
    .unwrap();
    col.flush().unwrap();
    let hits = col
        .search(&SearchQuery {
            text: None,
            vector: Some(vec![1.0, 0.0]),
            top_k: 2,
            mode: SearchMode::Vector,
            fusion: FusionSpec::Rrf,
            filter: None,
            candidate_multiplier: 3,
        })
        .unwrap();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].id, "a");
}

#[test]
fn search_topk_over_1000_rejected() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn crate::vfs::Vfs>;
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let schema = Schema::new(vec![(
        "v".into(),
        FieldDef::Vector {
            dim: 2,
            metric: Metric::Cosine,
        },
    )])
    .unwrap();
    let col = db
        .collection("docs", schema, CollectionOptions::default())
        .unwrap();
    let r = col.search(&SearchQuery {
        text: None,
        vector: Some(vec![1.0, 0.0]),
        top_k: 1001,
        mode: SearchMode::Vector,
        fusion: FusionSpec::Rrf,
        filter: None,
        candidate_multiplier: 3,
    });
    assert!(matches!(r, Err(VaneError::InvalidArg(_))));
}

#[test]
fn search_filter_accepted_but_not_compiled_in_m1() {
    // 01-hnsw Task 5：M1 不再 reject filter（03-pre-filter 负责编译）。
    // filter 透传 None 占位，搜索正常返回（filter 不生效，等 03 接入）。
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn crate::vfs::Vfs>;
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let schema = Schema::new(vec![(
        "v".into(),
        FieldDef::Vector {
            dim: 2,
            metric: Metric::Cosine,
        },
    )])
    .unwrap();
    let col = db
        .collection("docs", schema, CollectionOptions::default())
        .unwrap();
    let r = col.search(&SearchQuery {
        text: None,
        vector: Some(vec![1.0, 0.0]),
        top_k: 10,
        mode: SearchMode::Vector,
        fusion: FusionSpec::Rrf,
        filter: Some(Filter {
            fields: vec![(
                "lang".into(),
                FilterCond::Eq(ScalarValue::Keyword("zh".into())),
            )],
        }),
        candidate_multiplier: 3,
    });
    // 无文档：返回空 Vec（Ok），不再 Err(InvalidArg)
    assert!(matches!(r, Ok(hits) if hits.is_empty()));
}

#[test]
fn search_hybrid_linear_fusion_returns_results() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn crate::vfs::Vfs>;
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let schema = Schema::new(vec![
        ("body".into(), FieldDef::Text),
        (
            "v".into(),
            FieldDef::Vector {
                dim: 3,
                metric: Metric::Cosine,
            },
        ),
    ])
    .unwrap();
    let col = db
        .collection("docs", schema, CollectionOptions::default())
        .unwrap();
    col.add(&[
        Doc {
            id: "cat".into(),
            text: Some("the cat sat on the mat".into()),
            vector: Some(vec![1.0, 0.0, 0.0]),
            meta: None,
        },
        Doc {
            id: "dog".into(),
            text: Some("the dog ran in the park".into()),
            vector: Some(vec![0.0, 1.0, 0.0]),
            meta: None,
        },
    ])
    .unwrap();
    col.flush().unwrap();
    let hits = col
        .search(&SearchQuery {
            text: Some("cat".into()),
            vector: Some(vec![1.0, 0.0, 0.0]),
            top_k: 2,
            mode: SearchMode::Hybrid,
            fusion: FusionSpec::Linear { alpha: 0.5 },
            filter: None,
            candidate_multiplier: 3,
        })
        .unwrap();
    assert!(
        !hits.is_empty(),
        "linear fusion should return non-empty results"
    );
    for w in hits.windows(2) {
        assert!(w[0].score >= w[1].score, "results should be sorted desc");
    }
}

// ---- Task 6: I-2 + 多段 + 占位 ----

#[test]
fn i2_dual_index_atomic_visibility() {
    // 不变量 I-2：flush 后向量与倒排在同一快照同时出现
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn crate::vfs::Vfs>;
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let schema = Schema::new(vec![
        ("body".into(), FieldDef::Text),
        (
            "v".into(),
            FieldDef::Vector {
                dim: 2,
                metric: Metric::Cosine,
            },
        ),
    ])
    .unwrap();
    let col = db
        .collection("docs", schema, CollectionOptions::default())
        .unwrap();
    col.add(&[Doc {
        id: "x".into(),
        text: Some("unique token".into()),
        vector: Some(vec![1.0, 1.0]),
        meta: None,
    }])
    .unwrap();
    // flush 前：vector 和 text 都不可搜
    let v_before = col
        .search(&SearchQuery {
            text: None,
            vector: Some(vec![1.0, 1.0]),
            top_k: 10,
            mode: SearchMode::Vector,
            fusion: FusionSpec::Rrf,
            filter: None,
            candidate_multiplier: 3,
        })
        .unwrap();
    assert!(v_before.is_empty());
    let t_before = col
        .search(&SearchQuery {
            text: Some("unique".into()),
            vector: None,
            top_k: 10,
            mode: SearchMode::Text,
            fusion: FusionSpec::Rrf,
            filter: None,
            candidate_multiplier: 3,
        })
        .unwrap();
    assert!(t_before.is_empty());
    // flush
    col.flush().unwrap();
    // flush 后：vector 和 text 同时可搜（同一快照）
    let v_after = col
        .search(&SearchQuery {
            text: None,
            vector: Some(vec![1.0, 1.0]),
            top_k: 10,
            mode: SearchMode::Vector,
            fusion: FusionSpec::Rrf,
            filter: None,
            candidate_multiplier: 3,
        })
        .unwrap();
    let t_after = col
        .search(&SearchQuery {
            text: Some("unique".into()),
            vector: None,
            top_k: 10,
            mode: SearchMode::Text,
            fusion: FusionSpec::Rrf,
            filter: None,
            candidate_multiplier: 3,
        })
        .unwrap();
    assert!(!v_after.is_empty(), "vector should be visible after flush");
    assert!(!t_after.is_empty(), "text should be visible after flush");
    assert_eq!(v_after[0].id, "x");
    assert_eq!(t_after[0].id, "x");
}

#[test]
fn delete_compact_implemented_reindex_export_still_unsupported() {
    // 02-tombstone-merge 实装后：delete/compact 已落地（不再 E_UNSUPPORTED）。
    // reindex（06）/export（M2）仍占位。
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn crate::vfs::Vfs>;
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let schema = Schema::new(vec![(
        "v".into(),
        FieldDef::Vector {
            dim: 2,
            metric: Metric::Cosine,
        },
    )])
    .unwrap();
    let col = db
        .collection("docs", schema, CollectionOptions::default())
        .unwrap();
    // delete 空集合返回 0（非 Unsupported）。
    assert_eq!(col.delete(&["x".into()]).unwrap(), 0);
    // compact 空集合 Ok（无段可合并）。
    assert!(col.compact().is_ok());
    // reindex/export 仍占位。
    assert!(matches!(col.reindex(), Err(VaneError::Unsupported)));
    assert!(matches!(db.export("/tmp/x"), Err(VaneError::Unsupported)));
}

#[test]
fn multi_segment_flush_and_search() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn crate::vfs::Vfs>;
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let schema = Schema::new(vec![(
        "v".into(),
        FieldDef::Vector {
            dim: 2,
            metric: Metric::Cosine,
        },
    )])
    .unwrap();
    let col = db
        .collection("docs", schema, CollectionOptions::default())
        .unwrap();
    col.add(&[Doc {
        id: "a".into(),
        text: None,
        vector: Some(vec![1.0, 0.0]),
        meta: None,
    }])
    .unwrap();
    col.flush().unwrap();
    col.add(&[Doc {
        id: "b".into(),
        text: None,
        vector: Some(vec![0.0, 1.0]),
        meta: None,
    }])
    .unwrap();
    col.flush().unwrap();
    let hits = col
        .search(&SearchQuery {
            text: None,
            vector: Some(vec![1.0, 0.0]),
            top_k: 2,
            mode: SearchMode::Vector,
            fusion: FusionSpec::Rrf,
            filter: None,
            candidate_multiplier: 3,
        })
        .unwrap();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].id, "a");
}

#[test]
fn restore_multi_segment_uses_stored_docid_base() {
    // 验证 restore_from_manifest 从段头读 docid_base（而非累加 doc_count 推断），
    // 多段 reopen 后 search 命中正确的 external_id（不会因 offset 错位串段）。
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn crate::vfs::Vfs>;
    let schema = Schema::new(vec![(
        "v".into(),
        FieldDef::Vector {
            dim: 2,
            metric: Metric::Cosine,
        },
    )])
    .unwrap();

    // 第一段：doc a/b
    let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
    let col = db
        .collection("docs", schema.clone(), CollectionOptions::default())
        .unwrap();
    col.add(&[
        Doc {
            id: "a".into(),
            text: None,
            vector: Some(vec![1.0, 0.0]),
            meta: None,
        },
        Doc {
            id: "b".into(),
            text: None,
            vector: Some(vec![0.0, 1.0]),
            meta: None,
        },
    ])
    .unwrap();
    col.flush().unwrap();
    // 第二段：doc c/d
    col.add(&[
        Doc {
            id: "c".into(),
            text: None,
            vector: Some(vec![1.0, 1.0]),
            meta: None,
        },
        Doc {
            id: "d".into(),
            text: None,
            vector: Some(vec![-1.0, 0.0]),
            meta: None,
        },
    ])
    .unwrap();
    col.flush().unwrap();
    db.close().unwrap();

    // reopen：restore 应正确还原各段 docid_base
    let db2 = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col2 = db2
        .collection("docs", schema, CollectionOptions::default())
        .unwrap();
    // 查 c 的向量，应命中 c 而非 a/b/d（验证第二段 offset=2 正确）
    let hits = col2
        .search(&SearchQuery {
            text: None,
            vector: Some(vec![1.0, 1.0]),
            top_k: 4,
            mode: SearchMode::Vector,
            fusion: FusionSpec::Rrf,
            filter: None,
            candidate_multiplier: 3,
        })
        .unwrap();
    assert!(!hits.is_empty());
    assert_eq!(
        hits[0].id, "c",
        "restore 后应命中正确文档（docid_base 从段头读）"
    );
    // 再灌一篇验证 next_docid 正确（=4，不与已存在 docid 冲突）
    let report = col2
        .add(&[Doc {
            id: "e".into(),
            text: None,
            vector: Some(vec![0.0, -1.0]),
            meta: None,
        }])
        .unwrap();
    assert_eq!(report.accepted, 1);
    col2.flush().unwrap();
    let hits_e = col2
        .search(&SearchQuery {
            text: None,
            vector: Some(vec![0.0, -1.0]),
            top_k: 1,
            mode: SearchMode::Vector,
            fusion: FusionSpec::Rrf,
            filter: None,
            candidate_multiplier: 3,
        })
        .unwrap();
    assert_eq!(hits_e[0].id, "e");
}
