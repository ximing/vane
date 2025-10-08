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
    // magic + version 开头
    assert_eq!(&bytes[0..4], b"VANE");
    assert_eq!(&bytes[4..8], &[0, 0, 0, 1]);
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
        ("vec".into(), FieldDef::Vector { dim: 4, metric: Metric::Cosine }),
    ]).unwrap();
    let tok_id = TokenizerId([0x11; 32]);

    let mut writer = SegmentWriter::new(
        vfs.clone(), "segments", &schema, &tok_id, 0,
    ).unwrap();
    let d0 = writer.add_doc("doc-0", Some(&[1.0, 0.0, 0.0, 0.0]), r#"{"title":"hello"}"#).unwrap();
    let d1 = writer.add_doc("doc-1", Some(&[0.0, 1.0, 0.0, 0.0]), r#"{"title":"world"}"#).unwrap();
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
    let schema = Schema::new(vec![
        ("v".into(), FieldDef::Vector { dim: 4, metric: Metric::Cosine }),
    ]).unwrap();
    let tok_id = TokenizerId([0x22; 32]);

    let mut writer = SegmentWriter::new(vfs.clone(), "segments", &schema, &tok_id, 0).unwrap();
    writer.add_doc("alpha", Some(&[1.0, 2.0, 3.0, 4.0]), r#"{"x":1}"#).unwrap();
    writer.add_doc("beta", Some(&[5.0, 6.0, 7.0, 8.0]), r#"{"x":2}"#).unwrap();
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
    vfs.write_at(&format!("{}/header.bin", seg_dir), b"XXXX", 0).unwrap();
    let r = SegmentReader::open(&vfs, seg_dir);
    assert!(matches!(r, Err(VaneError::Corrupt(_))));
}
