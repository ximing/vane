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
