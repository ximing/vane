#![cfg(feature = "fault-injection")]
//! M4 阶段二 b：崩溃恢复测试（5 场景 FaultVfs 注入）
//!
//! 用 FaultVfs 在持久化关键点注入可控故障，验证崩溃后重开 recover 的数据一致性。
//! 全程 MemoryVfs（无真 fs 副作用），不真断电/真写满/真杀进程——可控可复现。
//!
//! 文件门控：FaultVfs 是 `#[cfg(any(test, feature="fault-injection"))]` 编译。
//! 集成测试（tests/）把 vane-core 当外部依赖编译，`cfg(test)` 对 lib 不生效，
//! 故须靠 `fault-injection` feature。CI `--all-features` 启用 → 跑这些测试；
//! 默认 `cargo test -p vane-core` 不启用 → 文件编译为空（0 测试，不报错）。
//!
//! 5 场景（M4-PLAN 阶段二）：
//! 1. meta_slot 翻转崩溃——save_atomic 的 sync(manifest.json.tmp) 失败
//! 2. WAL flush 崩溃——WAL append 失败，已确认事务重放、未确认不可见
//! 3. merge 中断崩溃——finalize_merge 的 write_inverted 失败，旧段保留
//! 4. ENOSPC——write_at 返 ENOSPC，不损已有数据
//! 5. 部分写——header.bin 写 8 字节后失败，flush 在返回前删掉半成品段，旧数据不丢

use std::sync::Arc;

use vane_core::api::{
    CollectionOptions, Db, Doc, FusionSpec, OpenOptions, SearchMode, SearchQuery,
};
use vane_core::persistence::AutoCommitConfig;
use vane_core::types::{FieldDef, Metric, Schema};
use vane_core::vfs::fault::{Fault, FaultVfs, VfsOp};
use vane_core::vfs::Vfs;

// ---------------------------------------------------------------------------
// 测试辅助
// ---------------------------------------------------------------------------

