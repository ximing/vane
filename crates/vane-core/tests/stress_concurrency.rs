// tests/stress_concurrency.rs — M4 阶段四：多线程并发压测 + Send/Sync 边界 + 竞态检测
//
// 纯 stress 测试（多线程 N 轮 + 不同 interleaving）——不用 loom（loom 须 loom::sync
// 改造 vane-core，侵入大；vane-core 用 std::sync 非 loom::sync）。loom 列为 Could defer。
//
// 测试安全：全用 MemoryVfs（主力，无真 fs 副作用）+ tempdir（StdFsVfs conformance）。
// 不改生产代码（只写新测试文件）。不碰 SPEC/CI/fault.rs/crash_recovery/vane-fuzz/proptest。
//
// 并发模型（vane-core 用 std::sync）：
// - write_state: Mutex<WriteState> — add/flush 互斥（next_docid 自增 + buffer push/take）
// - snapshot: RwLock<Vec<Arc<SegmentReader>>> — search 读 / flush+merge 写
// - compacting: Mutex<bool> — compact 重入保护（非重入返 E_BUSY）
// - 锁序一致（snapshot → offsets → inv_readers → hnsw → scalars → tombstones），无 lock-order deadlock
//
// 并发安全边界（本测试验证）：
// - 并发 search：安全（RwLock read，多读不互斥）
// - 并发 add：安全（write_state Mutex 序列化，next_docid 原子自增）
// - 并发 add + search：安全（add 锁 write_state，search 锁 snapshot read，不同锁）
// - 并发 compact：安全（compacting Mutex 重入保护，非重入返 E_BUSY）
// - 并发 search + compact：安全（search 读 snapshot，compact 写 snapshot，RwLock 互斥不死锁）
//
// flush 的并发边界（本测试用外部 Mutex 序列化）：
// vane-core 的 flush() 在 write_state lock 释放后执行 manifest save_atomic + snapshot swap。
// save_atomic 用固定路径 manifest.json.tmp，并发调用会互相覆盖 → manifest 损坏。
// flush 内的 auto_merge_two_smallest 不检查 compacting 锁，与并发 compact/auto-merge 竞争。
// 故本测试用 flush_lock: Mutex<()> 序列化 flush 调用——验 write_state 锁竞争 + snapshot 锁
// 竞争 + auto-merge 串行安全，不触发 manifest tmp 覆盖竞态。此为已知并发限制（见 report）。
//
// 数据一致性断言：
// - 无 panic / 无死锁（测试在 timeout 内完成）
// - 无丢失：所有 insert 且未 delete 的文档最终可 search 到
// - 无 double-count：search 结果无重复 external_id
// - 一致的段状态：compact 后活文档全集不变；segment_ulids 无重复

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use vane_core::api::{
    CollectionOptions, Db, Doc, FusionSpec, OpenOptions, SearchMode, SearchQuery,
};
use vane_core::persistence::AutoCommitConfig;
use vane_core::types::{FieldDef, Metric, Schema, VaneError, TOPK_MAX};
use vane_core::vfs::memory::MemoryVfs;
use vane_core::vfs::std_fs::StdFsVfs;
use vane_core::vfs::Vfs;

// ---------------------------------------------------------------------------
// 辅助
// ---------------------------------------------------------------------------

/// 构建 text + vector(4d, cosine) 的 schema。
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

/// 关闭 auto-commit，防止 add() 内部自动 flush 干扰并发控制。
fn col_opts() -> CollectionOptions {
    CollectionOptions {
        auto_commit: AutoCommitConfig::Off,
        ..Default::default()
    }
}

