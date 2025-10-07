use super::header::{decode_header, encode_header};
use super::ulid::gen_ulid;
use super::*;
use crate::types::TokenizerId;

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
