//! 04-wal 单元测试（Task 1/2）。

use super::*;
use crate::vfs::memory::MemoryVfs;
use std::sync::Arc;

#[test]
fn wal_append_read_roundtrip() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn crate::vfs::Vfs>;
    let wal = Wal::open(vfs, "db").unwrap();
    wal.append(&WalRecord::AddSegment {
        collection: "c".into(),
        ulid: "seg_001".into(),
    })
    .unwrap();
    wal.append(&WalRecord::AddTombstone {
        collection: "c".into(),
        ulid: "seg_001".into(),
        docids: vec![1, 3],
    })
    .unwrap();
    let records = wal.read_all().unwrap();
    assert_eq!(records.len(), 2);
    assert!(matches!(
        records[0],
        WalRecord::AddSegment { ref ulid, .. } if ulid == "seg_001"
    ));
    assert!(matches!(
        records[1],
        WalRecord::AddTombstone { ref docids, .. } if docids == &vec![1, 3]
    ));
}

#[test]
fn wal_open_is_idempotent() {
    // 多次 open 同一 WAL 不丢已有记录（追加语义）。
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn crate::vfs::Vfs>;
    {
        let wal = Wal::open(vfs.clone(), "db").unwrap();
        wal.append(&WalRecord::AddSegment {
            collection: "c".into(),
            ulid: "seg_a".into(),
        })
        .unwrap();
    }
    let wal = Wal::open(vfs, "db").unwrap();
    let records = wal.read_all().unwrap();
    assert_eq!(records.len(), 1);
}

#[test]
fn wal_read_all_empty_for_new_db() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn crate::vfs::Vfs>;
    let wal = Wal::open(vfs, "db").unwrap();
    let records = wal.read_all().unwrap();
    assert!(records.is_empty());
}

#[test]
fn wal_truncate_clears_records() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn crate::vfs::Vfs>;
    let wal = Wal::open(vfs, "db").unwrap();
    wal.append(&WalRecord::AddSegment {
        collection: "c".into(),
        ulid: "seg_001".into(),
    })
    .unwrap();
    wal.truncate().unwrap();
    let records = wal.read_all().unwrap();
    assert!(records.is_empty());
}

#[test]
fn wal_truncate_then_append_works() {
    // truncate 后文件可继续 append（create 重建空文件）。
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn crate::vfs::Vfs>;
    let wal = Wal::open(vfs, "db").unwrap();
    wal.append(&WalRecord::AddSegment {
        collection: "c".into(),
        ulid: "seg_a".into(),
    })
    .unwrap();
    wal.truncate().unwrap();
    wal.append(&WalRecord::AddSegment {
        collection: "c".into(),
        ulid: "seg_b".into(),
    })
    .unwrap();
    let records = wal.read_all().unwrap();
    assert_eq!(records.len(), 1);
    assert!(matches!(
        records[0],
        WalRecord::AddSegment { ref ulid, .. } if ulid == "seg_b"
    ));
}

// 2.1.4：recover 目录扫描——清理 manifest 不含的孤儿 seg_<ulid> 段目录。
// 场景：段文件已写盘但 WAL 未 append 即崩溃（SPEC §6.4 line 226）。
#[test]
fn recover_cleans_orphan_segment_dir_not_in_wal() {
    use crate::persistence::{CollectionMeta, Manifest};
    use crate::types::{FieldDef, Metric, ScalarKind, Schema};
    use crate::vfs::Vfs;

    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    // 构造 manifest 含一个合法段 ULID。
    let schema = Schema::new(vec![
        (
            "tag".into(),
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
    let mut manifest = Manifest::empty();
    manifest.collections.insert(
        "c".into(),
        CollectionMeta {
            schema,
            tokenizer_kind: crate::tokenizer::BuiltinTokenizer::Standard,
            tokenizer_id: crate::tokenizer::compute_tokenizer_id(
                crate::tokenizer::BuiltinTokenizer::Standard,
                &[],
            ),
            user_dict: vec![],
            segment_ulids: vec!["01LEGALULID".into()],
        },
    );

    // 合法段目录（在 manifest 中）——应保留。
    vfs.create("db/segments/seg_01LEGALULID/header.bin")
        .unwrap();
    vfs.write_at("db/segments/seg_01LEGALULID/header.bin", b"ok", 0)
        .unwrap();

    // 孤儿段目录（不在 manifest、不在 WAL）——应被清理。
    vfs.create("db/segments/seg_ORPHAN_NO_WAL/header.bin")
        .unwrap();
    vfs.write_at("db/segments/seg_ORPHAN_NO_WAL/header.bin", b"partial", 0)
        .unwrap();

    // 非 seg_ 前缀的杂项目录——不触碰。
    vfs.create("db/segments/_tmp/header.bin").unwrap();

    let _tombstones = recover(&vfs, "db", &manifest).unwrap();

    let entries = vfs.list("db/segments").unwrap();
    assert!(
        entries.iter().any(|e| e.contains("01LEGALULID")),
        "manifest 合法段必须保留: {:?}",
        entries
    );
    assert!(
        !entries.iter().any(|e| e.contains("ORPHAN_NO_WAL")),
        "孤儿段必须被清理: {:?}",
        entries
    );
    assert!(
        entries.iter().any(|e| e.contains("_tmp")),
        "非 seg_ 前缀目录不应被触碰: {:?}",
        entries
    );
}

// 2.1.4：空 segments 目录（新库）→ recover 无异常。
#[test]
fn recover_empty_segments_dir_no_error() {
    use crate::persistence::Manifest;

    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn crate::vfs::Vfs>;
    let manifest = Manifest::empty();
    // segments 目录不存在（新库）。
    let _tombstones = recover(&vfs, "db", &manifest).unwrap();
    // 无异常即通过。
}

/// M4 诊断重构：wal parse 错误的结构化 ErrorContext（op + hint）。
/// message 含 wal parse + path，op/hint 为独立字段。
#[test]
fn m4_5c_wal_parse_error_contains_context() {
    use crate::types::VaneError;
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn crate::vfs::Vfs>;
    // 写损坏的 wal.log（非 JSON 行）
    vfs.create("mydb/wal.log").unwrap();
    vfs.write_at("mydb/wal.log", b"not json {{{\n", 0).unwrap();
    let wal = Wal::open(vfs, "mydb").unwrap();
    match wal.read_all() {
        Err(VaneError::Corrupt(ctx)) => {
            assert!(
                ctx.message.contains("wal parse"),
                "message preserved: {}",
                ctx.message
            );
            assert!(
                ctx.message.contains("mydb"),
                "message must contain path: {}",
                ctx.message
            );
            assert_eq!(ctx.op, Some("wal recover"), "op field: {:?}", ctx.op);
            assert!(ctx.hint.is_some(), "hint field must be set");
        }
        other => panic!("expected Corrupt, got {:?}", other.map_err(|e| e.name())),
    }
}