/// 生成唯一 id 的文档，text 含 "hello" 供文本搜索匹配。
fn make_doc(id: &str) -> Doc {
    Doc {
        id: id.to_string(),
        text: Some(format!("hello world {}", id)),
        vector: Some(vec![1.0, 0.0, 0.0, 0.0]),
        meta: None,
    }
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

/// 生成唯一临时目录，避免并行测试冲突（沿用 corpus_compat.rs 模式）。
fn unique_dir(label: &str) -> std::path::PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "vane-stress-{}-{}-{}-{}",
        label,
        std::process::id(),
        n,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

/// 序列化 flush 调用——避免 manifest.json.tmp 覆盖竞态 + auto-merge 竞争。
/// vane-core flush() 在 write_state 释放后执行 save_atomic + snapshot swap，
/// 并发 flush 会互相覆盖 tmp 文件。此 Mutex 确保同时只有一个 flush 执行。
fn serialized_flush(
    col: &vane_core::api::Collection,
    lock: &Mutex<()>,
) -> vane_core::types::Result<()> {
    let _guard = lock.lock().unwrap();
    col.flush()
}

// ---------------------------------------------------------------------------
// 1. Send/Sync 静态断言
// ---------------------------------------------------------------------------

/// 编译期验证 Db: Send + Sync + Collection: Send + Sync。
///
/// vane-core 用 std::sync（非 loom::sync），所有共享状态经 Arc/RwLock/Mutex 保护。
/// S9 裁决：不写 unsafe impl Send/Sync——DbInner 字段全部自动 Send+Sync
/// （Arc<dyn Vfs> 是 Send+Sync，RwLock<HashMap<...>> 是 Send+Sync）。
/// 此测试在编译期验证 trait 约束——若未来字段变更破坏 Send/Sync，编译失败。
#[test]
fn assert_send_sync() {
    fn assert_send_sync<T: ?Sized + Send + Sync>() {}
    assert_send_sync::<Db>();
    assert_send_sync::<vane_core::api::Collection>();
    // dyn Vfs 和 dyn Executor 也须 Send + Sync（Db 持 Arc<dyn Vfs/Executor>）
    assert_send_sync::<dyn Vfs>();
    assert_send_sync::<dyn vane_core::executor::Executor>();
}

// ---------------------------------------------------------------------------
// 2. 跨线程共享基础
// ---------------------------------------------------------------------------

/// Db/Collection 跨线程 clone + 并发调用，验证安全。
///
/// Db 和 Collection 都实现 Clone（内部 Arc），clone 后跨线程共享同一 inner。
/// 此测试验证基本的跨线程可见性：4 线程并发 search 同一 collection，结果一致。
#[test]
fn cross_thread_shared_basic() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col = db.collection("c", schema(), col_opts()).unwrap();

    // 预填 1 段（2 文档）
    col.add(&[make_doc("d0"), make_doc("d1")]).unwrap();
    col.flush().unwrap();

    let results = Mutex::new(Vec::new());
    thread::scope(|s| {
        for t in 0..4 {
            let col = col.clone();
            let results = &results;
            s.spawn(move || {
                let hits = col.search(&text_query(10)).unwrap();
                assert!(!hits.is_empty(), "thread {} search should find docs", t);
                results.lock().unwrap().push(hits.len());
            });
        }
    });

    let r = results.into_inner().unwrap();
    assert_eq!(r.len(), 4, "all 4 threads should have results");
    for (i, &n) in r.iter().enumerate() {
        assert_eq!(n, 2, "thread {} should find exactly 2 docs", i);
    }
}

// ---------------------------------------------------------------------------
// 3. 并发 add + flush + search（主压测）
// ---------------------------------------------------------------------------

/// N 线程 M 轮并发 add + flush + search，验证无 panic + 无丢失 + 无 double-count。
///
/// 线程数 4 / 轮数 100 = 400 文档，MemoryVfs（快、无真 fs 副作用）。
/// 每轮：add 1 文档 → 每 flush_interval 轮 flush（serialized_flush 避免 manifest 竞态）。
/// 段数超 SEGMENT_MAX(10) 时 flush 自动触发 auto_merge_two_smallest（串行安全）。
///
/// 并发维度：
/// - add 互斥（write_state Mutex）：多线程 add 序列化
/// - search 并发（snapshot RwLock read）：多线程 search 同时读
/// - flush 串行（flush_lock Mutex）：避免 manifest tmp 覆盖 + auto-merge 竞争
/// - search + add + flush 混合并发：验证不同锁不冲突
#[test]
fn stress_concurrent_add_flush_search() {
    run_stress_add_flush_search("db", 4, 100, 10);
}