/// 构建含 text + vector(4d, cosine) 的 schema。
fn schema() -> Schema {
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

/// 关闭 auto-commit，防止 add() 内部自动 flush 干扰故障注入点位。
fn col_opts() -> CollectionOptions {
    CollectionOptions {
        auto_commit: AutoCommitConfig::Off,
        ..Default::default()
    }
}

/// 生成 count 个文档，id 为 d{start}..d{start+count-1}。
fn make_docs(start: usize, count: usize) -> Vec<Doc> {
    (0..count)
        .map(|i| {
            let id = format!("d{}", start + i);
            Doc {
                id,
                text: Some(format!("hello world {}", i)),
                vector: Some(vec![1.0, 0.0, 0.0, 0.0]),
                meta: None,
            }
        })
        .collect()
}

/// 文本搜索 query（搜 "hello"，topK = top_k）。
fn text_query(top_k: u32) -> SearchQuery {
    SearchQuery {
        text: Some("hello".into()),
        vector: None,
        top_k,
        mode: SearchMode::Text,
        fusion: FusionSpec::Rrf,
        filter: None,
        candidate_multiplier: 3,
    }
}

/// 判断 hits 是否包含某 external_id。
fn contains_id(hits: &[vane_core::api::Hit], id: &str) -> bool {
    hits.iter().any(|h| h.id == id)
}

// ---------------------------------------------------------------------------
// 场景 1：meta_slot 翻转崩溃
// ---------------------------------------------------------------------------

/// 在 `save_atomic` 的 `sync(manifest.json.tmp)` 注入 IoError（one_shot）。
/// save_atomic 在 sync 失败后不执行 rename → manifest 未切换，旧 manifest 完好。
/// 重开 Db 后 recover：WAL 有 AddSegment(新段) 但 manifest 不含该 ULID → 孤儿段清理。
/// 验证：第一次 flush 的数据可见，第二次 flush 的数据不可见（flush 失败，段为孤儿）。
#[test]
fn crash_1_meta_slot_switch() {
    let vfs = Arc::new(FaultVfs::wrap_memory());
    let db_path = "db";

    // ---- 会话 1：建库 + 第一批 flush 成功 ----
    {
        let db = Db::open(vfs.clone(), db_path, OpenOptions::default()).unwrap();
        let col = db.collection("c", schema(), col_opts()).unwrap();
        col.add(&make_docs(0, 3)).unwrap(); // d0, d1, d2
        col.flush().unwrap();
        assert_eq!(
            col.segment_count(),
            1,
            "first flush should produce 1 segment"
        );

        // 验证基线数据可见
        let hits = col.search(&text_query(100)).unwrap();
        assert!(
            contains_id(&hits, "d0"),
            "d0 should be visible after first flush"
        );
        assert!(
            contains_id(&hits, "d2"),
            "d2 should be visible after first flush"
        );

        // ---- 注入故障：manifest.json.tmp 的 sync 失败 ----
        vfs.inject(Fault::IoError {
            op: VfsOp::Sync,
            path_pattern: "*/manifest.json.tmp".to_string(),
            msg: "simulated sync failure on manifest.json.tmp".to_string(),
            one_shot: true,
            trigger_on_nth: 0,
        });

        // 第二批 add + flush → save_atomic 的 sync(tmp) 失败
        col.add(&make_docs(3, 3)).unwrap(); // d3, d4, d5
        let flush_err = col.flush();
        assert!(
            flush_err.is_err(),
            "second flush should fail (manifest sync fault injected)"
        );
        let err_msg = format!("{}", flush_err.unwrap_err());
        assert!(
            err_msg.contains("manifest.json.tmp"),
            "error should mention manifest.json.tmp, got: {}",
            err_msg
        );

        // manifest 未切换：segment_count 仍为 1（save_atomic 失败 → snapshot 未更新）
        assert_eq!(
            col.segment_count(),
            1,
            "segment_count should be 1 (manifest not switched)"
        );
        // 不 close（模拟崩溃）
    }

    // ---- 会话 2：重开 → recover ----
    {
        let db = Db::open(vfs.clone(), db_path, OpenOptions::default()).unwrap();
        let col = db.collection("c", schema(), col_opts()).unwrap();

        // 旧 manifest 完好：1 个段（第一批 flush 的段）
        assert_eq!(
            col.segment_count(),
            1,
            "after reopen: 1 segment (orphan cleaned by recover)"
        );

        // 第一批数据可见
        let hits = col.search(&text_query(100)).unwrap();
        assert!(
            contains_id(&hits, "d0"),
            "d0 visible after recover (confirmed data)"
        );
        assert!(contains_id(&hits, "d1"), "d1 visible after recover");
        assert!(contains_id(&hits, "d2"), "d2 visible after recover");

        // 第二批数据不可见（flush 失败，段为孤儿被清理）
        assert!(
            !contains_id(&hits, "d3"),
            "d3 should NOT be visible (flush failed, orphan cleaned)"
        );
        assert!(
            !contains_id(&hits, "d4"),
            "d4 should NOT be visible (flush failed, orphan cleaned)"
        );
        assert!(
            !contains_id(&hits, "d5"),
            "d5 should NOT be visible (flush failed, orphan cleaned)"
        );

        // 验证 manifest.json.tmp 不存在（save_atomic 的 delete(tmp) 在下次 save_atomic 时清理）
        // 或即使存在也不影响数据一致性。
    }
}

// ---------------------------------------------------------------------------
// 场景 2：WAL flush 崩溃
// ---------------------------------------------------------------------------

/// 第一批 flush 后，delete d0（WAL append 成功 = 已确认）。
/// 注入 IoError{op:Append, path:wal.log}（one_shot），再 delete d1 → WAL append 失败 = 未确认。
/// 重开 recover：WAL 有 AddTombstone(d0) 无 AddTombstone(d1) → d0 被重放删除，d1 仍可见。
#[test]
fn crash_2_wal_flush() {
    let vfs = Arc::new(FaultVfs::wrap_memory());
    let db_path = "db";

    // ---- 会话 1：建库 + flush + 两次 delete ----
    {
        let db = Db::open(vfs.clone(), db_path, OpenOptions::default()).unwrap();
        let col = db.collection("c", schema(), col_opts()).unwrap();
        col.add(&make_docs(0, 5)).unwrap(); // d0..d4
        col.flush().unwrap();

        // d0..d4 全可见
        let hits = col.search(&text_query(100)).unwrap();
        assert_eq!(hits.len(), 5, "all 5 docs visible before delete");
        assert!(contains_id(&hits, "d0") && contains_id(&hits, "d1"));

        // ---- 已确认事务：delete d0 → WAL append 成功 ----
        col.delete(&["d0".into()]).unwrap();
        // 验证 d0 在内存中已 tombstone
        let hits = col.search(&text_query(100)).unwrap();
        assert!(!contains_id(&hits, "d0"), "d0 deleted (confirmed, in WAL)");

        // ---- 注入故障：WAL append 失败 ----
        vfs.inject(Fault::IoError {
            op: VfsOp::Append,
            path_pattern: "*/wal.log".to_string(),
            msg: "simulated WAL append failure".to_string(),
            one_shot: true,
            trigger_on_nth: 0,
        });

        // ---- 未确认事务：delete d1 → WAL append 失败 ----
        let del_err = col.delete(&["d1".into()]);
        assert!(
            del_err.is_err(),
            "delete d1 should fail (WAL append fault injected)"
        );
        // d1 仍在内存中可见（位图未更新，因 WAL append 失败 → ? 传播）
        let hits = col.search(&text_query(100)).unwrap();
        assert!(
            contains_id(&hits, "d1"),
            "d1 still visible (delete failed, bitmap not updated)"
        );

        // 不 close（模拟崩溃）
    }

    // ---- 会话 2：重开 → recover ----
    {
        let db = Db::open(vfs.clone(), db_path, OpenOptions::default()).unwrap();
        let col = db.collection("c", schema(), col_opts()).unwrap();

        let hits = col.search(&text_query(100)).unwrap();

        // 已确认事务重放：d0 被删除
        assert!(
            !contains_id(&hits, "d0"),
            "d0 must be deleted (confirmed transaction replayed from WAL)"
        );
        // 未确认事务不可见：d1 仍可见
        assert!(
            contains_id(&hits, "d1"),
            "d1 must be visible (unconfirmed transaction NOT in WAL)"
        );
        // 其余文档不变
        assert!(contains_id(&hits, "d2"), "d2 visible");
        assert!(contains_id(&hits, "d3"), "d3 visible");
        assert!(contains_id(&hits, "d4"), "d4 visible");
        assert_eq!(
            hits.len(),
            4,
            "exactly 4 live docs (d0 deleted, d1..d4 alive)"
        );
    }
}

// ---------------------------------------------------------------------------
// 场景 3：merge 中断崩溃
// ---------------------------------------------------------------------------

/// 两次 flush 产生 2 段，delete d0（tombstone）。
/// 注入 IoError{op:WriteAt, path:*/segments/seg_*/inverted.bin}（one_shot）。
/// compact → finalize_merge 的 write_inverted 失败 → compact 返回 Err。
/// manifest 未切换（save_atomic 在 write_inverted 之后）→ 旧段保留。
/// 重开 recover：孤儿段（merge 新段）清理，旧段数据完整。
/// 验证：旧段数据可见 + d0 tombstone 重放 + compact 可重试成功。
#[test]
fn crash_3_merge_interrupted() {
    let vfs = Arc::new(FaultVfs::wrap_memory());
    let db_path = "db";

    // ---- 会话 1：建库 + 两次 flush（2 段）+ delete d0 ----
    {
        let db = Db::open(vfs.clone(), db_path, OpenOptions::default()).unwrap();
        let col = db.collection("c", schema(), col_opts()).unwrap();
        col.add(&make_docs(0, 3)).unwrap(); // d0, d1, d2
        col.flush().unwrap();
        col.add(&make_docs(3, 3)).unwrap(); // d3, d4, d5
        col.flush().unwrap();
        assert_eq!(col.segment_count(), 2, "two segments after two flushes");

        col.delete(&["d0".into()]).unwrap();
        let hits = col.search(&text_query(100)).unwrap();
        assert!(!contains_id(&hits, "d0"), "d0 deleted before compact");

        // ---- 注入故障：merge 新段的 inverted.bin write_at 失败 ----
        vfs.inject(Fault::IoError {
            op: VfsOp::WriteAt,
            path_pattern: "*/segments/seg_*/inverted.bin".to_string(),
            msg: "simulated write failure on inverted.bin during merge".to_string(),
            one_shot: true,
            trigger_on_nth: 0,
        });

        // ---- compact → finalize_merge 的 write_inverted 失败 ----
        let compact_err = col.compact();
        assert!(
            compact_err.is_err(),
            "compact should fail (write_inverted fault injected)"
        );
        let err_msg = format!("{}", compact_err.unwrap_err());
        assert!(
            err_msg.contains("inverted.bin"),
            "error should mention inverted.bin, got: {}",
            err_msg
        );

        // manifest 未切换：仍有 2 段（save_atomic 在 write_inverted 之后，未执行）
        assert_eq!(
            col.segment_count(),
            2,
            "segment_count still 2 (manifest not switched)"
        );
        // 不 close（模拟崩溃）
    }

    // ---- 会话 2：重开 → recover ----
    {
        let db = Db::open(vfs.clone(), db_path, OpenOptions::default()).unwrap();
        let col = db.collection("c", schema(), col_opts()).unwrap();

        // 旧段保留：2 段
        assert_eq!(
            col.segment_count(),
            2,
            "after reopen: 2 segments (merge failed, old segments retained)"
        );

        let hits = col.search(&text_query(100)).unwrap();
        // d0 tombstone 从 WAL 重放
        assert!(
            !contains_id(&hits, "d0"),
            "d0 deleted (tombstone replayed from WAL)"
        );
        // 活文档全集不变
        assert!(contains_id(&hits, "d1"), "d1 visible");
        assert!(contains_id(&hits, "d2"), "d2 visible");
        assert!(contains_id(&hits, "d3"), "d3 visible");
        assert!(contains_id(&hits, "d4"), "d4 visible");
        assert!(contains_id(&hits, "d5"), "d5 visible");
        assert_eq!(hits.len(), 5, "5 live docs (d1..d5, d0 tombstoned)");

        // ---- compact 可重试（故障已消费）----
        col.compact()
            .expect("compact retry should succeed (one_shot fault consumed)");
        assert_eq!(
            col.segment_count(),
            1,
            "after retry compact: 1 merged segment"
        );

        // 重试后活文档全集不变
        let hits = col.search(&text_query(100)).unwrap();
        assert!(
            !contains_id(&hits, "d0"),
            "d0 still deleted after retry compact"
        );
        assert_eq!(hits.len(), 5, "5 live docs after retry compact");
        assert!(contains_id(&hits, "d1") && contains_id(&hits, "d5"));
    }
}

// ---------------------------------------------------------------------------
// 场景 4：ENOSPC
// ---------------------------------------------------------------------------

/// 第一批 flush 成功（基线数据）。注入 Enospc{op:WriteAt, path:*}（one_shot）。
/// 第二批 flush → SegmentWriter::finalize 的首个 write_at 返 ENOSPC → flush 失败。
/// 验证：flush 返含 "ENOSPC" 的错误 + 已有数据（第一批段）不损 + 重开后数据一致。
#[test]
fn crash_4_enospc_graceful_degradation() {
    let vfs = Arc::new(FaultVfs::wrap_memory());
    let db_path = "db";

    // ---- 会话 1：建库 + 第一批 flush 成功 ----
    {
        let db = Db::open(vfs.clone(), db_path, OpenOptions::default()).unwrap();
        let col = db.collection("c", schema(), col_opts()).unwrap();
        col.add(&make_docs(0, 3)).unwrap(); // d0, d1, d2
        col.flush().unwrap();
        assert_eq!(col.segment_count(), 1, "first flush succeeds (baseline)");

        let hits = col.search(&text_query(100)).unwrap();
        assert_eq!(hits.len(), 3, "baseline 3 docs visible");

        // ---- 注入 ENOSPC ----
        vfs.inject(Fault::Enospc {
            op: VfsOp::WriteAt,
            path_pattern: "*".to_string(),
            one_shot: true,
            trigger_on_nth: 0,
        });

        // ---- 第二批 flush → ENOSPC ----
        col.add(&make_docs(3, 3)).unwrap(); // d3, d4, d5
        let flush_err = col.flush();
        assert!(
            flush_err.is_err(),
            "second flush should fail (ENOSPC injected)"
        );
        let err_msg = format!("{}", flush_err.unwrap_err());
        assert!(
            err_msg.contains("ENOSPC"),
            "error should contain ENOSPC, got: {}",
            err_msg
        );

        // 已有数据不损：search 不触发 WriteAt，基线数据仍可见
        let hits = col.search(&text_query(100)).unwrap();
        assert!(
            contains_id(&hits, "d0"),
            "d0 still visible (ENOSPC did not corrupt existing data)"
        );
        assert!(contains_id(&hits, "d2"), "d2 still visible");
        assert!(!contains_id(&hits, "d3"), "d3 NOT visible (flush failed)");
        // 不 close（模拟崩溃）
    }

    // ---- 会话 2：重开 → recover ----
    {
        let db = Db::open(vfs.clone(), db_path, OpenOptions::default()).unwrap();
        let col = db.collection("c", schema(), col_opts()).unwrap();

        // manifest 未切换（第二 flush 的 save_atomic 未执行）：1 段
        assert_eq!(
            col.segment_count(),
            1,
            "after reopen: 1 segment (ENOSPC flush did not reach save_atomic)"
        );

        let hits = col.search(&text_query(100)).unwrap();
        assert_eq!(hits.len(), 3, "baseline 3 docs survive ENOSPC");
        assert!(contains_id(&hits, "d0") && contains_id(&hits, "d2"));
        assert!(
            !contains_id(&hits, "d3"),
            "d3 not visible (ENOSPC flush failed)"
        );

        // 段目录中无孤儿段（ENOSPC 在 finalize 首个 write_at 即失败，段目录可能存在但被 recover 清理）
        let segs = vfs.list(&format!("{}/segments", db_path)).unwrap();
        let seg_dirs: Vec<_> = segs.iter().filter(|s| s.starts_with("seg_")).collect();
        assert_eq!(
            seg_dirs.len(),
            1,
            "exactly 1 segment dir after recover (orphan cleaned): {:?}",
            seg_dirs
        );
    }
}

// ---------------------------------------------------------------------------
// 场景 5：部分写
// ---------------------------------------------------------------------------

/// 第一批 flush 成功。注入 PartialWrite{op:WriteAt, path:*/header.bin, bytes_before_fail:8}。
/// 第二批 flush → SegmentWriter::finalize 的 header.bin write_at 写 8 字节后失败。
/// commit 前失败：FlushRollback 在 flush 返回前删掉半成品段（8 字节 header 的拒绝路径
/// 由 segment::header 单测覆盖）。manifest 不切换，旧数据仍在，新文档不可见。
#[test]
fn crash_5_partial_write() {
    let vfs = Arc::new(FaultVfs::wrap_memory());
    let db_path = "db";

    // ---- 会话 1：建库 + 第一批 flush 成功 ----
    let baseline_ulids: Vec<String>;
    {
        let db = Db::open(vfs.clone(), db_path, OpenOptions::default()).unwrap();
        let col = db.collection("c", schema(), col_opts()).unwrap();
        col.add(&make_docs(0, 3)).unwrap(); // d0, d1, d2
        col.flush().unwrap();
        assert_eq!(col.segment_count(), 1, "first flush succeeds (baseline)");
        baseline_ulids = col.segment_ulids();
        assert_eq!(baseline_ulids.len(), 1, "1 baseline ULID");

        let hits = col.search(&text_query(100)).unwrap();
        assert_eq!(hits.len(), 3, "baseline 3 docs visible");

        // ---- 注入部分写故障 ----
        vfs.inject(Fault::PartialWrite {
            op: VfsOp::WriteAt,
            path_pattern: "*/header.bin".to_string(),
            bytes_before_fail: 8,
            one_shot: true,
            trigger_on_nth: 0,
        });

        // ---- 第二批 flush → header.bin 写 8 字节后失败 ----
        col.add(&make_docs(3, 3)).unwrap(); // d3, d4, d5
        let flush_err = col.flush();
        assert!(
            flush_err.is_err(),
            "second flush should fail (partial write on header.bin)"
        );
        let err_msg = format!("{}", flush_err.unwrap_err());
        assert!(
            err_msg.contains("partial write"),
            "error should mention partial write, got: {}",
            err_msg
        );

        // manifest 未切换：segment_count 仍为 1（snapshot 未更新）
        assert_eq!(
            col.segment_count(),
            1,
            "segment_count still 1 (flush failed)"
        );

        // commit 前失败会删掉半成品段，调用方看不到 8 字节损坏 header。
        let seg_entries = vfs.list(&format!("{}/segments", db_path)).unwrap();
        let orphans: Vec<_> = seg_entries
            .iter()
            .filter(|e| e.starts_with("seg_"))
            .filter(|e| {
                let ulid = &e[4..];
                !baseline_ulids.iter().any(|u| u == ulid)
            })
            .collect();
        assert!(
            orphans.is_empty(),
            "uncommitted segment must be deleted before flush returns: {:?}",
            orphans
        );
        let hits = col.search(&text_query(100)).unwrap();
        assert!(contains_id(&hits, "d0") && contains_id(&hits, "d2"));
        assert!(!contains_id(&hits, "d3"), "d3 not visible (flush failed)");

        // 不 close（模拟崩溃）
    }

    // ---- 会话 2：重开 → recover ----
    {
        let db = Db::open(vfs.clone(), db_path, OpenOptions::default()).unwrap();
        let col = db.collection("c", schema(), col_opts()).unwrap();

        // 孤儿段被 recover 清理：只有 1 段（基线段）
        assert_eq!(
            col.segment_count(),
            1,
            "after reopen: 1 segment (corrupt orphan cleaned by recover)"
        );

        let hits = col.search(&text_query(100)).unwrap();
        // 旧数据不丢
        assert_eq!(hits.len(), 3, "baseline 3 docs survive partial write crash");
        assert!(contains_id(&hits, "d0") && contains_id(&hits, "d2"));
        // 新数据不可见（flush 失败）
        assert!(!contains_id(&hits, "d3"), "d3 not visible (flush failed)");

        // 验证损坏段目录已清理
        let segs = vfs.list(&format!("{}/segments", db_path)).unwrap();
        let seg_dirs: Vec<_> = segs.iter().filter(|s| s.starts_with("seg_")).collect();
        assert_eq!(
            seg_dirs.len(),
            1,
            "exactly 1 segment dir after recover (orphan cleaned): {:?}",
            seg_dirs
        );
    }
}

// ---------------------------------------------------------------------------
// flush 失败不得丢 buffer（commit 前放回；commit 后禁止再写一份）
// ---------------------------------------------------------------------------

/// commit 前写段失败：文档留在 buffer，下一次 flush 能写进唯一一段。
#[test]
fn flush_failure_before_manifest_keeps_docs_for_retry() {
    let vfs = Arc::new(FaultVfs::wrap_memory());
    let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
    let col = db.collection("c", schema(), col_opts()).unwrap();
    col.add(&make_docs(0, 2)).unwrap();

    vfs.inject(Fault::Enospc {
        op: VfsOp::WriteAt,
        path_pattern: "*vectors.bin".to_string(),
        one_shot: true,
        trigger_on_nth: 0,
    });

    let flush_err = col.flush();
    assert!(
        flush_err.is_err(),
        "flush should fail when vectors.bin hits ENOSPC"
    );
    let hits = col.search(&text_query(100)).unwrap();
    assert!(
        !contains_id(&hits, "d0") && !contains_id(&hits, "d1"),
        "uncommitted docs must not be searchable, got {:?}",
        hits.iter().map(|h| h.id.as_str()).collect::<Vec<_>>()
    );

    // one_shot 已消费，重试应把同一批文档写进一段。
    col.flush().unwrap();
    let hits = col.search(&text_query(100)).unwrap();
    assert!(contains_id(&hits, "d0"), "d0 searchable after retry flush");
    assert!(contains_id(&hits, "d1"), "d1 searchable after retry flush");
    assert_eq!(
        col.segment_count(),
        1,
        "retry must not leave a duplicate segment"
    );
}

/// auto-commit 的 flush 失败仍返回 Ok(AddReport)，但文档必须留在 buffer。
/// 下一篇 add 触发的成功 flush 要把两篇一起写上。
#[test]
fn auto_commit_flush_failure_keeps_docs_until_next_add() {
    let vfs = Arc::new(FaultVfs::wrap_memory());
    let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
    let opts = CollectionOptions {
        auto_commit: AutoCommitConfig::On {
            interval_ms: 60_000,
            max_docs: 1,
        },
        ..Default::default()
    };
    let col = db.collection("c", schema(), opts).unwrap();

    vfs.inject(Fault::Enospc {
        op: VfsOp::WriteAt,
        path_pattern: "*vectors.bin".to_string(),
        one_shot: true,
        trigger_on_nth: 0,
    });

    let report = col.add(&make_docs(0, 1)).unwrap();
    assert_eq!(
        report.accepted, 1,
        "add must still return Ok after auto-commit failure"
    );
    let hits = col.search(&text_query(100)).unwrap();
    assert!(
        !contains_id(&hits, "d0"),
        "d0 must not be visible when auto-commit flush failed"
    );

    // 故障已消费。第二篇 add 再次触发 flush，应带上仍在 buffer 里的 d0。
    col.add(&make_docs(1, 1)).unwrap();
    let hits = col.search(&text_query(100)).unwrap();
    assert!(
        contains_id(&hits, "d0"),
        "d0 searchable after later successful flush"
    );
    assert!(
        contains_id(&hits, "d1"),
        "d1 searchable after later successful flush"
    );
}

/// add_segment 已成功之后的读失败：文档已提交，不得放回 buffer。
/// 否则下一次 flush 会再写一段，重开后同一 id 落在两个段里（段数变成 3）。
#[test]
fn flush_failure_after_manifest_commit_does_not_duplicate() {
    let vfs = Arc::new(FaultVfs::wrap_memory());
    let db_path = "db";
    {
        let db = Db::open(vfs.clone(), db_path, OpenOptions::default()).unwrap();
        let col = db.collection("c", schema(), col_opts()).unwrap();
        col.add(&make_docs(0, 1)).unwrap();
        col.flush().unwrap();
        assert_eq!(col.segment_count(), 1, "d0 committed");

        col.add(&make_docs(1, 1)).unwrap();
        // 写入是 WriteAt。add_segment 之后 InvertedIndexReader::open 才 ReadAt inverted.bin。
        vfs.inject(Fault::IoError {
            op: VfsOp::ReadAt,
            path_pattern: "*inverted.bin".to_string(),
            msg: "simulated post-commit inverted.bin read".to_string(),
            one_shot: true,
            trigger_on_nth: 0,
        });
        let flush_err = col.flush();
        assert!(
            flush_err.is_err(),
            "flush should fail when post-commit inverted.bin read fails"
        );

        // 无故障再 flush。若 d1 被错误放回 buffer，这里会多写一段。
        col.flush().unwrap();
    }

    let db = Db::open(vfs, db_path, OpenOptions::default()).unwrap();
    let col = db.collection("c", schema(), col_opts()).unwrap();
    let hits = col.search(&text_query(100)).unwrap();
    assert!(contains_id(&hits, "d0"), "d0 visible after reopen");
    assert!(contains_id(&hits, "d1"), "d1 visible after reopen");
    assert_eq!(
        col.segment_count(),
        2,
        "reopen must see exactly d0's segment and d1's committed segment"
    );
}
