pub mod header;
pub mod ulid;
#[cfg(test)]
mod tests;

use crate::types::TokenizerId;

/// 段元数据（SPEC §6.3）。写期由 SegmentWriter.finalize 产出，
/// 读期由 SegmentReader.open 从 header.bin 解码。
pub struct SegmentMeta {
    pub ulid: String,
    pub doc_count: u32,
    pub docid_base: u64,
    pub tokenizer_id: TokenizerId,
    /// tombstone 位图（SPEC §6.3）。M0 恒为空（delete 是 M1）。
    pub tombstones: roaring::RoaringBitmap,
}
