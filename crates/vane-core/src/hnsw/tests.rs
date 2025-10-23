use super::*;
use crate::types::Metric;
use std::sync::Arc;

#[test]
fn hnsw_writer_builds_empty_graph() {
    let mut w = HnswWriter::new(4, Metric::Cosine, 16, 200);
    let _ = &mut w;
    let g = w.build();
    assert_eq!(g.doc_count(), 0);
}

#[test]
fn hnsw_writer_insert_single_node() {
    let mut w = HnswWriter::new(2, Metric::Cosine, 4, 8);
    w.insert(0, &[1.0, 0.0]);
    let g = w.build();
    assert_eq!(g.doc_count(), 1);
}

#[test]
fn hnsw_insert_multiple_nodes_connects_neighbors() {
    let mut w = HnswWriter::new(2, Metric::L2, 4, 16);
    // 3 个点近邻：[0,0],[1,0],[2,0]
    w.insert(0, &[0.0, 0.0]);
    w.insert(1, &[1.0, 0.0]);
    w.insert(2, &[2.0, 0.0]);
    let g = w.build();
    assert_eq!(g.doc_count(), 3);
    // entry_point 存在
    assert!(g.entry_point().is_some());
    // 节点 1 的邻居含 0 或 2
    let neighbors = g.neighbors(1);
    assert!(neighbors.contains(&0) || neighbors.contains(&2));
}

#[test]
fn hnsw_search_returns_topk_nearest() {
    let mut w = HnswWriter::new(2, Metric::L2, 8, 32);
    for i in 0..50u32 {
        w.insert(i, &[i as f32 * 0.1, 0.0]);
    }
    let g = w.build();
    let vfs = Arc::new(crate::vfs::memory::MemoryVfs::new()) as Arc<dyn crate::vfs::Vfs>;
    vfs.create("seg").unwrap();
    write_hnsw(vfs.as_ref(), "seg", &g).unwrap();
    let r = HnswReader::open(&vfs, "seg").unwrap();
    // R-hnsw-vec：向量不进 hnsw.bin，search 由调用方传 vectors slice。
    let vectors: Vec<f32> = (0..50u32).flat_map(|i| [i as f32 * 0.1, 0.0]).collect();
    let res = r.search(&[2.5, 0.0], 5, 64, None, 0, &vectors);
    assert_eq!(res.len(), 5);
    // 最近的是 i=25 (2.5,0.0)
    assert_eq!(res[0].docid, 25);
    assert!(res[0].score >= res[1].score);
}

#[test]
fn hnsw_search_with_filter_skips_excluded() {
    let mut w = HnswWriter::new(2, Metric::L2, 8, 32);
    for i in 0..20u32 {
        w.insert(i, &[i as f32, 0.0]);
    }
    let g = w.build();
    let vfs = Arc::new(crate::vfs::memory::MemoryVfs::new()) as Arc<dyn crate::vfs::Vfs>;
    vfs.create("seg").unwrap();
    write_hnsw(vfs.as_ref(), "seg", &g).unwrap();
    let r = HnswReader::open(&vfs, "seg").unwrap();
    let vectors: Vec<f32> = (0..20u32).flat_map(|i| [i as f32, 0.0]).collect();
    let mut bm = roaring::RoaringBitmap::new();
    // 只允许 docid 5,6,7（绝对 = base+local，base=0）
    bm.insert(5);
    bm.insert(6);
    bm.insert(7);
    let res = r.search(&[6.0, 0.0], 3, 64, Some(&bm), 0, &vectors);
    assert!(res.iter().all(|d| d.docid >= 5 && d.docid <= 7));
    assert_eq!(res[0].docid, 6);
}

#[test]
fn hnsw_graph_bytes_stable_after_write() {
    let mut w = HnswWriter::new(2, Metric::L2, 4, 8);
    w.insert(0, &[0.0, 0.0]);
    w.insert(1, &[1.0, 0.0]);
    let g = w.build();
    let vfs = Arc::new(crate::vfs::memory::MemoryVfs::new()) as Arc<dyn crate::vfs::Vfs>;
    vfs.create("seg").unwrap();
    write_hnsw(vfs.as_ref(), "seg", &g).unwrap();
    let mut buf1 = Vec::new();
    let mut tmp = [0u8; 4096];
    let mut off = 0;
    loop {
        let n = vfs.read_at("seg/hnsw.bin", &mut tmp, off).unwrap();
        if n == 0 {
            break;
        }
        buf1.extend_from_slice(&tmp[..n]);
        off += n as u64;
    }
    // 再读一次，字节一致（不可变）
    let mut buf2 = Vec::new();
    let mut off = 0;
    loop {
        let n = vfs.read_at("seg/hnsw.bin", &mut tmp, off).unwrap();
        if n == 0 {
            break;
        }
        buf2.extend_from_slice(&tmp[..n]);
        off += n as u64;
    }
    assert_eq!(buf1, buf2);
}

// ---- 额外覆盖 ----

#[test]
fn hnsw_open_missing_file_returns_err() {
    // Q-5：缺失 hnsw.bin → Err，api 层 catch 后 fallback brute
    let vfs = Arc::new(crate::vfs::memory::MemoryVfs::new()) as Arc<dyn crate::vfs::Vfs>;
    assert!(HnswReader::open(&vfs, "nope").is_err());
}