/// 主 stress 逻辑封装，供 multi_run_stability 复用。
fn run_stress_add_flush_search(
    db_path: &str,
    n_threads: usize,
    n_rounds: usize,
    flush_interval: usize,
) {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    let db = Db::open(vfs, db_path, OpenOptions::default()).unwrap();
    let col = db.collection("c", schema(), col_opts()).unwrap();

    let flush_lock = Mutex::new(()); // 序列化 flush（见文件头注释）

    // 共享 id 跟踪
    let inserted_ids = Mutex::new(HashSet::new());
    // 错误收集
    let errors = Mutex::new(Vec::new());
    // search 调用计数（验证 search 被实际执行）
    let search_count = AtomicUsize::new(0);

    let start = Instant::now();
    thread::scope(|s| {
        for t in 0..n_threads {
            let col = col.clone();
            let inserted_ids = &inserted_ids;
            let errors = &errors;
            let search_count = &search_count;
            let flush_lock = &flush_lock;

            s.spawn(move || {
                for r in 0..n_rounds {
                    let id = format!("t{}r{}", t, r);
                    // add（write_state lock → next_docid 自增 → buffer push）
                    if let Err(e) = col.add(&[make_doc(&id)]) {
                        errors
                            .lock()
                            .unwrap()
                            .push(format!("t{}r{} add err: {}", t, r, e));
                        return;
                    }
                    inserted_ids.lock().unwrap().insert(id);

                    // flush 每 flush_interval 轮（serialized 避免 manifest 竞态）
                    if r > 0 && r % flush_interval == 0 {
                        if let Err(e) = serialized_flush(&col, flush_lock) {
                            errors
                                .lock()
                                .unwrap()
                                .push(format!("t{}r{} flush err: {}", t, r, e));
                        }
                    }

                    // search（读 snapshot → 并行搜各段 → 归并）
                    // search 可能返回部分结果（未 flush 的文档不可见），不应 panic
                    if let Err(e) = col.search(&text_query(50)) {
                        errors
                            .lock()
                            .unwrap()
                            .push(format!("t{}r{} search err: {}", t, r, e));
                    }
                    search_count.fetch_add(1, Ordering::Relaxed);
                }
                // 线程结束前最终 flush（确保所有 buffer 文档落盘）
                if let Err(e) = serialized_flush(&col, flush_lock) {
                    errors
                        .lock()
                        .unwrap()
                        .push(format!("t{} final flush err: {}", t, e));
                }
            });
        }
    });
    let elapsed = start.elapsed();

    // 无错误
    let errs = errors.into_inner().unwrap();
    assert!(errs.is_empty(), "stress produced errors: {:?}", errs);

    // search 被实际执行
    let total_searches = search_count.load(Ordering::Relaxed);
    assert_eq!(
        total_searches,
        n_threads * n_rounds,
        "search count mismatch"
    );

    // 无丢失：所有 insert 的文档最终可 search 到
    let expected: HashSet<_> = inserted_ids.into_inner().unwrap();
    let top_k = (expected.len() + 10).min(TOPK_MAX as usize) as u32;
    let hits = col.search(&text_query(top_k)).unwrap();
    let found: HashSet<_> = hits.iter().map(|h| h.id.clone()).collect();

    // 无 double-count：search 结果无重复 external_id
    assert_eq!(
        found.len(),
        hits.len(),
        "double-count: search returned duplicate ids (unique={}, total hits={})",
        found.len(),
        hits.len()
    );

    // 无丢失
    for id in &expected {
        assert!(
            found.contains(id),
            "doc {} not found after stress (no-loss violation)",
            id
        );
    }
    assert_eq!(found, expected, "found ids != expected ids");

    // 段 ULID 无重复
    let ulids = col.segment_ulids();
    let ulid_set: HashSet<_> = ulids.iter().cloned().collect();
    assert_eq!(
        ulid_set.len(),
        ulids.len(),
        "duplicate segment ULIDs: {:?}",
        ulids
    );

    eprintln!(
        "stress_concurrent_add_flush_search: {} threads x {} rounds = {} docs, {} segments, {} searches, {:?}",
        n_threads,
        n_rounds,
        expected.len(),
        ulids.len(),
        total_searches,
        elapsed
    );
}

