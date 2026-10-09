//! 04-wal 单元测试（Task 1/2）。

use super::*;
use crate::vfs::memory::MemoryVfs;
use crate::vfs::Vfs;
use std::sync::Arc;

fn wal_file(vfs: &dyn Vfs, path: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = [0u8; 1024];
    let mut off = 0u64;
    loop {
        let n = vfs.read_at(path, &mut buf, off).expect("read wal");
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
        off += n as u64;
    }
    out
}

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

/// 完整行之后的无换行半截 JSON 不是 E_CORRUPT：丢掉残尾，并把它从磁盘截掉。
#[test]
fn wal_read_all_drops_unterminated_partial_tail() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    let wal = Wal::open(vfs.clone(), "db").unwrap();
    wal.append(&WalRecord::AddSegment {
        collection: "c".into(),
        ulid: "seg_ok".into(),
    })
    .unwrap();
    let len = wal_file(&*vfs, "db/wal.log").len() as u64;
    vfs.write_at("db/wal.log", b"{\"partial\"", len).unwrap();

    let records = wal.read_all().unwrap();
    assert_eq!(records.len(), 1);
    assert!(matches!(
        records[0],
        WalRecord::AddSegment { ref ulid, .. } if ulid == "seg_ok"
    ));
    let bytes = wal_file(&*vfs, "db/wal.log");
    assert!(
        bytes.ends_with(b"\n"),
        "torn tail must be truncated: {bytes:?}"
    );
    assert!(
        !bytes
            .windows(b"{\"partial\"".len())
            .any(|w| w == b"{\"partial\""),
        "partial bytes must leave the file: {bytes:?}"
    );
}

/// 整个文件都是无换行残尾：恢复为空，磁盘上也不再留半截。
#[test]
fn wal_read_all_only_torn_tail_is_empty() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    let wal = Wal::open(vfs.clone(), "db").unwrap();
    vfs.write_at("db/wal.log", b"{\"partial\"", 0).unwrap();
    let records = wal.read_all().unwrap();
    assert!(records.is_empty());
    assert!(
        wal_file(&*vfs, "db/wal.log").is_empty(),
        "only-torn file must be truncated"
    );
}

/// 末尾没有换行，但最后一段本身是合法 WalRecord：保留。
#[test]
fn wal_read_all_keeps_valid_record_without_newline() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    let record = WalRecord::AddTombstone {
        collection: "c".into(),
        ulid: "seg_ok".into(),
        docids: vec![4],
    };
    let bytes = serde_json::to_vec(&record).unwrap();
    assert!(!bytes.ends_with(b"\n"));
    let wal = Wal::open(vfs.clone(), "db").unwrap();
    vfs.write_at("db/wal.log", &bytes, 0).unwrap();

    let records = wal.read_all().unwrap();
    assert_eq!(records.len(), 1);
    assert!(matches!(
        records[0],
        WalRecord::AddTombstone { ref docids, .. } if docids == &vec![4]
    ));
    assert_eq!(
        wal_file(&*vfs, "db/wal.log"),
        bytes,
        "a valid partial is not an illegal torn tail"
    );
}

/// 残尾不能留在文件里，否则下一次 append 会粘成以换行结束的坏行。
#[test]
fn wal_append_after_torn_tail_preserves_prior_and_ends_with_newline() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    let wal = Wal::open(vfs.clone(), "db").unwrap();
    wal.append(&WalRecord::AddSegment {
        collection: "c".into(),
        ulid: "seg_old".into(),
    })
    .unwrap();
    let len = wal_file(&*vfs, "db/wal.log").len() as u64;
    vfs.write_at("db/wal.log", b"{\"partial\"", len).unwrap();
    wal.append(&WalRecord::AddTombstone {
        collection: "c".into(),
        ulid: "seg_old".into(),
        docids: vec![9],
    })
    .unwrap();

    let records = wal.read_all().unwrap();
    assert_eq!(records.len(), 2);
    assert!(matches!(
        records[0],
        WalRecord::AddSegment { ref ulid, .. } if ulid == "seg_old"
    ));
    assert!(matches!(
        records[1],
        WalRecord::AddTombstone { ref docids, .. } if docids == &vec![9]
    ));
    let bytes = wal_file(&*vfs, "db/wal.log");
    assert!(
        bytes.ends_with(b"\n"),
        "appended wal must end with newline: {bytes:?}"
    );
    assert!(
        !bytes
            .windows(b"{\"partial\"".len())
            .any(|w| w == b"{\"partial\""),
        "torn tail must not be glued into the next line: {bytes:?}"
    );
}

