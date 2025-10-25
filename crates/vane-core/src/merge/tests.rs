// 02-tombstone-merge 单元测试（Task 3/4）

use super::*;
use crate::bm25::{write_inverted, InvertedIndexBuilder};
use crate::segment::{SegmentReader, SegmentWriter};
use crate::tokenizer::{build_tokenizer, BuiltinTokenizer, Token};
use crate::types::{FieldDef, Metric, Schema};
use crate::vfs::memory::MemoryVfs;
use std::sync::Arc;

fn test_schema() -> Schema {
    Schema::new(vec![
        ("body".into(), FieldDef::Text),
        (
            "v".into(),
            FieldDef::Vector {
                dim: 2,
                metric: Metric::Cosine,
            },
        ),
    ])
    .unwrap()
}

fn test_tokenizer() -> Arc<dyn Tokenizer> {
    Arc::<dyn Tokenizer>::from(build_tokenizer(BuiltinTokenizer::Standard, &[]).unwrap())
}

fn mk_tok(text: &str) -> Token {
    Token {
        text: text.to_string(),
        position: 0,
    }
}

fn schema_tiny_id() -> crate::types::TokenizerId {
    use crate::tokenizer::compute_tokenizer_id;
    compute_tokenizer_id(BuiltinTokenizer::Standard, &[])
}

/// 写一个段：每文档 body="hello rust"，可选 header tombstone。
/// 返回 SegmentMeta（含 ulid）。
fn write_segment(
    vfs: &Arc<dyn Vfs>,
    doc_count: u32,
    base: u64,
    tombstone_local: &[u32],
) -> crate::types::Result<SegmentMeta> {
    let schema = test_schema();
    let tid = schema_tiny_id();
    let mut writer = SegmentWriter::new(vfs.clone(), "db/segments", &schema, &tid, base)?;
    let mut inv = InvertedIndexBuilder::new(doc_count as usize);
    for i in 0..doc_count {
        let abs = base + i as u64;
        writer.add_doc(&format!("d{}", abs), Some(&[abs as f32, 0.0]), "{}")?;
        writer.set_text("hello rust")?;
        inv.add_document(abs, &[mk_tok("hello"), mk_tok("rust")], 2);
    }
    let mut meta = writer.finalize()?;
    let seg_dir = format!("db/segments/seg_{}", meta.ulid);
    write_inverted(vfs.as_ref(), &seg_dir, &inv.build())?;
    // 注入 tombstone 到 header.bin（仅测试 fixture；生产 delete 走内存位图）。
    if !tombstone_local.is_empty() {
        let mut tombs = roaring::RoaringBitmap::new();
        for &l in tombstone_local {
            tombs.insert(l);
        }
        meta.tombstones = tombs;
        let hpath = format!("{}/header.bin", seg_dir);
        let hbytes = crate::segment::header::encode_header(&meta)?;
        let _ = vfs.delete(&hpath);
        vfs.create(&hpath)?;
        vfs.write_at(&hpath, &hbytes, 0)?;
        vfs.sync(&hpath)?;
    }
    // 写 hnsw.bin（保证图重建路径可测）。
    let (_, dim, metric) = schema.vector_field().unwrap();
    let mut hw = crate::hnsw::HnswWriter::new(dim, metric, 16, 200);
    for i in 0..doc_count {
        hw.insert(i, &[(base as f32 + i as f32), 0.0]);
    }
    crate::hnsw::write_hnsw(vfs.as_ref(), &seg_dir, &hw.build())?;
    Ok(meta)
}

#[test]
fn merge_single_segment_drops_tombstoned_docs() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    let meta = write_segment(&vfs, 5, 0, &[1, 3]).unwrap();
    let tok = test_tokenizer();
    let mut task = MergeTask::new(
        vec![meta.ulid.clone()],
        0,
        schema_tiny_id(),
        test_schema(),
        tok,
    );
    let ctx = MergeContext {
        vfs: &vfs,
        db_path: "db",
        segments_dir: "db/segments",
    };
    while !task.step(&ctx).unwrap() {}
    let new_meta = finalize_merge(task, &ctx).unwrap();
    let new_dir = format!("db/segments/seg_{}", new_meta.ulid);
    let reader = SegmentReader::open(&vfs, &new_dir).unwrap();
    assert_eq!(reader.doc_count(), 3, "5 docs - 2 tombstoned = 3");
    assert!(
        new_meta.tombstones.is_empty(),
        "tombstone physically cleared"
    );
    // 原文复用（B-1/00）。
    assert!(reader.text(0).is_some());
    assert_eq!(reader.text(0).unwrap(), "hello rust");
    // 倒排 posting remap：搜 "hello" 命中 3 条。
    let inv_reader = InvertedIndexReader::open(&vfs, &new_dir).unwrap();
    let hits = inv_reader.search(&[mk_tok("hello")], 10, None);
    assert_eq!(hits.len(), 3);
    // hnsw.bin 存在（图重建，I-3）。
    assert!(crate::hnsw::HnswReader::open(&vfs, &new_dir).is_ok());
}