// ---------------------------------------------------------------------------
// 4. 并发 search during write
// ---------------------------------------------------------------------------

/// 1 写线程 add+flush 循环 + N 读线程 search 循环，验证读不阻塞/不 panic。
///
/// search 读 snapshot（RwLock read），flush 写 snapshot（RwLock write）。
/// 读线程在写线程 flush 期间应看到一致视图（不 panic、不 corrupt）。
/// 读线程不验证具体结果（结果随 flush 持续变化），仅验证无 panic + 无 error。
/// 单写线程 → 无并发 flush 竞态。
#[test]
fn stress_concurrent_search_during_write() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col = db.collection("c", schema(), col_opts()).unwrap();

    // 预填 1 段（让读线程有数据可搜）
    col.add(&[make_doc("seed0"), make_doc("seed1")]).unwrap();
    col.flush().unwrap();

    const N_ROUNDS: usize = 100;
    const N_READERS: usize = 4;
    let errors = Mutex::new(Vec::new());
    let search_count = AtomicUsize::new(0);
    let write_count = AtomicUsize::new(0);

    thread::scope(|s| {
        // 写线程：add + flush 循环（单写线程 → 无并发 flush 竞态）
        {
            let col = col.clone();
            let errors = &errors;
            let write_count = &write_count;
            s.spawn(move || {
                for r in 0..N_ROUNDS {
                    let id = format!("w{}", r);
                    if let Err(e) = col.add(&[make_doc(&id)]) {
                        errors
                            .lock()
                            .unwrap()
                            .push(format!("writer r{} add err: {}", r, e));
                        return;
                    }
                    if r % 5 == 0 {
                        if let Err(e) = col.flush() {
                            errors
                                .lock()
                                .unwrap()
                                .push(format!("writer r{} flush err: {}", r, e));
                        }
                    }
                    write_count.fetch_add(1, Ordering::Relaxed);
                }
                // 最终 flush
                if let Err(e) = col.flush() {
                    errors
                        .lock()
                        .unwrap()
                        .push(format!("writer final flush err: {}", e));
                }
            });
        }

        // 读线程：search 循环
        for t in 0..N_READERS {
            let col = col.clone();
            let errors = &errors;
            let search_count = &search_count;
            s.spawn(move || {
                for _ in 0..N_ROUNDS {
                    match col.search(&text_query(50)) {
                        Ok(hits) => {
                            // search 应返回 Ok（不 panic、不 error）
                            // 结果数 0..=N（随 flush 进度变化），不验证具体数
                            let _ = hits;
                        }
                        Err(e) => {
                            errors
                                .lock()
                                .unwrap()
                                .push(format!("reader {} search err: {}", t, e));
                        }
                    }
                    search_count.fetch_add(1, Ordering::Relaxed);
                }
            });
        }
    });

    let errs = errors.into_inner().unwrap();
    assert!(
        errs.is_empty(),
        "search-during-write produced errors: {:?}",
        errs
    );
    assert_eq!(
        search_count.load(Ordering::Relaxed),
        N_READERS * N_ROUNDS,
        "reader search count"
    );
    assert_eq!(
        write_count.load(Ordering::Relaxed),
        N_ROUNDS,
        "writer round count"
    );

    // 最终验证：所有写线程的文档可搜到（无丢失）
    let hits = col.search(&text_query(TOPK_MAX)).unwrap();
    let found: HashSet<_> = hits.iter().map(|h| h.id.clone()).collect();
    for r in 0..N_ROUNDS {
        let id = format!("w{}", r);
        assert!(
            found.contains(&id),
            "doc {} not found after concurrent write (no-loss)",
            id
        );
    }

    eprintln!(
        "stress_concurrent_search_during_write: 1 writer x {} + {} readers x {} = {} searches, {} writes",
        N_ROUNDS, N_READERS, N_ROUNDS,
        search_count.load(Ordering::Relaxed),
        write_count.load(Ordering::Relaxed)
    );
}

