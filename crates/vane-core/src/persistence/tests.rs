use super::*;
use crate::tokenizer::{BuiltinTokenizer, UserDictEntry};
use crate::types::{FieldDef, Metric, Schema, TokenizerId};
use crate::vfs::memory::MemoryVfs;
use crate::vfs::Vfs;

#[test]
fn manifest_empty_serialize_roundtrip() {
    let m = Manifest::empty();
    let json = serde_json::to_string(&m).unwrap();
    let back: Manifest = serde_json::from_str(&json).unwrap();
    assert_eq!(back.version, 1);
    assert!(back.collections.is_empty());
}

#[test]
fn manifest_with_collection_roundtrip() {
    let schema = Schema::new(vec![
        ("body".into(), FieldDef::Text),
        (
            "v".into(),
            FieldDef::Vector {
                dim: 384,
                metric: Metric::Cosine,
            },
        ),
    ])
    .unwrap();
    let mut m = Manifest::empty();
    m.collections.insert(
        "docs".into(),
        CollectionMeta {
            schema,
            tokenizer_kind: BuiltinTokenizer::Standard,
            tokenizer_id: TokenizerId([0xab; 32]),
            user_dict: vec![UserDictEntry::Word("test".into())],
            segment_ulids: vec!["01HZX...".into()],
        },
    );
    let json = serde_json::to_string(&m).unwrap();
    let back: Manifest = serde_json::from_str(&json).unwrap();
    assert_eq!(back.collections.len(), 1);
    let col = &back.collections["docs"];
    assert_eq!(col.tokenizer_kind, BuiltinTokenizer::Standard);
    assert_eq!(col.segment_ulids, vec!["01HZX...".to_string()]);
}

#[test]
fn manifest_store_save_and_load() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
    let store = ManifestStore::new(vfs.clone(), "mydb");
    // 新库：load 返回 None
    assert!(matches!(store.load(), Ok(None)));

    let mut m = Manifest::empty();
    m.collections.insert(
        "c1".into(),
        CollectionMeta {
            schema: Schema::new(vec![(
                "v".into(),
                FieldDef::Vector {
                    dim: 8,
                    metric: Metric::Cosine,
                },
            )])
            .unwrap(),
            tokenizer_kind: BuiltinTokenizer::Standard,
            tokenizer_id: TokenizerId([0; 32]),
            user_dict: vec![],
            segment_ulids: vec![],
        },
    );
    store.save_atomic(&m).unwrap();

    let loaded = store.load().unwrap().unwrap();
    assert_eq!(loaded.collections.len(), 1);
    assert!(loaded.collections.contains_key("c1"));
}

#[test]
fn manifest_store_save_atomic_overwrites() {
    // 不变量 I-6：rename 覆盖旧 manifest，旧数据不损坏
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
    let store = ManifestStore::new(vfs.clone(), "db");
    let mut m1 = Manifest::empty();
    m1.collections.insert(
        "old".into(),
        CollectionMeta {
            schema: Schema::new(vec![(
                "v".into(),
                FieldDef::Vector {
                    dim: 4,
                    metric: Metric::Dot,
                },
            )])
            .unwrap(),
            tokenizer_kind: BuiltinTokenizer::Standard,
            tokenizer_id: TokenizerId([1; 32]),
            user_dict: vec![],
            segment_ulids: vec![],
        },
    );
    store.save_atomic(&m1).unwrap();

    let mut m2 = Manifest::empty();
    m2.collections.insert(
        "new".into(),
        CollectionMeta {
            schema: Schema::new(vec![(
                "v".into(),
                FieldDef::Vector {
                    dim: 4,
                    metric: Metric::Dot,
                },
            )])
            .unwrap(),
            tokenizer_kind: BuiltinTokenizer::Standard,
            tokenizer_id: TokenizerId([2; 32]),
            user_dict: vec![],
            segment_ulids: vec![],
        },
    );
    store.save_atomic(&m2).unwrap();

    let loaded = store.load().unwrap().unwrap();
    assert!(!loaded.collections.contains_key("old"));
    assert!(loaded.collections.contains_key("new"));
}

#[test]
fn manifest_store_corrupt_returns_error() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
    // 写损坏的 manifest
    vfs.create("db/manifest.json").unwrap();
    vfs.write_at("db/manifest.json", b"not json {{{", 0)
        .unwrap();
    let store = ManifestStore::new(vfs, "db");
    assert!(store.load().is_err());
}

/// M4 阶段五 c：VaneError 诊断上下文——manifest parse 错误 String 含
/// db 路径 + 操作 + 建议操作（§10 推荐"先丰富 String"）。
#[test]
fn m4_5c_manifest_parse_error_contains_context() {
    use crate::types::VaneError;
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
    vfs.create("mydb/manifest.json").unwrap();
    vfs.write_at("mydb/manifest.json", b"not json {{{", 0)
        .unwrap();
    let store = ManifestStore::new(vfs, "mydb");
    match store.load() {
        Err(VaneError::Corrupt(m)) => {
            assert!(
                m.contains("manifest parse"),
                "original msg preserved: {}",
                m
            );
            assert!(m.contains("mydb"), "msg must contain db path: {}", m);
            assert!(m.contains("op=load manifest"), "msg must contain op: {}", m);
            assert!(m.contains("建议"), "msg must contain suggestion: {}", m);
        }
        other => panic!("expected Corrupt, got {:?}", other.map_err(|e| e.name())),
    }
}

