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
    let col2 = db2
        .collection("c", schema(), CollectionOptions::default())
        .unwrap();
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
        // 手动写半成品段 + WAL，不切 manifest。ULID 为原始值（段目录 = seg_<ULID>）。
        let wal = Wal::open(vfs.clone(), "db").unwrap();
        wal.append(&WalRecord::AddSegment {
            collection: "c".into(),
            ulid: "HALF".into(),
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

// 06 遗留 #1：reindex 接 WAL — reindex 后 crash，tombstone 经 re-keyed AddTombstone
// 存活；旧段为孤儿被清理。
#[test]
fn reindex_crash_keeps_tombstone_and_cleans_old_segments() {
    use vane_core::tokenizer::UserDictEntry;
    let vfs = Arc::new(MemoryVfs::new());
    {
        let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
        let col = setup_col(&db);
        col.add(&docs_batch(0)).unwrap(); // d0, d1
        col.flush().unwrap();
        col.delete(&["d0".into()]).unwrap();
        // reindex：新词表触发重建。tombstone re-key 到新 ULID 并写 WAL。
        col.set_user_dict(&[UserDictEntry::WordWithFreq {
            term: "自定义词".into(),
            freq: 100,
        }])
        .unwrap();
        col.reindex().unwrap().wait().unwrap();
        // 不 close（模拟崩溃）。
    }
    let db2 = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    // reopen 时传与 reindex 后一致的 user_dict（manifest 已持久化新身份）。
    let col2 = db2
        .collection(
            "c",
            schema(),
            CollectionOptions {
                tokenizer: vane_core::tokenizer::BuiltinTokenizer::Standard,
                user_dict: vec![vane_core::tokenizer::UserDictEntry::WordWithFreq {
                    term: "自定义词".into(),
                    freq: 100,
                }],
                auto_commit: Default::default(),
            },
        )
        .unwrap();
    let hits = col2.search(&text_query()).unwrap();
    assert!(
        !hits.iter().any(|h| h.id == "d0"),
        "reindex 后 tombstone 必须存活（re-keyed AddTombstone）"
    );
    assert!(hits.iter().any(|h| h.id == "d1"));
    // 仅 1 个段（旧段孤儿清理后只剩 reindex 的新段）。
    assert_eq!(col2.segment_count(), 1);
}

fn read_all(vfs: &dyn Vfs, path: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
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

// manifest.json 在 rename 空窗里消失：已提交段还在时，重新 open 必须是 Corrupt，
// 不能当成空库把 seg_* 清掉。
#[test]
fn manifest_loss_reopen_is_corrupt_and_keeps_segments() {
    let vfs = Arc::new(MemoryVfs::new());
    {
        let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
        let col = setup_col(&db);
        col.add(&docs()).unwrap();
        col.flush().unwrap();
        let hits = col.search(&text_query()).unwrap();
        assert!(
            hits.iter().any(|h| h.id == "d0"),
            "d0 searchable before loss"
        );
        assert!(
            hits.iter().any(|h| h.id == "d1"),
            "d1 searchable before loss"
        );
    }
    vfs.delete("db/manifest.json").unwrap();
    let opened = Db::open(vfs.clone(), "db", OpenOptions::default());
    match opened {
        Err(vane_core::types::VaneError::Corrupt(ctx)) => {
            assert_eq!(ctx.op, Some("load manifest"));
            let hint = ctx.hint.unwrap_or_default();
            assert!(hint.contains("不要删"), "hint={hint}");
            assert!(hint.contains("export"), "hint={hint}");
        }
        Err(other) => panic!("expected Corrupt, got {other}"),
        Ok(_) => panic!("expected Corrupt, got Ok — committed segments would be wiped"),
    }
    let segs = vfs.list("db/segments").unwrap();
    assert!(
        segs.iter().any(|s| s.starts_with("seg_")),
        "committed seg_* must remain after refused open: {segs:?}"
    );
}

// 崩溃落在 tmp 已写完、rename 未完成：把读出的 manifest 放到 tmp 后重新 open，文档仍可搜。
#[test]
fn manifest_loss_tmp_promote_docs_remain_searchable() {
    let vfs = Arc::new(MemoryVfs::new());
    {
        let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
        let col = setup_col(&db);
        col.add(&docs()).unwrap();
        col.flush().unwrap();
        let hits = col.search(&text_query()).unwrap();
        assert!(hits.iter().any(|h| h.id == "d0"));
    }
    let bytes = read_all(vfs.as_ref(), "db/manifest.json");
    assert!(!bytes.is_empty(), "flushed manifest must be non-empty");
    vfs.delete("db/manifest.json").unwrap();
    vfs.create("db/manifest.json.tmp").unwrap();
    vfs.write_at("db/manifest.json.tmp", &bytes, 0).unwrap();
    assert!(
        vfs.read_at("db/manifest.json", &mut [0u8; 1], 0).is_err(),
        "manifest.json must stay absent until open promotes tmp"
    );
    let db2 = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col2 = db2
        .collection("c", schema(), CollectionOptions::default())
        .unwrap();
    let hits = col2.search(&text_query()).unwrap();
    assert!(
        hits.iter().any(|h| h.id == "d0"),
        "d0 must remain searchable after tmp promotion"
    );
    assert!(
        hits.iter().any(|h| h.id == "d1"),
        "d1 must remain searchable after tmp promotion"
    );
}

// 进程在 write 中途崩溃：wal.log 末尾留下没有换行的半截 JSON。
// 已提交的 tombstone 必须仍能重放，随后的 delete 也不能粘到残尾上。
#[test]
fn torn_wal_tail_reopen_keeps_tombstone_and_next_delete() {
    let vfs = Arc::new(MemoryVfs::new());
    {
        let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
        let col = setup_col(&db);
        col.add(&docs()).unwrap();
        col.flush().unwrap();
        col.delete(&["d1".into()]).unwrap();
        let hits = col.search(&text_query()).unwrap();
        assert!(
            !hits.iter().any(|h| h.id == "d1"),
            "d1 deleted before torn tail"
        );
        assert!(hits.iter().any(|h| h.id == "d0"), "d0 still searchable");
    }
    let wal = read_all(vfs.as_ref(), "db/wal.log");
    assert!(
        wal.ends_with(b"\n"),
        "committed wal must end with newline before the torn tail"
    );
    vfs.write_at("db/wal.log", b"{\"torn\"", wal.len() as u64)
        .unwrap();
    {
        let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
        let col = setup_col(&db);
        let hits = col.search(&text_query()).unwrap();
        assert!(
            !hits.iter().any(|h| h.id == "d1"),
            "torn tail must not block reopen or drop the committed tombstone"
        );
        assert!(hits.iter().any(|h| h.id == "d0"), "d0 still searchable");
        col.delete(&["d0".into()]).unwrap();
    }
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col = setup_col(&db);
    let hits = col.search(&text_query()).unwrap();
    assert!(
        !hits.iter().any(|h| h.id == "d0"),
        "d0 excluded after second reopen"
    );
    assert!(
        !hits.iter().any(|h| h.id == "d1"),
        "d1 excluded after second reopen"
    );
}

// 全库一份 wal.log。A 的 tombstone 还在 WAL 里时，B 的 compact 不能把它清掉。
// B 分两段 flush：单段且没有 tombstone 的 compact 会直接返回，根本不重写 WAL。
#[test]
fn compact_one_collection_keeps_other_collection_tombstone() {
    let vfs = Arc::new(MemoryVfs::new());
    {
        let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
        let a = db
            .collection("a", schema(), CollectionOptions::default())
            .unwrap();
        a.add(&docs()).unwrap();
        a.flush().unwrap();
        a.delete(&["d0".into()]).unwrap();
        let hits = a.search(&text_query()).unwrap();
        assert!(
            !hits.iter().any(|h| h.id == "d0"),
            "d0 hidden before the other collection compacts"
        );
        assert!(hits.iter().any(|h| h.id == "d1"), "d1 still searchable");

        let b = db
            .collection("b", schema(), CollectionOptions::default())
            .unwrap();
        b.add(&[doc("b0", "hello world")]).unwrap();
        b.flush().unwrap();
        b.add(&[doc("b1", "hello world")]).unwrap();
        b.flush().unwrap();
        assert!(
            b.segment_count() >= 2,
            "b must have two segments so compact rewrites the wal"
        );
        b.compact().unwrap();
        assert_eq!(b.segment_count(), 1, "b compact must merge");
    }
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let a = db
        .collection("a", schema(), CollectionOptions::default())
        .unwrap();
    let hits = a.search(&text_query()).unwrap();
    assert!(
        !hits.iter().any(|h| h.id == "d0"),
        "a tombstone must survive b compact, got {hits:?}"
    );
    assert!(hits.iter().any(|h| h.id == "d1"), "a live doc remains");
    let b = db
        .collection("b", schema(), CollectionOptions::default())
        .unwrap();
    let hits = b.search(&text_query()).unwrap();
    assert!(hits.iter().any(|h| h.id == "b0"), "b0 remains");
    assert!(hits.iter().any(|h| h.id == "b1"), "b1 remains");
}

// delete 与 compact 互斥：要么合并看见 tombstone，要么 delete 打在新段 ULID 上。
// 不 sleep。compact 拿不到锁时是 E_BUSY（delete 先持锁），delete 必须等到 Ok。
#[test]
fn delete_during_compact_stays_deleted_after_reopen() {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
    let col = setup_col(&db);
    col.add(&docs_batch(0)).unwrap();
    col.flush().unwrap();
    col.add(&docs_batch(2)).unwrap();
    col.flush().unwrap();
    assert!(
        col.segment_count() >= 2,
        "compact must merge at least two segments"
    );

    let col_compact = col.clone();
    let col_delete = col.clone();
    let compact_thread = std::thread::spawn(move || col_compact.compact());
    let delete_thread = std::thread::spawn(move || col_delete.delete(&["d0".into()]));
    let deleted = delete_thread
        .join()
        .expect("delete thread")
        .expect("delete waits, then Ok");
    assert_eq!(deleted, 1, "the id must be tombstoned once");
    match compact_thread.join().expect("compact thread") {
        Ok(()) => {}
        Err(vane_core::types::VaneError::Busy(_)) => {}
        Err(e) => panic!("compact failed: {e}"),
    }
    drop(col);
    drop(db);

    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col = setup_col(&db);
    let hits = col.search(&text_query()).unwrap();
    assert!(
        !hits.iter().any(|h| h.id == "d0"),
        "d0 must stay deleted across reopen, got {hits:?}"
    );
    assert!(hits.iter().any(|h| h.id == "d1"), "d1 remains");
    assert!(hits.iter().any(|h| h.id == "d2"), "d2 remains");
    assert!(hits.iter().any(|h| h.id == "d3"), "d3 remains");
}
