use super::header::{decode_header, encode_header};
use super::ulid::gen_ulid;
use super::*;
use crate::types::{FieldDef, Metric, Schema, TokenizerId};
use crate::vfs::memory::MemoryVfs;
use crate::vfs::Vfs;

#[test]
fn ulid_is_26_chars_and_monotonic() {
    let a = gen_ulid();
    let b = gen_ulid();
    assert_eq!(a.len(), 26);
    assert_eq!(b.len(), 26);
    // 单调递增（时间前缀）
    assert!(b >= a, "ulid should be monotonic: {} vs {}", a, b);
}

#[test]
fn header_roundtrip() {
    let meta = SegmentMeta {
        ulid: gen_ulid(),
        doc_count: 100,
        docid_base: 0,
        tokenizer_id: TokenizerId([0xab; 32]),
        tombstones: roaring::RoaringBitmap::new(),
    };
    let bytes = encode_header(&meta).unwrap();
    // magic + version 开头（FA2：全字段统一 LE）
    assert_eq!(&bytes[0..4], b"VANE");
    assert_eq!(&bytes[4..8], &[1, 0, 0, 0]);
    let decoded = decode_header(&bytes).unwrap();
    assert_eq!(decoded.ulid, meta.ulid);
    assert_eq!(decoded.doc_count, 100);
    assert_eq!(decoded.docid_base, 0);
    assert_eq!(decoded.tokenizer_id, meta.tokenizer_id);
}

