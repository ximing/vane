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
    w2.add_doc("c", Some(&[1.0, 1.0]), "{}").unwrap();
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
        let n = vfs.read_at(&format!("{}/vectors.bin", seg_dir), &mut tmp, off).unwrap();
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
    assert_eq!(&r.vectors()[..], &[1.0, 2.0, 3.0, 4.0]);
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
        let n = vfs.read_at(&format!("{}/vectors.bin", seg_dir), &mut tmp, off).unwrap();
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
