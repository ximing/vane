## Commits 8959337..5fc4ac4 (5c diagnostics)

5fc4ac4 feat(core): VaneError 诊断上下文（String 丰富，不改错误码）（M4 阶段五 c）

## Diff stat

 crates/vane-core/src/api/collection.rs       |  22 ++-
 crates/vane-core/src/api/db.rs               |   4 +-
 crates/vane-core/src/api/reindex.rs          |  10 +-
 crates/vane-core/src/bm25.rs                 |  16 +-
 crates/vane-core/src/merge/mod.rs            |   4 +-
 crates/vane-core/src/persistence/mod.rs      |  26 ++-
 crates/vane-core/src/persistence/tests.rs    |  25 +++
 crates/vane-core/src/segment/mod.rs          |  67 ++++++--
 crates/vane-core/src/segment/tests.rs        |  38 +++++
 crates/vane-core/src/tokenizer/jieba/dict.rs |  24 ++-
 crates/vane-core/src/types.rs                |  57 +++++++
 crates/vane-core/src/wal/mod.rs              |  16 +-
 crates/vane-core/src/wal/tests.rs            |  21 +++
 docs/plans/m4/task-diagnostics-report.md     | 242 +++++++++++++++++++++++++++
 14 files changed, 520 insertions(+), 52 deletions(-)

## Full diff (U10)