/// 合法但缺换行的尾记录要重写成规范行，再追加，不能把新行粘在它后面。
#[test]
fn wal_append_keeps_valid_record_missing_newline() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    let existing = WalRecord::AddSegment {
        collection: "c".into(),
        ulid: "old".into(),
    };
    let bytes = serde_json::to_vec(&existing).unwrap();
    assert!(!bytes.ends_with(b"\n"));
    let wal = Wal::open(vfs.clone(), "db").unwrap();
    vfs.write_at("db/wal.log", &bytes, 0).unwrap();
    wal.append(&WalRecord::AddTombstone {
        collection: "c".into(),
        ulid: "old".into(),
        docids: vec![3],
    })
    .unwrap();

    let records = wal.read_all().unwrap();
    assert_eq!(records.len(), 2);
    assert!(matches!(
        records[0],
        WalRecord::AddSegment { ref ulid, .. } if ulid == "old"
    ));
    assert!(matches!(
        records[1],
        WalRecord::AddTombstone { ref docids, .. } if docids == &vec![3]
    ));
    assert!(wal_file(&*vfs, "db/wal.log").ends_with(b"\n"));
}

/// 以换行结束的坏行夹在完整记录中间：仍然 Corrupt，且不改文件。
/// 残尾修复若先遇到这种前缀，append 也必须原样返回，不能先截断。
#[test]
fn wal_bad_complete_line_is_corrupt_and_not_rewritten() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    let wal = Wal::open(vfs.clone(), "db").unwrap();
    wal.append(&WalRecord::AddSegment {
        collection: "c".into(),
        ulid: "keep".into(),
    })
    .unwrap();
    let mut raw = wal_file(&*vfs, "db/wal.log");
    raw.extend_from_slice(b"not json {{\n");
    raw.extend_from_slice(b"{\"torn\"");
    vfs.write_at("db/wal.log", &raw, 0).unwrap();
    let raw = wal_file(&*vfs, "db/wal.log");

    match wal.read_all() {
        Err(crate::types::VaneError::Corrupt(ctx)) => {
            assert_eq!(ctx.op, Some("wal recover"));
        }
        other => panic!("expected Corrupt, got {other:?}"),
    }
    assert_eq!(wal_file(&*vfs, "db/wal.log"), raw);

    match wal.append(&WalRecord::AddTombstone {
        collection: "c".into(),
        ulid: "keep".into(),
        docids: vec![1],
    }) {
        Err(crate::types::VaneError::Corrupt(ctx)) => {
            assert_eq!(ctx.op, Some("wal recover"));
        }
        other => panic!("append must not rewrite a bad complete line, got {other:?}"),
    }
    assert_eq!(wal_file(&*vfs, "db/wal.log"), raw);
}

/// 两个 Wal 句柄并发 append。锁按路径共享，不能放在单个 Wal 值里。
#[test]
fn wal_two_threads_append_distinct_tombstones() {
    use std::collections::HashSet;
    use std::thread;

    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    let mut handles = Vec::new();
    for thread_idx in 0..2u64 {
        let vfs = vfs.clone();
        handles.push(thread::spawn(move || {
            let wal = Wal::open(vfs, "db").unwrap();
            for i in 0..50u64 {
                wal.append(&WalRecord::AddTombstone {
                    collection: "c".into(),
                    ulid: "seg".into(),
                    docids: vec![thread_idx * 50 + i],
                })
                .unwrap();
            }
        }));
    }
    for handle in handles {
        handle.join().expect("wal append thread");
    }
    let wal = Wal::open(vfs, "db").unwrap();
    let records = wal.read_all().unwrap();
    assert_eq!(records.len(), 100);
    let mut ids = HashSet::new();
    for record in records {
        match record {
            WalRecord::AddTombstone { docids, .. } => {
                assert_eq!(docids.len(), 1);
                assert!(ids.insert(docids[0]), "duplicate docid {}", docids[0]);
            }
            other => panic!("unexpected record {other:?}"),
        }
    }
    assert_eq!(ids, (0..100u64).collect::<HashSet<_>>());
}

