//! 01-hnsw Task 5 集成测试：api 层 HNSW 搜索 + Q-5 缺失 hnsw.bin 回退。
use std::sync::Arc;
use vane_core::api::*;
use vane_core::types::*;
use vane_core::vfs::memory::MemoryVfs;
use vane_core::vfs::Vfs;

#[test]
fn api_hnsw_vector_search_returns_results() {
    // api 层 HNSW 搜索冒烟：flush 写 hnsw.bin（graph-only）+ vectors.bin，
    // search 由 api 传 SegmentReader.vectors() 给 HnswReader 导航，返回 topK。
    // 真实 recall@10 五档回归由 12-recall-regression 负责。
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
    let schema = Schema::new(vec![(
        "v".into(),
        FieldDef::Vector {
            dim: 8,
            metric: Metric::Cosine,
        },
    )])
    .unwrap();
    let col = db
        .collection("c", schema, CollectionOptions::default())
        .unwrap();
    // 500 文档，确定性向量
    let docs: Vec<Doc> = (0..500)
        .map(|i| Doc {
            id: format!("d{}", i),
            text: None,
            vector: Some((0..8).map(|j| (i * j) as f32 * 0.01).collect()),
            meta: None,
        })
        .collect();
    col.add(&docs).unwrap();
    col.flush().unwrap();
    // HNSW 搜索
    let q = vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8];
    let hnsw_hits = col
        .search(&SearchQuery {
            vector: Some(q.clone()),
            top_k: 10,
            mode: SearchMode::Vector,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(hnsw_hits.len(), 10);
    // 冒烟：d0 的向量恰好是 q 本身（i=0 → 全零向量除外；i=1 → 0.01*[0..8]），
    // 这里只断言 HNSW 路返回 topK 且 score 降序，不替 12 做 recall 断言。
    for w in hnsw_hits.windows(2) {
        assert!(w[0].score >= w[1].score);
    }
}

#[test]
fn m0_corpus_without_hnsw_bin_falls_back_to_brute() {
    // Q-5：M0 corpus（无 hnsw.bin）被 M1 打开后，HnswReader::open 返回 Err，
    // api 层 catch → fallback brute_search，搜索仍正常返回。
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
    let schema = Schema::new(vec![(
        "v".into(),
        FieldDef::Vector {
            dim: 4,
            metric: Metric::Cosine,
        },
    )])
    .unwrap();
    let col = db
        .collection("c", schema.clone(), CollectionOptions::default())
        .unwrap();
    col.add(&[Doc {
        id: "d0".into(),
        text: None,
        vector: Some(vec![1.0, 0.0, 0.0, 0.0]),
        meta: None,
    }])
    .unwrap();
    col.flush().unwrap();
    // 模拟 M0 corpus：删除刚写入的 hnsw.bin（若 flush 已写）
    // （M1 flush 写 hnsw.bin；测试手动删该文件模拟 M0 段）
    let seg_ulid = col.segment_ulids()[0].clone();
    let _ = vfs.delete(&format!("db/segments/seg_{}/hnsw.bin", seg_ulid));
    // reopen 后 HnswReader::open 缺失文件 → fallback brute
    // （重新 open Db 让 restore_from_manifest 走 HnswReader::open 缺失路径）
    let db2 = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
    let col2 = db2
        .collection("c", schema, CollectionOptions::default())
        .unwrap();
    let hits = col2
        .search(&SearchQuery {
            vector: Some(vec![1.0, 0.0, 0.0, 0.0]),
            top_k: 10,
            mode: SearchMode::Vector,
            ..Default::default()
        })
        .unwrap();
    assert!(hits.iter().any(|h| h.id == "d0"));
}

#[test]
fn api_hnsw_multi_segment_merge_serial() {
    // 多段串行搜索归并：两次 flush 产生两段，search 归并全局 topK。
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let schema = Schema::new(vec![(
        "v".into(),
        FieldDef::Vector {
            dim: 2,
            metric: Metric::L2,
        },
    )])
    .unwrap();
    let col = db
        .collection("c", schema, CollectionOptions::default())
        .unwrap();
    // 第一段：docid 0..5，向量 [i, 0]
    col.add(
        &(0..5)
            .map(|i| Doc {
                id: format!("a{}", i),
                text: None,
                vector: Some(vec![i as f32, 0.0]),
                meta: None,
            })
            .collect::<Vec<_>>(),
    )
    .unwrap();
    col.flush().unwrap();
    // 第二段：docid 5..10，向量 [i+10, 0]
    col.add(
        &(0..5)
            .map(|i| Doc {
                id: format!("b{}", i),
                text: None,
                vector: Some(vec![i as f32 + 10.0, 0.0]),
                meta: None,
            })
            .collect::<Vec<_>>(),
    )
    .unwrap();
    col.flush().unwrap();
    assert_eq!(col.segment_ulids().len(), 2);
    // query=[0,0]：最近的是 a0 (0,0)，其次 a1 (1,0)
    let hits = col
        .search(&SearchQuery {
            vector: Some(vec![0.0, 0.0]),
            top_k: 3,
            mode: SearchMode::Vector,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(hits.len(), 3);
    assert_eq!(hits[0].id, "a0");
    assert_eq!(hits[1].id, "a1");
}