#[test]
fn auto_committer_default_is_on_1000_1000() {
    match AutoCommitConfig::default() {
        AutoCommitConfig::On {
            interval_ms,
            max_docs,
        } => {
            assert_eq!(interval_ms, 1000);
            assert_eq!(max_docs, 1000);
        }
        AutoCommitConfig::Off => panic!("default should be On"),
    }
}

#[test]
fn auto_committer_triggers_on_max_docs() {
    let mut ac = AutoCommitter::new(AutoCommitConfig::On {
        interval_ms: 60_000,
        max_docs: 100,
    });
    assert!(!ac.should_flush());
    ac.record_docs(50);
    assert!(!ac.should_flush());
    ac.record_docs(50);
    assert!(ac.should_flush());
    ac.reset();
    assert!(!ac.should_flush());
}

#[test]
fn auto_committer_triggers_on_interval() {
    let mut ac = AutoCommitter::new(AutoCommitConfig::On {
        interval_ms: 0,
        max_docs: 1_000_000,
    });
    // interval_ms=0 → 任何时间差都触发（只要有未 flush 文档）
    ac.record_docs(1);
    assert!(ac.should_flush());
}

#[test]
fn auto_committer_off_never_flushes() {
    let mut ac = AutoCommitter::new(AutoCommitConfig::Off);
    ac.record_docs(9999);
    assert!(!ac.should_flush());
}

#[test]
fn manifest_atomicity_crash_before_rename() {
    // 模拟：临时文件写了但未 rename → load 应返回旧 manifest 或 None
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
    let store = ManifestStore::new(vfs.clone(), "db");
    // 先保存一个有效 manifest
    let m1 = Manifest::empty();
    store.save_atomic(&m1).unwrap();
    // 模拟崩溃：写一个临时文件但不 rename
    vfs.create("db/manifest.json.tmp").unwrap();
    vfs.write_at("db/manifest.json.tmp", b"partial garbage", 0)
        .unwrap();
    // load 应返回上一次完整的 manifest（m1），不受 tmp 影响
    let loaded = store.load().unwrap().unwrap();
    assert_eq!(loaded.version, 1);
    assert!(loaded.collections.is_empty());
}

#[test]
fn orphan_segment_cleanup_on_open() {
    // 不变量 I-6：孤儿段文件（ULID 不在 manifest 中）可安全清理
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
    // 正常段
    vfs.create("db/segments/seg_01AAA").unwrap();
    // 孤儿段
    vfs.create("db/segments/seg_01BBB").unwrap();
    let store = ManifestStore::new(vfs.clone(), "db");
    let mut m = Manifest::empty();
    m.collections.insert(
        "c".into(),
        CollectionMeta {
            schema: Schema::new(vec![(
                "v".into(),
                FieldDef::Vector {
                    dim: 4,
                    metric: Metric::Cosine,
                },
            )])
            .unwrap(),
            tokenizer_kind: BuiltinTokenizer::Standard,
            tokenizer_id: TokenizerId([0; 32]),
            user_dict: vec![],
            segment_ulids: vec!["01AAA".into()],
        },
    );
    store.save_atomic(&m).unwrap();
    // 验证 manifest 只记 01AAA；01BBB 是孤儿
    let loaded = store.load().unwrap().unwrap();
    assert_eq!(
        loaded.collections["c"].segment_ulids,
        vec!["01AAA".to_string()]
    );
    // 清理孤儿段（api-core 在 open 时调用；此处验证逻辑）
    let known: std::collections::HashSet<_> = loaded
        .collections
        .values()
        .flat_map(|c| c.segment_ulids.iter().cloned())
        .collect();
    let all_segs = vfs.list("db/segments").unwrap();
    let orphans: Vec<_> = all_segs
        .iter()
        .filter(|s| !known.contains(s.trim_start_matches("seg_")))
        .collect();
    assert!(orphans.iter().any(|s| s.as_str() == "seg_01BBB"));
}

#[test]
fn save_atomic_with_stale_tmp_succeeds() {
    // I16: 残留 tmp 场景下 save_atomic 仍成功
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
    let store = ManifestStore::new(vfs.clone(), "db");
    // 模拟残留 tmp
    vfs.create("db/manifest.json.tmp").unwrap();
    vfs.write_at("db/manifest.json.tmp", b"stale garbage", 0)
        .unwrap();
    // save_atomic 应先删 tmp 再写
    let m = Manifest::empty();
    store.save_atomic(&m).unwrap();
    // manifest.json 正确写入
    let loaded = store.load().unwrap().unwrap();
    assert_eq!(loaded.version, 1);
    assert!(loaded.collections.is_empty());
}