#[test]
fn merge_multi_segments_remaps_docid_contiguous() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    let meta_a = write_segment(&vfs, 3, 0, &[]).unwrap();
    let meta_b = write_segment(&vfs, 3, 100, &[]).unwrap();
    let tok = test_tokenizer();
    let mut task = MergeTask::new(
        vec![meta_a.ulid, meta_b.ulid],
        0,
        schema_tiny_id(),
        test_schema(),
        tok,
    );
    let ctx = MergeContext {
        vfs: &vfs,
        db_path: "db",
        segments_dir: "db/segments",
    };
    while !task.step(&ctx).unwrap() {}
    let new_meta = finalize_merge(task, &ctx).unwrap();
    let new_dir = format!("db/segments/seg_{}", new_meta.ulid);
    let reader = SegmentReader::open(&vfs, &new_dir).unwrap();
    assert_eq!(reader.doc_count(), 6);
    assert_eq!(reader.meta().docid_base, 0);
    let inv_reader = InvertedIndexReader::open(&vfs, &new_dir).unwrap();
    let hits = inv_reader.search(&[mk_tok("hello")], 10, None);
    assert_eq!(hits.len(), 6);
    // 新 docid 连续 0..6。
    let mut docids: Vec<u64> = hits.iter().map(|h| h.docid).collect();
    docids.sort();
    assert_eq!(docids, vec![0, 1, 2, 3, 4, 5]);
}

#[test]
fn merge_progress_and_completion() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    let mut ulids = Vec::new();
    for base in [0u64, 10, 20] {
        let m = write_segment(&vfs, 2, base, &[]).unwrap();
        ulids.push(m.ulid);
    }
    let tok = test_tokenizer();
    let mut task = MergeTask::new(ulids, 0, schema_tiny_id(), test_schema(), tok);
    let ctx = MergeContext {
        vfs: &vfs,
        db_path: "db",
        segments_dir: "db/segments",
    };
    assert!((task.progress() - 0.0).abs() < 1e-6);
    assert!(!task.step(&ctx).unwrap());
    assert!((task.progress() - (1.0 / 3.0)).abs() < 1e-6);
    assert!(!task.step(&ctx).unwrap());
    assert!((task.progress() - (2.0 / 3.0)).abs() < 1e-6);
    assert!(task.step(&ctx).unwrap());
    assert!((task.progress() - 1.0).abs() < 1e-6);
    let new_meta = finalize_merge(task, &ctx).unwrap();
    assert_eq!(new_meta.doc_count, 6);
}

#[test]
fn pick_merge_candidates_prefers_small_and_tombstoned() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    // big: doc_count=100, small: doc_count=2, tombstoned: doc_count=50
    let big = write_segment(&vfs, 100, 0, &[]).unwrap();
    let small = write_segment(&vfs, 2, 200, &[]).unwrap();
    let tombstoned = write_segment(&vfs, 50, 300, &[]).unwrap();
    let segments: Vec<Arc<SegmentReader>> = [big, small, tombstoned]
        .iter()
        .map(|m| {
            let seg_dir = format!("db/segments/seg_{}", m.ulid);
            Arc::new(SegmentReader::open(&vfs, &seg_dir).unwrap())
        })
        .collect();
    let ratios = vec![
        (segments[0].meta().ulid.clone(), 0.0f32),
        (segments[1].meta().ulid.clone(), 0.0),
        (segments[2].meta().ulid.clone(), 0.5),
    ];
    let picked = pick_merge_candidates(&segments, &ratios);
    // tombstoned (ratio 0.5) first, then small (doc_count=2), then big.
    assert_eq!(picked[0], segments[2].meta().ulid);
    assert_eq!(picked[1], segments[1].meta().ulid);
    assert_eq!(picked[2], segments[0].meta().ulid);
}

#[test]
fn merge_with_injected_memory_tombstones() {
    // 验证 set_tombstones 注入的内存 tombstone 与 header.bin tombstone 取并集。
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    let meta = write_segment(&vfs, 4, 0, &[0]).unwrap(); // header tombstone: local 0
    let tok = test_tokenizer();
    let mut task = MergeTask::new(
        vec![meta.ulid.clone()],
        0,
        schema_tiny_id(),
        test_schema(),
        tok,
    );
    // 内存 tombstone: local 2（绝对 docid 2）。
    let mut mem_tombs: HashMap<String, roaring::RoaringBitmap> = HashMap::new();
    let mut bm = roaring::RoaringBitmap::new();
    bm.insert(2u32);
    mem_tombs.insert(meta.ulid.clone(), bm);
    task.set_tombstones(mem_tombs);
    let ctx = MergeContext {
        vfs: &vfs,
        db_path: "db",
        segments_dir: "db/segments",
    };
    while !task.step(&ctx).unwrap() {}
    let new_meta = finalize_merge(task, &ctx).unwrap();
    let new_dir = format!("db/segments/seg_{}", new_meta.ulid);
    let reader = SegmentReader::open(&vfs, &new_dir).unwrap();
    // 4 docs - 2 tombstoned (local 0 + local 2) = 2。
    assert_eq!(reader.doc_count(), 2);
}
