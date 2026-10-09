use super::*;
use crate::tokenizer::{BuiltinTokenizer, UserDictEntry};
use crate::types::{FieldDef, Metric, Schema, TokenizerId, VaneError};
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

/// M4 诊断重构：manifest parse 错误的结构化 ErrorContext（op + hint 独立字段）。
#[test]
fn m4_5c_manifest_parse_error_contains_context() {
    use crate::types::VaneError;
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
    vfs.create("mydb/manifest.json").unwrap();
    vfs.write_at("mydb/manifest.json", b"not json {{{", 0)
        .unwrap();
    let store = ManifestStore::new(vfs, "mydb");
    match store.load() {
        Err(VaneError::Corrupt(ctx)) => {
            assert!(
                ctx.message.contains("manifest parse"),
                "original msg preserved: {}",
                ctx.message
            );
            assert!(
                ctx.message.contains("mydb"),
                "message must contain db path: {}",
                ctx.message
            );
            assert_eq!(ctx.op, Some("load manifest"), "op field: {:?}", ctx.op);
            assert!(ctx.hint.is_some(), "hint field must be set");
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
    assert!(ac.interval_delay().is_none());
}

#[test]
fn auto_committer_interval_delay_ignores_max_docs() {
    let mut ac = AutoCommitter::new(AutoCommitConfig::On {
        interval_ms: 60_000,
        max_docs: 1,
    });
    ac.record_docs(1);
    assert!(ac.should_flush(), "max_docs 仍走 add 路径");
    let delay = ac.interval_delay().expect("pending docs wait for interval");
    assert!(
        delay > std::time::Duration::from_secs(30),
        "max_docs 已满也不能让空闲线程立刻重试，delay={delay:?}"
    );
}

#[test]
fn auto_committer_interval_delay_zero_when_elapsed() {
    let mut ac = AutoCommitter::new(AutoCommitConfig::On {
        interval_ms: 0,
        max_docs: 1_000_000,
    });
    assert!(ac.interval_delay().is_none());
    ac.record_docs(1);
    assert_eq!(ac.interval_delay(), Some(std::time::Duration::ZERO));
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

fn put_file(vfs: &dyn Vfs, path: &str, bytes: &[u8]) {
    vfs.create(path).unwrap();
    if !bytes.is_empty() {
        vfs.write_at(path, bytes, 0).unwrap();
    }
}

fn read_all(vfs: &dyn Vfs, path: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = [0u8; 64];
    let mut off = 0u64;
    loop {
        let n = vfs.read_at(path, &mut buf, off).unwrap();
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
        off += n as u64;
    }
    out
}

fn assert_load_manifest_corrupt(err: VaneError) {
    match err {
        VaneError::Corrupt(ctx) => {
            assert_eq!(ctx.op, Some("load manifest"));
            let hint = ctx.hint.unwrap_or_default();
            assert!(
                hint.contains("不要删"),
                "hint must say not to delete segments: {hint}"
            );
            assert!(
                hint.contains("export"),
                "hint must point at export backup: {hint}"
            );
        }
        other => panic!("expected Corrupt, got {other}"),
    }
}

fn manifest_naming_keep() -> Manifest {
    let mut m = Manifest::empty();
    m.collections.insert(
        "kept".into(),
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
            tokenizer_id: TokenizerId([9; 32]),
            user_dict: vec![],
            segment_ulids: vec!["KEEP".into()],
        },
    );
    m
}

const SEG_BYTES: &[u8] = b"segment-bytes";

fn seed_segment(vfs: &dyn Vfs) {
    put_file(vfs, "db/segments/seg_KEEP/header.bin", SEG_BYTES);
}

fn assert_segment_kept(vfs: &dyn Vfs) {
    assert_eq!(
        read_all(vfs, "db/segments/seg_KEEP/header.bin"),
        SEG_BYTES,
        "committed segment bytes must remain"
    );
    let segs = vfs.list("db/segments").unwrap();
    assert!(
        segs.iter().any(|s| s.starts_with("seg_")),
        "seg_* entry must remain: {segs:?}"
    );
}

/// manifest 缺失且已有段：不能当成新库；段文件不得被 load 清掉。
#[test]
fn manifest_loss_missing_with_segments_is_corrupt() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
    seed_segment(vfs.as_ref());
    let store = ManifestStore::new(vfs.clone(), "db");
    match store.load() {
        Err(e) => assert_load_manifest_corrupt(e),
        Ok(v) => panic!("expected Corrupt, got Ok({v:?})"),
    }
    assert_segment_kept(vfs.as_ref());
    assert!(
        vfs.list("db").unwrap().iter().all(|n| n != "manifest.json"),
        "load must not invent manifest.json"
    );
}

/// 可解析的 manifest.json.tmp 提升为 manifest，段保留。
#[test]
fn manifest_loss_promotes_valid_tmp_and_keeps_segment() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
    seed_segment(vfs.as_ref());
    let manifest = manifest_naming_keep();
    let bytes = serde_json::to_vec(&manifest).unwrap();
    put_file(vfs.as_ref(), "db/manifest.json.tmp", &bytes);
    let store = ManifestStore::new(vfs.clone(), "db");
    let loaded = store
        .load()
        .unwrap()
        .expect("parseable tmp must be promoted");
    assert!(loaded.collections.contains_key("kept"));
    assert_eq!(loaded.collections["kept"].segment_ulids, vec!["KEEP"]);
    assert_eq!(read_all(vfs.as_ref(), "db/manifest.json"), bytes);
    assert!(
        vfs.read_at("db/manifest.json.tmp", &mut [0u8; 1], 0)
            .is_err(),
        "tmp must be renamed away"
    );
    assert_segment_kept(vfs.as_ref());
}

