## Commits c7e3cdf..acbd23d (fix r1)

acbd23d fix(segment): decode_header off-by-one (< 8 → < 9) + crash_recovery 场景 5 Corrupt 断言（M4 阶段二 b fix r1）

## Diff stat

 crates/vane-core/src/segment/header.rs   | 25 ++++++++++++++++++++++++-
 crates/vane-core/tests/crash_recovery.rs | 14 ++++++++++----
 2 files changed, 34 insertions(+), 5 deletions(-)

## Full diff (U10)

diff --git a/crates/vane-core/src/segment/header.rs b/crates/vane-core/src/segment/header.rs
index 18b49a6..1b37ea7 100644
--- a/crates/vane-core/src/segment/header.rs
+++ b/crates/vane-core/src/segment/header.rs
@@ -29,21 +29,23 @@ pub fn encode_header(meta: &SegmentMeta) -> Result<Vec<u8>> {
     meta.tombstones
         .serialize_into(&mut tb)
         .map_err(|e| VaneError::Corrupt(format!("tombstone serialize: {}", e)))?;
     out.extend_from_slice(&(tb.len() as u32).to_le_bytes());
     out.extend_from_slice(&tb);
     Ok(out)
 }
 
 /// header.bin 解码。
 pub fn decode_header(buf: &[u8]) -> Result<SegmentMeta> {
-    if buf.len() < 8 {
+    // 9 = magic(4) + version(4) + ulid_len(1)。buf.len()==8 时 buf[8] 越界 panic，
+    // 故门限 < 9 而非 < 8（M4 阶段二 b fix：off-by-one → panic-on-corrupt）。
+    if buf.len() < 9 {
         return Err(VaneError::Corrupt("header too short".into()));
     }
     if &buf[0..4] != MAGIC {
         return Err(VaneError::Corrupt(format!("bad magic: {:?}", &buf[0..4])));
     }
     let version = u32::from_le_bytes(buf[4..8].try_into().unwrap());
     if version != HEADER_FORMAT_V1 {
         return Err(VaneError::Version(format!(
             "unsupported format_version: {} (expected {})",
             version, HEADER_FORMAT_V1
@@ -79,10 +81,31 @@ pub fn decode_header(buf: &[u8]) -> Result<SegmentMeta> {
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
+
+#[cfg(test)]
+mod tests {
+    use super::*;
+
+    /// 回归测试：8 字节 header（magic+version，无 ulid_len）必须返 Corrupt 而非 panic。
+    /// 修复前：`< 8` 长度门允许 buf.len()==8，后续 `buf[8]`（ulid_len）越界 panic。
+    /// 修复后：`< 9` 拒绝 8 字节，返 `Corrupt("header too short")`。
+    #[test]
+    fn decode_header_8_bytes_returns_corrupt_not_panic() {
+        let mut buf = Vec::new();
+        buf.extend_from_slice(MAGIC);
+        buf.extend_from_slice(&HEADER_FORMAT_V1.to_le_bytes());
+        assert_eq!(buf.len(), 8);
+        let result = decode_header(&buf);
+        assert!(
+            matches!(result, Err(VaneError::Corrupt(ref msg)) if msg.contains("too short")),
+            "8-byte header should return Corrupt(\"header too short\")"
+        );
+    }
+}
diff --git a/crates/vane-core/tests/crash_recovery.rs b/crates/vane-core/tests/crash_recovery.rs
index 53fff15..38f6d8f 100644
--- a/crates/vane-core/tests/crash_recovery.rs
+++ b/crates/vane-core/tests/crash_recovery.rs
@@ -15,21 +15,22 @@
 //! 3. merge 中断崩溃——finalize_merge 的 write_inverted 失败，旧段保留
 //! 4. ENOSPC——write_at 返 ENOSPC，不损已有数据
 //! 5. 部分写——header.bin 写 8 字节后失败，损坏段被清理
 
 use std::sync::Arc;
 
 use vane_core::api::{
     CollectionOptions, Db, Doc, FusionSpec, OpenOptions, SearchMode, SearchQuery,
 };
 use vane_core::persistence::AutoCommitConfig;
-use vane_core::types::{FieldDef, Metric, Schema};
+use vane_core::segment::header::decode_header;
+use vane_core::types::{FieldDef, Metric, Schema, VaneError};
 use vane_core::vfs::fault::{Fault, FaultVfs, VfsOp};
 use vane_core::vfs::Vfs;
 
 // ---------------------------------------------------------------------------
 // 测试辅助
 // ---------------------------------------------------------------------------
 
 /// 构建含 text + vector(4d, cosine) 的 schema。
 fn schema() -> Schema {
     Schema::new(vec![
@@ -545,23 +546,28 @@ fn crash_5_partial_write() {
         let mut buf = vec![0u8; 128];
         let n = vfs.read_at(&header_path, &mut buf, 0).unwrap();
         assert_eq!(
             n, 8,
             "corrupt header.bin should have exactly 8 bytes (magic+version), got {}",
             n
         );
         assert_eq!(&buf[..4], b"VANE", "first 4 bytes should be magic 'VANE'");
         let ver = u32::from_le_bytes(buf[4..8].try_into().unwrap());
         assert_eq!(ver, 1, "bytes 4-8 should be format_version=1 (LE)");
-        // 8 字节恰好含 magic+version 但缺少 ulid_len 及后续字段 → 不完整 header
-        // decode_header 在 < 8 字节时返 Corrupt("header too short")；
-        // 8 字节恰好过长度门但缺 ulid_len → 无效段。recover 不尝试 open 孤儿段，直接清理。
+        // 8 字节恰好含 magic+version 但缺少 ulid_len 及后续字段 → 不完整 header。
+        // decode_header 长度门 < 9 拒绝 8 字节，返 Corrupt("header too short")（非 panic）。
+        // 覆盖 decode_header 拒绝路径：损坏段被校验拒绝，recover 不尝试 open 孤儿段，直接清理。
+        let decode_result = decode_header(&buf[..n]);
+        assert!(
+            matches!(decode_result, Err(VaneError::Corrupt(ref msg)) if msg.contains("too short")),
+            "decode_header on 8-byte corrupt header should return Corrupt(\"header too short\")"
+        );
 
         // 不 close（模拟崩溃）
     }
 
     // ---- 会话 2：重开 → recover ----
     {
         let db = Db::open(vfs.clone(), db_path, OpenOptions::default()).unwrap();
         let col = db.collection("c", schema(), col_opts()).unwrap();
 
         // 孤儿段被 recover 清理：只有 1 段（基线段）