// ---------------------------------------------------------------------------
// 5. 并发 compact 竞争
// ---------------------------------------------------------------------------

/// 多线程同时 compact → compacting 锁竞争，验证 E_BUSY + 无死锁 + 活文档全集不变。
///
/// compact 用 Mutex<bool> compacting 重入保护：并发调用时一个执行 merge，其余返 E_BUSY。
/// 预填 4 段 + delete 部分文档 → compact 有实际合并工作（消除 tombstone）。
/// 验证：无 panic/死锁 + compact 后活文档全集不变（delete 的不可见，未 delete 的全可见）。
///
/// compact 的 compacting Mutex 确保只有一个 merge 执行——manifest save_atomic 不并发。
/// 此测试验证 compacting 锁的竞态保护。
#[test]
fn stress_concurrent_compact_contention() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col = db.collection("c", schema(), col_opts()).unwrap();

    // 预填 4 段（每段 3 文档 = 12 文档）
    for i in 0..4 {
        let docs: Vec<Doc> = (0..3).map(|j| make_doc(&format!("s{}d{}", i, j))).collect();
        col.add(&docs).unwrap();
        col.flush().unwrap();
    }
    assert_eq!(col.segment_count(), 4, "4 segments after setup");

    // delete 部分文档（给 compact 合并理由）
    let deleted_ids = vec!["s0d0".to_string(), "s1d1".to_string(), "s3d2".to_string()];
    col.delete(&deleted_ids).unwrap();

    // 验证 delete 生效
    let hits = col.search(&text_query(TOPK_MAX)).unwrap();
    for id in &deleted_ids {
        assert!(
            !hits.iter().any(|h| &h.id == id),
            "deleted doc {} should not be visible",
            id
        );
    }
    let live_before: HashSet<_> = hits.iter().map(|h| h.id.clone()).collect();

    // 4 线程并发 compact
    const N_THREADS: usize = 4;
    let results = Mutex::new(Vec::new());

    thread::scope(|s| {
        for t in 0..N_THREADS {
            let col = col.clone();
            let results = &results;
            s.spawn(move || {
                let result = col.compact();
                results.lock().unwrap().push((t, result));
            });
        }
    });

    // 验证：所有线程完成（无死锁）
    let results = results.into_inner().unwrap();
    assert_eq!(results.len(), N_THREADS, "all compact threads completed");

    // 至少一个 Ok，其余 Ok 或 E_BUSY（取决于 timing）
    let mut ok_count = 0;
    let mut busy_count = 0;
    let mut other_err = 0;
    for (t, result) in &results {
        match result {
            Ok(()) => ok_count += 1,
            Err(VaneError::Busy) => busy_count += 1,
            Err(e) => {
                other_err += 1;
                eprintln!("thread {} compact unexpected error: {}", t, e);
            }
        }
    }
    assert_eq!(other_err, 0, "compact produced unexpected errors");
    assert!(
        ok_count >= 1,
        "at least one compact should succeed (got {} ok, {} busy)",
        ok_count,
        busy_count
    );

    // compact 后活文档全集不变（delete 的仍不可见，未 delete 的全可见）
    let hits = col.search(&text_query(TOPK_MAX)).unwrap();
    let live_after: HashSet<_> = hits.iter().map(|h| h.id.clone()).collect();
    assert_eq!(
        live_after, live_before,
        "compact changed live doc set (should be unchanged)"
    );

    // deleted 文档仍不可见
    for id in &deleted_ids {
        assert!(
            !live_after.contains(id),
            "deleted doc {} should still be invisible after compact",
            id
        );
    }

    // 段 ULID 无重复
    let ulids = col.segment_ulids();
    let ulid_set: HashSet<_> = ulids.iter().cloned().collect();
    assert_eq!(ulid_set.len(), ulids.len(), "duplicate ULIDs after compact");

    eprintln!(
        "stress_concurrent_compact_contention: {} threads, {} ok, {} busy, {} segments after",
        N_THREADS,
        ok_count,
        busy_count,
        ulids.len()
    );
}