/// 能解析、但没有列出已有段的 tmp：不能提升。提升后孤儿清理会删掉这些段。
#[test]
fn manifest_loss_stale_tmp_that_omits_segments_is_corrupt() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
    seed_segment(vfs.as_ref());
    let bytes = serde_json::to_vec(&Manifest::empty()).unwrap();
    put_file(vfs.as_ref(), "db/manifest.json.tmp", &bytes);
    let store = ManifestStore::new(vfs.clone(), "db");
    match store.load() {
        Err(e) => assert_load_manifest_corrupt(e),
        Ok(v) => panic!("expected Corrupt, got Ok({v:?})"),
    }
    assert_eq!(read_all(vfs.as_ref(), "db/manifest.json.tmp"), bytes);
    assert!(
        vfs.read_at("db/manifest.json", &mut [0u8; 1], 0).is_err(),
        "stale tmp must not be promoted"
    );
    assert_segment_kept(vfs.as_ref());
}

#[test]
fn io_not_found_does_not_match_os_error_20() {
    let miss = VaneError::Io("not found: No such file or directory (os error 2)".into());
    let not_dir = VaneError::Io("Not a directory (os error 20)".into());
    let is_dir = VaneError::Io("Is a directory (os error 21)".into());
    assert!(super::io_is_not_found(&miss));
    assert!(!super::io_is_not_found(&not_dir));
    assert!(!super::io_is_not_found(&is_dir));
}

/// 坏 tmp 且已有段：Corrupt，不把坏 tmp 提升成 manifest。
#[test]
fn manifest_loss_bad_tmp_with_segments_is_corrupt_not_promoted() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
    seed_segment(vfs.as_ref());
    let bad = b"{not-json";
    put_file(vfs.as_ref(), "db/manifest.json.tmp", bad);
    let store = ManifestStore::new(vfs.clone(), "db");
    match store.load() {
        Err(e) => assert_load_manifest_corrupt(e),
        Ok(v) => panic!("expected Corrupt, got Ok({v:?})"),
    }
    assert_eq!(read_all(vfs.as_ref(), "db/manifest.json.tmp"), bad);
    assert!(
        vfs.read_at("db/manifest.json", &mut [0u8; 1], 0).is_err(),
        "bad tmp must not be promoted"
    );
    assert_segment_kept(vfs.as_ref());
}

/// 目录里已有 manifest.json，但 ReadAt 返回 Io：原样上抛，不能当成新库。
#[test]
fn manifest_loss_listed_manifest_read_io_is_not_absent() {
    use crate::vfs::fault::{Fault, FaultVfs, VfsOp};

    let inner = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
    let plain = ManifestStore::new(inner.clone(), "db");
    plain.save_atomic(&Manifest::empty()).unwrap();
    let fault = FaultVfs::new(inner);
    fault.inject(Fault::IoError {
        op: VfsOp::ReadAt,
        path_pattern: "db/manifest.json".to_string(),
        msg: "read failed".to_string(),
        one_shot: false,
        trigger_on_nth: 0,
    });
    let store = ManifestStore::new(std::sync::Arc::new(fault), "db");
    match store.load() {
        Err(VaneError::Io(ctx)) => {
            assert!(
                ctx.message.contains("read failed"),
                "original io error must surface: {}",
                ctx.message
            );
        }
        Ok(None) => panic!("read Io must not be treated as a new database"),
        other => panic!("expected Io, got {other:?}"),
    }
}
