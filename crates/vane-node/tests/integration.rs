//! Rust 集成测试：MemoryVfs + core 直连，校验 napi 转换函数与 core 语义一致。
//! 纯 Rust（不引入 JS），验证 binding 薄壳不破坏 core 行为（I-8）。

use std::sync::Arc;
use vane_core::api::{Db, OpenOptions};
use vane_core::vfs::memory::MemoryVfs;

#[test]
fn full_cycle_memory_vfs() {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs.clone(), "mem", OpenOptions::default()).unwrap();
    let schema = vane_core::types::Schema::new(vec![
        ("t".into(), vane_core::types::FieldDef::Text),
        (
            "v".into(),
            vane_core::types::FieldDef::Vector {
                dim: 2,
                metric: vane_core::types::Metric::Cosine,
            },
        ),
    ])
    .unwrap();
    let col = db.collection("c", schema, Default::default()).unwrap();
    let docs = vec![
        vane_core::api::Doc {
            id: "a".into(),
            text: Some("foo bar".into()),
            vector: Some(vec![1.0, 0.0]),
            meta: None,
        },
        vane_core::api::Doc {
            id: "b".into(),
            text: Some("foo baz".into()),
            vector: Some(vec![0.0, 1.0]),
            meta: None,
        },
    ];
    col.add(&docs).unwrap();
    col.flush().unwrap();
    let q = vane_core::api::SearchQuery {
        text: Some("foo".into()),
        vector: Some(vec![1.0, 0.0]),
        top_k: 10,
        mode: vane_core::api::SearchMode::Hybrid,
        fusion: vane_core::api::FusionSpec::Rrf,
        filter: None,
        candidate_multiplier: 3,
    };
    let hits = col.search(&q).unwrap();
    assert_eq!(hits.len(), 2);
    assert!(hits[0].score >= hits[1].score);
    db.close().unwrap();
}

#[test]
fn delete_returns_count_for_unknown_id() {
    // 02-tombstone-merge 实装后：delete 不再返回 E_UNSUPPORTED，
    // 对不存在的 id 返回 0（命中数）。
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs, "mem2", OpenOptions::default()).unwrap();
    let schema = vane_core::types::Schema::new(vec![(
        "v".into(),
        vane_core::types::FieldDef::Vector {
            dim: 2,
            metric: vane_core::types::Metric::Cosine,
        },
    )])
    .unwrap();
    let col = db.collection("c", schema, Default::default()).unwrap();
    let r = col.delete(&["x".into()]);
    assert!(r.is_ok());
    assert_eq!(r.unwrap(), 0);
}

#[test]
fn export_succeeds_m2_12() {
    // M2-12：export 从 Unsupported 占位变真实导出。
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn vane_core::vfs::Vfs>;
    let db = Db::open(vfs.clone(), "mem3", OpenOptions::default()).unwrap();
    let schema = vane_core::types::Schema::new(vec![(
        "v".into(),
        vane_core::types::FieldDef::Vector {
            dim: 2,
            metric: vane_core::types::Metric::Cosine,
        },
    )])
    .unwrap();
    let col = db
        .collection("c", schema, vane_core::api::CollectionOptions::default())
        .unwrap();
    let docs = vec![vane_core::api::Doc {
        id: "a".into(),
        text: None,
        vector: Some(vec![1.0, 0.0]),
        meta: None,
    }];
    col.add(&docs).unwrap();
    col.flush().unwrap();
    let r = db.export("backup.vane");
    assert!(r.is_ok(), "export should succeed: {:?}", r);
    // dest 存在且 magic 正确
    let mut buf = [0u8; 9];
    let n = vfs.read_at("backup.vane", &mut buf, 0).unwrap();
    assert_eq!(n, 9);
    assert_eq!(&buf, b"VANE_SNAP");
}

#[test]
fn reindex_requires_pending_reindex() {
    // 06-userdict-reindex：reindex 在 Stable 状态下返回 InvalidArg（需先 set_user_dict）。
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs, "mem4", OpenOptions::default()).unwrap();
    let schema = vane_core::types::Schema::new(vec![(
        "v".into(),
        vane_core::types::FieldDef::Vector {
            dim: 2,
            metric: vane_core::types::Metric::Cosine,
        },
    )])
    .unwrap();
    let col = db.collection("c", schema, Default::default()).unwrap();
    let r = col.reindex();
    assert!(matches!(r, Err(vane_core::types::VaneError::InvalidArg(_))));
}
