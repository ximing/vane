// tests/wal_crash.rs — 04-wal 集成崩溃恢复测试（Task 3/4/5b/6）。
//
// 验证 SPEC §6.4（崩溃恢复）/§7.2（tombstone 即时进 WAL）/不变量 I-6（manifest 原子性
// + WAL 一致）/B-2（flush 不 truncate，tombstone 不丢）端到端。

use std::sync::Arc;

use vane_core::api::{
    CollectionOptions, Db, Doc, FusionSpec, OpenOptions, SearchMode, SearchQuery,
};
use vane_core::types::{FieldDef, Metric, Schema};
use vane_core::vfs::memory::MemoryVfs;
use vane_core::vfs::Vfs;
use vane_core::wal::{Wal, WalRecord};

fn build_schema() -> Schema {
    Schema::new(vec![
        ("body".into(), FieldDef::Text),
        (
            "v".into(),
            FieldDef::Vector {
                dim: 4,
                metric: Metric::Cosine,
            },
        ),
    ])
    .unwrap()
}

fn doc(id: &str, text: &str) -> Doc {
    Doc {
        id: id.into(),
        text: Some(text.into()),
        vector: Some(vec![1.0, 0.0, 0.0, 0.0]),
        meta: None,
    }
}

fn docs_batch(start: usize) -> Vec<Doc> {
    (0..2)
        .map(|i| doc(&format!("d{}", start + i), "hello world"))
        .collect()
}

fn docs() -> Vec<Doc> {
    docs_batch(0)
}

fn schema() -> Schema {
    build_schema()
}

fn text_query() -> SearchQuery {
    SearchQuery {
        text: Some("hello".into()),
        vector: None,
        top_k: 100,
        mode: SearchMode::Text,
        fusion: FusionSpec::Rrf,
        filter: None,
        candidate_multiplier: 3,
    }
}

fn setup_col(db: &Db) -> vane_core::api::Collection {
    db.collection("c", schema(), CollectionOptions::default())
        .unwrap()
}

// Task 3：崩溃恢复 — tombstone 重放。flush + delete 后不 close（模拟崩溃），
// reopen 后 WAL 重放恢复 tombstone，d1 仍被排除。
#[test]
fn crash_recovery_replays_tombstone() {
    let vfs = Arc::new(MemoryVfs::new());
    {
        let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
        let col = setup_col(&db);
        col.add(&docs()).unwrap();
        col.flush().unwrap();
        col.delete(&["d1".into()]).unwrap();
        // 不调 close（模拟崩溃），WAL 未 truncate。
    }
    // 会话 2：reopen，WAL 重放恢复 tombstone。
    let db2 = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col2 = db2.collection("c", schema(), CollectionOptions::default()).unwrap();
    let hits = col2.search(&text_query()).unwrap();
    assert!(
        !hits.iter().any(|h| h.id == "d1"),
        "tombstone must be replayed"
    );
    assert!(hits.iter().any(|h| h.id == "d0"), "d0 must remain visible");
}

// Task 4：崩溃恢复 — 半成品段清理。WAL 有 AddSegment 但 ULID 不在 manifest → 孤儿段删除。
#[test]
fn crash_recovery_cleans_orphan_segment() {
    let vfs = Arc::new(MemoryVfs::new());
    {
        let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
        let col = setup_col(&db);
        col.add(&docs()).unwrap();
        col.flush().unwrap();
        // 模拟：写一个半成品段目录（不在 manifest）+ WAL 有 AddSegment。
        // ULID 为原始值（无 seg_ 前缀，段目录 = segments/seg_<ULID>）。
        let wal = Wal::open(vfs.clone(), "db").unwrap();
        wal.append(&WalRecord::AddSegment {
            collection: "c".into(),
            ulid: "ORPHAN".into(),
        })
        .unwrap();
        vfs.create("db/segments/seg_ORPHAN/header.bin").unwrap();
        vfs.write_at("db/segments/seg_ORPHAN/header.bin", b"partial", 0)
            .unwrap();
    }
    let db2 = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
    let _ = db2;
    // 孤儿段已清理：segments 下不再含 ORPHAN。
    let files = vfs.list("db/segments").unwrap();
    assert!(
        !files.iter().any(|f| f.contains("ORPHAN")),
        "orphan segment must be cleaned: {:?}",
        files
    );
}

// Task 5：flush 后 WAL 不 truncate（B-2）。
#[test]
fn flush_appends_add_segment_does_not_truncate() {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
    let col = setup_col(&db);
    col.add(&docs()).unwrap();
    col.flush().unwrap();
    let wal = Wal::open(vfs.clone(), "db").unwrap();
    let records = wal.read_all().unwrap();
    // B-2：flush 后 WAL **不** truncate，AddSegment 保留直到 compact。
    assert!(
        records
            .iter()
            .any(|r| matches!(r, WalRecord::AddSegment { .. })),
        "flush must NOT truncate WAL (B-2)"
    );
}