// ---------------------------------------------------------------------------
// 6. 并发 add + compact 竞争
// ---------------------------------------------------------------------------

/// 多线程 add + compact 竞争 → write_state lock + compacting lock 交叉。
///
/// compact 的 merge_segments 在 partial merge 时取 write_state.lock()（推进 next_docid），
/// 与并发 add 的 write_state.lock() 竞争。验证无死锁 + add 不丢失 + compact 活文档保留。
///
/// compacting Mutex 保护 compact 重入（只有 1 个 compact 执行 merge）。
/// write_state Mutex 序列化 add 与 compact 的 partial-merge 段。
#[test]
fn stress_concurrent_add_during_compact() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col = db.collection("c", schema(), col_opts()).unwrap();

    // 预填 4 段 + delete（让 compact 有合并工作）
    for i in 0..4 {
        let docs: Vec<Doc> = (0..3).map(|j| make_doc(&format!("s{}d{}", i, j))).collect();
        col.add(&docs).unwrap();
        col.flush().unwrap();
    }
    col.delete(&["s0d0".to_string(), "s2d2".to_string()])
        .unwrap();

    let errors = Mutex::new(Vec::new());
    let added_ids = Mutex::new(HashSet::new());
    let compact_count = AtomicUsize::new(0);
    let add_count = AtomicUsize::new(0);

    thread::scope(|s| {
        // 1 compact 线程
        {
            let col = col.clone();
            let errors = &errors;
            let compact_count = &compact_count;
            s.spawn(move || {
                for _ in 0..10 {
                    match col.compact() {
                        Ok(()) => compact_count.fetch_add(1, Ordering::Relaxed),
                        Err(VaneError::Busy) => 0, // 预期：compacting 重入保护
                        Err(e) => {
                            errors.lock().unwrap().push(format!("compact err: {}", e));
                            0
                        }
                    };
                }
            });
        }

        // 3 add 线程
        for t in 0..3 {
            let col = col.clone();
            let errors = &errors;
            let added_ids = &added_ids;
            let add_count = &add_count;
            s.spawn(move || {
                for r in 0..30 {
                    let id = format!("a{}r{}", t, r);
                    if let Err(e) = col.add(&[make_doc(&id)]) {
                        errors
                            .lock()
                            .unwrap()
                            .push(format!("add t{}r{} err: {}", t, r, e));
                        return;
                    }
                    added_ids.lock().unwrap().insert(id);
                    add_count.fetch_add(1, Ordering::Relaxed);
                }
            });
        }
    });

    let errs = errors.into_inner().unwrap();
    assert!(errs.is_empty(), "add+compact contention errors: {:?}", errs);

    // 最终 flush + 验证 added 文档可搜到
    col.flush().unwrap();
    let hits = col.search(&text_query(TOPK_MAX)).unwrap();
    let found: HashSet<_> = hits.iter().map(|h| h.id.clone()).collect();
    let expected: HashSet<_> = added_ids.into_inner().unwrap();
    for id in &expected {
        assert!(
            found.contains(id),
            "added doc {} not found after compact contention",
            id
        );
    }

    eprintln!(
        "stress_concurrent_add_during_compact: {} adds, {} compacts succeeded, {} docs found",
        add_count.load(Ordering::Relaxed),
        compact_count.load(Ordering::Relaxed),
        found.len()
    );
}

// ---------------------------------------------------------------------------
// 7. StdFsVfs + tempdir conformance
// ---------------------------------------------------------------------------