#[test]
fn segment_writer_roundtrip_with_memory_vfs() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
    let schema = Schema::new(vec![
        ("title".into(), FieldDef::Text),
        (
            "vec".into(),
            FieldDef::Vector {
                dim: 4,
                metric: Metric::Cosine,
            },
        ),
    ])
    .unwrap();
    let tok_id = TokenizerId([0x11; 32]);

    let mut writer = SegmentWriter::new(vfs.clone(), "segments", &schema, &tok_id, 0).unwrap();
    let d0 = writer
        .add_doc("doc-0", Some(&[1.0, 0.0, 0.0, 0.0]), r#"{"title":"hello"}"#)
        .unwrap();
    let d1 = writer
        .add_doc("doc-1", Some(&[0.0, 1.0, 0.0, 0.0]), r#"{"title":"world"}"#)
        .unwrap();
    assert_eq!(d0, 0);
    assert_eq!(d1, 1);
    let meta = writer.finalize().unwrap();
    assert_eq!(meta.doc_count, 2);
    assert_eq!(meta.docid_base, 0);
    assert_eq!(meta.tokenizer_id, tok_id);

    // 段不可变：finalize 消费 self，编译期保证不可再调 add_doc

    // 段目录存在
    let seg_dir = format!("segments/seg_{}", meta.ulid);
    let files = vfs.list(&seg_dir).unwrap();
    assert!(files.contains(&"header.bin".to_string()));
    assert!(files.contains(&"vectors.bin".to_string()));
    assert!(files.contains(&"stored.bin".to_string()));
}

#[test]
fn segment_reader_roundtrip() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
    let schema = Schema::new(vec![(
        "v".into(),
        FieldDef::Vector {
            dim: 4,
            metric: Metric::Cosine,
        },
    )])
    .unwrap();
    let tok_id = TokenizerId([0x22; 32]);

    let mut writer = SegmentWriter::new(vfs.clone(), "segments", &schema, &tok_id, 0).unwrap();
    writer
        .add_doc("alpha", Some(&[1.0, 2.0, 3.0, 4.0]), r#"{"x":1}"#)
        .unwrap();
    writer
        .add_doc("beta", Some(&[5.0, 6.0, 7.0, 8.0]), r#"{"x":2}"#)
        .unwrap();
    let meta = writer.finalize().unwrap();

    let seg_dir = format!("segments/seg_{}", meta.ulid);
    let reader = SegmentReader::open(&vfs, &seg_dir).unwrap();

    assert_eq!(reader.meta().doc_count, 2);
    assert_eq!(reader.dim(), 4);
    assert_eq!(reader.vectors().len(), 8);
    assert_eq!(&reader.vectors()[0..4], &[1.0, 2.0, 3.0, 4.0]);
    assert_eq!(reader.external_id(0), Some("alpha"));
    assert_eq!(reader.external_id(1), Some("beta"));
    assert_eq!(reader.external_id(999), None);
    // stored.bin 回填：stored_json(local_docid) 返回写入时的 JSON
    assert_eq!(reader.stored_json(0), Some(r#"{"x":1}"#));
    assert_eq!(reader.stored_json(1), Some(r#"{"x":2}"#));
    assert_eq!(reader.stored_json(999), None);
    assert_eq!(reader.segment_dir(), seg_dir);
}

#[test]
fn segment_reader_rejects_bad_magic() {
    use crate::types::VaneError;
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
    let seg_dir = "segments/seg_bad";
    vfs.create(&format!("{}/header.bin", seg_dir)).unwrap();
    vfs.write_at(&format!("{}/header.bin", seg_dir), b"XXXX", 0)
        .unwrap();
    let r = SegmentReader::open(&vfs, seg_dir);
    assert!(matches!(r, Err(VaneError::Corrupt(_))));
}

#[test]
fn segment_immutable_after_finalize() {
    // finalize 消费 self → 编译期保证不可再调 add_doc。
    // 此测试验证 finalize 后段文件不被修改。
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
    let schema = Schema::new(vec![(
        "v".into(),
        FieldDef::Vector {
            dim: 2,
            metric: Metric::Dot,
        },
    )])
    .unwrap();
    let mut w = SegmentWriter::new(vfs.clone(), "seg", &schema, &TokenizerId([0; 32]), 0).unwrap();
    w.add_doc("a", Some(&[1.0, 0.0]), "{}").unwrap();
    let meta = w.finalize().unwrap();
    // 读回段，验证内容不变
    let seg_dir = format!("seg/seg_{}", meta.ulid);
    let r1 = SegmentReader::open(&vfs, &seg_dir).unwrap();
    assert_eq!(r1.doc_count(), 1);
    let r2 = SegmentReader::open(&vfs, &seg_dir).unwrap();
    assert_eq!(r1.vectors(), r2.vectors()); // 两次读一致
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn segment_stdfs_roundtrip() {
    use crate::vfs::std_fs::StdFsVfs;
    let dir = std::env::temp_dir().join(format!("vane-seg-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let vfs =
        std::sync::Arc::new(StdFsVfs::with_root(dir.to_str().unwrap())) as std::sync::Arc<dyn Vfs>;
    let schema = Schema::new(vec![(
        "v".into(),
        FieldDef::Vector {
            dim: 3,
            metric: Metric::Cosine,
        },
    )])
    .unwrap();
    let mut w = SegmentWriter::new(
        vfs.clone(),
        "segments",
        &schema,
        &TokenizerId([0xff; 32]),
        0,
    )
    .unwrap();
    w.add_doc("x", Some(&[0.1, 0.2, 0.3]), r#"{"k":"v"}"#)
        .unwrap();
    let meta = w.finalize().unwrap();
    let seg_dir = format!("segments/seg_{}", meta.ulid);
    let r = SegmentReader::open(&vfs, &seg_dir).unwrap();
    assert_eq!(r.external_id(0), Some("x"));
    assert_eq!(r.dim(), 3);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn segment_writer_docid_base_nonzero() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
    let schema = Schema::new(vec![(
        "v".into(),
        FieldDef::Vector {
            dim: 2,
            metric: Metric::Cosine,
        },
    )])
    .unwrap();
    let tok_id = TokenizerId([0x33; 32]);
    // 第一段 base=0
    let mut w1 = SegmentWriter::new(vfs.clone(), "seg", &schema, &tok_id, 0).unwrap();
    w1.add_doc("a", Some(&[1.0, 0.0]), "{}").unwrap();
    w1.add_doc("b", Some(&[0.0, 1.0]), "{}").unwrap();
    let m1 = w1.finalize().unwrap();
    assert_eq!(m1.docid_base, 0);
    assert_eq!(m1.doc_count, 2);
    // 第二段 base=2（接续）
    let mut w2 = SegmentWriter::new(vfs.clone(), "seg", &schema, &tok_id, 2).unwrap();
    // FF2：add_doc 返回段内局部 docid（从 0 起），全局 docid = docid_base + 返回值。
    // base=2 时首 doc 应返回 0（局部），而非全局 2。
    let local_id = w2.add_doc("c", Some(&[1.0, 1.0]), "{}").unwrap();
    assert_eq!(
        local_id, 0,
        "add_doc 应返回局部 docid（0 起），而非全局 docid"
    );
    let global_id = m1.docid_base + m1.doc_count as u64 + local_id; // = 0 + 2 + 0 = 2
    assert_eq!(global_id, 2, "全局 docid = base + local");
    let m2 = w2.finalize().unwrap();
    assert_eq!(m2.docid_base, 2);
    assert_eq!(m2.doc_count, 1);
    // 读回验证
    let seg1_dir = format!("seg/seg_{}", m1.ulid);
    let r1 = SegmentReader::open(&vfs, &seg1_dir).unwrap();
    assert_eq!(r1.meta().docid_base, 0);
    let seg2_dir = format!("seg/seg_{}", m2.ulid);
    let r2 = SegmentReader::open(&vfs, &seg2_dir).unwrap();
    assert_eq!(r2.meta().docid_base, 2);
}

#[test]
fn vectors_bin_has_magic_version_header() {
    // FA1（SPEC §6.2）：vectors.bin 必须以 4 字节 magic + 4 字节 format_version(LE) 开头，
    // 随后才是 f32 LE payload。SegmentReader.open 跳过 8 字节头，vectors() 返回纯 f32。
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
    let schema = Schema::new(vec![(
        "v".into(),
        FieldDef::Vector {
            dim: 4,
            metric: Metric::Cosine,
        },
    )])
    .unwrap();
    let mut writer =
        SegmentWriter::new(vfs.clone(), "seg", &schema, &TokenizerId([0x55; 32]), 0).unwrap();
    writer
        .add_doc("a", Some(&[1.0, 2.0, 3.0, 4.0]), "{}")
        .unwrap();
    let meta = writer.finalize().unwrap();
    let seg_dir = format!("seg/seg_{}", meta.ulid);

    // 读原始 vectors.bin 字节，校验头
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    let mut off = 0u64;
    loop {
        let n = vfs
            .read_at(&format!("{}/vectors.bin", seg_dir), &mut tmp, off)
            .unwrap();
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        off += n as u64;
    }
    assert_eq!(&buf[0..4], b"VANE");
    assert_eq!(&buf[4..8], &[1, 0, 0, 0]); // LE
    assert_eq!(buf.len(), 8 + 4 * 4); // 头 + 1 文档 × 4 维

    // reader 跳过头，vectors() 返回纯 f32
    let r = SegmentReader::open(&vfs, &seg_dir).unwrap();
    assert_eq!(r.vectors().len(), 4);
    assert_eq!(r.vectors(), &[1.0, 2.0, 3.0, 4.0]);
}

#[test]
fn vectors_bin_empty_segment_still_writes_header() {
    // FA1：doc_count=0 时 vectors.bin 仍写 8 字节头（空段合规）。
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
    let schema = Schema::new(vec![(
        "v".into(),
        FieldDef::Vector {
            dim: 2,
            metric: Metric::Cosine,
        },
    )])
    .unwrap();
    let writer =
        SegmentWriter::new(vfs.clone(), "seg", &schema, &TokenizerId([0x66; 32]), 0).unwrap();
    let meta = writer.finalize().unwrap();
    let seg_dir = format!("seg/seg_{}", meta.ulid);

    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    let mut off = 0u64;
    loop {
        let n = vfs
            .read_at(&format!("{}/vectors.bin", seg_dir), &mut tmp, off)
            .unwrap();
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        off += n as u64;
    }
    assert_eq!(buf.len(), 8);
    assert_eq!(&buf[0..4], b"VANE");
    assert_eq!(&buf[4..8], &[1, 0, 0, 0]);
    // reader 读回 doc_count=0，vectors 为空
    let r = SegmentReader::open(&vfs, &seg_dir).unwrap();
    assert_eq!(r.doc_count(), 0);
    assert!(r.vectors().is_empty());
}

#[test]
fn stored_text_roundtrip() {
    // SPEC §6.2：stored.bin 含原文 + JSON meta。set_text 在 add_doc 之后、finalize 之前
    // 调用，为最近一次 add_doc 的文档设置原文；未调 set_text 的文档 text_len=0（空串）。
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
    let schema = Schema::new(vec![(
        "v".into(),
        FieldDef::Vector {
            dim: 2,
            metric: Metric::Cosine,
        },
    )])
    .unwrap();
    let tid = TokenizerId([0u8; 32]);
    let mut w = SegmentWriter::new(vfs.clone(), "db/segments", &schema, &tid, 0).unwrap();
    let _local0 = w.add_doc("d0", Some(&[1.0, 0.0]), "{}").unwrap();
    w.set_text("机器学习检索").unwrap();
    let _local1 = w.add_doc("d1", Some(&[0.0, 1.0]), "{}").unwrap();
    // d1 不调 set_text → text_len=0
    let meta = w.finalize().unwrap();
    let seg_dir = format!("db/segments/seg_{}", meta.ulid);
    let r = SegmentReader::open(&vfs, &seg_dir).unwrap();
    assert_eq!(r.text(0), Some("机器学习检索"));
    assert_eq!(r.text(1), Some("")); // 未调 set_text → 空串（text_len=0）
    assert_eq!(r.text(999), None);
    // meta JSON 语义不变
    assert_eq!(r.stored_json(0), Some("{}"));
}

#[test]
fn set_text_before_add_doc_errors() {
    // set_text 在 add_doc 之前调用应报错（无最近文档可绑定）。
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
    let schema = Schema::new(vec![(
        "v".into(),
        FieldDef::Vector {
            dim: 2,
            metric: Metric::Cosine,
        },
    )])
    .unwrap();
    let mut w =
        SegmentWriter::new(vfs, "db/segments", &schema, &TokenizerId([0u8; 32]), 0).unwrap();
    let err = w.set_text("nope").unwrap_err();
    assert!(matches!(err, crate::types::VaneError::Schema(_)));
}

#[test]
fn segment_writer_vector_none_fills_zeros() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
    let schema = Schema::new(vec![(
        "v".into(),
        FieldDef::Vector {
            dim: 3,
            metric: Metric::Cosine,
        },
    )])
    .unwrap();
    let tok_id = TokenizerId([0x44; 32]);
    let mut w = SegmentWriter::new(vfs.clone(), "seg", &schema, &tok_id, 0).unwrap();
    // doc0 有 vector
    w.add_doc("a", Some(&[1.0, 2.0, 3.0]), "{}").unwrap();
    // doc1 无 vector → 填零向量
    w.add_doc("b", None, "{}").unwrap();
    let meta = w.finalize().unwrap();
    let seg_dir = format!("seg/seg_{}", meta.ulid);
    let r = SegmentReader::open(&vfs, &seg_dir).unwrap();
    // doc0 的向量
    assert_eq!(&r.vectors()[0..3], &[1.0, 2.0, 3.0]);
    // doc1 的向量 = 零向量
    assert_eq!(&r.vectors()[3..6], &[0.0, 0.0, 0.0]);
}

// =============================================================================
// M2-07 冷启动懒加载（SPEC v1.2 §13.1）
// =============================================================================

/// 构造一段完整的段（v1 vectors.bin，M0/M1 产物格式）供懒加载测试复用。
fn build_v1_segment(dim: u32, docs: &[(&str, &[f32])]) -> (Arc<dyn Vfs>, String) {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    let schema = Schema::new(vec![(
        "v".into(),
        FieldDef::Vector {
            dim,
            metric: Metric::Cosine,
        },
    )])
    .unwrap();
    let mut w =
        SegmentWriter::new(vfs.clone(), "seg", &schema, &TokenizerId([0x77; 32]), 0).unwrap();
    for (eid, vec) in docs {
        w.add_doc(eid, Some(vec), "{}").unwrap();
    }
    let meta = w.finalize().unwrap();
    (vfs, format!("seg/seg_{}", meta.ulid))
}

/// 手工构造 vectors.bin v2 stub（12 字节头 `magic|version=2|dim(4 LE)|payload`）。
/// M2-08 将在 finalize 落实 v2 写入；本模块只读，故用 stub 验证读路径。
/// 同时写 header.bin / idmap.bin / stored.bin 以满足 SegmentReader::open。
fn build_v2_stub_segment(dim: u32, doc_count: u32) -> (Arc<dyn Vfs>, String) {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    let ulid = ulid::gen_ulid();
    let seg_dir = format!("seg/seg_{}", ulid);

    // header.bin：复用 SegmentWriter 产出的 header 编码（doc_count 可控）。
    let meta = SegmentMeta {
        ulid: ulid.clone(),
        doc_count,
        docid_base: 0,
        tokenizer_id: TokenizerId([0x88; 32]),
        tombstones: roaring::RoaringBitmap::new(),
    };
    let hpath = format!("{}/header.bin", seg_dir);
    vfs.create(&hpath).unwrap();
    vfs.write_at(&hpath, &header::encode_header(&meta).unwrap(), 0)
        .unwrap();

    // vectors.bin v2 stub：magic(4) | version=2(4 LE) | dim(4 LE) | payload(doc_count*dim f32 LE)
    let vpath = format!("{}/vectors.bin", seg_dir);
    vfs.create(&vpath).unwrap();
    let mut vbytes = Vec::with_capacity(12 + (doc_count as usize) * (dim as usize) * 4);
    vbytes.extend_from_slice(crate::types::MAGIC);
    vbytes.extend_from_slice(&2u32.to_le_bytes()); // version=2（VECTORS_FORMAT_V2 由 M2-08 落实）
    vbytes.extend_from_slice(&dim.to_le_bytes());
    for i in 0..(doc_count as usize) * (dim as usize) {
        vbytes.extend_from_slice(&(i as f32).to_le_bytes());
    }
    vfs.write_at(&vpath, &vbytes, 0).unwrap();

    // idmap.bin：magic|version|count=0|（空，doc_count 个空 entry 也可，此处简化为 0）
    let ipath = format!("{}/idmap.bin", seg_dir);
    vfs.create(&ipath).unwrap();
    let mut ibytes = Vec::new();
    ibytes.extend_from_slice(crate::types::MAGIC);
    ibytes.extend_from_slice(&crate::types::FORMAT_VERSION.to_le_bytes());
    ibytes.extend_from_slice(&0u32.to_le_bytes());
    vfs.write_at(&ipath, &ibytes, 0).unwrap();

    // stored.bin：magic|version|count=0
    let spath = format!("{}/stored.bin", seg_dir);
    vfs.create(&spath).unwrap();
    let mut sbytes = Vec::new();
    sbytes.extend_from_slice(crate::types::MAGIC);
    sbytes.extend_from_slice(&crate::types::FORMAT_VERSION.to_le_bytes());
    sbytes.extend_from_slice(&0u32.to_le_bytes());
    vfs.write_at(&spath, &sbytes, 0).unwrap();

    (vfs, seg_dir)
}

/// 测试 1：open 不加载 vectors（v2 stub）。
/// open 后 vectors OnceLock 未初始化（get() 返回 None）。
#[test]
fn m2_07_open_does_not_load_vectors() {
    let (vfs, seg_dir) = build_v2_stub_segment(384, 10);
    let r = SegmentReader::open(&vfs, &seg_dir).unwrap();
    // open 后 vectors OnceLock 未初始化
    assert!(
        r.vectors.get().is_none(),
        "vectors OnceLock should be uninit after open"
    );
    // dim() 读 v2 头，不触发 vectors 加载
    assert_eq!(r.dim(), 384);
    assert!(
        r.vectors.get().is_none(),
        "v2 dim() should not load vectors payload"
    );
}

/// 测试 3+5：首次 vectors() 触发加载；多次调用幂等返回同一 &[f32]。
#[test]
fn m2_07_vectors_lazy_load_and_idempotent() {
    let (vfs, seg_dir) = build_v1_segment(4, &[("a", &[1.0, 2.0, 3.0, 4.0])]);
    let r = SegmentReader::open(&vfs, &seg_dir).unwrap();
    // open 后未加载
    assert!(r.vectors.get().is_none());
    // 首次调用触发加载
    let v1 = r.vectors();
    assert_eq!(v1.len(), 4);
    assert_eq!(v1, &[1.0, 2.0, 3.0, 4.0]);
    assert!(r.vectors.get().is_some());
    // 幂等：多次调用返回同一指针
    let v2 = r.vectors();
    assert_eq!(v1.as_ptr(), v2.as_ptr());
}

/// 测试 6：vectors() 并发安全——多线程同时首次调用，只加载一次。
#[test]
fn m2_07_vectors_concurrent_load_once() {
    let (vfs, seg_dir) = build_v1_segment(4, &[("a", &[1.0, 2.0, 3.0, 4.0])]);
    let r = Arc::new(SegmentReader::open(&vfs, &seg_dir).unwrap());
    let r2 = r.clone();
    std::thread::scope(|s| {
        let h1 = s.spawn(|| r.vectors().as_ptr() as usize);
        let h2 = s.spawn(|| r2.vectors().as_ptr() as usize);
        let p1 = h1.join().unwrap();
        let p2 = h2.join().unwrap();
        assert_eq!(
            p1, p2,
            "concurrent first-call must return same backing slice"
        );
    });
}

/// 测试 7：stored 懒加载——open 后 stored OnceLock 未初始化；首次 stored_json/text 触发加载。
#[test]
fn m2_07_stored_lazy_load() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    let schema = Schema::new(vec![(
        "v".into(),
        FieldDef::Vector {
            dim: 2,
            metric: Metric::Cosine,
        },
    )])
    .unwrap();
    let mut w = SegmentWriter::new(vfs.clone(), "seg", &schema, &TokenizerId([0; 32]), 0).unwrap();
    w.add_doc("d0", Some(&[1.0, 0.0]), r#"{"k":"v"}"#).unwrap();
    w.set_text("原文").unwrap();
    let meta = w.finalize().unwrap();
    let seg_dir = format!("seg/seg_{}", meta.ulid);

    let r = SegmentReader::open(&vfs, &seg_dir).unwrap();
    // open 后 stored 未加载
    assert!(r.stored.get().is_none());
    // 首次 stored_json 触发加载
    assert_eq!(r.stored_json(0), Some(r#"{"k":"v"}"#));
    assert!(r.stored.get().is_some());
    // text 同一 OnceLock，已加载
    assert_eq!(r.text(0), Some("原文"));
    // 幂等
    assert_eq!(r.stored_json(0), Some(r#"{"k":"v"}"#));
}

/// 测试 8：dim 正确性 v2——vectors.bin v2 头含 dim=384，open 后 reader.dim()==384，
/// 且不触发 vectors 加载。
#[test]
fn m2_07_dim_from_v2_header() {
    let (vfs, seg_dir) = build_v2_stub_segment(128, 5);
    let r = SegmentReader::open(&vfs, &seg_dir).unwrap();
    assert_eq!(r.dim(), 128);
    assert!(r.vectors.get().is_none(), "v2 dim must not load vectors");
    // vectors() 加载 v2 payload（跳过 12 字节头）
    let v = r.vectors();
    assert_eq!(v.len(), 5 * 128);
    assert_eq!(v[0], 0.0);
    assert_eq!(v[1], 1.0);
}

/// 测试 9：dim 回退 v1——M0/M1 产物 vectors.bin v1（无 dim 字段），dim 从 payload 长度反推。
#[test]
fn m2_07_dim_v1_fallback() {
    let (vfs, seg_dir) = build_v1_segment(4, &[("a", &[1.0, 2.0, 3.0, 4.0])]);
    let r = SegmentReader::open(&vfs, &seg_dir).unwrap();
    assert_eq!(r.dim(), 4);
    // v1 dim() 走回退路径会触发 vectors 加载（v1 无 dim 字段）
    assert!(r.vectors.get().is_some());
}

/// 测试：dim() 先于 vectors() 调用（v1 回退路径触发 vectors 加载）。
/// 覆盖 merge/reindex 调用顺序（dim 在 vectors 之前）。
#[test]
fn m2_07_dim_before_vectors_v1() {
    let (vfs, seg_dir) = build_v1_segment(4, &[("a", &[1.0, 2.0, 3.0, 4.0])]);
    let r = SegmentReader::open(&vfs, &seg_dir).unwrap();
    assert!(r.vectors.get().is_none());
    assert_eq!(r.dim(), 4); // 触发 vectors 加载
    let v = r.vectors();
    assert_eq!(v, &[1.0, 2.0, 3.0, 4.0]);
}

/// 测试：vectors() 先于 dim() 调用（v1 回退路径复用已加载 vectors，不重复读）。
/// 覆盖 search 调用顺序（vectors 在 dim 之前）。
#[test]
fn m2_07_vectors_before_dim_v1() {
    let (vfs, seg_dir) = build_v1_segment(4, &[("a", &[1.0, 2.0, 3.0, 4.0])]);
    let r = SegmentReader::open(&vfs, &seg_dir).unwrap();
    let v = r.vectors();
    assert_eq!(v.len(), 4);
    assert_eq!(r.dim(), 4); // 复用已加载 vectors，无需重复读
}

/// 测试：空段（doc_count=0）dim()==0，vectors() 为空。
#[test]
fn m2_07_empty_segment_dim_zero() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    let schema = Schema::new(vec![(
        "v".into(),
        FieldDef::Vector {
            dim: 2,
            metric: Metric::Cosine,
        },
    )])
    .unwrap();
    let w = SegmentWriter::new(vfs.clone(), "seg", &schema, &TokenizerId([0; 32]), 0).unwrap();
    let meta = w.finalize().unwrap();
    let seg_dir = format!("seg/seg_{}", meta.ulid);
    let r = SegmentReader::open(&vfs, &seg_dir).unwrap();
    assert_eq!(r.dim(), 0);
    assert!(r.vectors().is_empty());
}