// Task 5：delete 后 WAL 含 AddTombstone。
#[test]
fn delete_appends_tombstone_to_wal() {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
    let col = setup_col(&db);
    col.add(&docs()).unwrap();
    col.flush().unwrap();
    col.delete(&["d1".into()]).unwrap();
    let wal = Wal::open(vfs.clone(), "db").unwrap();
    let records = wal.read_all().unwrap();
    assert!(
        records
            .iter()
            .any(|r| matches!(r, WalRecord::AddTombstone { .. })),
        "delete must append AddTombstone to WAL"
    );
}

// Task 5：compact 成功 + manifest 切换后 truncate（B-2：唯一 truncate 调用点）。
#[test]
fn compact_truncates_wal_after_manifest_switch() {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
    let col = setup_col(&db);
    col.add(&docs()).unwrap();
    col.flush().unwrap();
    col.delete(&["d1".into()]).unwrap();
    col.compact().unwrap();
    let wal = Wal::open(vfs.clone(), "db").unwrap();
    let records = wal.read_all().unwrap();
    assert!(
        records.is_empty(),
        "WAL must be truncated after compact (B-2)"
    );
}

// Task 5b：B-2 核心回归 — flush→delete→flush→崩溃 不丢 tombstone。
// 若 flush 调 truncate，此序列会丢失 AddTombstone → d0 复活（数据损坏）。
#[test]
fn crash_after_flush_delete_flush_keeps_tombstone() {
    let vfs = Arc::new(MemoryVfs::new());
    {
        let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
        let col = setup_col(&db);
        col.add(&docs_batch(0)).unwrap();
        col.flush().unwrap(); // flush1: AddSegment(seg_a)
        col.delete(&["d0".into()]).unwrap(); // AddTombstone(seg_a, d0)
        col.add(&docs_batch(1)).unwrap();
        col.flush().unwrap(); // flush2: AddSegment(seg_b)
        // 不 close（模拟崩溃）。flush 不 truncate → WAL 含
        // [AddSegment(a), AddTombstone(a,d0), AddSegment(b)]。
    }
    let db2 = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col2 = db2
        .collection("c", schema(), CollectionOptions::default())
        .unwrap();
    let hits = col2.search(&text_query()).unwrap();
    assert!(
        !hits.iter().any(|h| h.id == "d0"),
        "tombstone must survive (B-2: flush no-truncate)"
    );
    // d1（batch1）仍可见。
    assert!(hits.iter().any(|h| h.id == "d1"));
}

// Task 6：不变量 I-6 — manifest 原子性 + WAL 一致。
// 模拟 manifest rename 前崩溃：WAL 有 AddSegment，manifest 旧（无新段）。
// reopen 后：新段为孤儿被清理，manifest 旧状态完整。
#[test]
fn manifest_consistent_after_crash_mid_flush() {
    let vfs = Arc::new(MemoryVfs::new());
    {
        let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
        let col = setup_col(&db);
        col.add(&docs()).unwrap();
        // 手动写半成品段 + WAL，不切 manifest。
        let wal = Wal::open(vfs.clone(), "db").unwrap();
        wal.append(&WalRecord::AddSegment {
            collection: "c".into(),
            ulid: "seg_HALF".into(),
        })
        .unwrap();
        vfs.create("db/segments/seg_HALF/header.bin").unwrap();
    }
    let db2 = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col2 = db2
        .collection("c", schema(), CollectionOptions::default())
        .unwrap();
    // 旧状态：无文档可见（flush 未完成，seg_HALF 被清理，add 的文档在 buffer 未落盘）。
    assert_eq!(col2.segment_count(), 0);
    let hits = col2.search(&text_query()).unwrap();
    assert!(hits.is_empty(), "no docs visible after mid-flush crash");
}

// 补充：compact 后 reopen，tombstone 已物理清除（WAL truncate），搜索正常。
#[test]
fn compact_then_reopen_no_tombstone_needed() {
    let vfs = Arc::new(MemoryVfs::new());
    {
        let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
        let col = setup_col(&db);
        col.add(&docs_batch(0)).unwrap(); // d0, d1
        col.flush().unwrap();
        col.delete(&["d0".into()]).unwrap();
        col.compact().unwrap(); // 物理清除 d0，WAL truncate
    }
    let db2 = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col2 = db2
        .collection("c", schema(), CollectionOptions::default())
        .unwrap();
    let hits = col2.search(&text_query()).unwrap();
    assert_eq!(hits.len(), 1);
    assert!(hits.iter().any(|h| h.id == "d1"));
    assert!(!hits.iter().any(|h| h.id == "d0"));
}