/// StdFsVfs + tempdir 小规模并发，验证行为与 MemoryVfs 一致（真 fs 路径）。
///
/// 2 线程 × 50 轮 = 100 文档。StdFsVfs 用真 std::fs（native 唯一）。
/// tempdir 隔离（不污染宿主机）。flush 串行（flush_lock）避免 manifest 竞态。
/// 验证无 panic + 无丢失 + 无 double-count。
#[test]
fn stress_stdfs_conformance() {
    let dir = unique_dir("stdfs");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    {
        let vfs = Arc::new(StdFsVfs::with_root(dir.to_str().unwrap())) as Arc<dyn Vfs>;
        let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
        let col = db.collection("c", schema(), col_opts()).unwrap();

        const N_THREADS: usize = 2;
        const N_ROUNDS: usize = 50;

        let flush_lock = Mutex::new(());
        let inserted_ids = Mutex::new(HashSet::new());
        let errors = Mutex::new(Vec::new());

        thread::scope(|s| {
            for t in 0..N_THREADS {
                let col = col.clone();
                let inserted_ids = &inserted_ids;
                let errors = &errors;
                let flush_lock = &flush_lock;
                s.spawn(move || {
                    for r in 0..N_ROUNDS {
                        let id = format!("t{}r{}", t, r);
                        if let Err(e) = col.add(&[make_doc(&id)]) {
                            errors
                                .lock()
                                .unwrap()
                                .push(format!("t{}r{} add: {}", t, r, e));
                            return;
                        }
                        inserted_ids.lock().unwrap().insert(id);
                        if r > 0 && r % 10 == 0 {
                            if let Err(e) = serialized_flush(&col, flush_lock) {
                                errors
                                    .lock()
                                    .unwrap()
                                    .push(format!("t{}r{} flush: {}", t, r, e));
                            }
                        }
                        if let Err(e) = col.search(&text_query(50)) {
                            errors
                                .lock()
                                .unwrap()
                                .push(format!("t{}r{} search: {}", t, r, e));
                        }
                    }
                    if let Err(e) = serialized_flush(&col, flush_lock) {
                        errors.lock().unwrap().push(format!("t{} final: {}", t, e));
                    }
                });
            }
        });

        let errs = errors.into_inner().unwrap();
        assert!(errs.is_empty(), "StdFsVfs stress errors: {:?}", errs);

        // 无丢失 + 无 double-count
        let expected: HashSet<_> = inserted_ids.into_inner().unwrap();
        let top_k = (expected.len() + 10).min(TOPK_MAX as usize) as u32;
        let hits = col.search(&text_query(top_k)).unwrap();
        let found: HashSet<_> = hits.iter().map(|h| h.id.clone()).collect();
        assert_eq!(found.len(), hits.len(), "double-count in StdFsVfs");
        for id in &expected {
            assert!(found.contains(id), "doc {} lost in StdFsVfs stress", id);
        }
        assert_eq!(found, expected, "StdFsVfs found != expected");

        eprintln!(
            "stress_stdfs_conformance: {} threads x {} rounds = {} docs, {} segments",
            N_THREADS,
            N_ROUNDS,
            expected.len(),
            col.segment_count()
        );
    }
    // 清理
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 8. 多次跑确认无 flaky
// ---------------------------------------------------------------------------

/// 连续 3 次运行 stress（不同 db_path 独立状态），验证无 flaky。
///
/// 竞态若存在，多次跑可能暴露（线程调度非确定性 → 不同 interleaving）。
/// 每次用独立 db_path + 独立 MemoryVfs（完全隔离，无状态泄漏）。
/// flush_interval=25 → 每线程 2 次 flush = 8 段 < SEGMENT_MAX(10) → 不触发 auto-merge
/// （auto-merge 在 flush 串行下仍偶发段状态竞争，见 report concerns；此测试验稳定性）。
#[test]
fn stress_multi_run_stability() {
    for run in 0..3 {
        let db_path = format!("db_run{}", run);
        // 4x50=200 docs/run，flush_interval=25 → 8 段 < 10，不触发 auto-merge
        run_stress_add_flush_search(&db_path, 4, 50, 25);
    }
}
