// tests/tombstone_merge.rs — 02-tombstone-merge 集成测试（Task 1/5/6/7）
//
// 验证 SPEC §7.2（delete tombstone）/§7.3（段合并 + compact）/§3.3（段数上限 10）/
// 不变量 I-3（图不原地删，重建仅段合并）端到端。

use std::sync::Arc;

use vane_core::api::{
    CollectionOptions, Db, Doc, FusionSpec, OpenOptions, SearchMode, SearchQuery,
};
use vane_core::types::{FieldDef, Metric, Schema};
use vane_core::vfs::memory::MemoryVfs;

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

fn docs_batch(n: usize) -> Vec<Doc> {
    (0..n)
        .map(|i| doc(&format!("d{}", i), "hello world"))
        .collect()
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
    db.collection("c", build_schema(), CollectionOptions::default())
        .unwrap()
}

// Task 1: delete 追加 tombstone（内存 + 查询过滤）
#[test]
fn delete_hides_doc_from_search() {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col = setup_col(&db);
    col.add(&[doc("d1", "hello"), doc("d2", "hello")]).unwrap();
    col.flush().unwrap();
    let hits = col.search(&text_query()).unwrap();
    assert_eq!(hits.len(), 2);
    let n = col.delete(&["d1".into()]).unwrap();
    assert_eq!(n, 1);
    let hits2 = col.search(&text_query()).unwrap();
    assert_eq!(hits2.len(), 1);
    assert_eq!(hits2[0].id, "d2");
}

// Task 1: delete 在 vector 模式也过滤
#[test]
fn delete_hides_doc_from_vector_search() {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col = setup_col(&db);
    col.add(&[doc("d1", "hello"), doc("d2", "hello")]).unwrap();
    col.flush().unwrap();
    col.delete(&["d1".into()]).unwrap();
    let q = SearchQuery {
        text: None,
        vector: Some(vec![1.0, 0.0, 0.0, 0.0]),
        top_k: 100,
        mode: SearchMode::Vector,
        fusion: FusionSpec::Rrf,
        filter: None,
        candidate_multiplier: 3,
    };
    let hits = col.search(&q).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "d2");
}

// Task 1: delete 不存在的 id 返回 0
#[test]
fn delete_unknown_id_returns_zero() {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col = setup_col(&db);
    col.add(&[doc("d1", "hello")]).unwrap();
    col.flush().unwrap();
    let n = col.delete(&["nope".into()]).unwrap();
    assert_eq!(n, 0);
}

// Task 5: compact 合并全部段并删除旧段
#[test]
fn compact_merges_all_segments_and_removes_old() {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col = setup_col(&db);
    for _ in 0..3 {
        col.add(&docs_batch(2)).unwrap();
        col.flush().unwrap();
    }
    assert_eq!(col.segment_count(), 3);
    col.compact().unwrap();
    assert_eq!(col.segment_count(), 1);
    // 合并后搜索仍命中。
    let hits = col.search(&text_query()).unwrap();
    assert_eq!(hits.len(), 6);
}

// Task 5: compact 物理清除 tombstone
#[test]
fn compact_physically_clears_tombstone() {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col = setup_col(&db);
    col.add(&docs_batch(4)).unwrap();
    col.flush().unwrap();
    col.delete(&["d0".into(), "d1".into()]).unwrap();
    assert_eq!(col.search(&text_query()).unwrap().len(), 2);
    col.compact().unwrap();
    assert_eq!(col.segment_count(), 1);
    // compact 后仍只剩 2 条（tombstone 物理清除）。
    assert_eq!(col.search(&text_query()).unwrap().len(), 2);
    // 再次 delete 新 id 仍可工作（docid 重映射后连续）。
    let hits = col.search(&text_query()).unwrap();
    let first_id = hits[0].id.clone();
    let n = col.delete(&[first_id]).unwrap();
    assert_eq!(n, 1);
    assert_eq!(col.search(&text_query()).unwrap().len(), 1);
}

