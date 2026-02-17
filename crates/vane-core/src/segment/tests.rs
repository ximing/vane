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
    // M2-08：vectors.bin v2 头（version=2 + dim 字段，12 字节头）
    assert_eq!(&buf[4..8], &[2, 0, 0, 0]); // version=2 LE
    assert_eq!(&buf[8..12], &[4, 0, 0, 0]); // dim=4 LE
    assert_eq!(buf.len(), 12 + 4 * 4); // v2 头(12) + 1 文档 × 4 维

    // reader 跳过头，vectors() 返回纯 f32
    let r = SegmentReader::open(&vfs, &seg_dir).unwrap();
    assert_eq!(r.vectors().len(), 4);
    assert_eq!(r.vectors(), &[1.0, 2.0, 3.0, 4.0]);
}

#[test]
fn vectors_bin_empty_segment_still_writes_header() {
    // FA1 + M2-08：doc_count=0 时 vectors.bin 仍写 v2 头（12 字节，空段合规）。
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
    assert_eq!(buf.len(), 12); // v2 头（magic+version+dim），无 payload
    assert_eq!(&buf[0..4], b"VANE");
    assert_eq!(&buf[4..8], &[2, 0, 0, 0]); // version=2 LE
    assert_eq!(&buf[8..12], &[2, 0, 0, 0]); // dim=2 LE
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

/// 构造一段 v1 vectors.bin 段（M0/M1 产物格式，8 字节头 magic|version=1|payload）。
/// M2-08 起 finalize 写 v2，故 v1 段须手工构造（模拟旧 corpus，v1 回退路径测试用）。
/// 同时写 header.bin / idmap.bin / stored.bin（v1）以满足 SegmentReader::open。
fn build_v1_segment(dim: u32, docs: &[(&str, &[f32])]) -> (Arc<dyn Vfs>, String) {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    let ulid = ulid::gen_ulid();
    let seg_dir = format!("seg/seg_{}", ulid);
    let doc_count = docs.len() as u32;

    // header.bin
    let meta = SegmentMeta {
        ulid: ulid.clone(),
        doc_count,
        docid_base: 0,
        tokenizer_id: TokenizerId([0x77; 32]),
        tombstones: roaring::RoaringBitmap::new(),
    };
    let hpath = format!("{}/header.bin", seg_dir);
    vfs.create(&hpath).unwrap();
    vfs.write_at(&hpath, &header::encode_header(&meta).unwrap(), 0)
        .unwrap();

    // vectors.bin v1：magic(4) | version=1(4 LE) | payload(doc_count*dim f32 LE)
    let vpath = format!("{}/vectors.bin", seg_dir);
    vfs.create(&vpath).unwrap();
    let mut vbytes = Vec::with_capacity(8 + (doc_count as usize) * (dim as usize) * 4);
    vbytes.extend_from_slice(crate::types::MAGIC);
    vbytes.extend_from_slice(&crate::types::VECTORS_FORMAT_V1.to_le_bytes());
    for (_, vec) in docs {
        for f in *vec {
            vbytes.extend_from_slice(&f.to_le_bytes());
        }
    }
    vfs.write_at(&vpath, &vbytes, 0).unwrap();

    // idmap.bin v1：magic|version=1|count|{docid|len|bytes}
    let ipath = format!("{}/idmap.bin", seg_dir);
    vfs.create(&ipath).unwrap();
    let mut ibytes = Vec::new();
    ibytes.extend_from_slice(crate::types::MAGIC);
    ibytes.extend_from_slice(&crate::types::IDMAP_FORMAT_V1.to_le_bytes());
    ibytes.extend_from_slice(&doc_count.to_le_bytes());
    for (i, (eid, _)) in docs.iter().enumerate() {
        ibytes.extend_from_slice(&(i as u64).to_le_bytes());
        ibytes.extend_from_slice(&(eid.len() as u32).to_le_bytes());
        ibytes.extend_from_slice(eid.as_bytes());
    }
    vfs.write_at(&ipath, &ibytes, 0).unwrap();

    // stored.bin v1：magic|version=1|count=0（懒加载测试不读 stored 内容）
    let spath = format!("{}/stored.bin", seg_dir);
    vfs.create(&spath).unwrap();
    let mut sbytes = Vec::new();
    sbytes.extend_from_slice(crate::types::MAGIC);
    sbytes.extend_from_slice(&crate::types::STORED_FORMAT_V1.to_le_bytes());
    sbytes.extend_from_slice(&0u32.to_le_bytes());
    vfs.write_at(&spath, &sbytes, 0).unwrap();

    (vfs, seg_dir)
}

/// 构造一段 v2 段（M2-08 finalize 产物：vectors.bin v2 头含 dim + stored.bin v2 zstd）。
/// M2-07 stub 已切到真实 finalize 产物（spec 要求）。dim/doc_count 由 schema+docs 决定。
/// 向量内容为 `0.0, 1.0, 2.0, ...` 连续递增（与原 stub 一致，断言对齐）。
fn build_v2_segment(dim: u32, doc_count: u32) -> (Arc<dyn Vfs>, String) {
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
        SegmentWriter::new(vfs.clone(), "seg", &schema, &TokenizerId([0x88; 32]), 0).unwrap();
    let total = (doc_count as usize) * (dim as usize);
    let v: Vec<f32> = (0..total).map(|i| i as f32).collect();
    for (i, chunk) in v.chunks(dim as usize).enumerate() {
        w.add_doc(&format!("d{}", i), Some(chunk), "{}").unwrap();
    }
    let meta = w.finalize().unwrap();
    (vfs, format!("seg/seg_{}", meta.ulid))
}

/// 测试 1：open 不加载 vectors（v2 stub）。
/// open 后 vectors OnceLock 未初始化（get() 返回 None）。
#[test]
fn m2_07_open_does_not_load_vectors() {
    let (vfs, seg_dir) = build_v2_segment(384, 10);
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
    let (vfs, seg_dir) = build_v2_segment(128, 5);
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

// =============================================================================
// M2-07 fix round 1（I-1 + M-3）：open 期廉价头校验恢复 loud 失败
// =============================================================================

/// I-1：corrupt vectors.bin magic（header.bin 合法）→ open 返 Err（非静默空）。
#[test]
fn m2_07_open_rejects_vectors_bin_bad_magic() {
    use crate::types::VaneError;
    let (vfs, seg_dir) = build_v1_segment(4, &[("a", &[1.0, 2.0, 3.0, 4.0])]);
    // corrupt vectors.bin magic（保留 header.bin / stored.bin 合法）
    let vpath = format!("{}/vectors.bin", seg_dir);
    let mut hdr = [0u8; 8];
    let _ = vfs.read_at(&vpath, &mut hdr, 0).unwrap();
    hdr[0] = b'X';
    vfs.write_at(&vpath, &hdr, 0).unwrap();
    let r = SegmentReader::open(&vfs, &seg_dir);
    assert!(
        matches!(r, Err(VaneError::Corrupt(ref ctx)) if ctx.message.contains("vectors.bin bad magic")),
        "open should loudly reject vectors.bin bad magic, got err variant"
    );
}

/// I-1：corrupt stored.bin magic → open 返 Err。
#[test]
fn m2_07_open_rejects_stored_bin_bad_magic() {
    use crate::types::VaneError;
    let (vfs, seg_dir) = build_v1_segment(4, &[("a", &[1.0, 2.0, 3.0, 4.0])]);
    let spath = format!("{}/stored.bin", seg_dir);
    let mut hdr = [0u8; 8];
    let _ = vfs.read_at(&spath, &mut hdr, 0).unwrap();
    hdr[0] = b'X';
    vfs.write_at(&spath, &hdr, 0).unwrap();
    let r = SegmentReader::open(&vfs, &seg_dir);
    assert!(
        matches!(r, Err(VaneError::Corrupt(ref ctx)) if ctx.message.contains("stored.bin bad magic")),
        "open should loudly reject stored.bin bad magic, got err variant"
    );
}

/// I-1：vectors.bin version=99（不支持）→ open 返 Err(Version)。
#[test]
fn m2_07_open_rejects_vectors_bin_bad_version() {
    use crate::types::VaneError;
    let (vfs, seg_dir) = build_v1_segment(4, &[("a", &[1.0, 2.0, 3.0, 4.0])]);
    // 改 version 字段（offset 4..8）为 99
    let vpath = format!("{}/vectors.bin", seg_dir);
    vfs.write_at(&vpath, &99u32.to_le_bytes(), 4).unwrap();
    let r = SegmentReader::open(&vfs, &seg_dir);
    assert!(
        matches!(r, Err(VaneError::Version(_))),
        "open should loudly reject vectors.bin bad version, got err variant"
    );
}

/// I-1：v2 头截断（<12 字节）→ open 返 Err(Corrupt)。
#[test]
fn m2_07_open_rejects_truncated_v2_header() {
    use crate::types::VaneError;
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    let ulid = ulid::gen_ulid();
    let seg_dir = format!("seg/seg_{}", ulid);
    // header.bin：doc_count=1
    let meta = SegmentMeta {
        ulid: ulid.clone(),
        doc_count: 1,
        docid_base: 0,
        tokenizer_id: TokenizerId([0x88; 32]),
        tombstones: roaring::RoaringBitmap::new(),
    };
    let hpath = format!("{}/header.bin", seg_dir);
    vfs.create(&hpath).unwrap();
    vfs.write_at(&hpath, &header::encode_header(&meta).unwrap(), 0)
        .unwrap();
    // vectors.bin：v2 头但只写 8 字节（magic+version=2，缺 dim 字段）
    let vpath = format!("{}/vectors.bin", seg_dir);
    vfs.create(&vpath).unwrap();
    let mut vbytes = Vec::new();
    vbytes.extend_from_slice(crate::types::MAGIC);
    vbytes.extend_from_slice(&crate::types::VECTORS_FORMAT_V2.to_le_bytes());
    vfs.write_at(&vpath, &vbytes, 0).unwrap();
    // idmap.bin + stored.bin 合法（v1，复用 build_v1_segment 的写法）
    let ipath = format!("{}/idmap.bin", seg_dir);
    vfs.create(&ipath).unwrap();
    let mut ibytes = Vec::new();
    ibytes.extend_from_slice(crate::types::MAGIC);
    ibytes.extend_from_slice(&crate::types::IDMAP_FORMAT_V1.to_le_bytes());
    ibytes.extend_from_slice(&0u32.to_le_bytes());
    vfs.write_at(&ipath, &ibytes, 0).unwrap();
    let spath = format!("{}/stored.bin", seg_dir);
    vfs.create(&spath).unwrap();
    let mut sbytes = Vec::new();
    sbytes.extend_from_slice(crate::types::MAGIC);
    sbytes.extend_from_slice(&crate::types::STORED_FORMAT_V1.to_le_bytes());
    sbytes.extend_from_slice(&0u32.to_le_bytes());
    vfs.write_at(&spath, &sbytes, 0).unwrap();

    let r = SegmentReader::open(&vfs, &seg_dir);
    assert!(
        matches!(r, Err(VaneError::Corrupt(ref ctx)) if ctx.message.contains("v2 header truncated")),
        "open should reject truncated v2 header, got err variant"
    );
}

/// M4 诊断重构：SegmentReader::open 的 VaneError::Corrupt 含结构化 ErrorContext
/// （seg=ULID + op=open + hint）。断言结构化字段而非 String contains。
#[test]
fn m4_5c_open_error_contains_segment_context() {
    use crate::types::VaneError;
    let (vfs, seg_dir) = build_v1_segment(4, &[("a", &[1.0, 2.0, 3.0, 4.0])]);
    // 提取段 ULID（seg_dir 末段 seg_<ulid>）
    let ulid = seg_dir.rsplit('/').next().unwrap();
    let ulid = ulid.strip_prefix("seg_").unwrap_or(ulid);

    // corrupt vectors.bin magic → Corrupt error 含 seg=ULID + op=open
    let vpath = format!("{}/vectors.bin", seg_dir);
    let mut hdr = [0u8; 8];
    let _ = vfs.read_at(&vpath, &mut hdr, 0).unwrap();
    hdr[0] = b'X';
    vfs.write_at(&vpath, &hdr, 0).unwrap();
    let r = SegmentReader::open(&vfs, &seg_dir);
    match r {
        Err(VaneError::Corrupt(ctx)) => {
            assert!(
                ctx.message.contains("vectors.bin bad magic"),
                "original message preserved: {}",
                ctx.message
            );
            assert_eq!(
                ctx.seg.as_deref(),
                Some(ulid),
                "seg field must be ULID: {:?}",
                ctx.seg
            );
            assert_eq!(
                ctx.op,
                Some("open vectors.bin"),
                "op field must be set: {:?}",
                ctx.op
            );
            assert!(ctx.hint.is_some(), "hint field must be set");
        }
        other => panic!("expected Corrupt, got {:?}", other.err().map(|e| e.name())),
    }
}

/// 评审测试缺口：reindex/merge 路径首次访问前 vectors.get().is_none()、后 .is_some()。
/// 用 merge 路径验证（merge_ctx 读 reader.vectors()）——这里直接验证 SegmentReader
/// 在 reindex/merge 典型调用顺序（dim 先于 vectors）下的懒加载行为。
#[test]
fn m2_07_reindex_merge_lazy_load_path() {
    let (vfs, seg_dir) = build_v1_segment(4, &[("a", &[1.0, 2.0, 3.0, 4.0])]);
    let r = SegmentReader::open(&vfs, &seg_dir).unwrap();
    // open 后未加载（reindex/merge 在 open 后、读 vectors 前可能做其他事）
    assert!(
        r.vectors.get().is_none(),
        "vectors should be uninit after open"
    );
    assert!(r.dim.get().is_none(), "dim should be uninit after open");
    // reindex/merge 调用顺序：dim() 先于 vectors()
    assert_eq!(r.dim(), 4);
    // v1 dim() 触发 vectors 加载（v1 无 dim 字段，必须从 payload 反推）
    assert!(r.vectors.get().is_some(), "v1 dim() should load vectors");
    assert_eq!(r.vectors(), &[1.0, 2.0, 3.0, 4.0]);
    // dim 已缓存
    assert!(r.dim.get().is_some());

    // v2 路径：dim() 不触发 vectors 加载（用预存 v2_header_dim）
    let (vfs2, seg2) = build_v2_segment(64, 3);
    let r2 = SegmentReader::open(&vfs2, &seg2).unwrap();
    assert!(r2.vectors.get().is_none());
    assert_eq!(r2.dim(), 64); // 用预存 v2 dim，不加载 vectors
    assert!(r2.vectors.get().is_none(), "v2 dim() must not load vectors");
    let v = r2.vectors();
    assert_eq!(v.len(), 3 * 64);
    assert!(r2.vectors.get().is_some());
}

// =============================================================================
// M2-08 stored.bin zstd + per-file format_version（SPEC §6.2）
// =============================================================================

/// 测试 2+3（zstd-encode）：finalize 写 stored.bin v2（zstd 块），decode_stored 读回一致。
/// v2 布局：magic|version=2|raw_payload_len(4 LE)|zstd_block_len(4 LE)|zstd_block。
#[cfg(feature = "zstd-encode")]
#[test]
fn m2_08_stored_v2_zstd_roundtrip() {
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
    w.add_doc("d0", Some(&[1.0, 0.0]), r#"{"k":"v0"}"#).unwrap();
    w.set_text("原文 d0").unwrap();
    w.add_doc("d1", Some(&[0.0, 1.0]), r#"{"k":"v1"}"#).unwrap();
    let meta = w.finalize().unwrap();
    let seg_dir = format!("seg/seg_{}", meta.ulid);

    // 读 stored.bin 原始字节，校验 v2 头布局
    let buf = {
        let mut b = Vec::new();
        let mut tmp = [0u8; 8192];
        let mut off = 0u64;
        loop {
            let n = vfs
                .read_at(&format!("{}/stored.bin", seg_dir), &mut tmp, off)
                .unwrap();
            if n == 0 {
                break;
            }
            b.extend_from_slice(&tmp[..n]);
            off += n as u64;
        }
        b
    };
    assert_eq!(&buf[0..4], b"VANE");
    let sver = u32::from_le_bytes(buf[4..8].try_into().unwrap());
    assert_eq!(sver, crate::types::STORED_FORMAT_V2, "stored v2 version=2");
    let raw_len = u32::from_le_bytes(buf[8..12].try_into().unwrap()) as usize;
    let zstd_len = u32::from_le_bytes(buf[12..16].try_into().unwrap()) as usize;
    assert!(raw_len > 0, "raw_payload_len 应 > 0");
    assert!(zstd_len > 0, "zstd_block_len 应 > 0（zstd 压缩非空）");
    assert!(
        zstd_len < raw_len,
        "zstd 应压缩 stored（{} < {}）",
        zstd_len,
        raw_len
    );
    assert_eq!(
        buf.len(),
        16 + zstd_len,
        "v2 stored 总长 = 16 头 + zstd_block"
    );

    // decode_stored 读回：HashMap 内容与写入一致
    let r = SegmentReader::open(&vfs, &seg_dir).unwrap();
    assert_eq!(r.stored_json(0), Some(r#"{"k":"v0"}"#));
    assert_eq!(r.text(0), Some("原文 d0"));
    assert_eq!(r.stored_json(1), Some(r#"{"k":"v1"}"#));
    assert_eq!(r.text(1), Some("")); // 未调 set_text → 空串
}

/// 测试 4：stored v1 读兼容（M0/M1 产物 v1 裸 JSON → 新 decode_stored 读回一致）。
/// 用 build_v1_segment 产 v1 stored（手工构造 v1 段，模拟旧 corpus）。
#[test]
fn m2_08_stored_v1_read_compat() {
    // build_v1_segment 写 v1 stored（count=0）；另手工写非空 v1 stored 验证 entries 解析。
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    let ulid = ulid::gen_ulid();
    let seg_dir = format!("seg/seg_{}", ulid);
    let meta = SegmentMeta {
        ulid: ulid.clone(),
        doc_count: 1,
        docid_base: 0,
        tokenizer_id: TokenizerId([0; 32]),
        tombstones: roaring::RoaringBitmap::new(),
    };
    let hpath = format!("{}/header.bin", seg_dir);
    vfs.create(&hpath).unwrap();
    vfs.write_at(&hpath, &header::encode_header(&meta).unwrap(), 0)
        .unwrap();
    // vectors.bin v1
    let vpath = format!("{}/vectors.bin", seg_dir);
    vfs.create(&vpath).unwrap();
    let mut vb = Vec::new();
    vb.extend_from_slice(crate::types::MAGIC);
    vb.extend_from_slice(&crate::types::VECTORS_FORMAT_V1.to_le_bytes());
    vb.extend_from_slice(&1.0f32.to_le_bytes());
    vb.extend_from_slice(&0.0f32.to_le_bytes());
    vfs.write_at(&vpath, &vb, 0).unwrap();
    // idmap v1
    let ipath = format!("{}/idmap.bin", seg_dir);
    vfs.create(&ipath).unwrap();
    let mut ib = Vec::new();
    ib.extend_from_slice(crate::types::MAGIC);
    ib.extend_from_slice(&crate::types::IDMAP_FORMAT_V1.to_le_bytes());
    ib.extend_from_slice(&1u32.to_le_bytes());
    ib.extend_from_slice(&0u64.to_le_bytes());
    ib.extend_from_slice(&1u32.to_le_bytes());
    ib.push(b'd');
    vfs.write_at(&ipath, &ib, 0).unwrap();
    // stored.bin v1 裸 JSON：magic|version=1|count=1|{docid(8)|text_len(4)|text|meta_len(4)|meta}
    let spath = format!("{}/stored.bin", seg_dir);
    vfs.create(&spath).unwrap();
    let mut sb = Vec::new();
    sb.extend_from_slice(crate::types::MAGIC);
    sb.extend_from_slice(&crate::types::STORED_FORMAT_V1.to_le_bytes());
    sb.extend_from_slice(&1u32.to_le_bytes()); // count=1
    sb.extend_from_slice(&0u64.to_le_bytes()); // docid=0
    let text = "旧原文";
    sb.extend_from_slice(&(text.len() as u32).to_le_bytes());
    sb.extend_from_slice(text.as_bytes());
    let meta_json = r#"{"old":true}"#;
    sb.extend_from_slice(&(meta_json.len() as u32).to_le_bytes());
    sb.extend_from_slice(meta_json.as_bytes());
    vfs.write_at(&spath, &sb, 0).unwrap();

    let r = SegmentReader::open(&vfs, &seg_dir).unwrap();
    assert_eq!(r.text(0), Some("旧原文"));
    assert_eq!(r.stored_json(0), Some(r#"{"old":true}"#));
}

/// 测试 5：stored v1 写（无 zstd-encode）——finalize 写 v1 裸 JSON，version=1。
#[cfg(not(feature = "zstd-encode"))]
#[test]
fn m2_08_stored_v1_written_without_zstd_encode() {
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
    let meta = w.finalize().unwrap();
    let seg_dir = format!("seg/seg_{}", meta.ulid);
    let buf = {
        let mut b = Vec::new();
        let mut tmp = [0u8; 8192];
        let mut off = 0u64;
        loop {
            let n = vfs
                .read_at(&format!("{}/stored.bin", seg_dir), &mut tmp, off)
                .unwrap();
            if n == 0 {
                break;
            }
            b.extend_from_slice(&tmp[..n]);
            off += n as u64;
        }
        b
    };
    assert_eq!(&buf[0..4], b"VANE");
    let sver = u32::from_le_bytes(buf[4..8].try_into().unwrap());
    assert_eq!(sver, crate::types::STORED_FORMAT_V1, "无 zstd-encode 写 v1");
    // v1 body 直接可读：count 在 offset 8
    let count = u32::from_le_bytes(buf[8..12].try_into().unwrap());
    assert_eq!(count, 1);
}

/// 测试 11：vectors.bin v2 头含 dim——finalize 写 v2（version=2 + dim 字段），open 读 dim 正确。
#[test]
fn m2_08_vectors_v2_header_contains_dim() {
    let (vfs, seg_dir) = build_v2_segment(96, 4);
    let r = SegmentReader::open(&vfs, &seg_dir).unwrap();
    // v2 头含 dim，open 期预存，dim() 不触发 vectors 加载
    assert_eq!(r.dim(), 96);
    assert!(r.vectors.get().is_none(), "v2 dim 不触发 vectors 加载");
    assert_eq!(r.vectors().len(), 4 * 96);
}

/// 测试 12：vectors.bin v1 读兼容——M0/M1 产物 v1 vectors.bin → open 读 dim 回退 payload。
#[test]
fn m2_08_vectors_v1_read_compat_dim_fallback() {
    let (vfs, seg_dir) = build_v1_segment(8, &[("a", &[1.0; 8])]);
    let r = SegmentReader::open(&vfs, &seg_dir).unwrap();
    // v1 无 dim 字段，dim 从 payload_len/doc_count/4 反推
    assert_eq!(r.dim(), 8);
    assert_eq!(r.vectors().len(), 8);
}

/// 测试 15：zstd-encode feature 隔离——
/// zstd-encode 启用时 stored v2；禁用时 stored v1。两配置下 vectors.bin 均 v2（无 feature 门）。
#[cfg(feature = "zstd-encode")]
#[test]
fn m2_08_zstd_encode_feature_writes_v2_stored() {
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
    w.add_doc("d0", Some(&[1.0, 0.0]), "{}").unwrap();
    let meta = w.finalize().unwrap();
    let seg_dir = format!("seg/seg_{}", meta.ulid);
    let mut hdr = [0u8; 8];
    vfs.read_at(&format!("{}/stored.bin", seg_dir), &mut hdr, 0)
        .unwrap();
    let sver = u32::from_le_bytes(hdr[4..8].try_into().unwrap());
    assert_eq!(
        sver,
        crate::types::STORED_FORMAT_V2,
        "zstd-encode 写 v2 stored"
    );
}

/// I-1 fix：v1 回退路径写出的 stored.bin 可被 decode_stored v1 读回。
/// 验证 encode_stored 在无 zstd-encode（或压缩失败回退）时产 v1 布局，
/// 且 decode_stored 能正确读回 entries（文件始终可读，降级而非损坏）。
#[cfg(not(feature = "zstd-encode"))]
#[test]
fn m2_08_stored_v1_fallback_is_readable() {
    // 构造 raw_payload（v1 body：count=2 + 2 条 entry）
    let mut raw = Vec::new();
    raw.extend_from_slice(&2u32.to_le_bytes()); // count=2
                                                // entry 0: docid=0, text="t0", meta='{"m":0}'
    raw.extend_from_slice(&0u64.to_le_bytes());
    let t0 = "t0";
    raw.extend_from_slice(&(t0.len() as u32).to_le_bytes());
    raw.extend_from_slice(t0.as_bytes());
    let m0 = r#"{"m":0}"#;
    raw.extend_from_slice(&(m0.len() as u32).to_le_bytes());
    raw.extend_from_slice(m0.as_bytes());
    // entry 1: docid=1, text="", meta='{"m":1}'
    raw.extend_from_slice(&1u64.to_le_bytes());
    raw.extend_from_slice(&0u32.to_le_bytes()); // text_len=0
    let m1 = r#"{"m":1}"#;
    raw.extend_from_slice(&(m1.len() as u32).to_le_bytes());
    raw.extend_from_slice(m1.as_bytes());

    // encode_stored 无 zstd-encode → 写 v1
    let encoded = super::encode_stored(&raw);
    assert_eq!(&encoded[0..4], b"VANE");
    let sver = u32::from_le_bytes(encoded[4..8].try_into().unwrap());
    assert_eq!(sver, crate::types::STORED_FORMAT_V1, "回退写 v1");

    // decode_stored 读回：内容一致（文件可读，未损坏）
    let map = super::decode_stored(&encoded).unwrap();
    assert_eq!(map.len(), 2);
    assert_eq!(map.get(&0).unwrap().text, "t0");
    assert_eq!(map.get(&0).unwrap().meta_json, r#"{"m":0}"#);
    assert_eq!(map.get(&1).unwrap().text, "");
    assert_eq!(map.get(&1).unwrap().meta_json, r#"{"m":1}"#);
}

/// I-1 fix（zstd-encode 配置）：zstd 压缩成功写 v2，decode_stored 读回一致。
/// 此测试守护 v2 正常路径（压缩成功）仍正确——确保回退逻辑不影响成功路径。
#[cfg(feature = "zstd-encode")]
#[test]
fn m2_08_stored_v2_normal_path_still_readable() {
    let mut raw = Vec::new();
    raw.extend_from_slice(&1u32.to_le_bytes()); // count=1
    raw.extend_from_slice(&0u64.to_le_bytes()); // docid=0
    let t = "压缩成功路径";
    raw.extend_from_slice(&(t.len() as u32).to_le_bytes());
    raw.extend_from_slice(t.as_bytes());
    let m = r#"{"ok":true}"#;
    raw.extend_from_slice(&(m.len() as u32).to_le_bytes());
    raw.extend_from_slice(m.as_bytes());

    let encoded = super::encode_stored(&raw);
    let sver = u32::from_le_bytes(encoded[4..8].try_into().unwrap());
    // 压缩成功 → v2（若极端情况下压缩失败回退 v1，也必须可读——两路径都守护）
    assert!(
        sver == crate::types::STORED_FORMAT_V2 || sver == crate::types::STORED_FORMAT_V1,
        "encode_stored 应产 v1 或 v2，实际 {}",
        sver
    );
    // 无论 v1/v2，decode_stored 必须读回正确数据（I-1 核心：文件始终可读）
    let map = super::decode_stored(&encoded).unwrap();
    assert_eq!(map.len(), 1);
    assert_eq!(map.get(&0).unwrap().text, "压缩成功路径");
    assert_eq!(map.get(&0).unwrap().meta_json, r#"{"ok":true}"#);
}

// ---------------------------------------------------------------------------
// M2-09：SQ8 量化缓存懒加载测试（feature `sq8`）
// ---------------------------------------------------------------------------

#[cfg(feature = "sq8")]
#[test]
fn sq8_vectors_lazy_load_returns_some() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
    let schema = Schema::new(vec![(
        "v".into(),
        FieldDef::Vector {
            dim: 4,
            metric: Metric::Cosine,
        },
    )])
    .unwrap();
    let tok_id = TokenizerId([0x33; 32]);
    let mut writer = SegmentWriter::new(vfs.clone(), "segments", &schema, &tok_id, 0).unwrap();
    writer
        .add_doc("a", Some(&[1.0, 2.0, 3.0, 4.0]), r#"{"x":1}"#)
        .unwrap();
    writer
        .add_doc("b", Some(&[5.0, 6.0, 7.0, 8.0]), r#"{"x":2}"#)
        .unwrap();
    let meta = writer.finalize().unwrap();
    let seg_dir = format!("segments/seg_{}", meta.ulid);
    let reader = SegmentReader::open(&vfs, &seg_dir).unwrap();

    // 首次调用触发编码，返回 Some
    let bundle = reader.sq8_vectors();
    assert!(
        bundle.is_some(),
        "sq8_vectors() should return Some for non-empty segment"
    );
    let b = bundle.unwrap();
    assert_eq!(b.data.len(), 8); // 2 docs × 4 dim
    assert_eq!(b.min.len(), 4);
    assert_eq!(b.max.len(), 4);
    // 验证 min/max 正确
    assert_eq!(b.min, vec![1.0, 2.0, 3.0, 4.0]);
    assert_eq!(b.max, vec![5.0, 6.0, 7.0, 8.0]);

    // 二次调用幂等（同一引用）
    let bundle2 = reader.sq8_vectors();
    assert!(bundle2.is_some());
    let b2 = bundle2.unwrap();
    assert_eq!(b2.data, b.data);
}

#[cfg(feature = "sq8")]
#[test]
fn sq8_vectors_empty_segment_returns_none() {
    // 空段（doc_count==0）→ sq8_vectors() 返回 None
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
    let schema = Schema::new(vec![(
        "v".into(),
        FieldDef::Vector {
            dim: 4,
            metric: Metric::Cosine,
        },
    )])
    .unwrap();
    let tok_id = TokenizerId([0x44; 32]);
    let writer = SegmentWriter::new(vfs.clone(), "segments", &schema, &tok_id, 0).unwrap();
    let meta = writer.finalize().unwrap();
    let seg_dir = format!("segments/seg_{}", meta.ulid);
    let reader = SegmentReader::open(&vfs, &seg_dir).unwrap();
    assert_eq!(reader.doc_count(), 0);
    assert!(reader.sq8_vectors().is_none());
}

#[cfg(feature = "sq8")]
#[test]
fn sq8_vectors_does_not_write_segment_files() {
    // I-1 守护：SQ8 是内存缓存，不写段文件（vectors.bin 仍 f32 落盘）
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
    let schema = Schema::new(vec![(
        "v".into(),
        FieldDef::Vector {
            dim: 4,
            metric: Metric::Cosine,
        },
    )])
    .unwrap();
    let tok_id = TokenizerId([0x55; 32]);
    let mut writer = SegmentWriter::new(vfs.clone(), "segments", &schema, &tok_id, 0).unwrap();
    writer
        .add_doc("a", Some(&[1.0, 2.0, 3.0, 4.0]), r#"{}"#)
        .unwrap();
    let meta = writer.finalize().unwrap();
    let seg_dir = format!("segments/seg_{}", meta.ulid);

    // 记录 finalize 后的文件列表
    let files_before = vfs.list(&seg_dir).unwrap();

    let reader = SegmentReader::open(&vfs, &seg_dir).unwrap();
    // 触发 sq8_vectors 编码
    let _ = reader.sq8_vectors();
    // 也触发 vectors 加载
    let _ = reader.vectors();

    // 文件列表不变（SQ8 不写段文件）
    let files_after = vfs.list(&seg_dir).unwrap();
    assert_eq!(
        files_before, files_after,
        "SQ8 encoding must not write segment files (I-1)"
    );
    // 不应有 sq8 相关文件
    assert!(
        !files_after.iter().any(|f| f.contains("sq8")),
        "no sq8 files should exist in segment dir"
    );
}