#[test]
fn hnsw_search_cosine_metric() {
    let mut w = HnswWriter::new(3, Metric::Cosine, 8, 32);
    let mut vectors: Vec<f32> = Vec::new();
    for i in 0..40u32 {
        let v = [(i as f32).sin(), (i as f32).cos(), (i as f32 * 0.1).sin()];
        w.insert(i, &v);
        vectors.extend_from_slice(&v);
    }
    let g = w.build();
    let vfs = Arc::new(crate::vfs::memory::MemoryVfs::new()) as Arc<dyn crate::vfs::Vfs>;
    vfs.create("seg").unwrap();
    write_hnsw(vfs.as_ref(), "seg", &g).unwrap();
    let r = HnswReader::open(&vfs, "seg").unwrap();
    let q = [0.0_f32, 1.0, 0.0];
    let res = r.search(&q, 5, 64, None, 0, &vectors);
    assert_eq!(res.len(), 5);
    // 结果降序
    for w in res.windows(2) {
        assert!(w[0].score >= w[1].score);
    }
}

#[test]
fn hnsw_search_docid_base_offset() {
    let mut w = HnswWriter::new(1, Metric::L2, 4, 8);
    let mut vectors: Vec<f32> = Vec::new();
    for i in 0..10u32 {
        w.insert(i, &[i as f32]);
        vectors.push(i as f32);
    }
    let g = w.build();
    let vfs = Arc::new(crate::vfs::memory::MemoryVfs::new()) as Arc<dyn crate::vfs::Vfs>;
    vfs.create("seg").unwrap();
    write_hnsw(vfs.as_ref(), "seg", &g).unwrap();
    let r = HnswReader::open(&vfs, "seg").unwrap();
    // docid_base=100：结果 docid = local + 100
    let res = r.search(&[5.0], 3, 32, None, 100, &vectors);
    assert_eq!(res.len(), 3);
    assert!(res.iter().all(|d| d.docid >= 100 && d.docid <= 109));
    assert_eq!(res[0].docid, 105); // local 5
}

#[test]
fn hnsw_recall_vs_brute_small_scale() {
    // 小规模 recall 验证：HNSW vs 暴力，recall@10 ≥ 0.95
    use crate::vector::brute_search;
    let dim = 8u32;
    let n = 300u32;
    // 确定性伪随机向量
    let vectors: Vec<f32> = (0..(n * dim))
        .map(|i| {
            let x = i.wrapping_mul(2654435761);
            ((x >> 8) as f32) / (u32::MAX as f32) * 2.0 - 1.0
        })
        .collect();
    let mut w = HnswWriter::new(dim, Metric::L2, 16, 200);
    for i in 0..n {
        let s = (i * dim) as usize;
        let e = s + dim as usize;
        w.insert(i, &vectors[s..e]);
    }
    let g = w.build();
    let vfs = Arc::new(crate::vfs::memory::MemoryVfs::new()) as Arc<dyn crate::vfs::Vfs>;
    vfs.create("seg").unwrap();
    write_hnsw(vfs.as_ref(), "seg", &g).unwrap();
    let r = HnswReader::open(&vfs, "seg").unwrap();

    let mut recall_sum = 0.0_f32;
    let queries = 20u32;
    for qi in 0..queries {
        let s = (qi * dim) as usize;
        let e = s + dim as usize;
        let q = &vectors[s..e];
        let brute = brute_search(&vectors, dim, q, Metric::L2, 10, None, 0);
        let hnsw = r.search(q, 10, 200, None, 0, &vectors);
        let brute_set: std::collections::HashSet<u64> = brute.iter().map(|d| d.docid).collect();
        let hits = hnsw.iter().filter(|d| brute_set.contains(&d.docid)).count();
        recall_sum += hits as f32 / 10.0;
    }
    let recall = recall_sum / queries as f32;
    assert!(recall >= 0.95, "recall {} < 0.95", recall);
}

#[test]
fn hnsw_bin_is_graph_only_no_embedded_vectors() {
    // R-hnsw-vec 修复验证：hnsw.bin 不含向量（graph-only）。
    // 断言 hnsw.bin 大小 == 头(33) + 每节点 graph-only 记录（无 dim*4 向量尾部）。
    let dim = 4u32;
    let mut w = HnswWriter::new(dim, Metric::L2, 4, 8);
    for i in 0..10u32 {
        let v: Vec<f32> = (0..dim).map(|j| (i * j) as f32 * 0.1).collect();
        w.insert(i, &v);
    }
    let g = w.build();
    let vfs = Arc::new(crate::vfs::memory::MemoryVfs::new()) as Arc<dyn crate::vfs::Vfs>;
    vfs.create("seg").unwrap();
    write_hnsw(vfs.as_ref(), "seg", &g).unwrap();
    // 读 hnsw.bin 全字节
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let mut off = 0u64;
    loop {
        let n = vfs.read_at("seg/hnsw.bin", &mut tmp, off).unwrap();
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        off += n as u64;
    }
    // 计算预期 graph-only 大小：头 33 + 每节点 { local_docid(4) + level(1) + 各层 num_neighbors(4)+neighbors }
    let mut expected = 33usize;
    for n in &g.nodes {
        expected += 4 + 1;
        for layer in &n.neighbors {
            expected += 4 + layer.len() * 4;
        }
    }
    assert_eq!(
        buf.len(),
        expected,
        "hnsw.bin size {} != graph-only expected {} (vectors would add {} bytes)",
        buf.len(),
        expected,
        g.nodes.len() * dim as usize * 4
    );
}