// 回归（02-review B-2）：partial auto-merge 的 target_docid_base 碰撞。
// 构造大段（base=0, 5 docs）+ 10 个小段（各 1 doc）。auto_merge 选最小两段
// （小段）合并，保留 base=0 大段。缺陷下新段 target_docid_base=0 → 与大段
// [0,5) docid 重叠 → search 回填误命中大段、fusion 去重丢 2 条文档。
// 修复后新段 base = max(保留段 base+count)，docid 不重叠 → 15 条全唯一。
#[test]
fn partial_auto_merge_does_not_overlap_docid_with_retained_segments() {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col = setup_col(&db);
    // 大段：5 docs，base=0（doc_count 大，不会被选入"最小两段"）。
    col.add(&docs_batch(5)).unwrap();
    col.flush().unwrap();
    // 10 个小段：各 1 doc，使段数超 SEGMENT_MAX(10) 触发 auto_merge。
    for i in 0..10 {
        col.add(&[doc(&format!("s{}", i), "hello world")]).unwrap();
        col.flush().unwrap();
    }
    assert!(col.segment_count() <= 10);
    // 搜索全部 "hello"：5（大段）+ 10（小段）= 15 条不重复文档。
    // 缺陷下新段 [0,2) 与大段 [0,5) 重叠 → fusion 去重后 unique 仅 13。
    let hits = col.search(&text_query()).unwrap();
    let unique: std::collections::HashSet<&String> = hits.iter().map(|h| &h.id).collect();
    assert_eq!(
        unique.len(),
        15,
        "all 15 docs must be distinct after partial auto-merge; got {:?}",
        unique
    );
}

// Task 6: 段数超 10 自动合并
#[test]
fn flush_auto_merges_when_exceeding_segment_max() {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col = setup_col(&db);
    for _ in 0..11 {
        col.add(&docs_batch(1)).unwrap();
        col.flush().unwrap();
    }
    // SEGMENT_MAX=10，第 11 次 flush 后自动合并两小段 → ≤10。
    assert!(col.segment_count() <= 10);
}

// Task 7: I-3 不变量——delete 不改 hnsw.bin，compact 重建
#[test]
fn graph_rebuilt_only_during_merge() {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
    let col = setup_col(&db);
    col.add(&docs_batch(3)).unwrap();
    col.flush().unwrap();
    let ulids = col.segment_ulids();
    let hnsw_path = format!("db/segments/seg_{}/hnsw.bin", ulids[0]);
    let size_before = file_size(&vfs, &hnsw_path);
    col.delete(&["d0".into()]).unwrap();
    let size_after = file_size(&vfs, &hnsw_path);
    assert_eq!(
        size_before, size_after,
        "hnsw.bin must not change on delete (I-3)"
    );
    // compact 后旧段目录被删，新段有 hnsw.bin。
    col.compact().unwrap();
    let new_ulids = col.segment_ulids();
    assert_eq!(new_ulids.len(), 1);
    let new_hnsw = format!("db/segments/seg_{}/hnsw.bin", new_ulids[0]);
    assert!(
        file_size(&vfs, &new_hnsw).unwrap_or(0) > 0,
        "new segment has hnsw.bin"
    );
    // 旧段 hnsw.bin 已删除（文件不存在）。
    assert!(file_size(&vfs, &hnsw_path).is_none());
}

// Task 2 跳过说明：tombstone 持久化经 WAL 由 04 实装；02 reopen 后 tombstone 丢失（预期）。
#[test]
fn tombstone_not_persisted_without_wal() {
    // 02 范围：tombstone 仅内存，reopen 后丢失（04 WAL 补持久化）。
    let vfs = Arc::new(MemoryVfs::new());
    {
        let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
        let col = setup_col(&db);
        col.add(&docs_batch(2)).unwrap();
        col.flush().unwrap();
        col.delete(&["d0".into()]).unwrap();
        assert_eq!(col.search(&text_query()).unwrap().len(), 1);
    }
    let db2 = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col2 = setup_col(&db2);
    // reopen 后 tombstone 丢失（无 WAL）：d0 复活（02 预期行为，04 修复）。
    assert_eq!(col2.search(&text_query()).unwrap().len(), 2);
}

fn file_size(vfs: &Arc<MemoryVfs>, path: &str) -> Option<usize> {
    use vane_core::vfs::Vfs;
    let mut buf = [0u8; 1];
    // 探测文件是否存在：read_at 返回 Err(Io) 即不存在。
    let mut probe = [0u8; 8192];
    match vfs.read_at(path, &mut probe, 0) {
        Ok(0) => Some(0),
        Ok(n) => {
            // 读全部计大小。
            let mut total = n;
            let mut off = n as u64;
            while let Ok(m) = vfs.read_at(path, &mut buf, off) {
                if m == 0 {
                    break;
                }
                total += m;
                off += m as u64;
            }
            Some(total)
        }
        Err(_) => None,
    }
}
