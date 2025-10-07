use crate::segment::SegmentMeta;
use crate::types::{FORMAT_VERSION, MAGIC, Result, TokenizerId, VaneError};

// header.bin 布局（SPEC §6.3）：
// magic(4) | format_version(4 BE) | ulid_len(1) | ulid(26) |
// doc_count(4 LE) | docid_base(8 LE) | tokenizer_id(32) |
// tombstone_bytes(4 LE) | tombstone_data
//
// 注：format_version 采用大端（to_be_bytes），与计划 Task 1 测试断言
// bytes[4..8] == [0,0,0,1] 一致；其余数值字段沿用 LE。

/// header.bin 编码。
pub fn encode_header(meta: &SegmentMeta) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&FORMAT_VERSION.to_be_bytes());
    let ulid_bytes = meta.ulid.as_bytes();
    out.push(ulid_bytes.len() as u8);
    out.extend_from_slice(ulid_bytes);
    out.extend_from_slice(&meta.doc_count.to_le_bytes());
    out.extend_from_slice(&meta.docid_base.to_le_bytes());
    out.extend_from_slice(meta.tokenizer_id.as_bytes());
    let mut tb = Vec::new();
    meta.tombstones
        .serialize_into(&mut tb)
        .map_err(|e| VaneError::Corrupt(format!("tombstone serialize: {}", e)))?;
    out.extend_from_slice(&(tb.len() as u32).to_le_bytes());
    out.extend_from_slice(&tb);
    Ok(out)
}

/// header.bin 解码。
pub fn decode_header(buf: &[u8]) -> Result<SegmentMeta> {
    if buf.len() < 8 {
        return Err(VaneError::Corrupt("header too short".into()));
    }
    if &buf[0..4] != MAGIC {
        return Err(VaneError::Corrupt(format!(
            "bad magic: {:?}",
            &buf[0..4]
        )));
    }
    let version = u32::from_be_bytes(buf[4..8].try_into().unwrap());
    if version != FORMAT_VERSION {
        return Err(VaneError::Version(format!(
            "unsupported format_version: {} (expected {})",
            version,
            FORMAT_VERSION
        )));
    }
    let mut pos = 8;
    let ulid_len = buf[pos] as usize;
    pos += 1;
    if pos + ulid_len > buf.len() {
        return Err(VaneError::Corrupt("header truncated at ulid".into()));
    }
    let ulid = std::str::from_utf8(&buf[pos..pos + ulid_len])
        .map_err(|e| VaneError::Corrupt(format!("ulid utf8: {}", e)))?
        .to_string();
    pos += ulid_len;
    if pos + 4 + 8 + 32 + 4 > buf.len() {
        return Err(VaneError::Corrupt("header truncated at fixed fields".into()));
    }
    let doc_count = u32::from_le_bytes(buf[pos..pos + 4].try_into().unwrap());
    pos += 4;
    let docid_base = u64::from_le_bytes(buf[pos..pos + 8].try_into().unwrap());
    pos += 8;
    let mut tid = [0u8; 32];
    tid.copy_from_slice(&buf[pos..pos + 32]);
    pos += 32;
    let tb_len = u32::from_le_bytes(buf[pos..pos + 4].try_into().unwrap()) as usize;
    pos += 4;
    if pos + tb_len > buf.len() {
        return Err(VaneError::Corrupt("header truncated at tombstone".into()));
    }
    let tombstones = roaring::RoaringBitmap::deserialize_from(&buf[pos..pos + tb_len])
        .map_err(|e| VaneError::Corrupt(format!("tombstone deserialize: {}", e)))?;
    Ok(SegmentMeta {
        ulid,
        doc_count,
        docid_base,
        tokenizer_id: TokenizerId(tid),
        tombstones,
    })
}
