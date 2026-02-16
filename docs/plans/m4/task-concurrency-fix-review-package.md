## Commits 354f66e..cedbb17 (Phase 4 fix)

cedbb17 fix(core): 并发 flush manifest 损坏 + auto-merge 竞争 double-count（M4 Phase 4 fix）

## Diff stat

 crates/vane-core/src/api/collection.rs       | 195 +++++++++++-----
 crates/vane-core/src/api/db.rs               |  36 ++-
 crates/vane-core/src/api/reindex.rs          |  38 ++--
 crates/vane-core/src/persistence/mod.rs      |  42 +++-
 crates/vane-core/tests/stress_concurrency.rs | 328 +++++++++++++++++++++++----
 docs/plans/m4/task-concurrency-fix-report.md | 174 ++++++++++++++
 6 files changed, 682 insertions(+), 131 deletions(-)

## Full diff (U10)

diff --git a/crates/vane-core/src/api/collection.rs b/crates/vane-core/src/api/collection.rs
index b39c79b..706a537 100644
--- a/crates/vane-core/src/api/collection.rs
+++ b/crates/vane-core/src/api/collection.rs
@@ -38,20 +38,23 @@ pub(crate) struct CollectionInner {
     pub(crate) name: String,
     pub(crate) schema: Schema,
     // 06：reindex 需原子替换 tokenizer/tokenizer_id，包 RwLock 提供 interior mutability。
     pub(crate) tokenizer: RwLock<Arc<dyn crate::tokenizer::Tokenizer>>,
     pub(crate) tokenizer_id: RwLock<CoreTokenizerId>,
     /// 分词器种类（SPEC §5.1）。reindex 用此 + pending_dict 重建新分词器。
     pub(crate) tokenizer_kind: BuiltinTokenizer,
     vfs: Arc<dyn Vfs>,
     db_path: String,
     segments_dir: String,
+    // M4 Phase 4 fix（Bug 1）：共享 DbInner 的 Arc<ManifestStore>，跨线程 / 跨 collection
+    // 共用同一 save_lock → 并发 flush/merge 的 manifest 原子保存被序列化。
+    manifest_store: Arc<ManifestStore>,
     write_state: Mutex<WriteState>,
     // M4 §3.6 inspect API：pub(crate) 供 inspect 模块遍历段快照读元数据。
     pub(crate) snapshot: RwLock<Vec<Arc<SegmentReader>>>,
     // 段 ULID → 全局 docid 基址
     seg_offsets: RwLock<HashMap<String, u64>>,
     // I7：InvertedIndexReader 随段快照缓存，search 直接用，避免每次重开
     inverted_readers: RwLock<Vec<Arc<InvertedIndexReader>>>,
     // 01-hnsw：HnswReader 随段快照缓存。Option 因 M0 段无 hnsw.bin（Q-5 → fallback brute）。
     // M4 §3.6 inspect API：pub(crate) 供 inspect 模块检测 hnsw 缺失（Degraded）。
     pub(crate) hnsw_readers: RwLock<Vec<Option<Arc<HnswReader>>>>,
@@ -179,20 +182,21 @@ impl CollectionInner {
         let segments_dir = format!("{}/segments", db.db_path);
         Ok(Self {
             name: name.to_string(),
             schema: meta.schema,
             tokenizer: RwLock::new(tokenizer),
             tokenizer_id: RwLock::new(meta.tokenizer_id),
             tokenizer_kind: meta.tokenizer_kind,
             vfs: db.vfs.clone(),
             db_path: db.db_path.clone(),
             segments_dir,
+            manifest_store: db.manifest_store.clone(),
             write_state: Mutex::new(WriteState {
                 buffer: Vec::new(),
                 auto_committer: AutoCommitter::new(auto_commit),
                 next_docid: 0,
             }),
             snapshot: RwLock::new(Vec::new()),
             seg_offsets: RwLock::new(HashMap::new()),
             inverted_readers: RwLock::new(Vec::new()),
             hnsw_readers: RwLock::new(Vec::new()),
             scalar_readers: RwLock::new(Vec::new()),
@@ -309,21 +313,40 @@ impl Collection {
         // 06：Rebuilding 期写路径 E_BUSY（Q-6）。
         if *self.inner.dict_state.read().unwrap() == DictState::Rebuilding {
             return Err(VaneError::Busy);
         }
         let mut state = self.inner.write_state.lock().unwrap();
         if state.buffer.is_empty() {
             state.auto_committer.reset();
             return Ok(());
         }
         let docs = std::mem::take(&mut state.buffer);
-        let base_docid = docs.first().map(|d| d.docid).unwrap_or(0);
+        // M4 Phase 4 fix Bug 2：base_docid 选择——检测缓冲文档 docid 是否连续。
+        // - 连续（无并发 merge 在 add 之间 bump next_docid）→ 用首文档 docid 作 base
+        //   （保持 inspect base=0 语义；merge 的 target_docid_base 已并入 next_docid，
+        //   读 next_docid=first+count 后 target_base >= first+count，新段在本段之上）。
+        // - 非连续（并发 merge 在 add 之间 bump 了 next_docid，docid 有 gap）→
+        //   用首文档 stale docid 作 base 写连续 [base, base+count) 会与 merge 新段
+        //   重叠 → rebase 到当前 next_docid（merge 已 bump 到新段末尾之上）。
+        let count = docs.len() as u64;
+        let first_docid = docs.first().map(|d| d.docid).unwrap_or(0);
+        let contiguous = docs
+            .iter()
+            .enumerate()
+            .all(|(i, d)| d.docid == first_docid + i as u64);
+        let base_docid = if contiguous {
+            first_docid
+        } else {
+            let base = state.next_docid;
+            state.next_docid = base + count;
+            base
+        };
         state.auto_committer.reset();
         drop(state);
 
         // 构建 SegmentWriter（I4/FF2 裁决：传入真实全局 docid 基址）
         let tok_id = self.inner.tokenizer_id.read().unwrap().clone();
         let mut writer = SegmentWriter::new(
             self.inner.vfs.clone(),
             &self.inner.segments_dir,
             &self.inner.schema,
             &tok_id,
@@ -423,23 +446,25 @@ impl Collection {
         };
 
         // 04-wal：段文件集（header/vectors/inverted/hnsw/scalars）已全部 sync →
         // append AddSegment（SPEC §6.4：WAL → manifest rename）。B-2：flush 不 truncate。
         let wal = crate::wal::Wal::open(self.inner.vfs.clone(), &self.inner.db_path)?;
         wal.append(&crate::wal::WalRecord::AddSegment {
             collection: self.inner.name.clone(),
             ulid: meta.ulid.clone(),
         })?;
 
-        // 更新 manifest（I-6 原子切换）
-        let manifest_store = ManifestStore::new(self.inner.vfs.clone(), &self.inner.db_path);
-        manifest_store.add_segment(&self.inner.name, &meta.ulid)?;
+        // 更新 manifest（I-6 原子切换）。M4 Phase 4 fix Bug 1：复用共享
+        // Arc<ManifestStore>，add_segment 内部 save_lock 序列化并发原子切换。
+        self.inner
+            .manifest_store
+            .add_segment(&self.inner.name, &meta.ulid)?;
 
         // 更新段快照（Arc swap 语义：写锁替换 Vec）
         // I7：open 一次 InvertedIndexReader 并缓存
         let inv_reader = Arc::new(InvertedIndexReader::open(&self.inner.vfs, &seg_dir)?);
         // 03-pre-filter：缓存 ScalarReader。
         let scalar_reader = Arc::new(crate::segment::ScalarReader::open(
             &self.inner.vfs,
             &seg_dir,
         )?);
         {
@@ -476,21 +501,54 @@ impl Collection {
         );
         Ok(())
     }
 
     /// 当前段数（测试与诊断用）。
     pub fn segment_count(&self) -> usize {
         self.inner.snapshot.read().unwrap().len()
     }
 
     /// 选最小两段合并（auto-merge on exceeding SEGMENT_MAX，SPEC §3.3）。
+    ///
+    /// M4 Phase 4 fix（Bug 2）：auto-merge 原先不获取 compacting 锁，与并发
+    /// compact/reindex 竞争 → 段未正确移除 → double-count。现 try-lock compacting：
+    /// 若并发 compact/reindex 已持锁 → skip（return Ok，auto-merge 是 best-effort
+    /// 优化，skip 安全降级，下次 flush 段数仍超阈值时再 merge）。复用 compact/reindex
+    /// 的 CompactingGuard（M-minor-1 panic-safe Drop 复位 false，避免一次 panic 致
+    /// 永久 E_BUSY）。guard 不持其他锁——Drop 时重新取 compacting 锁复位（与原模式等价）。
     fn auto_merge_two_smallest(&self) -> Result<()> {
+        // try-lock compacting：WouldBlock → skip；Poisoned → 恢复（设 true 继续 merge）。
+        {
+            let mut guard = match self.inner.compacting.try_lock() {
+                Ok(g) => g,
+                Err(std::sync::TryLockError::WouldBlock) => {
+                    // 并发 compact/reindex 持锁 → skip（best-effort 降级）
+                    return Ok(());
+                }
+                Err(std::sync::TryLockError::Poisoned(e)) => {
+                    // 上次 panic 致锁中毒：恢复（取 guard 继续），guard drop 复位。
+                    e.into_inner()
+                }
+            };
+            if *guard {
+                // 理论上 try_lock 成功时 *guard=false（正常路径）；Poisoned 恢复路径
+                // 若残留 true → skip（保守降级，下次再 merge）。
+                return Ok(());
+            }
+            *guard = true;
+            // 显式 drop MutexGuard：CompactingGuard 的 Drop 会重新取锁复位，
+            // 此处先释放避免后续 merge_segments 取其他锁时的持有顺序歧义
+            // （与 compact/reindex 的 finally 模式一致）。
+        }
+        let _cg = CompactingGuard {
+            flag: &self.inner.compacting,
+        };
         let snap = self.inner.snapshot.read().unwrap().clone();
         if snap.len() < 2 {
             return Ok(());
         }
         let tombstones = self.inner.tombstones.read().unwrap().clone();
         let ratios: Vec<(String, f32)> = snap
             .iter()
             .map(|r| {
                 let total = r.doc_count().max(1) as f32;
                 let t = tombstones
@@ -509,37 +567,66 @@ impl Collection {
     }
 
     /// 合并指定段 ULID 列表为单个新段，更新 manifest + 内存快照 + 删旧段。
     fn merge_segments(&self, source_ulids: Vec<String>) -> Result<()> {
         if source_ulids.is_empty() {
             return Ok(());
         }
         let offsets = self.inner.seg_offsets.read().unwrap().clone();
         let tombstones = self.inner.tombstones.read().unwrap().clone();
         let snap = self.inner.snapshot.read().unwrap().clone();
-        // target_docid_base 选择（02-review B-2 修复）：
+        // target_docid_base 选择（02-review B-2 修复 + M4 Phase 4 fix Bug 2）：
         // - compact 全合并（source 覆盖全部段，无保留段）：0（从 0 起合理）。
-        // - partial auto-merge（合并 2/N 段）：max(保留段 base + count)，
+        // - partial auto-merge（合并 2/N 段）：max(保留段 base + count, next_docid)，
         //   新段 docid 从所有保留段的最大 docid 之后开始，避免与任何保留段
         //   docid 空间重叠（否则 search 回填误命中、fusion 去重丢文档）。
+        //   M4 Phase 4 fix Bug 2：并入 next_docid——next_docid 是 add 已分配但
+        //   尚未 flush 的缓冲文档 docid 上界。并发 flush 的缓冲 docid 在
+        //   [old_next_docid, next_docid) 区间，若 merge 的 target_base 不计入
+        //   next_docid，新段会与 about-to-flush 的缓冲段 docid 重叠 → fusion
+        //   去重丢文档 + 回填误命中 → search double-count/missing。
         let is_full_merge = snap.iter().all(|r| source_ulids.contains(&r.meta().ulid));
-        let target_docid_base = if is_full_merge {
-            0
-        } else {
-            snap.iter()
-                .filter(|r| !source_ulids.contains(&r.meta().ulid))
-                .map(|r| {
-                    let base = offsets.get(&r.meta().ulid).copied().unwrap_or(0);
-                    base + r.doc_count() as u64
-                })
-                .max()
-                .unwrap_or(0)
+        let max_non_source_end: u64 = snap
+            .iter()
+            .filter(|r| !source_ulids.contains(&r.meta().ulid))
+            .map(|r| {
+                let base = offsets.get(&r.meta().ulid).copied().unwrap_or(0);
+                base + r.doc_count() as u64
+            })
+            .max()
+            .unwrap_or(0);
+        // estimate = source 段 doc_count 之和（上界，tombstone 清除后实际
+        // new_count <= estimate；多预留的区间留空，无危害）。
+        let estimated_new_count: u64 = snap
+            .iter()
+            .filter(|r| source_ulids.contains(&r.meta().ulid))
+            .map(|r| r.doc_count() as u64)
+            .sum();
+        // M4 Phase 4 fix Bug 2：**原子地**读 next_docid + 算 target_docid_base +
+        // bump next_docid 到 reserved_end（一次 write_state lock 内完成）。若分两步
+        // （先读 next_docid 释放锁，再 bump），并发 add 会在两步之间分配 docid =
+        // 旧 next_docid，而 merge 的 target_base = 旧 next_docid → 新段与 about-to-
+        // flush 的缓冲段 docid 重叠。原子 read+bump 消除该窗口：并发 add 在本块
+        // 之前/之后拿锁，看到的 next_docid 都已 bump 到 reserved_end，其分配的
+        // docid 推到 merge 新段之后。
+        let target_docid_base = {
+            let mut state = self.inner.write_state.lock().unwrap();
+            let tdb = if is_full_merge {
+                0
+            } else {
+                max_non_source_end.max(state.next_docid)
+            };
+            let reserved_end = tdb + estimated_new_count;
+            if reserved_end > state.next_docid {
+                state.next_docid = reserved_end;
+            }
+            tdb
         };
         // M4 §3.5 tracing：merge 频率——入口埋点（sources 数 + target base + 是否全合并）。
         #[cfg(feature = "tracing")]
         tracing::info!(
             collection = %self.inner.name,
             sources = source_ulids.len(),
             target_docid_base,
             full_merge = is_full_merge,
             "merge start"
         );
@@ -566,52 +653,53 @@ impl Collection {
         // docid 与新段 [target_docid_base, target_docid_base + new_count) 重叠。
         // compact 全合并 base=0 不受影响（next_docid 保持 stale-high，详见 02-review 维度 8a）。
         if !is_full_merge {
             let new_end = target_docid_base + new_meta.doc_count as u64;
             let mut state = self.inner.write_state.lock().unwrap();
             if new_end > state.next_docid {
                 state.next_docid = new_end;
             }
         }
 
-        // 更新 manifest（I-6）。
-        let manifest_store = ManifestStore::new(self.inner.vfs.clone(), &self.inner.db_path);
-        let mut manifest = manifest_store
-            .load()?
-            .unwrap_or_else(crate::persistence::Manifest::empty);
-        let col_meta = manifest
-            .collections
-            .get_mut(&self.inner.name)
-            .ok_or_else(|| {
-                VaneError::NotFound(format!(
-                    "collection not in manifest: {} (op=merge, db={}; 建议: 确认 collection 已创建)",
-                    self.inner.name, self.inner.db_path
-                ))
-            })?;
-        col_meta.segment_ulids.retain(|u| !source_ulids.contains(u));
-        col_meta.segment_ulids.push(new_meta.ulid.clone());
-        // 04-wal：manifest 切换前 append 段增删记录（SPEC §6.4）：
-        // DeleteSegment(旧) + AddSegment(新)。crash 在 manifest 切换前 →
-        // AddSegment(new) 不在 manifest → 孤儿清理；DeleteSegment(old) → 旧段保留。
-        // B-2：truncate 仅 compact 调（此处 merge_segments 不 truncate）。
-        let wal = crate::wal::Wal::open(self.inner.vfs.clone(), &self.inner.db_path)?;
-        for u in &source_ulids {
-            wal.append(&crate::wal::WalRecord::DeleteSegment {
+        // 更新 manifest（I-6）。M4 Phase 4 fix Bug 1：用 update 闭包把
+        // load-modify-WAL-save 整个事务包在 save_lock 内，杜绝并发 flush/merge 的
+        // lost-update 与 manifest.json.tmp 覆盖竞态。WAL → manifest 的 §6.4 顺序保持
+        // （WAL append 在闭包内、save_atomic_locked 在闭包返回后）。
+        self.inner.manifest_store.update(|manifest| {
+            let col_meta = manifest
+                .collections
+                .get_mut(&self.inner.name)
+                .ok_or_else(|| {
+                    VaneError::NotFound(format!(
+                        "collection not in manifest: {} (op=merge, db={}; 建议: 确认 collection 已创建)",
+                        self.inner.name, self.inner.db_path
+                    ))
+                })?;
+            col_meta.segment_ulids.retain(|u| !source_ulids.contains(u));
+            col_meta.segment_ulids.push(new_meta.ulid.clone());
+            // 04-wal：manifest 切换前 append 段增删记录（SPEC §6.4）：
+            // DeleteSegment(旧) + AddSegment(新)。crash 在 manifest 切换前 →
+            // AddSegment(new) 不在 manifest → 孤儿清理；DeleteSegment(old) → 旧段保留。
+            // B-2：truncate 仅 compact 调（此处 merge_segments 不 truncate）。
+            let wal = crate::wal::Wal::open(self.inner.vfs.clone(), &self.inner.db_path)?;
+            for u in &source_ulids {
+                wal.append(&crate::wal::WalRecord::DeleteSegment {
+                    collection: self.inner.name.clone(),
+                    ulid: u.clone(),
+                })?;
+            }
+            wal.append(&crate::wal::WalRecord::AddSegment {
                 collection: self.inner.name.clone(),
-                ulid: u.clone(),
+                ulid: new_meta.ulid.clone(),
             })?;
-        }
-        wal.append(&crate::wal::WalRecord::AddSegment {
-            collection: self.inner.name.clone(),
-            ulid: new_meta.ulid.clone(),
+            Ok(())
         })?;
-        manifest_store.save_atomic(&manifest)?;
 
         // 更新内存快照。
         let new_reader = Arc::new(SegmentReader::open(&self.inner.vfs, &new_seg_dir)?);
         let new_inv = Arc::new(InvertedIndexReader::open(&self.inner.vfs, &new_seg_dir)?);
         let new_hnsw = match HnswReader::open(&self.inner.vfs, &new_seg_dir) {
             Ok(r) => Some(Arc::new(r)),
             Err(_) => None,
         };
         let new_scalar = Arc::new(crate::segment::ScalarReader::open(
             &self.inner.vfs,
@@ -624,24 +712,26 @@ impl Collection {
             let mut hnsw_w = self.inner.hnsw_readers.write().unwrap();
             let mut scalar_w = self.inner.scalar_readers.write().unwrap();
             let mut tomb_w = self.inner.tombstones.write().unwrap();
             let old_snap = std::mem::take(&mut *snap_w);
             let old_inv = std::mem::take(&mut *inv_w);
             let old_hnsw = std::mem::take(&mut *hnsw_w);
             let old_scalar = std::mem::take(&mut *scalar_w);
             for (i, r) in old_snap.iter().enumerate() {
                 if !source_ulids.contains(&r.meta().ulid) {
                     snap_w.push(r.clone());
-                    offsets_w.insert(
-                        r.meta().ulid.clone(),
-                        offsets.get(&r.meta().ulid).copied().unwrap_or(0),
-                    );
+                    // M4 Phase 4 fix Bug 2：保留段不覆写 offsets_w——offsets_w 在写锁内
+                    // 已含并发 flush 新推入段的正确 base。若用 merge 入口读的 stale
+                    // `offsets` 覆写，并发 flush 在 merge 读 offsets（line 555）后推入的
+                    // 段会落入 unwrap_or(0) → 偏移被错置为 0 → search 回填算错 docid
+                    // → 活文档「丢失」（reopen 重建 offsets 后又可见，故 manifest 一致）。
+                    // 段不可变（I-1），保留段 offset 不变，无需 re-insert。
                     if let Some(inv) = old_inv.get(i) {
                         inv_w.push(inv.clone());
                     }
                     if let Some(h) = old_hnsw.get(i) {
                         hnsw_w.push(h.clone());
                     }
                     if let Some(sr) = old_scalar.get(i) {
                         scalar_w.push(sr.clone());
                     }
                 } else {
@@ -1299,21 +1389,22 @@ impl Collection {
                 &self.inner.segments_dir,
                 ulid,
                 &self.inner.schema,
                 &new_tokenizer_id,
                 &new_tokenizer,
             )?;
             new_segments.push(reindexed);
         }
 
         // 原子切换 manifest（I-6）：ULID 替换 + tokenizer_id/user_dict 更新。
-        let manifest_store = ManifestStore::new(self.inner.vfs.clone(), &self.inner.db_path);
+        // M4 Phase 4 fix Bug 1：复用共享 Arc<ManifestStore>（save_lock 序列化并发）。
+        let manifest_store = &self.inner.manifest_store;
         let new_ulids: Vec<String> = new_segments.iter().map(|s| s.ulid.clone()).collect();
         // 04-wal（06 遗留 #1）：manifest 切换前 append 段增删 + re-keyed tombstone 记录。
         // - AddSegment(新段)：crash 在 manifest 前 → 孤儿清理。
         // - DeleteSegment(旧段)：信息记录（recover 不动作）。
         // - AddTombstone(新 ULID, 绝对 docid)：reindex 保留 tombstone（re-key 到新 ULID），
         //   需重新记录到 WAL，否则 crash 后新 ULID 在 manifest 但 tombstone 仅内存 → 丢失。
         //   docid 顺序不变 → 位图原值（绝对 docid）对新段同样有效。
         // reindex **不** truncate：tombstone 未物理清除（与 compact 区分），WAL 累积到下次 compact。
         let wal = crate::wal::Wal::open(self.inner.vfs.clone(), &self.inner.db_path)?;
         for new_u in &new_ulids {
@@ -1343,21 +1434,21 @@ impl Collection {
             }
         }
         let new_col_meta = CollectionMeta {
             schema: self.inner.schema.clone(),
             tokenizer_kind: self.inner.tokenizer_kind,
             tokenizer_id: new_tokenizer_id.clone(),
             user_dict: pending.clone(),
             segment_ulids: new_ulids.clone(),
         };
         crate::api::reindex::update_manifest_after_reindex(
-            &manifest_store,
+            manifest_store,
             &self.inner.name,
             &old_ulids,
             new_ulids.clone(),
             new_col_meta,
         )?;
 
         // 更新内存快照：旧段移除 → 新段插入。tombstone 按 ULID re-key
         // （docid 顺序不变，位图原值有效）。
         // I-4 原子性（06-review #1）：tokenizer/tokenizer_id 必须与 snapshot 段列表
         // 在同一写锁块内切换，杜绝「snapshot 已切到新段（新 TokenizerId）但
diff --git a/crates/vane-core/src/api/db.rs b/crates/vane-core/src/api/db.rs
index 53601cd..c6c157e 100644
--- a/crates/vane-core/src/api/db.rs
+++ b/crates/vane-core/src/api/db.rs
@@ -10,21 +10,23 @@ use std::sync::{Arc, RwLock};
 use super::collection::{Collection, CollectionInner};
 use super::types::{CollectionOptions, OpenOptions};
 
 pub struct Db {
     inner: Arc<DbInner>,
 }
 
 pub(crate) struct DbInner {
     pub(crate) vfs: Arc<dyn Vfs>,
     pub(crate) db_path: String,
-    pub(crate) manifest_store: ManifestStore,
+    // M4 Phase 4 fix（Bug 1）：Arc<ManifestStore> → CollectionInner 克隆同一 Arc，
+    // 共享 save_lock。跨 collection / 跨线程的 save_atomic 全部序列化。
+    pub(crate) manifest_store: Arc<ManifestStore>,
     pub(crate) collections: RwLock<HashMap<String, Arc<CollectionInner>>>,
     // M2-10：Executor（SPEC §11）。open 时经 executor::default_executor() 工厂构造，
     // 平台分支集中在 executor/mod.rs（I-5）。search 路径用 Executor::scope 并行搜各段。
     pub(crate) executor: Arc<dyn crate::executor::Executor>,
     // I3：Db 级 fallback，restore 时用（M0 restore 直接用 opts.auto_commit 传入参数；
     // 此字段保留供未来 reopen/动态配置场景，故 allow dead_code）
     #[allow(dead_code)]
     pub(crate) auto_commit: AutoCommitConfig,
     // 07-dict-distribution-node：Db 级 jieba 词典（dict-zh feature 启用时 Db::open 加载）。
     // collection 创建时若 tokenizer=Jieba 且此字段 Some → build_jieba_tokenizer；
@@ -32,21 +34,21 @@ pub(crate) struct DbInner {
     // pub(crate) 扩展，非 M0 冻结破坏（DbInner 内部结构，不暴露 pub API）。
     // M2-11：改为 RwLock 以支持 FFI vane_load_dict 运行时注入（dict-zh off 时
     // Db::open 设 None，FFI 调 set_jieba_dict 注入 Go embed 词典）。
     #[cfg(feature = "jieba")]
     pub(crate) jieba_dict:
         std::sync::RwLock<Option<std::sync::Arc<crate::tokenizer::jieba::JiebaDict>>>,
 }
 
 impl Db {
     pub fn open(vfs: Arc<dyn Vfs>, path: &str, opts: OpenOptions) -> Result<Self> {
-        let manifest_store = ManifestStore::new(vfs.clone(), path);
+        let manifest_store = Arc::new(ManifestStore::new(vfs.clone(), path));
         let manifest = manifest_store.load()?.unwrap_or_else(Manifest::empty);
         let collections = RwLock::new(HashMap::new());
         // 07：dict-zh feature 启用时 Db::open 自动加载预编译 dict.bin（冷加载 <150ms，§13.1）。
         // 加载失败不抛错（SPEC §13.2-2 ④）：jieba_dict=None → collection 创建时降级 CjkBigram。
         #[cfg(feature = "jieba")]
         let jieba_dict = load_default_jieba_dict();
         // M2-10：Executor 工厂构造（平台分支在 executor/mod.rs，I-5）。
         let executor = crate::executor::default_executor();
         let inner = Arc::new(DbInner {
             vfs: vfs.clone(),
@@ -132,37 +134,33 @@ impl Db {
             segment_ulids: vec![],
         };
         let col_inner =
             CollectionInner::create_new(&self.inner, name, meta, opts.auto_commit.clone())?;
         let arc = Arc::new(col_inner);
         self.inner
             .collections
             .write()
             .unwrap()
             .insert(name.to_string(), arc.clone());
-        // 持久化 manifest
-        let mut m = self
-            .inner
-            .manifest_store
-            .load()?
-            .unwrap_or_else(Manifest::empty);
-        m.collections.insert(
-            name.to_string(),
-            CollectionMeta {
-                schema,
-                tokenizer_kind: opts.tokenizer,
-                tokenizer_id: tok_id,
-                user_dict: opts.user_dict,
-                segment_ulids: vec![],
-            },
-        );
-        self.inner.manifest_store.save_atomic(&m)?;
+        // 持久化 manifest（M4 Phase 4 fix Bug 1：update 闭包内 load-modify-save
+        // 在 save_lock 内，杜绝并发建表/flush 的 lost-update 与 tmp 覆盖）。
+        let col_meta = CollectionMeta {
+            schema,
+            tokenizer_kind: opts.tokenizer,
+            tokenizer_id: tok_id,
+            user_dict: opts.user_dict,
+            segment_ulids: vec![],
+        };
+        self.inner.manifest_store.update(|m| {
+            m.collections.insert(name.to_string(), col_meta);
+            Ok(())
+        })?;
         Ok(Collection { inner: arc })
     }
 
     pub fn collections(&self) -> Vec<String> {
         self.inner
             .collections
             .read()
             .unwrap()
             .keys()
             .cloned()
diff --git a/crates/vane-core/src/api/reindex.rs b/crates/vane-core/src/api/reindex.rs
index 0ae9703..e93af12 100644
--- a/crates/vane-core/src/api/reindex.rs
+++ b/crates/vane-core/src/api/reindex.rs
@@ -2,21 +2,21 @@
 //!
 //! ReindexHandle 持有 `Arc<ReindexInner>`，inner 含 `Mutex<RebuildState>` +
 //! Condvar。`progress()` 读进度；`wait()` 阻塞直到完成（native Condvar，WASM
 //! 轮询——M1 同步执行下两者均立即返回）。
 //!
 //! M1 同步执行（R-4/R-6）：`reindex()` 同步完成重建后返回已完成的 handle
 //! （progress=1.0, wait 立即返回）。后台化留 M2 Executor。
 
 use crate::bm25::{write_inverted, InvertedIndexBuilder, InvertedIndexReader};
 use crate::hnsw::{write_hnsw, HnswReader, HnswWriter};
-use crate::persistence::{CollectionMeta, Manifest, ManifestStore};
+use crate::persistence::{CollectionMeta, ManifestStore};
 use crate::segment::{ScalarReader, SegmentReader, SegmentWriter};
 use crate::tokenizer::Tokenizer;
 use crate::types::{Result, Schema, TokenizerId, VaneError};
 use crate::vfs::Vfs;
 use std::sync::{Arc, Condvar, Mutex};
 
 /// SPEC §4.1 ReindexHandle（可轮询可阻塞）。
 ///
 /// M2-11 fix：derive Clone——FFI 注册表需 clone 出 ReindexHandle 后释放锁再调
 /// `wait()`，避免持读锁阻塞（I-4）。inner 是 Arc，clone 廉价。
@@ -208,35 +208,39 @@ pub(crate) fn reindex_segment(
         doc_count: meta.doc_count,
         docid_base: meta.docid_base,
         inv_reader,
         hnsw_reader,
         scalar_reader: new_scalar,
         reader: new_reader,
     })
 }
 
 /// 更新 manifest 中的 collection meta（段 ULID 替换 + tokenizer_id/user_dict 更新）。
+///
+/// M4 Phase 4 fix（Bug 1）：用 `update` 闭包把 load-modify-save 包在 save_lock 内，
+/// 杜绝并发 reindex/flush 的 lost-update 与 tmp 覆盖。
 pub(crate) fn update_manifest_after_reindex(
     manifest_store: &ManifestStore,
     col_name: &str,
     old_ulids: &[String],
     new_ulids: Vec<String>,
     new_meta: CollectionMeta,
 ) -> Result<()> {
-    let mut manifest = manifest_store.load()?.unwrap_or_else(Manifest::empty);
-    let col = manifest.collections.get_mut(col_name).ok_or_else(|| {
-        VaneError::NotFound(format!(
-            "collection not in manifest: {} (op=reindex; 建议: 确认 collection 已创建)",
-            col_name
-        ))
-    })?;
-    // 替换 ULID：移除旧 ULID，追加新 ULID（保持其余顺序）。
-    col.segment_ulids.retain(|u| !old_ulids.contains(u));
-    for u in &new_ulids {
-        if !col.segment_ulids.contains(u) {
-            col.segment_ulids.push(u.clone());
+    manifest_store.update(|manifest| {
+        let col = manifest.collections.get_mut(col_name).ok_or_else(|| {
+            VaneError::NotFound(format!(
+                "collection not in manifest: {} (op=reindex; 建议: 确认 collection 已创建)",
+                col_name
+            ))
+        })?;
+        // 替换 ULID：移除旧 ULID，追加新 ULID（保持其余顺序）。
+        col.segment_ulids.retain(|u| !old_ulids.contains(u));
+        for u in &new_ulids {
+            if !col.segment_ulids.contains(u) {
+                col.segment_ulids.push(u.clone());
+            }
         }
-    }
-    col.tokenizer_id = new_meta.tokenizer_id;
-    col.user_dict = new_meta.user_dict;
-    manifest_store.save_atomic(&manifest)
+        col.tokenizer_id = new_meta.tokenizer_id;
+        col.user_dict = new_meta.user_dict;
+        Ok(())
+    })
 }
diff --git a/crates/vane-core/src/persistence/mod.rs b/crates/vane-core/src/persistence/mod.rs
index 7693c9e..0ddb614 100644
--- a/crates/vane-core/src/persistence/mod.rs
+++ b/crates/vane-core/src/persistence/mod.rs
@@ -1,21 +1,21 @@
 //! 持久化模块（SPEC §6.4 / §7.1）：
 //! - Manifest 原子读写（临时文件 → sync → rename，不变量 I-6）
 //! - AutoCommitter（计数 + 时间双触发）
 //!
 //! 本模块不直接读写段内容（段由 04-segment-format 产出），只管 manifest 指针。
 
 use crate::tokenizer::{BuiltinTokenizer, UserDictEntry};
 use crate::types::{Result, Schema, TokenizerId, VaneError};
 use crate::vfs::Vfs;
 use serde::{Deserialize, Serialize};
-use std::sync::Arc;
+use std::sync::{Arc, Mutex};
 
 #[cfg(test)]
 mod tests;
 
 /// SPEC §6.2 manifest.json 结构。
 #[derive(Debug, Clone, Serialize, Deserialize)]
 pub struct Manifest {
     pub version: u32,
     pub collections: std::collections::HashMap<String, CollectionMeta>,
 }
@@ -42,27 +42,35 @@ pub struct CollectionMeta {
 const MANIFEST_FILENAME: &str = "manifest.json";
 const MANIFEST_TMP: &str = "manifest.json.tmp";
 
 /// manifest.json 原子读写（SPEC §6.4）。
 ///
 /// 封装 manifest 的加载与原子保存（临时文件 → sync → rename，不变量 I-6）。
 /// 通过 Vfs trait 读写，core 不直接使用 std::fs。
 pub struct ManifestStore {
     vfs: Arc<dyn Vfs>,
     db_path: String,
+    // M4 Phase 4 fix（Bug 1）：序列化 manifest 原子保存。并发 save_atomic 共享固定
+    // tmp 路径 `manifest.json.tmp`，delete/create/write_at/sync/rename 交错会互相覆写
+    // tmp → manifest 损坏（E_CORRUPT）。save_lock 串行化原子的 manifest 切换；
+    // 段文件写仍并发（各自 seg_<ulid>/ 目录，互不冲突）。private 字段，不改 pub API。
+    // 共享实例（DbInner 持 Arc<ManifestStore>，CollectionInner 克隆同一 Arc）→ 跨
+    // collection / 跨线程的 save_atomic 全部序列化，覆盖同一 db_path 的并发场景。
+    save_lock: Mutex<()>,
 }
 
 impl ManifestStore {
     pub fn new(vfs: Arc<dyn Vfs>, db_path: &str) -> Self {
         Self {
             vfs,
             db_path: db_path.to_string(),
+            save_lock: Mutex::new(()),
         }
     }
 
     fn manifest_path(&self) -> String {
         format!("{}/{}", self.db_path, MANIFEST_FILENAME)
     }
 
     fn tmp_path(&self) -> String {
         format!("{}/{}", self.db_path, MANIFEST_TMP)
     }
@@ -94,52 +102,82 @@ impl ManifestStore {
                 "manifest parse: {} (db={}, op=load manifest; 建议: 检查 manifest.json 完整性或从备份恢复)",
                 e, self.db_path
             ))
         })?;
         Ok(Some(m))
     }
 
     /// SPEC §6.4 原子切换：写临时文件 → sync → rename。
     /// 不变量 I-6：任何崩溃后 manifest 指向完整状态（rename 前崩溃 → 旧 manifest 完好；
     /// rename 是原子操作 → manifest 永远指向完整新状态或完整旧状态）。
+    ///
+    /// M4 Phase 4 fix（Bug 1）：入口获取 save_lock，串行化并发原子切换。并发 save_atomic
+    /// 共享固定 tmp 路径 `manifest.json.tmp`，无锁则 delete/create/write_at/sync/rename
+    /// 交错覆写 → manifest 损坏（E_CORRUPT）。段文件写仍并发（各自 seg_<ulid>/ 目录）。
     pub fn save_atomic(&self, manifest: &Manifest) -> Result<()> {
+        let _save_guard = self.save_lock.lock().unwrap();
+        self.save_atomic_locked(manifest)
+    }
+
+    /// save_atomic 的落盘实现——调用者**必须**已持有 save_lock。
+    ///
+    /// 拆出私有方法是为了让 [`add_segment`] / [`update`] 在持锁的 load-modify-save
+    /// 事务中复用落盘逻辑而不重入 save_lock（std::sync::Mutex 不可重入，重入死锁）。
+    fn save_atomic_locked(&self, manifest: &Manifest) -> Result<()> {
         let json = serde_json::to_vec(manifest).map_err(|e| {
             VaneError::Corrupt(format!(
                 "manifest serialize: {} (db={}, op=save manifest; 建议: 重试或检查磁盘空间)",
                 e, self.db_path
             ))
         })?;
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
+    ///
+    /// M4 Phase 4 fix（Bug 1）：load-modify-save 须在同一 save_lock 内完成，否则
+    /// 并发 add_segment 的 load 互相看不到对方未保存的修改 → lost-update（一方段 ULID 丢失）。
     pub fn add_segment(&self, collection: &str, ulid: &str) -> Result<()> {
+        let _save_guard = self.save_lock.lock().unwrap();
         let mut m = self.load()?.unwrap_or_else(Manifest::empty);
         let col = m.collections.get_mut(collection).ok_or_else(|| {
             VaneError::NotFound(format!(
                 "collection not found: {} (db={}, seg={}, op=add_segment; 建议: 确认 collection 名称正确)",
                 collection, self.db_path, ulid
             ))
         })?;
         if !col.segment_ulids.contains(&ulid.to_string()) {
             col.segment_ulids.push(ulid.to_string());
         }
-        self.save_atomic(&m)
+        self.save_atomic_locked(&m)
+    }
+
+    /// 在 save_lock 保护下执行 load→f→save 原子事务（M4 Phase 4 fix，Bug 1）。
+    ///
+    /// 用于 `merge_segments` / `update_manifest_after_reindex` / `Db::collection` 等
+    /// 需要自定义 load-modify-save 序列的路径：整个事务在持锁期间完成，杜绝并发
+    /// lost-update 与 tmp 覆盖。`f` 修改 manifest（并可在此期间做 WAL append——
+    /// WAL → manifest 的 §6.4 顺序保持）。`pub(crate)`：同 crate 内调用，不扩 pub API。
+    pub(crate) fn update<F: FnOnce(&mut Manifest) -> Result<()>>(&self, f: F) -> Result<()> {
+        let _save_guard = self.save_lock.lock().unwrap();
+        let mut m = self.load()?.unwrap_or_else(Manifest::empty);
+        f(&mut m)?;
+        self.save_atomic_locked(&m)
     }
 }
 
 /// SPEC §7.1 auto-commit 配置。默认 `On { interval_ms=1000, max_docs=1000 }`。
 #[derive(Debug, Clone)]
 pub enum AutoCommitConfig {
     Off,
     On { interval_ms: u32, max_docs: u32 },
 }
 
diff --git a/crates/vane-core/tests/stress_concurrency.rs b/crates/vane-core/tests/stress_concurrency.rs
index 182319d..e3b39d5 100644
--- a/crates/vane-core/tests/stress_concurrency.rs
+++ b/crates/vane-core/tests/stress_concurrency.rs
@@ -1,43 +1,45 @@
 // tests/stress_concurrency.rs — M4 阶段四：多线程并发压测 + Send/Sync 边界 + 竞态检测
 //
 // 纯 stress 测试（多线程 N 轮 + 不同 interleaving）——不用 loom（loom 须 loom::sync
 // 改造 vane-core，侵入大；vane-core 用 std::sync 非 loom::sync）。loom 列为 Could defer。
 //
 // 测试安全：全用 MemoryVfs（主力，无真 fs 副作用）+ tempdir（StdFsVfs conformance）。
-// 不改生产代码（只写新测试文件）。不碰 SPEC/CI/fault.rs/crash_recovery/vane-fuzz/proptest。
+// 不碰 SPEC/CI/fault.rs/crash_recovery/vane-fuzz/proptest。
 //
 // 并发模型（vane-core 用 std::sync）：
 // - write_state: Mutex<WriteState> — add/flush 互斥（next_docid 自增 + buffer push/take）
 // - snapshot: RwLock<Vec<Arc<SegmentReader>>> — search 读 / flush+merge 写
-// - compacting: Mutex<bool> — compact 重入保护（非重入返 E_BUSY）
+// - compacting: Mutex<bool> — compact/reindex 重入保护（非重入返 E_BUSY）；
+//   auto_merge_two_smallest try-lock，并发 compact 持锁则 skip（best-effort 降级）
+// - ManifestStore.save_lock: Mutex<()> — 序列化 manifest 原子保存（save_atomic /
+//   add_segment / update 闭包），杜绝并发 tmp 覆盖 + lost-update（M4 Phase 4 Bug 1 fix）
 // - 锁序一致（snapshot → offsets → inv_readers → hnsw → scalars → tombstones），无 lock-order deadlock
 //
 // 并发安全边界（本测试验证）：
 // - 并发 search：安全（RwLock read，多读不互斥）
 // - 并发 add：安全（write_state Mutex 序列化，next_docid 原子自增）
 // - 并发 add + search：安全（add 锁 write_state，search 锁 snapshot read，不同锁）
 // - 并发 compact：安全（compacting Mutex 重入保护，非重入返 E_BUSY）
 // - 并发 search + compact：安全（search 读 snapshot，compact 写 snapshot，RwLock 互斥不死锁）
-//
-// flush 的并发边界（本测试用外部 Mutex 序列化）：
-// vane-core 的 flush() 在 write_state lock 释放后执行 manifest save_atomic + snapshot swap。
-// save_atomic 用固定路径 manifest.json.tmp，并发调用会互相覆盖 → manifest 损坏。
-// flush 内的 auto_merge_two_smallest 不检查 compacting 锁，与并发 compact/auto-merge 竞争。
-// 故本测试用 flush_lock: Mutex<()> 序列化 flush 调用——验 write_state 锁竞争 + snapshot 锁
-// 竞争 + auto-merge 串行安全，不触发 manifest tmp 覆盖竞态。此为已知并发限制（见 report）。
+// - 并发 flush：安全（save_lock 序列化 manifest 原子保存；段文件写并发因 ULID 唯一不冲突）
+//   —— M4 Phase 4 Bug 1 fix 后，无需外部 flush_lock workaround（原 stress 用外部 Mutex
+//   序列化 flush 是临时规避；fix 后直测并发 flush 不损坏 manifest）
+// - 并发 auto-merge：安全（compacting try-lock 串行化，skip 安全降级）
+//   —— M4 Phase 4 Bug 2 fix 后，并发 auto-merge 不再 double-count
 //
 // 数据一致性断言：
 // - 无 panic / 无死锁（测试在 timeout 内完成）
 // - 无丢失：所有 insert 且未 delete 的文档最终可 search 到
 // - 无 double-count：search 结果无重复 external_id
 // - 一致的段状态：compact 后活文档全集不变；segment_ulids 无重复
+// - 并发 flush 后 reopen Db 加载 manifest 不 E_CORRUPT（manifest 一致）
 
 use std::collections::HashSet;
 use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
 use std::sync::{Arc, Mutex};
 use std::thread;
 use std::time::Instant;
 
 use vane_core::api::{
     CollectionOptions, Db, Doc, FusionSpec, OpenOptions, SearchMode, SearchQuery,
 };
@@ -106,31 +108,20 @@ fn unique_dir(label: &str) -> std::path::PathBuf {
         label,
         std::process::id(),
         n,
         std::time::SystemTime::now()
             .duration_since(std::time::UNIX_EPOCH)
             .unwrap()
             .as_nanos()
     ))
 }
 
-/// 序列化 flush 调用——避免 manifest.json.tmp 覆盖竞态 + auto-merge 竞争。
-/// vane-core flush() 在 write_state 释放后执行 save_atomic + snapshot swap，
-/// 并发 flush 会互相覆盖 tmp 文件。此 Mutex 确保同时只有一个 flush 执行。
-fn serialized_flush(
-    col: &vane_core::api::Collection,
-    lock: &Mutex<()>,
-) -> vane_core::types::Result<()> {
-    let _guard = lock.lock().unwrap();
-    col.flush()
-}
-
 // ---------------------------------------------------------------------------
 // 1. Send/Sync 静态断言
 // ---------------------------------------------------------------------------
 
 /// 编译期验证 Db: Send + Sync + Collection: Send + Sync。
 ///
 /// vane-core 用 std::sync（非 loom::sync），所有共享状态经 Arc/RwLock/Mutex 保护。
 /// S9 裁决：不写 unsafe impl Send/Sync——DbInner 字段全部自动 Send+Sync
 /// （Arc<dyn Vfs> 是 Send+Sync，RwLock<HashMap<...>> 是 Send+Sync）。
 /// 此测试在编译期验证 trait 约束——若未来字段变更破坏 Send/Sync，编译失败。
@@ -182,97 +173,99 @@ fn cross_thread_shared_basic() {
     }
 }
 
 // ---------------------------------------------------------------------------
 // 3. 并发 add + flush + search（主压测）
 // ---------------------------------------------------------------------------
 
 /// N 线程 M 轮并发 add + flush + search，验证无 panic + 无丢失 + 无 double-count。
 ///
 /// 线程数 4 / 轮数 100 = 400 文档，MemoryVfs（快、无真 fs 副作用）。
-/// 每轮：add 1 文档 → 每 flush_interval 轮 flush（serialized_flush 避免 manifest 竞态）。
-/// 段数超 SEGMENT_MAX(10) 时 flush 自动触发 auto_merge_two_smallest（串行安全）。
+/// 每轮：add 1 文档 → 每 flush_interval 轮 flush（直接并发 flush，无外部序列化）。
+/// 段数超 SEGMENT_MAX(10) 时 flush 自动触发 auto_merge_two_smallest。
+///
+/// M4 Phase 4 fix 后（Bug 1 save_lock + Bug 2 auto-merge compacting guard），
+/// 并发 flush 不再损坏 manifest，并发 auto-merge 不再 double-count —— 本测试
+/// 直测修复后的并发路径（原 stress 用外部 flush_lock Mutex 序列化是临时规避，已去）。
 ///
 /// 并发维度：
 /// - add 互斥（write_state Mutex）：多线程 add 序列化
 /// - search 并发（snapshot RwLock read）：多线程 search 同时读
-/// - flush 串行（flush_lock Mutex）：避免 manifest tmp 覆盖 + auto-merge 竞争
+/// - flush 并发（save_lock 序列化 manifest 切换；段文件写 ULID 唯一不冲突）
+/// - auto-merge 串行化（compacting try-lock，并发持锁则 skip）
 /// - search + add + flush 混合并发：验证不同锁不冲突
 #[test]
 fn stress_concurrent_add_flush_search() {
     run_stress_add_flush_search("db", 4, 100, 10);
 }
 
 /// 主 stress 逻辑封装，供 multi_run_stability 复用。
 fn run_stress_add_flush_search(
     db_path: &str,
     n_threads: usize,
     n_rounds: usize,
     flush_interval: usize,
 ) {
     let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
-    let db = Db::open(vfs, db_path, OpenOptions::default()).unwrap();
+    let db = Db::open(vfs.clone(), db_path, OpenOptions::default()).unwrap();
     let col = db.collection("c", schema(), col_opts()).unwrap();
 
-    let flush_lock = Mutex::new(()); // 序列化 flush（见文件头注释）
-
     // 共享 id 跟踪
     let inserted_ids = Mutex::new(HashSet::new());
     // 错误收集
     let errors = Mutex::new(Vec::new());
     // search 调用计数（验证 search 被实际执行）
     let search_count = AtomicUsize::new(0);
 
     let start = Instant::now();
     thread::scope(|s| {
         for t in 0..n_threads {
             let col = col.clone();
             let inserted_ids = &inserted_ids;
             let errors = &errors;
             let search_count = &search_count;
-            let flush_lock = &flush_lock;
 
             s.spawn(move || {
                 for r in 0..n_rounds {
                     let id = format!("t{}r{}", t, r);
                     // add（write_state lock → next_docid 自增 → buffer push）
                     if let Err(e) = col.add(&[make_doc(&id)]) {
                         errors
                             .lock()
                             .unwrap()
                             .push(format!("t{}r{} add err: {}", t, r, e));
                         return;
                     }
                     inserted_ids.lock().unwrap().insert(id);
 
-                    // flush 每 flush_interval 轮（serialized 避免 manifest 竞态）
+                    // flush 每 flush_interval 轮（直接并发——save_lock 序列化 manifest）
                     if r > 0 && r % flush_interval == 0 {
-                        if let Err(e) = serialized_flush(&col, flush_lock) {
+                        if let Err(e) = col.flush() {
                             errors
                                 .lock()
                                 .unwrap()
                                 .push(format!("t{}r{} flush err: {}", t, r, e));
                         }
                     }
 
                     // search（读 snapshot → 并行搜各段 → 归并）
                     // search 可能返回部分结果（未 flush 的文档不可见），不应 panic
                     if let Err(e) = col.search(&text_query(50)) {
                         errors
                             .lock()
                             .unwrap()
                             .push(format!("t{}r{} search err: {}", t, r, e));
                     }
                     search_count.fetch_add(1, Ordering::Relaxed);
                 }
                 // 线程结束前最终 flush（确保所有 buffer 文档落盘）
-                if let Err(e) = serialized_flush(&col, flush_lock) {
+                if let Err(e) = col.flush() {
                     errors
                         .lock()
                         .unwrap()
                         .push(format!("t{} final flush err: {}", t, e));
                 }
             });
         }
     });
     let elapsed = start.elapsed();
 
@@ -316,20 +309,33 @@ fn run_stress_add_flush_search(
     // 段 ULID 无重复
     let ulids = col.segment_ulids();
     let ulid_set: HashSet<_> = ulids.iter().cloned().collect();
     assert_eq!(
         ulid_set.len(),
         ulids.len(),
         "duplicate segment ULIDs: {:?}",
         ulids
     );
 
+    // 并发 flush 后 manifest 一致：reopen Db 加载 manifest 不 E_CORRUPT，
+    // 且 reopen 后文档数一致（manifest 指向完整状态）。
+    let db2 = Db::open(vfs, db_path, OpenOptions::default()).unwrap();
+    let col2 = db2.collection("c", schema(), col_opts()).unwrap();
+    let hits2 = col2.search(&text_query(top_k)).unwrap();
+    assert_eq!(
+        hits2.len(),
+        hits.len(),
+        "reopen changed hit count (manifest inconsistency; got={}, prev={})",
+        hits2.len(),
+        hits.len()
+    );
+
     eprintln!(
         "stress_concurrent_add_flush_search: {} threads x {} rounds = {} docs, {} segments, {} searches, {:?}",
         n_threads,
         n_rounds,
         expected.len(),
         ulids.len(),
         total_searches,
         elapsed
     );
 }
@@ -670,73 +676,73 @@ fn stress_concurrent_add_during_compact() {
     );
 }
 
 // ---------------------------------------------------------------------------
 // 7. StdFsVfs + tempdir conformance
 // ---------------------------------------------------------------------------
 
 /// StdFsVfs + tempdir 小规模并发，验证行为与 MemoryVfs 一致（真 fs 路径）。
 ///
 /// 2 线程 × 50 轮 = 100 文档。StdFsVfs 用真 std::fs（native 唯一）。
-/// tempdir 隔离（不污染宿主机）。flush 串行（flush_lock）避免 manifest 竞态。
+/// tempdir 隔离（不污染宿主机）。M4 Phase 4 fix 后直接并发 flush（save_lock
+/// 序列化 manifest 原子保存，无需外部 flush_lock workaround）。
 /// 验证无 panic + 无丢失 + 无 double-count。
 #[test]
 fn stress_stdfs_conformance() {
     let dir = unique_dir("stdfs");
     let _ = std::fs::remove_dir_all(&dir);
     std::fs::create_dir_all(&dir).unwrap();
 
     {
         let vfs = Arc::new(StdFsVfs::with_root(dir.to_str().unwrap())) as Arc<dyn Vfs>;
         let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
         let col = db.collection("c", schema(), col_opts()).unwrap();
 
         const N_THREADS: usize = 2;
         const N_ROUNDS: usize = 50;
 
-        let flush_lock = Mutex::new(());
         let inserted_ids = Mutex::new(HashSet::new());
         let errors = Mutex::new(Vec::new());
 
         thread::scope(|s| {
             for t in 0..N_THREADS {
                 let col = col.clone();
                 let inserted_ids = &inserted_ids;
                 let errors = &errors;
-                let flush_lock = &flush_lock;
                 s.spawn(move || {
                     for r in 0..N_ROUNDS {
                         let id = format!("t{}r{}", t, r);
                         if let Err(e) = col.add(&[make_doc(&id)]) {
                             errors
                                 .lock()
                                 .unwrap()
                                 .push(format!("t{}r{} add: {}", t, r, e));
                             return;
                         }
                         inserted_ids.lock().unwrap().insert(id);
                         if r > 0 && r % 10 == 0 {
-                            if let Err(e) = serialized_flush(&col, flush_lock) {
+                            // 直接并发 flush——save_lock 序列化 manifest 原子保存
+                            if let Err(e) = col.flush() {
                                 errors
                                     .lock()
                                     .unwrap()
                                     .push(format!("t{}r{} flush: {}", t, r, e));
                             }
                         }
                         if let Err(e) = col.search(&text_query(50)) {
                             errors
                                 .lock()
                                 .unwrap()
                                 .push(format!("t{}r{} search: {}", t, r, e));
                         }
                     }
-                    if let Err(e) = serialized_flush(&col, flush_lock) {
+                    if let Err(e) = col.flush() {
                         errors.lock().unwrap().push(format!("t{} final: {}", t, e));
                     }
                 });
             }
         });
 
         let errs = errors.into_inner().unwrap();
         assert!(errs.is_empty(), "StdFsVfs stress errors: {:?}", errs);
 
         // 无丢失 + 无 double-count
@@ -759,24 +765,264 @@ fn stress_stdfs_conformance() {
         );
     }
     // 清理
     let _ = std::fs::remove_dir_all(&dir);
 }
 
 // ---------------------------------------------------------------------------
 // 8. 多次跑确认无 flaky
 // ---------------------------------------------------------------------------
 
-/// 连续 3 次运行 stress（不同 db_path 独立状态），验证无 flaky。
+/// 连续 5 次运行 stress（不同 db_path 独立状态），验证无 flaky。
 ///
 /// 竞态若存在，多次跑可能暴露（线程调度非确定性 → 不同 interleaving）。
 /// 每次用独立 db_path + 独立 MemoryVfs（完全隔离，无状态泄漏）。
-/// flush_interval=25 → 每线程 2 次 flush = 8 段 < SEGMENT_MAX(10) → 不触发 auto-merge
-/// （auto-merge 在 flush 串行下仍偶发段状态竞争，见 report concerns；此测试验稳定性）。
+/// flush_interval=10 → 4x50=200 docs/run，每线程 5 次 flush = 20 段 > SEGMENT_MAX(10)
+/// → 触发 auto_merge_two_smallest（M4 Phase 4 fix 后并发 auto-merge 安全，不再
+/// double-count）。此测试验修复后并发 flush + auto-merge 多次跑稳定性。
 #[test]
 fn stress_multi_run_stability() {
-    for run in 0..3 {
+    for run in 0..5 {
         let db_path = format!("db_run{}", run);
-        // 4x50=200 docs/run，flush_interval=25 → 8 段 < 10，不触发 auto-merge
-        run_stress_add_flush_search(&db_path, 4, 50, 25);
+        // flush_interval=10 → 每线程 5 次 flush = 20 段 > 10 → 触发 auto-merge
+        run_stress_add_flush_search(&db_path, 4, 50, 10);
     }
 }
+
+// ---------------------------------------------------------------------------
+// 9. 并发 flush 不损坏 manifest（Bug 1 fix 验证）
+// ---------------------------------------------------------------------------
+
+/// 多线程并发 flush（无外部序列化），验证 manifest 不损坏 + 数据不丢 + 无 E_CORRUPT。
+///
+/// M4 Phase 4 fix（Bug 1）前：并发 flush 的 save_atomic 共享固定 tmp 路径
+/// manifest.json.tmp → delete/create/write_at/sync/rename 交错覆写 → E_CORRUPT。
+/// fix 后：ManifestStore.save_lock 序列化原子切换；段文件写仍并发（ULID 唯一）。
+///
+/// 本测试直测修复路径：4 线程各 15 轮 add + 每轮 flush（60 次并发 flush）。
+/// 断言：无 E_CORRUPT / 无错误 + 全部文档可搜到 + reopen Db 加载 manifest 一致。
+#[test]
+fn stress_concurrent_flush_no_corruption() {
+    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
+    let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
+    let col = db.collection("c", schema(), col_opts()).unwrap();
+
+    // 预填 1 段（让并发 flush 有初始状态）
+    col.add(&[make_doc("seed0"), make_doc("seed1")]).unwrap();
+    col.flush().unwrap();
+
+    const N_THREADS: usize = 4;
+    const N_ROUNDS: usize = 15;
+    let errors = Mutex::new(Vec::new());
+    let inserted_ids = Mutex::new(HashSet::new());
+    let flush_count = AtomicUsize::new(0);
+
+    thread::scope(|s| {
+        for t in 0..N_THREADS {
+            let col = col.clone();
+            let errors = &errors;
+            let inserted_ids = &inserted_ids;
+            let flush_count = &flush_count;
+            s.spawn(move || {
+                for r in 0..N_ROUNDS {
+                    let id = format!("t{}r{}", t, r);
+                    if let Err(e) = col.add(&[make_doc(&id)]) {
+                        errors
+                            .lock()
+                            .unwrap()
+                            .push(format!("t{}r{} add err: {}", t, r, e));
+                        return;
+                    }
+                    inserted_ids.lock().unwrap().insert(id);
+                    // 直接并发 flush——save_lock 序列化 manifest 原子保存
+                    if let Err(e) = col.flush() {
+                        errors
+                            .lock()
+                            .unwrap()
+                            .push(format!("t{}r{} flush err: {}", t, r, e));
+                    }
+                    flush_count.fetch_add(1, Ordering::Relaxed);
+                }
+            });
+        }
+    });
+
+    let errs = errors.into_inner().unwrap();
+    assert!(
+        errs.is_empty(),
+        "concurrent flush produced errors (Bug 1 regression?): {:?}",
+        errs
+    );
+    assert_eq!(
+        flush_count.load(Ordering::Relaxed),
+        N_THREADS * N_ROUNDS,
+        "flush count"
+    );
+
+    // 全部文档可搜到（无丢失）
+    let expected: HashSet<_> = inserted_ids.into_inner().unwrap();
+    // seed 文档也应可见
+    let mut expected_all = expected.clone();
+    expected_all.insert("seed0".to_string());
+    expected_all.insert("seed1".to_string());
+    let top_k = (expected_all.len() + 10).min(TOPK_MAX as usize) as u32;
+    let hits = col.search(&text_query(top_k)).unwrap();
+    let found: HashSet<_> = hits.iter().map(|h| h.id.clone()).collect();
+    // 无 double-count
+    assert_eq!(
+        found.len(),
+        hits.len(),
+        "double-count after concurrent flush (Bug 1 regression?)"
+    );
+    for id in &expected_all {
+        assert!(
+            found.contains(id),
+            "doc {} lost after concurrent flush (Bug 1 regression?)",
+            id
+        );
+    }
+
+    // reopen Db 加载 manifest 不 E_CORRUPT（manifest 一致性核心断言）
+    let db2 = Db::open(vfs, "db", OpenOptions::default()).unwrap();
+    let col2 = db2.collection("c", schema(), col_opts()).unwrap();
+    let hits2 = col2.search(&text_query(top_k)).unwrap();
+    assert_eq!(
+        hits2.len(),
+        hits.len(),
+        "reopen changed hit count (manifest corruption; Bug 1 regression?)"
+    );
+
+    // 段 ULID 无重复
+    let ulids = col.segment_ulids();
+    let ulid_set: HashSet<_> = ulids.iter().cloned().collect();
+    assert_eq!(
+        ulid_set.len(),
+        ulids.len(),
+        "duplicate ULIDs after concurrent flush (Bug 1 regression?)"
+    );
+
+    eprintln!(
+        "stress_concurrent_flush_no_corruption: {} threads x {} flushes = {} concurrent flushes, {} docs, {} segments",
+        N_THREADS,
+        N_ROUNDS,
+        flush_count.load(Ordering::Relaxed),
+        expected_all.len(),
+        ulids.len()
+    );
+}
+
+// ---------------------------------------------------------------------------
+// 10. 并发 auto-merge 不 double-count（Bug 2 fix 验证）
+// ---------------------------------------------------------------------------
+
+/// 多线程并发 flush 触发 auto-merge，验证不 double-count + 活文档全集无重复。
+///
+/// M4 Phase 4 fix（Bug 2）前：auto_merge_two_smallest 不获取 compacting 锁，
+/// 与并发 compact/auto-merge 竞争 → 段未正确移除 → double-count（search 结果
+/// 出现重复 external_id）。fix 后：auto_merge try-lock compacting，并发持锁则
+/// skip（best-effort 降级），下次 flush 段数仍超阈值时再 merge。
+///
+/// 本测试预填 > SEGMENT_MAX(10) 段强制 auto-merge 活跃，再 4 线程并发 flush
+/// 持续触发 auto-merge。断言：无 double-count + 全部文档可搜到 + ULID 无重复。
+#[test]
+fn stress_concurrent_auto_merge_no_double_count() {
+    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
+    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
+    let col = db.collection("c", schema(), col_opts()).unwrap();
+
+    // 预填 12 段（> SEGMENT_MAX=10，触发 auto-merge），每段 1 文档
+    for i in 0..12 {
+        col.add(&[make_doc(&format!("seed{}", i))]).unwrap();
+        col.flush().unwrap();
+    }
+
+    const N_THREADS: usize = 4;
+    const N_ROUNDS: usize = 15;
+    let errors = Mutex::new(Vec::new());
+    let inserted_ids = Mutex::new(HashSet::new());
+    let flush_count = AtomicUsize::new(0);
+
+    thread::scope(|s| {
+        for t in 0..N_THREADS {
+            let col = col.clone();
+            let errors = &errors;
+            let inserted_ids = &inserted_ids;
+            let flush_count = &flush_count;
+            s.spawn(move || {
+                for r in 0..N_ROUNDS {
+                    let id = format!("t{}r{}", t, r);
+                    if let Err(e) = col.add(&[make_doc(&id)]) {
+                        errors
+                            .lock()
+                            .unwrap()
+                            .push(format!("t{}r{} add err: {}", t, r, e));
+                        return;
+                    }
+                    inserted_ids.lock().unwrap().insert(id);
+                    // flush 触发 auto-merge（段数 > 10）
+                    if let Err(e) = col.flush() {
+                        errors
+                            .lock()
+                            .unwrap()
+                            .push(format!("t{}r{} flush err: {}", t, r, e));
+                    }
+                    flush_count.fetch_add(1, Ordering::Relaxed);
+                }
+            });
+        }
+    });
+
+    let errs = errors.into_inner().unwrap();
+    assert!(
+        errs.is_empty(),
+        "concurrent auto-merge produced errors (Bug 2 regression?): {:?}",
+        errs
+    );
+
+    // 全部文档可搜到（无丢失）—— seed + 线程文档
+    let mut expected: HashSet<String> = (0..12).map(|i| format!("seed{}", i)).collect();
+    for t in 0..N_THREADS {
+        for r in 0..N_ROUNDS {
+            expected.insert(format!("t{}r{}", t, r));
+        }
+    }
+    let top_k = (expected.len() + 10).min(TOPK_MAX as usize) as u32;
+    let hits = col.search(&text_query(top_k)).unwrap();
+    let found: HashSet<_> = hits.iter().map(|h| h.id.clone()).collect();
+    // 无 double-count（Bug 2 核心断言）
+    assert_eq!(
+        found.len(),
+        hits.len(),
+        "double-count after concurrent auto-merge (Bug 2 regression?): unique={}, total hits={}",
+        found.len(),
+        hits.len()
+    );
+    for id in &expected {
+        assert!(
+            found.contains(id),
+            "doc {} lost after concurrent auto-merge (Bug 2 regression?)",
+            id
+        );
+    }
+    assert_eq!(
+        found, expected,
+        "found != expected after concurrent auto-merge"
+    );
+
+    // 段 ULID 无重复（auto-merge 误移除段会导致 ULID 残留重复）
+    let ulids = col.segment_ulids();
+    let ulid_set: HashSet<_> = ulids.iter().cloned().collect();
+    assert_eq!(
+        ulid_set.len(),
+        ulids.len(),
+        "duplicate ULIDs after concurrent auto-merge (Bug 2 regression?): {:?}",
+        ulids
+    );
+
+    eprintln!(
+        "stress_concurrent_auto_merge_no_double_count: {} threads x {} flushes, {} docs, {} segments",
+        N_THREADS,
+        N_ROUNDS,
+        expected.len(),
+        ulids.len()
+    );
+}
diff --git a/docs/plans/m4/task-concurrency-fix-report.md b/docs/plans/m4/task-concurrency-fix-report.md
new file mode 100644
index 0000000..59653ab
--- /dev/null
+++ b/docs/plans/m4/task-concurrency-fix-report.md
@@ -0,0 +1,174 @@
+# M4 阶段四 并发 Bug Fix — Report
+
+> 分支 `feat/m4-prod-readiness`。BASE=354f66e（Phase 4 stress）。
+> Task: 修复 Phase 4 stress 撞出的 2 个真实生产并发 bug + 更新 stress 去 workaround。
+> 类型: FIX（生产代码 + stress 测试）。
+
+## 1. Bug 概述
+
+Phase 4 stress（commit 354f66e）撞出 2 个并发 bug（已 grep 核实）：
+
+| # | Bug | 位置 | 根因（stress 实测） |
+|---|---|---|---|
+| 1 | 并发 flush manifest 损坏 | `persistence/mod.rs:104-121` `ManifestStore::save_atomic` | 固定 tmp 路径 `manifest.json.tmp`；并发 save_atomic 的 delete/create/write_at/sync/rename 交错覆写 → E_CORRUPT。并发 add_segment 的 load-modify-save 也有 lost-update（一方段 ULID 丢失）。 |
+| 2 | auto-merge 段状态竞争 → double-count/missing | `api/collection.rs:486-508` `auto_merge_two_smallest` | stress 实测揭示三层竞态：(a) auto_merge 不获取 compacting 锁，与并发 compact/merge 竞争；(b) merge 的 `target_docid_base` 未并入 `next_docid`，与并发 flush 的缓冲段 docid 重叠；(c) merge 快照重建用入口读的 stale `offsets` 覆写 `seg_offsets`，并发 flush 新推入段的 offset 被错置为 0 → search 回填算错 docid → 活文档「丢失」（reopen 重建 offsets 后又可见）。 |
+
+## 2. Bug 1 fix：ManifestStore save_lock 序列化 manifest 原子保存
+
+**方案 A（reviewer 推荐）**：`ManifestStore` 加 `save_lock: Mutex<()>`，save_atomic 入口序列化。
+
+### 2.1 关键设计：共享 Arc<ManifestStore>
+
+原代码在 `flush`/`merge_segments`/`reindex` 三处 `ManifestStore::new(...)` **构造新实例**——若 save_lock 是 per-instance，并发调用各持新实例的锁 → 不序列化。故 fix 需让 ManifestStore **共享**：
+
+- `DbInner.manifest_store: ManifestStore` → `Arc<ManifestStore>`（`pub(crate)`，不改 pub API）。
+- `CollectionInner` 新增 `manifest_store: Arc<ManifestStore>` 字段（`create_new` 从 `db.manifest_store.clone()` 注入）。
+- flush/merge/reindex 三处 `ManifestStore::new(...)` → `self.inner.manifest_store`（共享同一 Arc → 同一 save_lock）。
+
+### 2.2 save_atomic 拆分 + update 闭包
+
+`save_atomic`（pub，签名不变）入口取 save_lock，调用私有 `save_atomic_locked`（落盘实现）。拆出私有方法避免 `add_segment` / `update` 在持锁的 load-modify-save 事务中重入 save_lock（`std::sync::Mutex` 不可重入）。
+
+新增 `pub(crate) fn update<F: FnOnce(&mut Manifest) -> Result<()>>`：在 save_lock 内 load→f→save_atomic_locked，供 `merge_segments`（load-modify-WAL-save）、`update_manifest_after_reindex`、`Db::collection`（建表 load-modify-save）复用——整个事务在持锁期间完成，杜绝并发 lost-update 与 tmp 覆盖。WAL → manifest 的 §6.4 顺序保持（WAL append 在闭包内、save_atomic_locked 在闭包返回后）。
+
+### 2.3 I16 残留 tmp 清理语义保留
+
+`save_atomic_locked` 开头 `let _ = self.vfs.delete(&tmp);` 保留（处理上次崩溃残留 tmp）。
+
+## 3. Bug 2 fix：auto-merge compacting guard + docid 防重叠 + seg_offsets 修正
+
+stress 实测揭示 Bug 2 是**三层**竞态，需三处 fix：
+
+### 3.1 auto_merge_two_smallest compacting guard（task 要求）
+
+`auto_merge_two_smallest` 入口 `try_lock` compacting：
+- `Ok(guard)` 且 `*guard == false` → 设 true，drop guard，建 `CompactingGuard`（复用 `collection.rs:100-109` 的 M-minor-1 panic-safe Drop guard，Drop 复位 false）。再做 pick + merge_segments。
+- `Err(WouldBlock)` → 并发 compact/reindex 持锁 → `return Ok(())`（skip，best-effort 降级，下次 flush 段数仍超阈值时再 merge）。
+- `Err(Poisoned(e))` → 恢复（取 `e.into_inner()`，设 true 继续 merge），guard drop 复位。
+
+**死锁分析**：compacting guard 不持其他锁（与 compact/reindex 模式一致）。auto_merge 持 compacting → 调 merge_segments（不重入 compacting）→ 内部取 write_state（短持，推进 next_docid）、snapshot/offsets 写锁（短持，重建快照）。compact 用**阻塞** lock() 等 auto_merge 释放（不 try_lock）→ auto_merge 完成后 compact 获取锁、见 `*guard==false`、设 true、执行。锁序一致（compacting → write_state → snapshot → offsets → ...），无 lock-order deadlock。
+
+### 3.2 merge_segments target_docid_base 并入 next_docid + 原子预留（关键 fix）
+
+**根因**：原 `target_docid_base = max(保留段 base+count)`，未并入 `next_docid`（add 已分配但未 flush 的缓冲文档 docid 上界）。并发 flush 的缓冲段 docid 在 `[old_next_docid, next_docid)` 区间；merge 的 target_base 不计入 → 新段与 about-to-flush 的缓冲段 docid 重叠 → fusion 去重丢文档 + 回填误命中 → double-count/missing。
+
+**fix**：`target_docid_base = max(保留段 base+count, next_docid)`。且**原子地**（一次 write_state lock 内）读 next_docid + 算 target_base + bump `next_docid = target_base + estimated_new_count`（estimate = source 段 doc_count 之和，上界；tombstone 清除后实际 new_count <= estimate，多预留的区间留空无危害）。原子 read+bump 消除「merge 读 next_docid → 并发 add 在旧 next_docid 分配 docid → merge 写新段覆盖该 docid」的窗口。compact 全合并（`is_full_merge`，target_base=0）也 bump next_docid=estimate。
+
+### 3.3 flush base_docid 连续性检测（兼容 inspect base=0）
+
+**问题**：3.2 的 merge fix 后，若 flush 用 `base_docid = docs.first().docid`（stale），并发 merge 在两次 add 之间 bump next_docid → 缓冲文档 docid 非连续（`[100, 121, ...]`）→ flush 写连续 `[100, 100+count)` 与 merge 新段 `[101, 121)` 在 101 重叠。
+
+**fix**：flush 检测缓冲文档 docid 是否连续：
+- **连续**（无并发 merge 在 add 之间 bump）→ 用首文档 docid 作 base（保持 inspect `base=0` 语义；merge 的 target_base 已并入 next_docid=first+count，新段在本段之上）。
+- **非连续**（并发 merge 在 add 之间 bump 了 next_docid）→ rebase 到当前 `next_docid`（merge 已 bump 到新段末尾之上）+ bump next_docid 预留本 flush 区间。
+
+此条件 fix 兼容 inspect（连续 → base=0）并修并发（非连续 → rebase）。不碰 inspect 测试。
+
+### 3.4 merge 快照重建不再覆写 seg_offsets（关键 fix）
+
+**根因**：merge 快照重建块（`std::mem::take(&mut *snap_w)` + 遍历重建）对保留段做 `offsets_w.insert(ulid, offsets.get(ulid).unwrap_or(0))`——`offsets` 是 merge 入口（line 555）读的 **stale** clone。并发 flush 在 merge 读 offsets 后推入的新段，其 offset 在 stale `offsets` 中不存在 → `unwrap_or(0)` → 偏移被错置为 0 → search 回填算错 docid → 活文档「丢失」。reopen 重建 offsets（从 header.bin 读 docid_base）后文档又可见 → 故 manifest 一致、内存快照 offsets 不一致。
+
+**fix**：保留段**不覆写** `offsets_w`——段不可变（I-1），保留段 offset 不变，无需 re-insert。仅移除 source 段 offset + 插入新段 offset。并发 flush 新推入段的正确 offset 保留在 `offsets_w` 中不被覆写。
+
+## 4. stress 测试更新（去 workaround + 新并发测试）
+
+`tests/stress_concurrency.rs` 更新：
+
+### 4.1 去 flush_lock workaround
+
+- 删 `serialized_flush` helper（原用外部 `Mutex<()>` 序列化 flush 规避 Bug 1）。
+- `stress_concurrent_add_flush_search` + `stress_stdfs_conformance`：直接 `col.flush()`（并发，无外部序列化）——save_lock 序列化 manifest 原子保存。
+- 文件头注释更新（flush 并发边界从「外部 Mutex 序列化」改为「save_lock 序列化 manifest」）。
+
+### 4.2 新增并发 flush 不损坏测试（Bug 1 fix 验证）
+
+`stress_concurrent_flush_no_corruption`：4 线程 × 15 轮 add + 每轮 flush（60 次并发 flush）。断言：无 E_CORRUPT / 无错误 + 全部文档可搜到 + 无 double-count + **reopen Db 加载 manifest 不 E_CORRUPT**（manifest 一致性核心断言）+ ULID 无重复。
+
+### 4.3 新增并发 auto-merge 不 double-count 测试（Bug 2 fix 验证）
+
+`stress_concurrent_auto_merge_no_double_count`：预填 12 段（> SEGMENT_MAX=10）强制 auto-merge 活跃，再 4 线程 × 15 轮 flush 持续触发 auto-merge。断言：无错误 + 无 double-count（Bug 2 核心断言）+ 全部文档可搜到 + ULID 无重复。
+
+### 4.4 multi-run 升级
+
+`stress_multi_run_stability`：3 次 → **5 次**，flush_interval 25→10（4×50=200 docs/run，每线程 5 次 flush = 20 段 > 10 → 触发 auto-merge），直测修复后并发 flush + auto-merge 多次跑稳定性。
+
+### 4.5 保留其他测试
+
+`assert_send_sync`、`cross_thread_shared_basic`、`stress_concurrent_search_during_write`、`stress_concurrent_compact_contention`、`stress_concurrent_add_during_compact` 保留不变。
+
+## 5. 多次 multi-run 结果（无 flaky）
+
+```
+stress_concurrent_flush_no_corruption + stress_concurrent_auto_merge_no_double_count
++ stress_concurrent_add_flush_search + stress_multi_run_stability 等 10 测试，
+连续 8 次全跑均 10 passed / 0 failed：
+
+=== run 1 === test result: ok. 10 passed; 0 failed
+=== run 2 === test result: ok. 10 passed; 0 failed
+=== run 3 === test result: ok. 10 passed; 0 failed
+=== run 4 === test result: ok. 10 passed; 0 failed
+=== run 5 === test result: ok. 10 passed; 0 failed
+=== run 6 === test result: ok. 10 passed; 0 failed
+=== run 7 === test result: ok. 10 passed; 0 failed
+=== run 8 === test result: ok. 10 passed; 0 failed
+```
+
+修复前（仅 compacting guard，无 docid/seg_offsets fix）：8x stress 均 FAILED（double-count 18 duplicates / missing 85 docs）。
+
+## 6. 各门禁真实输出
+
+| 门禁 | 命令 | 结果 |
+|---|---|---|
+| 格式 | `cargo fmt --all -- --check` | 绿（无 diff） |
+| 静态检查 | `cargo clippy --all-targets --all-features --workspace --exclude vane-fuzz -- -D warnings` | 绿（`Finished dev profile`） |
+| stress | `cargo test -p vane-core --all-features --test stress_concurrency` | 10 passed; 0 failed（8x 均 OK） |
+| 全工作区 | `cargo test --workspace --all-features --exclude vane-fuzz` | 见下（全绿） |
+| crash recovery | `cargo test -p vane-core --all-features --test crash_recovery` | 5 passed; 0 failed |
+| 依赖 | `cargo deny check` | 绿（advisories/bans/licenses/sources ok；warning 为 pre-existing regex wrapper，无新 dep） |
+| WASM | `cargo check --target wasm32-unknown-unknown -p vane-core` | 绿（fix 不引 std::fs） |
+
+## 7. 自审
+
+### 7.1 死锁分析
+
+- **save_lock**：`save_atomic` / `add_segment` / `update` 互斥（同一 ManifestStore 实例的 save_lock）。不重入（`add_segment`/`update` 调 `save_atomic_locked`，不调 `save_atomic`）。WAL append 在 `update` 闭包内持 save_lock，但 WAL 是独立文件，不与 manifest tmp 冲突。
+- **compacting**：auto_merge try_lock + skip（WouldBlock）；compact/reindex 阻塞 lock。auto_merge 持 compacting → merge_segments（不重入 compacting）→ 内部 write_state/snapshot 短持。锁序：compacting → write_state → snapshot → offsets → inv → hnsw → scalar → tomb，同序无死锁。
+- **write_state**：flush 持 write_state（take buffer + base_docid 计算 + 连续性检测，短持）；merge 持 write_state（原子读 next_docid + bump，短持）；add 持 write_state（docid 分配 + buffer push，短持）。三者不重入（flush drop write_state 后才做 segment write / add_segment / auto_merge）。
+
+### 7.2 lock 序
+
+- search：snapshot.read → seg_offsets.read → inv.read → hnsw.read → scalar.read → tombstones.read（全 read，不互斥）。
+- flush：write_state（短持，drop 后做 segment write）→ save_lock（add_segment）→ snapshot.write + seg_offsets.write（短持，push）。
+- merge：compacting → write_state（原子读+bump next_docid，短持）→ save_lock（update 闭包：load-modify-WAL-save）→ snapshot.write + seg_offsets.write + inv.write + hnsw.write + scalar.write + tombstones.write（短持，take+rebuild）。
+- compact：compacting → merge → ...
+- 一致，无 lock-order deadlock。
+
+### 7.3 compacting guard 复用
+
+复用 `collection.rs:100-109` 的 `CompactingGuard<'a> { flag: &'a Mutex<bool> }`（M-minor-1 panic-safe Drop 复位 false）。auto_merge 创建 guard 的模式与 compact（line 1136-1145）/ reindex（line 1227-1238）一致：先 `{ let mut guard = lock(); if *guard { return; } *guard = true; }`（块结束 drop guard），再 `let _cg = CompactingGuard { flag }`。guard 不持有 MutexGuard——仅持 `&Mutex<bool>`，Drop 时重新取锁复位（与原模式等价，panic-safe）。
+
+### 7.4 frozen API 未改
+
+- `ManifestStore`：`pub struct`（字段私有，加 `save_lock: Mutex<()>` private 字段不改 pub API）；`pub fn new` / `pub fn load` / `pub fn save_atomic` / `pub fn add_segment` 签名不变。新增 `pub(crate) fn update`（crate 内可见，不扩 pub API）+ 私有 `save_atomic_locked`。
+- `DbInner` / `CollectionInner`：`pub(crate)` 结构，字段改 type / 加字段不改 pub API。`manifest_store: ManifestStore` → `Arc<ManifestStore>`（`pub(crate)` 字段）。
+- `flush` / `auto_merge_two_smallest` / `merge_segments` / `compact` / `reindex` / `update_manifest_after_reindex`：私有 fn 或 pub fn 签名不变。
+- 不碰 SPEC.md / CI yml / fault.rs / crash_recovery / vane-fuzz / proptest / cross_version / tracing / inspect / VaneError 诊断。
+
+### 7.5 core 禁 std::fs
+
+fix 用 `std::sync::{Arc, Mutex}`（已在用），不引 std::fs/std::net/mmap。WASM check 绿。
+
+## 8. commit
+
+```
+fix(core): 并发 flush manifest 损坏 + auto-merge 竞争 double-count（M4 Phase 4 fix）
+```
+
+含：`persistence/mod.rs`（Bug 1 save_lock + save_atomic_locked + update）+ `api/db.rs`（Bug 1 Arc<ManifestStore> + update）+ `api/collection.rs`（Bug 1 共享 manifest_store；Bug 2 compacting guard + target_docid_base 并入 next_docid + 原子预留 + flush 连续性检测 + seg_offsets 不覆写）+ `api/reindex.rs`（Bug 1 update）+ `tests/stress_concurrency.rs`（去 workaround + 新并发 flush/auto-merge 测试 + multi-run 升级）。
+
+不含 SPEC/CI/fault.rs/crash_recovery/vane-fuzz/proptest/cross_version/tracing/inspect/diagnostics 误改。
+
+## 9. concerns
+
+- **auto-merge skip 频率**：compacting guard 的 try_lock+skip 在高并发 flush 下会 skip 部分 auto-merge（段数临时累积）。但下次 flush 段数仍超阈值时再 merge，最终收敛。stress 实测段数 10-22 范围，无无限累积。skip 是 best-effort 降级（task 要求）。
+- **full merge（compact）与并发 flush 的 docid 重叠**：compact 的 target_base=0（reset 语义），3.2 的 fix 也 bump next_docid=estimate 让后续 add 从 estimate 起分配，但 compact 前已 buffer 的 stale docid 仍可能与 compact 新段 [0, total) 重叠。compact 是用户触发（非 flush 自动），且 `stress_concurrent_add_during_compact` 测试通过（compact 持 compacting，add 不持 compacting，但 compact 的 full merge + WAL truncate 是重操作，add 的 buffer 在 compact 期间不被 flush）。列为 Could defer（compact + 并发 flush 的更深层 fix）。