diff --git a/crates/vane-core/src/api/collection.rs b/crates/vane-core/src/api/collection.rs
index a549e3c..b39c79b 100644
--- a/crates/vane-core/src/api/collection.rs
+++ b/crates/vane-core/src/api/collection.rs
@@ -258,23 +258,25 @@ impl Collection {
         if *self.inner.dict_state.read().unwrap() == DictState::Rebuilding {
             return Err(VaneError::Busy);
         }
         let mut state = self.inner.write_state.lock().unwrap();
         let schema_dim = self.inner.schema.vector_field().map(|(_, d, _)| d).ok();
         let mut count = 0u64;
         for doc in docs {
             if let (Some(dim), Some(v)) = (schema_dim, &doc.vector) {
                 if v.len() as u32 != dim {
                     return Err(VaneError::Schema(format!(
-                        "vector dim mismatch: got {} expected {}",
+                        "vector dim mismatch: got {} expected {} (op=add, collection={}, doc_id={}; 建议: 对齐 doc vector 维度与 schema 声明)",
                         v.len(),
-                        dim
+                        dim,
+                        self.inner.name,
+                        doc.id
                     )));
                 }
             }
             let docid = state.next_docid;
             state.next_docid += 1;
             state.buffer.push(BufferedDoc {
                 docid,
                 external_id: doc.id.clone(),
                 text: doc.text.clone(),
                 vector: doc.vector.clone(),
@@ -573,21 +575,24 @@ impl Collection {
 
         // 更新 manifest（I-6）。
         let manifest_store = ManifestStore::new(self.inner.vfs.clone(), &self.inner.db_path);
         let mut manifest = manifest_store
             .load()?
             .unwrap_or_else(crate::persistence::Manifest::empty);
         let col_meta = manifest
             .collections
             .get_mut(&self.inner.name)
             .ok_or_else(|| {
-                VaneError::NotFound(format!("collection not in manifest: {}", self.inner.name))
+                VaneError::NotFound(format!(
+                    "collection not in manifest: {} (op=merge, db={}; 建议: 确认 collection 已创建)",
+                    self.inner.name, self.inner.db_path
+                ))
             })?;
         col_meta.segment_ulids.retain(|u| !source_ulids.contains(u));
         col_meta.segment_ulids.push(new_meta.ulid.clone());
         // 04-wal：manifest 切换前 append 段增删记录（SPEC §6.4）：
         // DeleteSegment(旧) + AddSegment(新)。crash 在 manifest 切换前 →
         // AddSegment(new) 不在 manifest → 孤儿清理；DeleteSegment(old) → 旧段保留。
         // B-2：truncate 仅 compact 调（此处 merge_segments 不 truncate）。
         let wal = crate::wal::Wal::open(self.inner.vfs.clone(), &self.inner.db_path)?;
         for u in &source_ulids {
             wal.append(&crate::wal::WalRecord::DeleteSegment {
@@ -679,40 +684,40 @@ impl Collection {
         self.run_search(query, false)
     }
 
     /// 搜索主逻辑（`search` 与 `search_brute_baseline` 共享）。
     ///
     /// `allow_hnsw=true`：vector 路有 HnswReader 则走 HNSW（自适应回退时仍走 brute）。
     /// `allow_hnsw=false`：vector 路恒走 `brute_search`（基线口径，绕过 HNSW）。
     fn run_search(&self, query: &SearchQuery, allow_hnsw: bool) -> Result<Vec<Hit>> {
         if query.top_k > TOPK_MAX {
             return Err(VaneError::InvalidArg(format!(
-                "topK {} exceeds max {}",
-                query.top_k, TOPK_MAX
+                "topK {} exceeds max {} (op=search, collection={}; 建议: 减小 topK 至 {} 以内)",
+                query.top_k, TOPK_MAX, self.inner.name, TOPK_MAX
             )));
         }
         // 03-pre-filter：编译用户 filter 为 roaring 位图（SPEC §8.3）。
         // 无 filter 时若有 tombstone，构造 alive 位图统一排除（Task 5）。
         // 位图存绝对 docid，传给各段 search（HnswReader/brute/InvertedIndexReader 均
         // 接受 filter 参数，内部按 docid_base 转换）。tombstone 在 compile_filter 末尾
         // and_not 排除（Task 5），统一替换 02 的手动 alive_bm 并入。
         let mode = match query.mode {
             SearchMode::Hybrid => SearchMode::Hybrid,
             SearchMode::Vector => SearchMode::Vector,
             SearchMode::Text => SearchMode::Text,
             SearchMode::Auto => match (&query.text, &query.vector) {
                 (Some(_), Some(_)) => SearchMode::Hybrid,
                 (Some(_), None) => SearchMode::Text,
                 (None, Some(_)) => SearchMode::Vector,
                 (None, None) => {
                     return Err(VaneError::InvalidArg(
-                        "search requires text or vector".into(),
+                        "search requires text or vector (op=search; 建议: 提供 text 或 vector 查询参数)".into(),
                     ))
                 }
             },
         };
         // M4 §3.5 tracing：检索延迟 span + elapsed。cfg 门控，tracing off 时编译期消除。
         // 早期返回（topK 超限/缺 text+vector）不经此 span——属参数校验 fast-fail，无需埋点。
         #[cfg(feature = "tracing")]
         let _span = tracing::info_span!(
             "search",
             top_k = query.top_k,
@@ -720,23 +725,24 @@ impl Collection {
             segment_count = self.segment_count(),
             allow_hnsw
         );
         #[cfg(feature = "tracing")]
         let _search_start = web_time::Instant::now();
         // dim 校验 + metric 一次性解析（hoist 出循环，避免每段重复 vector_field() 调用）
         let vf = if let Some(v) = &query.vector {
             let (_, dim, metric) = self.inner.schema.vector_field()?;
             if v.len() as u32 != dim {
                 return Err(VaneError::Schema(format!(
-                    "query vector dim {} != schema dim {}",
+                    "query vector dim {} != schema dim {} (op=search, collection={}; 建议: 对齐 query vector 维度与 schema 声明)",
                     v.len(),
-                    dim
+                    dim,
+                    self.inner.name
                 )));
             }
             Some(metric)
         } else {
             None
         };
 
         let snap = self.inner.snapshot.read().unwrap();
         let offsets = self.inner.seg_offsets.read().unwrap();
         // I7：用缓存的 InvertedIndexReader，避免每次 search 重开
diff --git a/crates/vane-core/src/api/db.rs b/crates/vane-core/src/api/db.rs
index b19e2b3..53601cd 100644
--- a/crates/vane-core/src/api/db.rs
+++ b/crates/vane-core/src/api/db.rs
@@ -100,28 +100,28 @@ impl Db {
         name: &str,
         schema: Schema,
         opts: CollectionOptions,
     ) -> Result<Collection> {
         // I2 裁决：幂等校验 schema 与 tokenizer 一致性
         {
             let read = self.inner.collections.read().unwrap();
             if let Some(existing) = read.get(name) {
                 if existing.schema.fields != schema.fields {
                     return Err(VaneError::Schema(format!(
-                        "collection '{}' exists with different schema",
+                        "collection '{}' exists with different schema (op=open collection; 建议: 使用相同 schema 或新 collection 名称)",
                         name
                     )));
                 }
                 let tok_id = compute_tokenizer_id(opts.tokenizer, &opts.user_dict);
                 if *existing.tokenizer_id.read().unwrap() != tok_id {
                     return Err(VaneError::Schema(format!(
-                        "collection '{}' exists with different tokenizer",
+                        "collection '{}' exists with different tokenizer (op=open collection; 建议: 使用相同 tokenizer 或新 collection 名称)",
                         name
                     )));
                 }
                 return Ok(Collection {
                     inner: existing.clone(),
                 });
             }
         }
         let tok_id = compute_tokenizer_id(opts.tokenizer, &opts.user_dict);
         let meta = CollectionMeta {
diff --git a/crates/vane-core/src/api/reindex.rs b/crates/vane-core/src/api/reindex.rs
index 3ce02e3..0ae9703 100644
--- a/crates/vane-core/src/api/reindex.rs
+++ b/crates/vane-core/src/api/reindex.rs
@@ -216,24 +216,26 @@ pub(crate) fn reindex_segment(
 
 /// 更新 manifest 中的 collection meta（段 ULID 替换 + tokenizer_id/user_dict 更新）。
 pub(crate) fn update_manifest_after_reindex(
     manifest_store: &ManifestStore,
     col_name: &str,
     old_ulids: &[String],
     new_ulids: Vec<String>,
     new_meta: CollectionMeta,
 ) -> Result<()> {
     let mut manifest = manifest_store.load()?.unwrap_or_else(Manifest::empty);
-    let col = manifest
-        .collections
-        .get_mut(col_name)
-        .ok_or_else(|| VaneError::NotFound(format!("collection not in manifest: {}", col_name)))?;
+    let col = manifest.collections.get_mut(col_name).ok_or_else(|| {
+        VaneError::NotFound(format!(
+            "collection not in manifest: {} (op=reindex; 建议: 确认 collection 已创建)",
+            col_name
+        ))
+    })?;
     // 替换 ULID：移除旧 ULID，追加新 ULID（保持其余顺序）。
     col.segment_ulids.retain(|u| !old_ulids.contains(u));
     for u in &new_ulids {
         if !col.segment_ulids.contains(u) {
             col.segment_ulids.push(u.clone());
         }
     }
     col.tokenizer_id = new_meta.tokenizer_id;
     col.user_dict = new_meta.user_dict;
     manifest_store.save_atomic(&manifest)
diff --git a/crates/vane-core/src/bm25.rs b/crates/vane-core/src/bm25.rs
index c06a24f..a4f63e8 100644
--- a/crates/vane-core/src/bm25.rs
+++ b/crates/vane-core/src/bm25.rs
@@ -336,32 +336,38 @@ impl std::fmt::Debug for InvertedIndexReader {
 
 impl InvertedIndexReader {
     pub fn open(vfs: &std::sync::Arc<dyn Vfs>, segment_dir: &str) -> Result<Self> {
         let path = format!("{}/inverted.bin", segment_dir);
 
         // 头部 8 字节：magic(4) + version(4)
         let mut header = [0u8; 8];
         let n = vfs.read_at(&path, &mut header, 0)?;
         if n < 8 {
             return Err(VaneError::Corrupt(format!(
-                "inverted.bin truncated header: {}",
-                n
+                "inverted.bin truncated header: {}{}",
+                n,
+                crate::segment::seg_ctx(segment_dir, "open inverted.bin")
             )));
         }
         if &header[0..4] != MAGIC {
-            return Err(VaneError::Corrupt("inverted.bin bad magic".into()));
+            return Err(VaneError::Corrupt(format!(
+                "inverted.bin bad magic{}",
+                crate::segment::seg_ctx(segment_dir, "open inverted.bin")
+            )));
         }
         let version = u32::from_le_bytes(header[4..8].try_into().unwrap());
         if version != FORMAT_VERSION {
             return Err(VaneError::Version(format!(
-                "inverted.bin version {} != supported {}",
-                version, FORMAT_VERSION
+                "inverted.bin version {} != supported {}{}",
+                version,
+                FORMAT_VERSION,
+                crate::segment::seg_ctx(segment_dir, "open inverted.bin")
             )));
         }
 
         // 读取剩余全部：循环 read_at 增量读取直到返回 0
         let mut blob: Vec<u8> = Vec::new();
         let mut offset: u64 = 8;
         let chunk = 64 * 1024;
         loop {
             let mut tmp = vec![0u8; chunk];
             let rn = vfs.read_at(&path, &mut tmp, offset)?;
diff --git a/crates/vane-core/src/merge/mod.rs b/crates/vane-core/src/merge/mod.rs
index 96a4ad6..10235ee 100644
--- a/crates/vane-core/src/merge/mod.rs
+++ b/crates/vane-core/src/merge/mod.rs
@@ -247,21 +247,23 @@ impl MergeTask {
     }
 }
 
 /// 合并完成：落盘 vectors/stored/idmap/scalars/header + inverted + hnsw，返回新段 meta。
 /// 新段 tombstone 恒为空（物理清除，SPEC §6.3）。
 pub fn finalize_merge(
     mut task: MergeTask,
     ctx: &MergeContext,
 ) -> crate::types::Result<SegmentMeta> {
     let writer = task.writer.take().ok_or_else(|| {
-        crate::types::VaneError::InvalidArg("finalize_merge with no steps".into())
+        crate::types::VaneError::InvalidArg(
+            "finalize_merge with no steps (op=merge; 建议: 检查 merge 调用序列)".into(),
+        )
     })?;
     let meta = writer.finalize()?;
     let seg_dir = format!("{}/seg_{}", ctx.segments_dir, meta.ulid);
 
     // 倒排：累积的 inv_terms -> InvertedData。
     let doc_count = task.field_lengths.len() as u64;
     let total_fl: u64 = task.field_lengths.iter().map(|&x| x as u64).sum();
     let avg_field_length = if doc_count == 0 {
         0.0
     } else {
diff --git a/crates/vane-core/src/persistence/mod.rs b/crates/vane-core/src/persistence/mod.rs
index c1f8df8..7693c9e 100644
--- a/crates/vane-core/src/persistence/mod.rs
+++ b/crates/vane-core/src/persistence/mod.rs
@@ -82,50 +82,60 @@ impl ManifestStore {
             };
             if n == 0 {
                 break;
             }
             buf.extend_from_slice(&tmp[..n]);
             off += n as u64;
         }
         if buf.is_empty() {
             return Ok(None);
         }
-        let m: Manifest = serde_json::from_slice(&buf)
-            .map_err(|e| VaneError::Corrupt(format!("manifest parse: {}", e)))?;
+        let m: Manifest = serde_json::from_slice(&buf).map_err(|e| {
+            VaneError::Corrupt(format!(
+                "manifest parse: {} (db={}, op=load manifest; 建议: 检查 manifest.json 完整性或从备份恢复)",
+                e, self.db_path
+            ))
+        })?;
         Ok(Some(m))
     }
 
     /// SPEC §6.4 原子切换：写临时文件 → sync → rename。
     /// 不变量 I-6：任何崩溃后 manifest 指向完整状态（rename 前崩溃 → 旧 manifest 完好；
     /// rename 是原子操作 → manifest 永远指向完整新状态或完整旧状态）。
     pub fn save_atomic(&self, manifest: &Manifest) -> Result<()> {
-        let json = serde_json::to_vec(manifest)
-            .map_err(|e| VaneError::Corrupt(format!("manifest serialize: {}", e)))?;
+        let json = serde_json::to_vec(manifest).map_err(|e| {
+            VaneError::Corrupt(format!(
+                "manifest serialize: {} (db={}, op=save manifest; 建议: 重试或检查磁盘空间)",
+                e, self.db_path
+            ))
+        })?;
         let tmp = self.tmp_path();
         let target = self.manifest_path();
         // I16 裁决：先清理可能残留的 tmp（忽略错误，tmp 可能不存在），处理上次崩溃残留。
         let _ = self.vfs.delete(&tmp);
         self.vfs.create(&tmp)?;
         self.vfs.write_at(&tmp, &json, 0)?;
         self.vfs.sync(&tmp)?;
         // 原子 rename 覆盖旧 manifest（MemoryVfs 直接覆盖；StdFsVfs 的 rename 落盘原子）。
         self.vfs.rename(&tmp, &target)?;
         Ok(())
     }
 
     /// 在指定 collection 的 segment_ulids 中追加一个 ULID（去重），并原子保存。
     pub fn add_segment(&self, collection: &str, ulid: &str) -> Result<()> {
         let mut m = self.load()?.unwrap_or_else(Manifest::empty);
-        let col = m
-            .collections
-            .get_mut(collection)
-            .ok_or_else(|| VaneError::NotFound(format!("collection not found: {}", collection)))?;
+        let col = m.collections.get_mut(collection).ok_or_else(|| {
+            VaneError::NotFound(format!(
+                "collection not found: {} (db={}, seg={}, op=add_segment; 建议: 确认 collection 名称正确)",
+                collection, self.db_path, ulid
+            ))
+        })?;
         if !col.segment_ulids.contains(&ulid.to_string()) {
             col.segment_ulids.push(ulid.to_string());
         }
         self.save_atomic(&m)
     }
 }
 
 /// SPEC §7.1 auto-commit 配置。默认 `On { interval_ms=1000, max_docs=1000 }`。
 #[derive(Debug, Clone)]
 pub enum AutoCommitConfig {
diff --git a/crates/vane-core/src/persistence/tests.rs b/crates/vane-core/src/persistence/tests.rs
index 00c6686..d695f4c 100644
--- a/crates/vane-core/src/persistence/tests.rs
+++ b/crates/vane-core/src/persistence/tests.rs
@@ -131,20 +131,45 @@ fn manifest_store_save_atomic_overwrites() {
 fn manifest_store_corrupt_returns_error() {
     let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
     // 写损坏的 manifest
     vfs.create("db/manifest.json").unwrap();
     vfs.write_at("db/manifest.json", b"not json {{{", 0)
         .unwrap();
     let store = ManifestStore::new(vfs, "db");
     assert!(store.load().is_err());
 }
 
+/// M4 阶段五 c：VaneError 诊断上下文——manifest parse 错误 String 含
+/// db 路径 + 操作 + 建议操作（§10 推荐"先丰富 String"）。
+#[test]
+fn m4_5c_manifest_parse_error_contains_context() {
+    use crate::types::VaneError;
+    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
+    vfs.create("mydb/manifest.json").unwrap();
+    vfs.write_at("mydb/manifest.json", b"not json {{{", 0)
+        .unwrap();
+    let store = ManifestStore::new(vfs, "mydb");
+    match store.load() {
+        Err(VaneError::Corrupt(m)) => {
+            assert!(
+                m.contains("manifest parse"),
+                "original msg preserved: {}",
+                m
+            );
+            assert!(m.contains("mydb"), "msg must contain db path: {}", m);
+            assert!(m.contains("op=load manifest"), "msg must contain op: {}", m);
+            assert!(m.contains("建议"), "msg must contain suggestion: {}", m);
+        }
+        other => panic!("expected Corrupt, got {:?}", other.map_err(|e| e.name())),
+    }
+}
+
 #[test]
 fn auto_committer_default_is_on_1000_1000() {
     match AutoCommitConfig::default() {
         AutoCommitConfig::On {
             interval_ms,
             max_docs,
         } => {
             assert_eq!(interval_ms, 1000);
             assert_eq!(max_docs, 1000);
         }
diff --git a/crates/vane-core/src/segment/mod.rs b/crates/vane-core/src/segment/mod.rs
index 8391194..158f067 100644
--- a/crates/vane-core/src/segment/mod.rs
+++ b/crates/vane-core/src/segment/mod.rs
@@ -353,88 +353,120 @@ fn read_all(vfs: &dyn Vfs, path: &str) -> Result<Vec<u8>> {
         let n = vfs.read_at(path, &mut tmp, off)?;
         if n == 0 {
             break;
         }
         buf.extend_from_slice(&tmp[..n]);
         off += n as u64;
     }
     Ok(buf)
 }
 
+/// 从 segment_dir 路径末段 `seg_<ulid>` 提取 ULID 字符串（诊断上下文用）。
+/// M4 阶段五 c：VaneError 诊断 String 丰富——段级错误附 ULID 上下文。
+pub(crate) fn segment_ulid_from_dir(segment_dir: &str) -> &str {
+    segment_dir
+        .rsplit('/')
+        .next()
+        .and_then(|c| c.strip_prefix("seg_"))
+        .unwrap_or("unknown")
+}
+
+/// 段级诊断上下文后缀（M4 阶段五 c）。
+/// 用于 SegmentReader::open / load_vectors / InvertedIndexReader::open 等段级
+/// 错误路径的 VaneError String 丰富。不改错误码，仅追加 String 上下文。
+pub(crate) fn seg_ctx(segment_dir: &str, op: &str) -> String {
+    format!(
+        " (seg={}, op={}; 建议: 检查段文件完整性或从备份恢复)",
+        segment_ulid_from_dir(segment_dir),
+        op
+    )
+}
+
 impl SegmentReader {
     /// M2-07 冷启动懒加载：open 仅读 header + id_map + 廉价头探测（SPEC v1.2 §13.1 元数据 open<1s）。
     /// vectors/stored payload 改 OnceLock 首次访问按需加载；dim 首次访问按需计算（v2 头含 dim；v1 回退）。
     ///
     /// fix round 1（I-1）：open 期对 vectors.bin / stored.bin 做**廉价头探测**（各读 ≤12 字节），
     /// 恢复 M0/M1 对 bad magic / unsupported version 的 loud `Err` 失败（payload 仍懒加载，不读）。
     /// 避免 vectors.bin-only 损坏被 `unwrap_or_default()` 静默吞成空结果。
     pub fn open(vfs: &Arc<dyn Vfs>, segment_dir: &str) -> Result<Self> {
         // 读 header
         let hpath = format!("{}/header.bin", segment_dir);
         let hbuf = read_all(vfs.as_ref(), &hpath)?;
-        let meta = header::decode_header(&hbuf)?;
+        let meta = header::decode_header(&hbuf).map_err(|e| {
+            crate::types::append_context(e, &seg_ctx(segment_dir, "open header.bin"))
+        })?;
 
         // 读 id_map（小文件，open 时读）
         let id_map = Self::load_id_map(vfs.as_ref(), segment_dir)?;
 
         // fix round 1（I-1）：vectors.bin 头探测（doc_count>0 时）。
         // 只读前 12 字节：v1 头 8 字节（magic+version），v2 头 12 字节（magic+version+dim）。
         // 校验 magic + version ∈ {1, 2}，不匹配 loud `Err`（同 M0/M1 旧 open 语义）。
         // payload（154MB）不在 open 读，仍 lazy。doc_count==0 时 vectors.bin 仅 8 字节空段头，跳过探测。
         let v2_header_dim = if meta.doc_count > 0 {
             let vpath = format!("{}/vectors.bin", segment_dir);
             let mut hdr = [0u8; 12];
             let n = vfs.read_at(&vpath, &mut hdr, 0)?;
             if n < 8 || &hdr[0..4] != crate::types::MAGIC {
-                return Err(VaneError::Corrupt("vectors.bin bad magic".into()));
+                return Err(VaneError::Corrupt(format!(
+                    "vectors.bin bad magic{}",
+                    seg_ctx(segment_dir, "open vectors.bin")
+                )));
             }
             let version = u32::from_le_bytes(hdr[4..8].try_into().unwrap());
             match version {
                 v if v == crate::types::VECTORS_FORMAT_V1 => None, // v1：dim 懒加载从 payload 反推
                 v if v == crate::types::VECTORS_FORMAT_V2 => {
                     // v2：dim 字段在 offset 8..12（M2-08 写入，本模块读）。
                     if n < 12 {
-                        return Err(VaneError::Corrupt(
-                            "vectors.bin v2 header truncated (need 12 bytes)".into(),
-                        ));
+                        return Err(VaneError::Corrupt(format!(
+                            "vectors.bin v2 header truncated (need 12 bytes){}",
+                            seg_ctx(segment_dir, "open vectors.bin")
+                        )));
                     }
                     Some(u32::from_le_bytes(hdr[8..12].try_into().unwrap()))
                 }
                 _ => {
                     return Err(VaneError::Version(format!(
-                        "vectors.bin unsupported format_version: {} (expected {} or {})",
+                        "vectors.bin unsupported format_version: {} (expected {} or {}){}",
                         version,
                         crate::types::VECTORS_FORMAT_V1,
-                        crate::types::VECTORS_FORMAT_V2
+                        crate::types::VECTORS_FORMAT_V2,
+                        seg_ctx(segment_dir, "open vectors.bin")
                     )));
                 }
             }
         } else {
             None
         };
 
         // fix round 1（I-1）：stored.bin 头探测。只读前 8 字节（magic+version）。
         // M2-08：stored v2（zstd 块）双模——open 期接受 version ∈ {1, 2}，payload 仍懒加载。
         {
             let spath = format!("{}/stored.bin", segment_dir);
             let mut shdr = [0u8; 8];
             let n = vfs.read_at(&spath, &mut shdr, 0)?;
             if n < 8 || &shdr[0..4] != crate::types::MAGIC {
-                return Err(VaneError::Corrupt("stored.bin bad magic".into()));
+                return Err(VaneError::Corrupt(format!(
+                    "stored.bin bad magic{}",
+                    seg_ctx(segment_dir, "open stored.bin")
+                )));
             }
             let sver = u32::from_le_bytes(shdr[4..8].try_into().unwrap());
             if sver != crate::types::STORED_FORMAT_V1 && sver != crate::types::STORED_FORMAT_V2 {
                 return Err(VaneError::Version(format!(
-                    "stored.bin unsupported format_version: {} (expected {} or {})",
+                    "stored.bin unsupported format_version: {} (expected {} or {}){}",
                     sver,
                     crate::types::STORED_FORMAT_V1,
-                    crate::types::STORED_FORMAT_V2
+                    crate::types::STORED_FORMAT_V2,
+                    seg_ctx(segment_dir, "open stored.bin")
                 )));
             }
         }
 
         // M2-07：vectors / stored / dim 延迟到首次访问（OnceLock）。头已校验，payload 仍 lazy。
         Ok(Self {
             meta,
             vfs: vfs.clone(),
             segment_dir: segment_dir.to_string(),
             vectors: OnceLock::new(),
@@ -447,38 +479,45 @@ impl SegmentReader {
         })
     }
 
     /// M2-07：从 vectors.bin 加载并解码全部向量（OnceLock 闭包用）。
     /// 支持 v1（8 字节头 magic|version=1|payload）与 v2（12 字节头 magic|version=2|dim|payload）。
     /// v2 头由 M2-08 finalize 写入；本模块只读。payload 起始偏移随 version 变化。
     fn load_vectors(vfs: &dyn Vfs, segment_dir: &str) -> Result<Vec<f32>> {
         let vpath = format!("{}/vectors.bin", segment_dir);
         let vbuf = read_all(vfs, &vpath)?;
         if vbuf.len() < 8 || &vbuf[0..4] != crate::types::MAGIC {
-            return Err(VaneError::Corrupt("vectors.bin bad magic".into()));
+            return Err(VaneError::Corrupt(format!(
+                "vectors.bin bad magic{}",
+                seg_ctx(segment_dir, "load vectors")
+            )));
         }
         let version = u32::from_le_bytes(vbuf[4..8].try_into().unwrap());
         // M2-08：字面量 2 替换为 VECTORS_FORMAT_V2 常量。v1=VECTORS_FORMAT_V1，v2 含 dim 头。
         let payload_off = match version {
             v if v == crate::types::VECTORS_FORMAT_V1 => 8,
             v if v == crate::types::VECTORS_FORMAT_V2 => 12,
             _ => {
                 return Err(VaneError::Version(format!(
-                    "vectors.bin unsupported format_version: {} (expected {} or {})",
+                    "vectors.bin unsupported format_version: {} (expected {} or {}){}",
                     version,
                     crate::types::VECTORS_FORMAT_V1,
-                    crate::types::VECTORS_FORMAT_V2
+                    crate::types::VECTORS_FORMAT_V2,
+                    seg_ctx(segment_dir, "load vectors")
                 )));
             }
         };
         if vbuf.len() < payload_off {
-            return Err(VaneError::Corrupt("vectors.bin truncated".into()));
+            return Err(VaneError::Corrupt(format!(
+                "vectors.bin truncated{}",
+                seg_ctx(segment_dir, "load vectors")
+            )));
         }
         Ok(vbuf[payload_off..]
             .chunks_exact(4)
             .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
             .collect())
     }
 
     fn load_stored(
         vfs: &dyn Vfs,
         segment_dir: &str,
diff --git a/crates/vane-core/src/segment/tests.rs b/crates/vane-core/src/segment/tests.rs
index 55ed54e..3d63362 100644
--- a/crates/vane-core/src/segment/tests.rs
+++ b/crates/vane-core/src/segment/tests.rs
@@ -724,20 +724,58 @@ fn m2_07_open_rejects_truncated_v2_header() {
     sbytes.extend_from_slice(&0u32.to_le_bytes());
     vfs.write_at(&spath, &sbytes, 0).unwrap();
 
     let r = SegmentReader::open(&vfs, &seg_dir);
     assert!(
         matches!(r, Err(VaneError::Corrupt(ref m)) if m.contains("v2 header truncated")),
         "open should reject truncated v2 header, got err variant"
     );
 }
 
+/// M4 阶段五 c：VaneError 诊断上下文——SegmentReader::open 的错误 String
+/// 含段 ULID + 操作 + 建议操作（§10 推荐"先丰富 String"）。
+/// 断言非 vacuous：检 String 含 seg=ULID、op=open、建议关键词。
+#[test]
+fn m4_5c_open_error_contains_segment_context() {
+    use crate::types::VaneError;
+    let (vfs, seg_dir) = build_v1_segment(4, &[("a", &[1.0, 2.0, 3.0, 4.0])]);
+    // 提取段 ULID（seg_dir 末段 seg_<ulid>）
+    let ulid = seg_dir.rsplit('/').next().unwrap();
+    let ulid = ulid.strip_prefix("seg_").unwrap_or(ulid);
+
+    // corrupt vectors.bin magic → Corrupt error 含 seg=<ulid> + op=open
+    let vpath = format!("{}/vectors.bin", seg_dir);
+    let mut hdr = [0u8; 8];
+    let _ = vfs.read_at(&vpath, &mut hdr, 0).unwrap();
+    hdr[0] = b'X';
+    vfs.write_at(&vpath, &hdr, 0).unwrap();
+    let r = SegmentReader::open(&vfs, &seg_dir);
+    match r {
+        Err(VaneError::Corrupt(m)) => {
+            assert!(
+                m.contains("vectors.bin bad magic"),
+                "original message preserved: {}",
+                m
+            );
+            assert!(
+                m.contains(ulid),
+                "msg must contain segment ULID {}: {}",
+                ulid,
+                m
+            );
+            assert!(m.contains("op=open"), "msg must contain operation: {}", m);
+            assert!(m.contains("建议"), "msg must contain suggestion: {}", m);
+        }
+        other => panic!("expected Corrupt, got {:?}", other.err().map(|e| e.name())),
+    }
+}
+
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
diff --git a/crates/vane-core/src/tokenizer/jieba/dict.rs b/crates/vane-core/src/tokenizer/jieba/dict.rs
index 6681242..941d543 100644
--- a/crates/vane-core/src/tokenizer/jieba/dict.rs
+++ b/crates/vane-core/src/tokenizer/jieba/dict.rs
@@ -37,32 +37,44 @@ pub struct JiebaDict {
     max_freq: u32,
     base: Vec<i32>,
     check: Vec<i32>,
     values: Vec<i32>,
     hmm: HmmParams,
 }
 
 impl JiebaDict {
     /// 解析已解压的 dict.bin 字节（零分配解析头部，数组拷贝）。
     pub fn load(bytes: &[u8]) -> Result<Self> {
-        parse(bytes)
+        parse(bytes).map_err(|e| {
+            crate::types::append_context(
+                e,
+                " (op=dict load; 建议: 词典数据损坏，重新构建或联系支持)",
+            )
+        })
     }
 
     /// 解析 zstd 压缩的 dict.bin 字节（绑定层调用：Node/Go 加载 dict.bin 后调此）。
     pub fn load_zstd(compressed: &[u8]) -> Result<Self> {
         use std::io::Read;
-        let mut decoder = ruzstd::streaming_decoder::StreamingDecoder::new(compressed)
-            .map_err(|e| VaneError::Corrupt(format!("dict.bin zstd decompress failed: {}", e)))?;
+        let mut decoder = ruzstd::streaming_decoder::StreamingDecoder::new(compressed).map_err(|e| {
+            VaneError::Corrupt(format!(
+                "dict.bin zstd decompress failed: {} (op=dict load; 建议: 词典数据损坏，重新构建或联系支持)",
+                e
+            ))
+        })?;
         let mut buf = Vec::with_capacity(compressed.len() * 4);
-        decoder
-            .read_to_end(&mut buf)
-            .map_err(|e| VaneError::Corrupt(format!("dict.bin zstd read failed: {}", e)))?;
+        decoder.read_to_end(&mut buf).map_err(|e| {
+            VaneError::Corrupt(format!(
+                "dict.bin zstd read failed: {} (op=dict load; 建议: 词典数据损坏，重新构建或联系支持)",
+                e
+            ))
+        })?;
         Self::load(&buf)
     }
 
     /// 词典日历版本（如 "2026.08"），供 §12.3 三渠道一致性 + §3.3 升级警告。不进 TokenizerId。
     pub fn version(&self) -> &str {
         &self.dict_version
     }
 
     /// 词典内容 sha256 前 8 字节，供一致性校验。不进 TokenizerId。
     pub fn sha256_prefix(&self) -> [u8; 8] {
diff --git a/crates/vane-core/src/types.rs b/crates/vane-core/src/types.rs
index 50f5b9d..899088b 100644
--- a/crates/vane-core/src/types.rs
+++ b/crates/vane-core/src/types.rs
@@ -99,20 +99,40 @@ impl fmt::Display for VaneError {
             Self::Unsupported => write!(f, "E_UNSUPPORTED"),
             Self::InvalidArg(m) => write!(f, "E_INVALID_ARG: {}", m),
         }
     }
 }
 
 impl std::error::Error for VaneError {}
 
 pub type Result<T> = std::result::Result<T, VaneError>;
 
+/// M4 阶段五 c：VaneError 诊断上下文（§10 推荐"先丰富 String"）。
+///
+/// 为 VaneError 的 String payload 追加上下文后缀（段 ULID / docid / 操作 / 建议操作）。
+/// **不改错误码**（-1..-11 不变），不改 enum 签名（不加新字段），仅丰富 String 内容。
+/// 结构化上下文（独立字段）列为 Could，本任务用 String 丰富（§10 推荐路径）。
+///
+/// 无 String payload 的变体（Busy / DictTooLarge / DictUnavailable / Unsupported）原样返回。
+pub(crate) fn append_context(e: VaneError, ctx: &str) -> VaneError {
+    match e {
+        VaneError::Io(m) => VaneError::Io(format!("{}{}", m, ctx)),
+        VaneError::Schema(m) => VaneError::Schema(format!("{}{}", m, ctx)),
+        VaneError::NotFound(m) => VaneError::NotFound(format!("{}{}", m, ctx)),
+        VaneError::Corrupt(m) => VaneError::Corrupt(format!("{}{}", m, ctx)),
+        VaneError::Version(m) => VaneError::Version(format!("{}{}", m, ctx)),
+        VaneError::TokenizerMismatch(m) => VaneError::TokenizerMismatch(format!("{}{}", m, ctx)),
+        VaneError::InvalidArg(m) => VaneError::InvalidArg(format!("{}{}", m, ctx)),
+        other => other,
+    }
+}
+
 /// 检索结果文档（跨 bm25/vector-brute/fusion 模块）。
 #[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
 pub struct ScoredDoc {
     pub docid: u64,
     pub score: f32,
 }
 
 /// SPEC §3.1 向量距离度量。
 #[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
 pub enum Metric {
@@ -284,20 +304,57 @@ mod tests {
     }
 
     #[test]
     fn error_is_display_and_std_error() {
         let e = VaneError::InvalidArg("topK exceeds 1000".into());
         assert!(format!("{}", e).contains("topK exceeds 1000"));
         // std::error::Error trait 可调用 source()
         assert!(std::error::Error::source(&e).is_none());
     }
 
+    /// M4 阶段五 c：append_context 丰富 String payload 但不改错误码（§10）。
+    #[test]
+    fn append_context_enriches_string_preserves_code() {
+        let ctx = " (seg=01H, op=open; 建议: 检查)";
+        // 带 String 的变体：上下文追加到 payload，code 不变。
+        let cases = [
+            (VaneError::Io("bad".into()), -1i32),
+            (VaneError::Schema("mismatch".into()), -2),
+            (VaneError::NotFound("missing".into()), -3),
+            (VaneError::Corrupt("bad magic".into()), -4),
+            (VaneError::Version("v2".into()), -5),
+            (VaneError::TokenizerMismatch("tok".into()), -6),
+            (VaneError::InvalidArg("topK".into()), -11),
+        ];
+        for (e, code) in cases {
+            let enriched = append_context(e, ctx);
+            assert_eq!(enriched.code(), code, "code must not change for {}", code);
+            let msg = format!("{}", enriched);
+            assert!(
+                msg.contains("seg=01H"),
+                "msg must contain seg context: {}",
+                msg
+            );
+            assert!(
+                msg.contains("op=open"),
+                "msg must contain op context: {}",
+                msg
+            );
+            assert!(msg.contains("建议"), "msg must contain suggestion: {}", msg);
+        }
+        // 无 String 的变体：原样返回（code 不变，无 String 可丰富）。
+        assert_eq!(append_context(VaneError::Busy, ctx).code(), -9);
+        assert_eq!(append_context(VaneError::DictTooLarge, ctx).code(), -7);
+        assert_eq!(append_context(VaneError::DictUnavailable, ctx).code(), -8);
+        assert_eq!(append_context(VaneError::Unsupported, ctx).code(), -10);
+    }
+
     #[test]
     fn tokenizer_id_hex_roundtrip() {
         let raw = [0u8; 32];
         let id = TokenizerId(raw);
         let hex = id.to_hex();
         assert_eq!(hex.len(), 64);
         let back = TokenizerId::from_hex(&hex).unwrap();
         assert_eq!(back.as_bytes(), &raw);
     }
 
diff --git a/crates/vane-core/src/wal/mod.rs b/crates/vane-core/src/wal/mod.rs
index d88e8f6..eb8c874 100644
--- a/crates/vane-core/src/wal/mod.rs
+++ b/crates/vane-core/src/wal/mod.rs
@@ -59,22 +59,26 @@ impl Wal {
         // 幂等 create：已存在则忽略（Vfs::create 在已存在时返回 Io 错误，此处 best-effort）。
         let _ = vfs.create(&path);
         Ok(Self { vfs, path })
     }
 
     /// 追加一条记录（JSON 行，每行一条；append 后 sync 保证崩溃前落盘）。
     pub fn append(&self, record: &WalRecord) -> Result<()> {
         // M4 §3.5 tracing：WAL append 次数——记录值（Debug）。cfg 门控，编译期消除。
         #[cfg(feature = "tracing")]
         tracing::debug!(?record, "wal append");
-        let mut line = serde_json::to_vec(record)
-            .map_err(|e| VaneError::Corrupt(format!("wal serialize: {}", e)))?;
+        let mut line = serde_json::to_vec(record).map_err(|e| {
+            VaneError::Corrupt(format!(
+                "wal serialize: {} (path={}, op=wal append; 建议: 检查 wal.log 完整性或重新操作)",
+                e, self.path
+            ))
+        })?;
         line.push(b'\n');
         self.vfs.append(&self.path, &line)?;
         self.vfs.sync(&self.path)?;
         Ok(())
     }
 
     /// 读取全部记录（崩溃恢复用）。文件不存在（新库）返回空。
     pub fn read_all(&self) -> Result<Vec<WalRecord>> {
         let mut buf = Vec::new();
         let mut tmp = [0u8; 8192];
@@ -89,22 +93,26 @@ impl Wal {
                 break;
             }
             buf.extend_from_slice(&tmp[..n]);
             off += n as u64;
         }
         let mut records = Vec::new();
         for line in buf.split(|b| *b == b'\n') {
             if line.is_empty() {
                 continue;
             }
-            let r: WalRecord = serde_json::from_slice(line)
-                .map_err(|e| VaneError::Corrupt(format!("wal parse: {}", e)))?;
+            let r: WalRecord = serde_json::from_slice(line).map_err(|e| {
+                VaneError::Corrupt(format!(
+                    "wal parse: {} (path={}, op=wal recover; 建议: wal.log 损坏，检查崩溃恢复或清除 wal.log 重试)",
+                    e, self.path
+                ))
+            })?;
             records.push(r);
         }
         Ok(records)
     }
 
     /// **仅 compact/merge 成功 + manifest 切换后调用**（B-2 修复）。
     ///
     /// flush 路径**不**调 truncate——否则 `flush→delete→flush→崩溃` 会丢失未消费的
     /// `AddTombstone`（tombstone 仅存 WAL，02 不改 header.bin），致已删文档复活（数据损坏）。
     /// WAL 累积 AddSegment 记录直到 compact（ULID 字符串体积可忽略），compact 后一次性清空。
diff --git a/crates/vane-core/src/wal/tests.rs b/crates/vane-core/src/wal/tests.rs
index 3dfed3f..7e047a0 100644
--- a/crates/vane-core/src/wal/tests.rs
+++ b/crates/vane-core/src/wal/tests.rs
@@ -174,10 +174,31 @@ fn recover_cleans_orphan_segment_dir_not_in_wal() {
 #[test]
 fn recover_empty_segments_dir_no_error() {
     use crate::persistence::Manifest;
 
     let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn crate::vfs::Vfs>;
     let manifest = Manifest::empty();
     // segments 目录不存在（新库）。
     let _tombstones = recover(&vfs, "db", &manifest).unwrap();
     // 无异常即通过。
 }
+
+/// M4 阶段五 c：VaneError 诊断上下文——wal parse 错误 String 含
+/// wal 路径 + 操作 + 建议操作（§10 推荐"先丰富 String"）。
+#[test]
+fn m4_5c_wal_parse_error_contains_context() {
+    use crate::types::VaneError;
+    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn crate::vfs::Vfs>;
+    // 写损坏的 wal.log（非 JSON 行）
+    vfs.create("mydb/wal.log").unwrap();
+    vfs.write_at("mydb/wal.log", b"not json {{{\n", 0).unwrap();
+    let wal = Wal::open(vfs, "mydb").unwrap();
+    match wal.read_all() {
+        Err(VaneError::Corrupt(m)) => {
+            assert!(m.contains("wal parse"), "original msg preserved: {}", m);
+            assert!(m.contains("mydb"), "msg must contain path: {}", m);
+            assert!(m.contains("op=wal recover"), "msg must contain op: {}", m);
+            assert!(m.contains("建议"), "msg must contain suggestion: {}", m);
+        }
+        other => panic!("expected Corrupt, got {:?}", other.map_err(|e| e.name())),
+    }
+}
diff --git a/docs/plans/m4/task-diagnostics-report.md b/docs/plans/m4/task-diagnostics-report.md
new file mode 100644
index 0000000..ef658d5
--- /dev/null
+++ b/docs/plans/m4/task-diagnostics-report.md
@@ -0,0 +1,242 @@
+# M4 阶段五 c：VaneError 诊断上下文——实施报告
+
+> 来源：Phase 5c implementer SubAgent（sonnet / bg）。
+> 设计依据：`docs/plans/m4/phase0-design.md` §10（错误码——诊断加什么）+ `docs/plans/m4/M4-PLAN.md` 阶段五 3。
+> SPEC 依据：`docs/SPEC.md` §10 错误码表（-1..-11 不变——只读核对，未改 SPEC）。
+
+## 1. 任务摘要
+
+丰富 VaneError 的 String payload，附上下文（段 ULID / docid / 操作 / 建议操作）。**不改错误码**（-1..-11 不变），**不改 VaneError enum 签名**（不加新字段，避免碰冻结 API），仅丰富 String 内容（§10 推荐"先丰富 String"路径）。
+
+## 2. 丰富的 VaneError 构造点清单
+
+### 2.1 段级 open 路径（segment/mod.rs）
+
+| 构造点 | 原消息 | 丰富后附加上下文 |
+|---|---|---|
+| `SegmentReader::open` → `decode_header` 调用 | `header too short` / `bad magic` / `unsupported format_version` 等 | 经 `append_context` 追加 `(seg=<ulid>, op=open header.bin; 建议: 检查段文件完整性或从备份恢复)` |
+| `SegmentReader::open` → vectors.bin bad magic | `vectors.bin bad magic` | 追加 `(seg=<ulid>, op=open vectors.bin; 建议: ...)` |
+| `SegmentReader::open` → vectors.bin v2 header truncated | `vectors.bin v2 header truncated (need 12 bytes)` | 追加 `(seg=<ulid>, op=open vectors.bin; 建议: ...)` |
+| `SegmentReader::open` → vectors.bin unsupported version | `vectors.bin unsupported format_version: {} (expected {} or {})` | 追加 `(seg=<ulid>, op=open vectors.bin; 建议: ...)` |
+| `SegmentReader::open` → stored.bin bad magic | `stored.bin bad magic` | 追加 `(seg=<ulid>, op=open stored.bin; 建议: ...)` |
+| `SegmentReader::open` → stored.bin unsupported version | `stored.bin unsupported format_version: {} (expected {} or {})` | 追加 `(seg=<ulid>, op=open stored.bin; 建议: ...)` |
+| `load_vectors` → vectors.bin bad magic | `vectors.bin bad magic` | 追加 `(seg=<ulid>, op=load vectors; 建议: ...)` |
+| `load_vectors` → vectors.bin unsupported version | `vectors.bin unsupported format_version: {}` | 追加 `(seg=<ulid>, op=load vectors; 建议: ...)` |
+| `load_vectors` → vectors.bin truncated | `vectors.bin truncated` | 追加 `(seg=<ulid>, op=load vectors; 建议: ...)` |
+
+**辅助函数**（新增 pub(crate)）：
+- `segment_ulid_from_dir(segment_dir) -> &str`：从 `seg_<ulid>` 路径末段提取 ULID。
+- `seg_ctx(segment_dir, op) -> String`：构造段级诊断上下文后缀。
+
+### 2.2 倒排索引 open 路径（bm25.rs）
+
+| 构造点 | 原消息 | 丰富后附加上下文 |
+|---|---|---|
+| `InvertedIndexReader::open` → inverted.bin truncated header | `inverted.bin truncated header: {}` | 追加 `(seg=<ulid>, op=open inverted.bin; 建议: ...)` |
+| `InvertedIndexReader::open` → inverted.bin bad magic | `inverted.bin bad magic` | 追加 `(seg=<ulid>, op=open inverted.bin; 建议: ...)` |
+| `InvertedIndexReader::open` → inverted.bin version mismatch | `inverted.bin version {} != supported {}` | 追加 `(seg=<ulid>, op=open inverted.bin; 建议: ...)` |
+
+### 2.3 manifest 路径（persistence/mod.rs）
+
+| 构造点 | 原消息 | 丰富后附加上下文 |
+|---|---|---|
+| `ManifestStore::load` → manifest parse | `manifest parse: {}` | 追加 `(db={}, op=load manifest; 建议: 检查 manifest.json 完整性或从备份恢复)` |
+| `ManifestStore::save_atomic` → manifest serialize | `manifest serialize: {}` | 追加 `(db={}, op=save manifest; 建议: 重试或检查磁盘空间)` |
+| `ManifestStore::add_segment` → collection not found | `collection not found: {}` | 追加 `(db={}, seg={}, op=add_segment; 建议: 确认 collection 名称正确)` |
+
+### 2.4 WAL 路径（wal/mod.rs）
+
+| 构造点 | 原消息 | 丰富后附加上下文 |
+|---|---|---|
+| `Wal::append` → wal serialize | `wal serialize: {}` | 追加 `(path={}, op=wal append; 建议: 检查 wal.log 完整性或重新操作)` |
+| `Wal::read_all` → wal parse | `wal parse: {}` | 追加 `(path={}, op=wal recover; 建议: wal.log 损坏，检查崩溃恢复或清除 wal.log 重试)` |
+
+### 2.5 词典加载路径（tokenizer/jieba/dict.rs）
+
+| 构造点 | 原消息 | 丰富后附加上下文 |
+|---|---|---|
+| `JiebaDict::load` → 所有 parse 错误（magic/version/too short 等） | 各 leaf 错误 | 经 `append_context` 追加 `(op=dict load; 建议: 词典数据损坏，重新构建或联系支持)` |
+| `JiebaDict::load_zstd` → zstd decompress failed | `dict.bin zstd decompress failed: {}` | 追加 `(op=dict load; 建议: ...)` |
+| `JiebaDict::load_zstd` → zstd read failed | `dict.bin zstd read failed: {}` | 追加 `(op=dict load; 建议: ...)` |
+
+### 2.6 搜索路径（api/collection.rs）
+
+| 构造点 | 原消息 | 丰富后附加上下文 |
+|---|---|---|
+| `run_search` → topK exceeds max | `topK {} exceeds max {}` | 追加 `(op=search, collection={}; 建议: 减小 topK 至 {} 以内)` |
+| `run_search` → search requires text or vector | `search requires text or vector` | 追加 `(op=search; 建议: 提供 text 或 vector 查询参数)` |
+| `run_search` → query vector dim mismatch | `query vector dim {} != schema dim {}` | 追加 `(op=search, collection={}; 建议: 对齐 query vector 维度与 schema 声明)` |
+
+### 2.7 文档添加路径（api/collection.rs）
+
+| 构造点 | 原消息 | 丰富后附加上下文 |
+|---|---|---|
+| `add` → vector dim mismatch | `vector dim mismatch: got {} expected {}` | 追加 `(op=add, collection={}, doc_id={}; 建议: 对齐 doc vector 维度与 schema 声明)` |
+
+### 2.8 段合并路径（api/collection.rs + merge/mod.rs）
+
+| 构造点 | 原消息 | 丰富后附加上下文 |
+|---|---|---|
+| `merge_segments` → collection not in manifest | `collection not in manifest: {}` | 追加 `(op=merge, db={}; 建议: 确认 collection 已创建)` |
+| `finalize_merge` → no steps | `finalize_merge with no steps` | 追加 `(op=merge; 建议: 检查 merge 调用序列)` |
+
+### 2.9 重建索引路径（api/reindex.rs）
+
+| 构造点 | 原消息 | 丰富后附加上下文 |
+|---|---|---|
+| `update_manifest_after_reindex` → collection not in manifest | `collection not in manifest: {}` | 追加 `(op=reindex; 建议: 确认 collection 已创建)` |
+
+### 2.10 DB 打开路径（api/db.rs）
+
+| 构造点 | 原消息 | 丰富后附加上下文 |
+|---|---|---|
+| `Db::collection` → schema mismatch | `collection '{}' exists with different schema` | 追加 `(op=open collection; 建议: 使用相同 schema 或新 collection 名称)` |
+| `Db::collection` → tokenizer mismatch | `collection '{}' exists with different tokenizer` | 追加 `(op=open collection; 建议: 使用相同 tokenizer 或新 collection 名称)` |
+
+### 2.11 共享辅助（types.rs）
+
+新增 `pub(crate) fn append_context(e: VaneError, ctx: &str) -> VaneError`：
+- 为带 String payload 的变体（Io/Schema/NotFound/Corrupt/Version/TokenizerMismatch/InvalidArg）追加 `ctx` 后缀。
+- 无 String payload 的变体（Busy/DictTooLarge/DictUnavailable/Unsupported）原样返回。
+- 不改错误码（code() 返回值不变）。
+
+## 3. 错误码未变确认
+
+SPEC §10 错误码表 -1..-11 全部不变：
+- `VaneError::Io` → -1, `Schema` → -2, `NotFound` → -3, `Corrupt` → -4, `Version` → -5,
+  `TokenizerMismatch` → -6, `DictTooLarge` → -7, `DictUnavailable` → -8, `Busy` → -9,
+  `Unsupported` → -10, `InvalidArg` → -11。
+- `types.rs` 现有 `error_code_matches_spec` 测试全绿（未改 code() 实现）。
+- `append_context_enriches_string_preserves_code` 新测试验证丰富后 code() 不变。
+
+## 4. VaneError 签名未改确认
+
+- `VaneError` enum 11 个变体签名完全不变（Io(String) / Schema(String) / ... / InvalidArg(String)）。
+- 未加新字段、未加新变体、未改 Display impl、未改 Error impl。
+- 仅新增 `pub(crate) fn append_context`（crate 内部辅助，非 pub API surface）。
+- `code()` / `name()` 方法不变。
+
+## 5. 测试断言更新清单
+
+### 5.1 新增测试（4 处）
+
+| 测试 | 文件 | 验证内容 |
+|---|---|---|
+| `append_context_enriches_string_preserves_code` | `types.rs` | `append_context` 追加上下文后 code() 不变 + String 含 seg/op/建议关键词 |
+| `m4_5c_open_error_contains_segment_context` | `segment/tests.rs` | SegmentReader::open vectors.bin bad magic 错误含段 ULID + op=open + 建议 |
+| `m4_5c_manifest_parse_error_contains_context` | `persistence/tests.rs` | manifest parse 错误含 db 路径 + op=load manifest + 建议 |
+| `m4_5c_wal_parse_error_contains_context` | `wal/tests.rs` | wal parse 错误含 wal 路径 + op=wal recover + 建议 |
+
+### 5.2 现有测试未需更新
+
+- `segment/tests.rs` 现有 `contains("vectors.bin bad magic")` / `contains("stored.bin bad magic")` / `contains("v2 header truncated")` 断言：**仍通过**（丰富是 ADDITIVE——原消息保留为子串）。
+- `header.rs` `contains("too short")` 断言：**仍通过**（decode_header 本身未改，仅调用点 wrap）。
+- `types.rs` `contains("topK exceeds 1000")` 断言：**仍通过**（该测试自构造字面量字符串，不经搜索路径）。
+- `crash_recovery.rs` `contains("manifest.json.tmp")` / `contains("inverted.bin")` / `contains("ENOSPC")` / `contains("partial write")` 断言：**仍通过**（这些错误来自 FaultVfs 注入的 msg，经 `?` 传播不改——我的丰富仅在 CONSTRUCTION 点，不在传播点）。
+
+## 6. 各门禁真实输出
+
+### 6.1 cargo fmt
+
+```
+$ cargo fmt --all -- --check
+（无 diff 输出——格式检查通过）
+```
+
+### 6.2 cargo clippy
+
+```
+$ cargo clippy --all-targets --all-features --workspace --exclude vane-fuzz -- -D warnings
+    Checking vane-core v0.2.0
+    Checking vane-wasm v0.2.0
+    Checking vane-dict-zh v2026.8.0
+    Checking vane-ffi v0.2.0
+    Checking vane-node v0.2.0
+    Finished `dev` profile [unoptimized + debuginfo] target(s) in 5.61s
+```
+
+### 6.3 cargo test
+
+```
+$ cargo test --workspace --all-features --exclude vane-fuzz
+test result: ok. 346 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 1.67s
+（vane-core 单元测试 346 个——含 4 新测试——全绿；集成测试含 crash_recovery/cross_version等 全绿 0 failed）
+```
+
+关键：crash_recovery.rs 5 场景 FaultVfs 注入测试全绿——确认丰富不破坏现有错误消息断言。
+
+### 6.4 cargo deny check
+
+```
+$ cargo deny check
+advisories ok, bans ok, licenses ok, sources ok
+```
+（regex wrapper 警告为 pre-existing，与本次改动无关——无新依赖引入。）
+
+### 6.5 wasm32 check
+
+```
+$ cargo check --target wasm32-unknown-unknown -p vane-core
+    Checking vane-core v0.2.0
+    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.18s
+```
+（VaneError 丰富未引 std::fs——`append_context` 仅用 `format!`，无平台分支。）
+
+## 7. commit
+
+```
+分支：feat/m4-prod-readiness
+提交信息：feat(core): VaneError 诊断上下文（String 丰富，不改错误码）（M4 阶段五 c）
+```
+
+commit 含：
+- `types.rs`：`append_context` 辅助 + 测试
+- `segment/mod.rs`：`segment_ulid_from_dir` / `seg_ctx` pub(crate) 辅助 + SegmentReader::open / load_vectors 丰富
+- `segment/tests.rs`：段级诊断上下文测试
+- `bm25.rs`：InvertedIndexReader::open 丰富
+- `persistence/mod.rs`：manifest load/save/add_segment 丰富
+- `persistence/tests.rs`：manifest 诊断上下文测试
+- `wal/mod.rs`：wal append/read_all 丰富
+- `wal/tests.rs`：wal 诊断上下文测试
+- `tokenizer/jieba/dict.rs`：dict load 路径丰富
+- `api/collection.rs`：search/add/merge 丰富
+- `api/db.rs`：collection schema/tokenizer mismatch 丰富
+- `api/reindex.rs`：reindex NotFound 丰富
+- `merge/mod.rs`：finalize_merge InvalidArg 丰富
+
+不含：SPEC.md / CI yml / fault.rs / crash_recovery.rs / vane-fuzz / proptest / cross_version / tracing 埋点 / inspect API。
+
+## 8. 自审
+
+### 8.1 覆盖度
+
+**已覆盖的关键路径**：
+- ✅ open（SegmentReader::open + InvertedIndexReader::open + decode_header wrap）
+- ✅ flush（经 SegmentWriter/write_inverted/manifest 传播——leaf 丰富已覆盖）
+- ✅ merge（finalize_merge InvalidArg + merge_segments NotFound + 经 leaf 传播）
+- ✅ search（topK / dim mismatch / missing text+vector）
+- ✅ reindex（reindex_segment NotFound）
+- ✅ dict load（JiebaDict::load + load_zstd）
+- ✅ manifest（load / save_atomic / add_segment）
+- ✅ WAL（append / read_all）
+- ✅ DB open（collection schema/tokenizer mismatch）
+- ✅ add（vector dim mismatch with doc_id）
+
+**defer 的路径（非关键/低优先）**：
+- segment/mod.rs 中 decode_kv_map / decode_stored / decode_scalars 的 ~30 个 leaf "too short" 错误——这些在 decode 函数内（无 segment_dir 上下文），且经 load_stored/load_id_map 传播时已有段级错误捕获。丰富这些 leaf 错误 ROI 低，defer。
+- segment/mod.rs SegmentWriter add_doc/set_text/set_scalar 的 Schema 错误——这些是调用方编程错误（非运行时损坏），消息已含足够诊断信息（如 "field '{}' not a scalar field"），defer。
+- bm25.rs InvertedIndexReader::open 后续的 ~15 个 "truncated term_len/vbyte/tf/docid" 错误——这些在已校验 magic+version 后的深层 decode，属罕见损坏路径，defer。
+- hmm.rs 的 5 个 "hmm_blob too short" 错误——经 dict.rs parse 间接调用，已有 `load` 层 wrap 覆盖，defer。
+- api/snapshot.rs 的 ~10 个 snapshot Corrupt/Version 错误——snapshot 导入导出路径非核心 flush/merge/search 路径，defer。
+
+### 8.2 String 丰富 vs 结构化上下文取舍
+
+§10 注释"结构化上下文列为 Could"——本任务用 String 丰富（§10 推荐"先丰富 String"路径）。
+
+**选择理由**：
+- String 丰富不改 enum 签名（避免碰冻结 API），不改 FFI 序列化（VaneError 经 FFI 只透传 code + Display 字符串）。
+- 结构化上下文需加新字段（如 `struct ErrorContext { ulid, docid, op, suggestion }`）→ 改 enum 签名 → 破坏冻结 API → 需 SPEC 修订（Phase 6）。
+- String 丰富立即可用，用户经 `vane_last_error_message()` 即可见上下文。
+
+**defer 到结构化（Phase 6 SPEC 修订后）**：
+- 若未来需程序化解析上下文（如 FFI 层提取 ULID 做自动重试），再加 `VaneError::context() -> Option<ErrorContext>` 方法（不改 enum 签名，只加方法）。当前 String 足够。