/// 读 wal 中途失败不能当成空日志。空日志会让已提交的 tombstone 在重开时消失。
#[test]
fn wal_read_io_error_is_not_empty_log() {
    use crate::types::VaneError;
    use crate::vfs::fault::{Fault, FaultVfs, VfsOp};

    let inner = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    let wal = Wal::open(inner.clone(), "db").unwrap();
    wal.append(&WalRecord::AddTombstone {
        collection: "c".into(),
        ulid: "seg".into(),
        docids: vec![7],
    })
    .unwrap();
    let faulty = Arc::new(FaultVfs::new(inner));
    faulty.inject(Fault::IoError {
        op: VfsOp::ReadAt,
        path_pattern: "*wal.log".into(),
        msg: "read failed".into(),
        one_shot: false,
        trigger_on_nth: 0,
    });
    let wal = Wal::open(faulty, "db").unwrap();
    match wal.read_all() {
        Err(VaneError::Io(ctx)) => {
            assert!(
                ctx.message.contains("read failed"),
                "read error must surface: {}",
                ctx.message
            );
        }
        other => panic!("expected Io, got {other:?}"),
    }
}

/// 两条 AddTombstone，retain 只留 manifest 里还在的 ulid。
/// 禁止先删 wal.log：删除 wal.log 被故障挡住时，仍在的那条不能丢。
#[test]
fn wal_retain_keeps_live_tombstone_without_deleting_wal() {
    use crate::persistence::{CollectionMeta, Manifest};
    use crate::types::{FieldDef, Metric, ScalarKind, Schema};
    use crate::vfs::fault::{Fault, FaultVfs, VfsOp};

    let inner = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    let wal = Wal::open(inner.clone(), "db").unwrap();
    wal.append(&WalRecord::AddTombstone {
        collection: "c".into(),
        ulid: "keep".into(),
        docids: vec![1],
    })
    .unwrap();
    wal.append(&WalRecord::AddTombstone {
        collection: "c".into(),
        ulid: "drop".into(),
        docids: vec![2],
    })
    .unwrap();
    let before = wal_file(&*inner, "db/wal.log");
    assert!(
        before.windows(b"keep".len()).any(|w| w == b"keep")
            && before.windows(b"drop".len()).any(|w| w == b"drop"),
        "both tombstones committed before retain"
    );

    let faulty = Arc::new(FaultVfs::new(inner.clone()));
    faulty.inject(Fault::IoError {
        op: VfsOp::Delete,
        path_pattern: "db/wal.log".into(),
        msg: "refuse delete wal.log".into(),
        one_shot: false,
        trigger_on_nth: 0,
    });
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
            segment_ulids: vec!["keep".into()],
        },
    );
    let wal = Wal::open(faulty, "db").unwrap();
    wal.retain(|| Ok(manifest)).unwrap();

    let wal = Wal::open(inner.clone(), "db").unwrap();
    let records = wal.read_all().unwrap();
    assert_eq!(
        records.len(),
        1,
        "only the live ulid stays, got {records:?}"
    );
    assert!(matches!(
        &records[0],
        WalRecord::AddTombstone { ulid, docids, .. } if ulid == "keep" && docids == &vec![1]
    ));
    let after = wal_file(&*inner, "db/wal.log");
    assert!(
        after.windows(b"keep".len()).any(|w| w == b"keep"),
        "live tombstone must still be on disk: {after:?}"
    );
    assert!(
        !after.windows(b"drop".len()).any(|w| w == b"drop"),
        "dead ulid tombstone must be gone: {after:?}"
    );
}
