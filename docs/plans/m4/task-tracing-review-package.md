## Commits 0cb50e5..dae29c6 (5a tracing)

dae29c6 feat(core): tracing feature（cfg 门控零开销埋点，默认 off）（M4 阶段五 a）

## Diff stat

 Cargo.lock                             |  32 +++++
 crates/vane-core/Cargo.toml            |  12 ++
 crates/vane-core/src/api/collection.rs |  70 +++++++++++
 crates/vane-core/src/vfs/page_cache.rs |  12 ++
 crates/vane-core/src/wal/mod.rs        |   3 +
 docs/plans/m4/task-tracing-report.md   | 216 +++++++++++++++++++++++++++++++++
 6 files changed, 345 insertions(+)

## Full diff (U10)

diff --git a/Cargo.lock b/Cargo.lock
index 2fb789e..2bfcb8a 100644
--- a/Cargo.lock
+++ b/Cargo.lock
@@ -1046,20 +1046,51 @@ dependencies = [
 [[package]]
 name = "tinytemplate"
 version = "1.2.1"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "be4d6b5f19ff7664e8c98d03e2139cb510db9b0a60b55f8e8709b689d939b6bc"
 dependencies = [
  "serde",
  "serde_json",
 ]
 
+[[package]]
+name = "tracing"
+version = "0.1.44"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "63e71662fa4b2a2c3a26f570f037eb95bb1f85397f3cd8076caed2f026a6d100"
+dependencies = [
+ "pin-project-lite",
+ "tracing-attributes",
+ "tracing-core",
+]
+
+[[package]]
+name = "tracing-attributes"
+version = "0.1.31"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "7490cfa5ec963746568740651ac6781f701c9c5ea257c58e057f3ba8cf69e8da"
+dependencies = [
+ "proc-macro2",
+ "quote",
+ "syn 2.0.119",
+]
+
+[[package]]
+name = "tracing-core"
+version = "0.1.36"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "db97caf9d906fbde555dd62fa95ddba9eecfd14cb388e4f491a66d74cd5fb79a"
+dependencies = [
+ "once_cell",
+]
+
 [[package]]
 name = "twox-hash"
 version = "1.6.3"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "97fee6b57c6a41524a810daee9286c02d7752c4253064d0b05472833a438f675"
 dependencies = [
  "cfg-if",
  "static_assertions",
 ]
 
@@ -1102,20 +1133,21 @@ version = "0.2.0"
 dependencies = [
  "criterion",
  "proptest",
  "rayon",
  "roaring",
  "rust-stemmers",
  "ruzstd",
  "serde",
  "serde_json",
  "sha2",
+ "tracing",
  "ulid",
  "unicode-segmentation",
  "vane-dict-zh",
  "web-time",
  "zstd",
 ]
 
 [[package]]
 name = "vane-dict-zh"
 version = "2026.8.0"
diff --git a/crates/vane-core/Cargo.toml b/crates/vane-core/Cargo.toml
index e3b3d9a..0b21b8f 100644
--- a/crates/vane-core/Cargo.toml
+++ b/crates/vane-core/Cargo.toml
@@ -28,20 +28,27 @@ zstd = { version = "0.13", optional = true }
 vane-dict-zh = { path = "../vane-dict-zh", optional = true }
 # web-time：跨平台 Instant（SPEC §13.2）。
 # native：零开销 re-export std::time::Instant；wasm32：performance.now()（经 js-sys）。
 # 必要因：std::time::Instant::now() 在 wasm32-unknown-unknown panic（无单调时钟），
 # AutoCommitter 需 Instant 做 auto-commit 时间触发。
 web-time = "1"
 # rayon：native 并行搜索（SPEC §11，M2-10 Executor）。
 # 仅 executor-native feature 启用；wasm32 永不启用（无线程模型，红线）。
 # rayon 非依赖黑名单；传递依赖无黑名单项（cargo deny 守护）。
 rayon = { version = "1", optional = true }
+# tracing：可观测性埋点（M4 §3.5，I-5 能力开关）。
+# optional + cfg(feature="tracing") 门控，默认 off——不启用时编译期消除，
+# wasm/native 体积不变（800KB gzip 红线）。vane-wasm 不启用 tracing（守护红线）。
+# 不触黑名单：传递依赖 tracing-core/thread_local/cfg-if（无 regex/tokio/prost/tonic/
+# openssl/lindera/ndarray/wee_alloc/dashmap/parking_lot），cargo deny check 守护。
+# tracing-subscriber（消费侧）不进 core——vane-ffi/vane-node 按需加 dev-dep。
+tracing = { version = "0.1", optional = true }
 
 [features]
 # zstd-decode：启用 ruzstd 读期解码（stored.bin v2 + jieba dict.bin）。
 # wasm32 可启用（纯 Rust）。vane-wasm 启此 feature 读 v2 stored。
 zstd-decode = ["dep:ruzstd"]
 # zstd-encode：启用 zstd C 库写期编码（stored.bin v2 zstd 块压缩）。
 # 仅 native/node 启用；wasm32 不启（zstd-sys C 库不进 wasm）。
 # 隐含 zstd-decode：写 v2 的配置也必须能读 v2（roundtrip）。
 zstd-encode = ["dep:zstd", "zstd-decode"]
 # jieba：复用 ruzstd 解码 dict.bin（原 jieba=["ruzstd"] 解耦为 zstd-decode）。
@@ -56,20 +63,25 @@ sq8 = []
 # executor-native：启用 rayon 并行搜索（SPEC §11，M2-10）。
 # 仅 native 启用（vane-ffi/vane-node）；wasm32 不启（rayon 不进 wasm，红线）。
 # default_executor() 在 executor-native off 时返 SerialExecutor（串行，等价 M1）。
 # cfg(target_arch) 仅在 executor/mod.rs（I-5 不变量核心）。
 executor-native = ["dep:rayon"]
 # fault-injection：FaultVfs 故障注入 VFS（M4 §3.1）。
 # dev/optional，默认不启用。cfg(test) 或本 feature 启用时编译 fault.rs，
 # 供崩溃恢复测试精确模拟 IO 错误 / 部分写 / ENOSPC / 延迟。
 # 绝不进生产/wasm 二进制——wasm32 check 不启此 feature、不设 test cfg。
 fault-injection = []
+# tracing：埋点能力开关（I-5，M4 §3.5）。启用时检索延迟/段数/merge 频率/
+# 缓存命中率/WAL append/词典状态指标可观测；不启用时编译期消除，wasm/native
+# 体积不变（800KB gzip 红线）。所有 tracing 调用经 `#[cfg(feature="tracing")]`
+# 门控。默认 off；vane-wasm 不启用（守护红线）；vane-ffi/vane-node native 可启。
+tracing = ["dep:tracing"]
 
 [dev-dependencies]
 criterion = "0.5"
 # proptest：property-based 不变量测试（M4 §3.3）。
 # dev-dep，不进 wasm/native 生产构建（wasm32 check 不含 dev-deps）。
 # 传递依赖无黑名单项（regex/tokio/prost/tonic/openssl/lindera/ndarray/
 # wee_alloc/dashmap/parking_lot）。proptest default 拉 regex-syntax（独立
 # regex 解析器，非 deny 黑名单的 regex crate），不拉 regex crate。
 # cargo deny check 守护（bans ok）。Strategy 用 a-z 字符生成（非 string_regex）。
 proptest = "1"
diff --git a/crates/vane-core/src/api/collection.rs b/crates/vane-core/src/api/collection.rs
index c120bec..edca82c 100644
--- a/crates/vane-core/src/api/collection.rs
+++ b/crates/vane-core/src/api/collection.rs
@@ -453,20 +453,29 @@ impl Collection {
         // 选最小两段合并（pick_merge_candidates 已按 doc_count 升序）。
         if self.segment_count() > crate::types::SEGMENT_MAX {
             // 自动合并失败不阻塞 flush 返回值（flush 已成功）；记录到 stderr。
             if let Err(e) = self.auto_merge_two_smallest() {
                 eprintln!(
                     "[vane] auto-merge after flush for collection '{}' failed: {}",
                     self.inner.name, e
                 );
             }
         }
+        // M4 §3.5 tracing：flush 后段数 + 新段 ULID + 文档数。cfg 门控，编译期消除。
+        #[cfg(feature = "tracing")]
+        tracing::info!(
+            collection = %self.inner.name,
+            segment_ulid = %meta.ulid,
+            doc_count = meta.doc_count,
+            segment_count = self.segment_count(),
+            "flush done"
+        );
         Ok(())
     }
 
     /// 当前段数（测试与诊断用）。
     pub fn segment_count(&self) -> usize {
         self.inner.snapshot.read().unwrap().len()
     }
 
     /// 选最小两段合并（auto-merge on exceeding SEGMENT_MAX，SPEC §3.3）。
     fn auto_merge_two_smallest(&self) -> Result<()> {
@@ -513,20 +522,29 @@ impl Collection {
         } else {
             snap.iter()
                 .filter(|r| !source_ulids.contains(&r.meta().ulid))
                 .map(|r| {
                     let base = offsets.get(&r.meta().ulid).copied().unwrap_or(0);
                     base + r.doc_count() as u64
                 })
                 .max()
                 .unwrap_or(0)
         };
+        // M4 §3.5 tracing：merge 频率——入口埋点（sources 数 + target base + 是否全合并）。
+        #[cfg(feature = "tracing")]
+        tracing::info!(
+            collection = %self.inner.name,
+            sources = source_ulids.len(),
+            target_docid_base,
+            full_merge = is_full_merge,
+            "merge start"
+        );
         let tokenizer_arc = self.inner.tokenizer.read().unwrap().clone();
         let tok_id = self.inner.tokenizer_id.read().unwrap().clone();
         let mut task = crate::merge::MergeTask::new(
             source_ulids.clone(),
             target_docid_base,
             tok_id,
             self.inner.schema.clone(),
             tokenizer_arc,
         );
         task.set_tombstones(tombstones);
@@ -624,20 +642,29 @@ impl Collection {
                     let old_seg_dir = format!("{}/seg_{}", self.inner.segments_dir, r.meta().ulid);
                     let _ = crate::merge::delete_segment_dir(self.inner.vfs.as_ref(), &old_seg_dir);
                 }
             }
             offsets_w.insert(new_meta.ulid.clone(), new_meta.docid_base);
             snap_w.push(new_reader);
             inv_w.push(new_inv);
             hnsw_w.push(new_hnsw);
             scalar_w.push(new_scalar);
         }
+        // M4 §3.5 tracing：merge 完成——新段 ULID + 段数。cfg 门控，编译期消除。
+        #[cfg(feature = "tracing")]
+        tracing::info!(
+            collection = %self.inner.name,
+            new_segment_ulid = %new_meta.ulid,
+            new_doc_count = new_meta.doc_count,
+            segment_count = self.segment_count(),
+            "merge done"
+        );
         Ok(())
     }
 
     pub fn search(&self, query: &SearchQuery) -> Result<Vec<Hit>> {
         self.run_search(query, true)
     }
 
     /// 12-recall-regression 测试/bench 辅助：暴力双路+RRF 基线（绕过 HNSW）。
     ///
     /// SPEC §13.2-1 基线口径 = `brute_search`（vector 路）+ `InvertedIndexReader::search`
@@ -673,20 +700,32 @@ impl Collection {
                 (Some(_), Some(_)) => SearchMode::Hybrid,
                 (Some(_), None) => SearchMode::Text,
                 (None, Some(_)) => SearchMode::Vector,
                 (None, None) => {
                     return Err(VaneError::InvalidArg(
                         "search requires text or vector".into(),
                     ))
                 }
             },
         };
+        // M4 §3.5 tracing：检索延迟 span + elapsed。cfg 门控，tracing off 时编译期消除。
+        // 早期返回（topK 超限/缺 text+vector）不经此 span——属参数校验 fast-fail，无需埋点。
+        #[cfg(feature = "tracing")]
+        let _span = tracing::info_span!(
+            "search",
+            top_k = query.top_k,
+            mode = ?mode,
+            segment_count = self.segment_count(),
+            allow_hnsw
+        );
+        #[cfg(feature = "tracing")]
+        let _search_start = web_time::Instant::now();
         // dim 校验 + metric 一次性解析（hoist 出循环，避免每段重复 vector_field() 调用）
         let vf = if let Some(v) = &query.vector {
             let (_, dim, metric) = self.inner.schema.vector_field()?;
             if v.len() as u32 != dim {
                 return Err(VaneError::Schema(format!(
                     "query vector dim {} != schema dim {}",
                     v.len(),
                     dim
                 )));
             }
@@ -981,20 +1020,26 @@ impl Collection {
                 }
             }
             if let Some(id) = found_id {
                 hits.push(Hit {
                     id,
                     score: sd.score,
                     fields: found_fields,
                 });
             }
         }
+        #[cfg(feature = "tracing")]
+        tracing::info!(
+            elapsed_us = _search_start.elapsed().as_micros() as u64,
+            hits = hits.len(),
+            "search done"
+        );
         Ok(hits)
     }
 
     /// 当前段快照的 ULID 列表（测试与诊断用；01-hnsw Task 5 测试依赖）。
     pub fn segment_ulids(&self) -> Vec<String> {
         self.inner
             .snapshot
             .read()
             .unwrap()
             .iter()
@@ -1127,20 +1172,29 @@ impl Collection {
     /// - 词表上限 10 万词条（§5.3），超限返回 DictTooLarge。
     /// - 多次调用覆盖暂存词表（放弃旧暂存，SPEC §7.4 状态机「放弃」路径）。
     pub fn set_user_dict(&self, dict: &[UserDictEntry]) -> Result<()> {
         if dict.len() > crate::tokenizer::MAX_USER_DICT_ENTRIES {
             return Err(VaneError::DictTooLarge);
         }
         let mut state = self.inner.dict_state.write().unwrap();
         if *state == DictState::Rebuilding {
             return Err(VaneError::Busy);
         }
+        // M4 §3.5 tracing：词典状态迁移 Stable→PendingReindex（state transition）。
+        #[cfg(feature = "tracing")]
+        tracing::info!(
+            collection = %self.inner.name,
+            from = ?*state,
+            to = ?DictState::PendingReindex,
+            dict_entries = dict.len(),
+            "dict state transition"
+        );
         *self.inner.pending_dict.write().unwrap() = dict.to_vec();
         *state = DictState::PendingReindex;
         Ok(())
     }
 
     /// 查询当前词表状态（绑定层暴露 needsReindex）。
     pub fn dict_state(&self) -> DictState {
         *self.inner.dict_state.read().unwrap()
     }
 
@@ -1177,20 +1231,28 @@ impl Collection {
         {
             let state = self.inner.dict_state.read().unwrap();
             if *state != DictState::PendingReindex {
                 return Err(VaneError::InvalidArg(
                     "reindex requires PendingReindex state; call set_user_dict first".into(),
                 ));
             }
         }
         // state → Rebuilding。
         *self.inner.dict_state.write().unwrap() = DictState::Rebuilding;
+        // M4 §3.5 tracing：词典状态迁移 PendingReindex→Rebuilding（state transition）。
+        #[cfg(feature = "tracing")]
+        tracing::info!(
+            collection = %self.inner.name,
+            from = ?DictState::PendingReindex,
+            to = ?DictState::Rebuilding,
+            "dict state transition"
+        );
         let result = self.run_reindex();
         match result {
             Ok(handle) => Ok(handle),
             Err(e) => {
                 // 重建失败：回退状态为 PendingReindex（词表仍在 pending_dict），
                 // 允许调用方修正后重试。旧段未被删除（reindex 先建新段再删旧）。
                 *self.inner.dict_state.write().unwrap() = DictState::PendingReindex;
                 Err(e)
             }
         }
@@ -1354,13 +1416,21 @@ impl Collection {
         }
 
         // 删除旧段目录。
         for ulid in &old_ulids {
             let old_seg_dir = format!("{}/seg_{}", self.inner.segments_dir, ulid);
             let _ = crate::merge::delete_segment_dir(self.inner.vfs.as_ref(), &old_seg_dir);
         }
 
         // state → Stable。
         *self.inner.dict_state.write().unwrap() = DictState::Stable;
+        // M4 §3.5 tracing：词典状态迁移 Rebuilding→Stable（reindex 完成）。
+        #[cfg(feature = "tracing")]
+        tracing::info!(
+            collection = %self.inner.name,
+            from = ?DictState::Rebuilding,
+            to = ?DictState::Stable,
+            "dict state transition"
+        );
         Ok(ReindexHandle::completed())
     }
 }
diff --git a/crates/vane-core/src/vfs/page_cache.rs b/crates/vane-core/src/vfs/page_cache.rs
index 8f4805f..59bb00e 100644
--- a/crates/vane-core/src/vfs/page_cache.rs
+++ b/crates/vane-core/src/vfs/page_cache.rs
@@ -42,29 +42,41 @@ impl PageCache {
             let page_off = (cur_off % self.page_size as u64) as usize;
             let chunk = remaining.min(self.page_size - page_off);
 
             let page_data = {
                 let key = (path.to_string(), page_idx);
                 let hit = self.inner.pages.get(&key).cloned();
                 match hit {
                     Some(data) => {
                         // 命中：移动到 LRU 尾
                         self.inner.touch(path.to_string(), page_idx);
+                        // M4 §3.5 tracing：缓存命中率——命中事件。cfg 门控，编译期消除。
+                        #[cfg(feature = "tracing")]
+                        tracing::debug!(hit = true, path = path, page_idx, "page_cache");
                         data
                     }
                     None => {
                         // 未命中：从 vfs 加载整页
                         let mut page_buf = vec![0u8; self.page_size];
                         let page_start = page_idx * self.page_size as u64;
                         let n = vfs.read_at(path, &mut page_buf, page_start)?;
                         page_buf.truncate(n);
                         self.inner.put(path.to_string(), page_idx, page_buf.clone());
+                        // M4 §3.5 tracing：缓存命中率——未命中事件。cfg 门控，编译期消除。
+                        #[cfg(feature = "tracing")]
+                        tracing::debug!(
+                            hit = false,
+                            path = path,
+                            page_idx,
+                            bytes = n,
+                            "page_cache"
+                        );
                         page_buf
                     }
                 }
             };
 
             let copy_n = chunk.min(page_data.len().saturating_sub(page_off));
             if copy_n > 0 {
                 result[out_off..out_off + copy_n]
                     .copy_from_slice(&page_data[page_off..page_off + copy_n]);
             }
diff --git a/crates/vane-core/src/wal/mod.rs b/crates/vane-core/src/wal/mod.rs
index 4ff5b61..d88e8f6 100644
--- a/crates/vane-core/src/wal/mod.rs
+++ b/crates/vane-core/src/wal/mod.rs
@@ -56,20 +56,23 @@ impl Wal {
     /// 打开（或首次创建）`<db>/wal.log`。幂等：文件已存在则保留（追加语义）。
     pub fn open(vfs: Arc<dyn Vfs>, db_path: &str) -> Result<Self> {
         let path = format!("{}/{}", db_path, WAL_FILENAME);
         // 幂等 create：已存在则忽略（Vfs::create 在已存在时返回 Io 错误，此处 best-effort）。
         let _ = vfs.create(&path);
         Ok(Self { vfs, path })
     }
 
     /// 追加一条记录（JSON 行，每行一条；append 后 sync 保证崩溃前落盘）。
     pub fn append(&self, record: &WalRecord) -> Result<()> {
+        // M4 §3.5 tracing：WAL append 次数——记录值（Debug）。cfg 门控，编译期消除。
+        #[cfg(feature = "tracing")]
+        tracing::debug!(?record, "wal append");
         let mut line = serde_json::to_vec(record)
             .map_err(|e| VaneError::Corrupt(format!("wal serialize: {}", e)))?;
         line.push(b'\n');
         self.vfs.append(&self.path, &line)?;
         self.vfs.sync(&self.path)?;
         Ok(())
     }
 
     /// 读取全部记录（崩溃恢复用）。文件不存在（新库）返回空。
     pub fn read_all(&self) -> Result<Vec<WalRecord>> {
diff --git a/docs/plans/m4/task-tracing-report.md b/docs/plans/m4/task-tracing-report.md
new file mode 100644
index 0000000..0fdb98e
--- /dev/null
+++ b/docs/plans/m4/task-tracing-report.md
@@ -0,0 +1,216 @@
+# M4 Phase 5a — tracing feature（cfg 门控零开销埋点）报告
+
+- **分支**：`feat/m4-prod-readiness`
+- **BASE**：`0cb50e5`（M4 Phase 3 跨版本兼容报告/审查 artifacts 提交）
+- **commit**：`86a2b81`（`feat(core): tracing feature（cfg 门控零开销埋点，默认 off）（M4 阶段五 a）`）
+- **范围**：`crates/vane-core` 首个触动生产代码的 M4 phase——新增 `tracing` feature + `#[cfg(feature="tracing")]` 门控埋点。纯新增，不改 M0-M3 冻结 pub API。
+- **brief**：`docs/plans/m4/phase0-design.md` §3.5（tracing feature 骨架）。
+
+## 1. feature 定义（`crates/vane-core/Cargo.toml`）
+
+按设计 §3.5 字面采用。
+
+```toml
+[dependencies]
+# tracing：可观测性埋点（M4 §3.5，I-5 能力开关）。
+# optional + cfg(feature="tracing") 门控，默认 off——不启用时编译期消除，
+# wasm/native 体积不变（800KB gzip 红线）。vane-wasm 不启用 tracing（守护红线）。
+# 不触黑名单：传递依赖 tracing-core/thread_local/cfg-if（无 regex/tokio/prost/tonic/
+# openssl/lindera/ndarray/wee_alloc/dashmap/parking_lot），cargo deny check 守护。
+# tracing-subscriber（消费侧）不进 core——vane-ffi/vane-node 按需加 dev-dep。
+tracing = { version = "0.1", optional = true }
+
+[features]
+# tracing：埋点能力开关（I-5，M4 §3.5）。启用时检索延迟/段数/merge 频率/
+# 缓存命中率/WAL append/词典状态指标可观测；不启用时编译期消除，wasm/native
+# 体积不变（800KB gzip 红线）。所有 tracing 调用经 `#[cfg(feature="tracing")]`
+# 门控。默认 off；vane-wasm 不启用（守护红线）；vane-ffi/vane-node native 可启。
+tracing = ["dep:tracing"]
+```
+
+**机制选型**：设计 §3.5 给出两选一（telemetry.rs 宏 vs 直接在各模块 `#[cfg(feature="tracing")] tracing::info!(...)`），**推荐后者**（少一层抽象）。采用推荐——不新建 `telemetry.rs`，所有埋点直接在各模块用 `#[cfg(feature="tracing")]` + `tracing::{span,info,debug}!` 宏。`tracing` crate 经 `dep:tracing` 隔离（Cargo 自动 hash 不会因 feature 名与 crate 名相同而误解析为 implicit feature）。
+
+## 2. 埋点位置清单（§3.5 表字面落实）
+
+| 指标 | 埋点位置 | span/事件 | 文件:行 |
+|---|---|---|---|
+| 检索延迟 | `api/collection.rs::run_search` 入口 span + 出口 elapsed | `tracing::info_span!("search", top_k, mode=?mode, segment_count, allow_hnsw)` + `tracing::info!(elapsed_us, hits, "search done")` | `api/collection.rs:683-695`（span）+ `:1003-1009`（done） |
+| 段数 | `flush` 末 | `tracing::info!(collection, segment_ulid, doc_count, segment_count, "flush done")` | `api/collection.rs:464-472` |
+| merge 频率 | `merge_segments` 入口 | `tracing::info!(collection, sources, target_docid_base, full_merge, "merge start")` | `api/collection.rs:533-541` |
+| merge 完成 | `merge_segments` 末 | `tracing::info!(collection, new_segment_ulid, new_doc_count, segment_count, "merge done")` | `api/collection.rs:658-666` |
+| 缓存命中率 | `PageCache::read` 命中/未命中 | `tracing::debug!(hit=true, path, page_idx, "page_cache")` / `tracing::debug!(hit=false, path, page_idx, bytes, "page_cache")` | `vfs/page_cache.rs:53-54` / `:65-73` |
+| WAL append | `Wal::append` | `tracing::debug!(?record, "wal append")` | `wal/mod.rs:66-67` |
+| 词典状态（set_user_dict） | `set_user_dict` 状态改写前 | `tracing::info!(collection, from=?state, to=?PendingReindex, dict_entries, "dict state transition")` | `api/collection.rs:1184-1191` |
+| 词典状态（reindex 进入） | `reindex` PendingReindex→Rebuilding | `tracing::info!(collection, from=?PendingReindex, to=?Rebuilding, "dict state transition")` | `api/collection.rs:1244-1251` |
+| 词典状态（reindex 完成） | `run_reindex` Rebuilding→Stable | `tracing::info!(collection, from=?Rebuilding, to=?Stable, "dict state transition")` | `api/collection.rs:1432-1439` |
+
+合计 9 处埋点（8 个 `info!`/`info_span!` + 2 个 `debug!`；其中 PageCache 2 个 `debug!` 算同位置 1 处）。所有埋点经 `#[cfg(feature = "tracing")]` 门控，`feature="tracing"` off 时编译期消除（空展开），运行期零开销 + wasm/native 体积零增量。
+
+## 3. telemetry.rs（不采用）
+
+设计 §3.5 推荐"直接在各模块用 `#[cfg(feature="tracing")] tracing::info!(...)`，少一层抽象"。按推荐执行——**不新建 `crates/vane-core/src/telemetry.rs`**，埋点直接落在各业务模块。理由：
+
+- 抽象层（`trace_span!`/`trace_info!`/`trace_debug!` 宏）增加一层间接，调试时读者须先查宏定义才知埋点语义；
+- `#[cfg(feature="tracing")]` 散布度可控（9 处，集中在 search/flush/merge/PageCache/Wal/词典状态机关键路径，非散乱全模块）；
+- `tracing` crate 的宏本身在 `tracing` feature off 时经 `dep:tracing` 隔离 + `#[cfg(feature)]` 门控已编译期消除，再加一层内部宏不增效益。
+
+## 4. wasm 体积对比（关键：tracing off 体积不变 + 编译期消除 grep=0）
+
+### 4.1 `bash scripts/check-wasm-size.sh`（默认 tracing off）
+
+```
+=== vane-wasm default (real deliverable) ===
+vane-wasm default gzip size: 349261 bytes (max 819200)
+OK: vane-wasm default gzip ≤ 800KB
+
+=== vane-core --export-all (conservative upper bound) ===
+vane-core --export-all gzip size: 641277 bytes (max 819200)
+OK: vane-core --export-all gzip ≤ 800KB
+
+=== Summary ===
+vane-wasm default:      349261 bytes (gzip)
+vane-core --export-all: 641277 bytes (gzip)
+```
+
+对比 Phase 2 基线（Phase 2a 全量门禁确认记录：vane-wasm 349261B / core --export-all 641275B gzip）——**tracing off 体积不变**（349261 持平；641275→641277 = +2B 属构建非确定性噪声，非 tracing 引入）。`vane-wasm` 不引 `tracing` feature（Cargo.toml 未透传），守护 800KB 红线。
+
+### 4.2 编译期消除验证（tracing off 无符号）
+
+```
+$ wasm-objdump -x target/wasm32-unknown-unknown/release/vane_core.wasm | grep -c tracing
+0
+```
+
+`vane_core.wasm`（tracing off，--export-all）内 0 个 tracing 符号——`#[cfg(feature="tracing")]` 门控 + `dep:tracing` 隔离确保 tracing crate 不进 wasm 二进制。**编译期消除验证通过**。
+
+### 4.3 tracing on wasm 体积对比（验证 +15KB 增量，仍 ≤800KB）
+
+```
+$ RUSTFLAGS="-C link-arg=--export-all" cargo build --release --target wasm32-unknown-unknown -p vane-core --features tracing
+$ wasm-opt -Oz ... -o /tmp/vane_core_tracing_on.wasm
+$ gzip -c /tmp/vane_core_tracing_on.wasm | wc -c
+656422
+
+$ wasm-objdump -x target/wasm32-unknown-unknown/release/vane_core.wasm | grep -c tracing
+918
+```
+
+| 配置 | vane-core --export-all gzip | tracing 符号数 |
+|---|---|---|
+| tracing off（默认） | 641277 B | 0 |
+| tracing on | 656422 B | 918 |
+| **增量** | **+15145 B（~15KB）** | +918 |
+
+设计 §3.5 估算 +30-50KB gzip；实测 +15KB gzip，**低于估算下限**（tracing 0.1.44 + tracing-core 0.1.36 + once_cell 较历史版本更精简）。tracing on 体积 656422B ≤ 800KB（819200B）红线，余量 162778B。**vane-wasm 不启 tracing**，故此增量仅 native/ffi 可观测，wasm deliverable 永不变。
+
+## 5. cargo deny check（tracing 依赖链不触黑名单——关键）
+
+### 5.1 tracing 依赖树
+
+```
+$ cargo tree -p vane-core --features tracing -e normal,build | grep -iE "tracing|regex|tokio|prost|tonic|openssl|lindera|ndarray|wee_alloc|dashmap|parking_lot|cfg-if|thread_local|once_cell"
+│   ├── cfg-if v1.0.4   # 来自 sha2，非 tracing
+├── tracing v0.1.44
+│   ├── pin-project-lite v0.2.17
+│   ├── tracing-attributes v0.1.31 (proc-macro)
+│   └── tracing-core v0.1.36
+│       └── once_cell v1.21.4
+```
+
+tracing 0.1.44 传递依赖：
+- `pin-project-lite`（无黑名单依赖）
+- `tracing-attributes` v0.1.31（proc-macro，build-time，→ proc-macro2/quote/syn，无黑名单）
+- `tracing-core` v0.1.36（→ `once_cell`，无黑名单）
+
+设计 §3.5 预判 `tracing-core`→`thread_local`+`cfg-if`；实测新版 `tracing-core` v0.1.36 改用 `once_cell`（非 `thread_local`），`cfg-if` 来自 `sha2` 而非 tracing。**均不触黑名单**（regex/tokio/prost/tonic/openssl/lindera/ndarray/wee_alloc/dashmap/parking_lot 均不在 tracing 依赖链）。
+
+### 5.2 `cargo deny check` 全量输出
+
+```
+$ cargo deny check
+warning[unused-wrapper]: wrapper for banned crate was not encountered
+   ┌─ deny.toml:16:36
+16 │     { name = "regex", wrappers = ["napi-derive-backend", "criterion"] },
+   │                                    ━━━━━━━━━━━━━━━━━ unmatched wrapper
+advisories ok, bans ok, licenses ok, sources ok
+```
+
+- advisories **ok**
+- bans **ok**（tracing 链无黑名单 crate）
+- licenses **ok**（tracing = MIT，tracing-core = MIT，tracing-attributes = MIT，proc-macro2/quote/syn/once_cell/pin-project-lite 均 MIT/Apache-2.0，均在 allow 列表）
+- sources **ok**
+
+唯一 warning 是预存的 `regex` wrappers 未匹配（Phase 1a 之前就有，非 tracing 引入）——非 error，exit 0。
+
+## 6. 各门禁真实输出
+
+| 门禁 | 命令 | 结果 |
+|---|---|---|
+| 格式 | `cargo fmt --all -- --check` | rc=0，无 diff（首次 `tracing::debug!` 多参数 rustfmt 折行已修正） |
+| 静态检查 | `cargo clippy --all-targets --all-features --workspace --exclude vane-fuzz -- -D warnings` | rc=0，0 warnings（stable clippy 能编 tracing crate） |
+| 工作区测试 | `cargo test --workspace --all-features --exclude vane-fuzz` | **EXIT=0**，全 0 failed；代表性：vane-core unittest 322 passed / crash_recovery 5 / proptest_invariants 3 / cross_version_compat 3 / recall 7 / pre_filter 9 / tombstone_merge 9 / userdict_reindex 5 / wal_crash 8 / hnsw_recall 2 / corpus_compat 11 / million_scale 3（105s）/ ndcg_wiki_zh 3（72s）/ vane-node integration 84 / vane-dict-zh 24。tracing on 时埋点编译期参与但无 subscriber 安装→事件 emit 后丢弃，行为与 tracing off 等价（零行为漂移）。 |
+| cargo deny | `cargo deny check` | advisories ok / bans ok / licenses ok / sources ok（1 预存 unused-wrapper warning，非本任务引入） |
+| wasm32 check（tracing off） | `cargo check --target wasm32-unknown-unknown -p vane-core` | rc=0 |
+| wasm32 check（tracing on） | `cargo check --target wasm32-unknown-unknown -p vane-core --features tracing` | rc=0（tracing 在 wasm 可编，once_cell/pin-project-lite/tracing-core 均 wasm32 兼容） |
+| wasm 体积 | `bash scripts/check-wasm-size.sh` | rc=0；vane-wasm 349261B / vane-core --export-all 641277B gzip，均 ≤800KB；tracing off 体积不变 |
+| 编译期消除 | `wasm-objdump -x vane_core.wasm \| grep -c tracing` | **0**（tracing off 无符号） |
+| no-std-fs | `bash scripts/check-no-std-fs.sh` | OK（tracing 埋点不引 std::fs/std::net/mmap） |
+
+## 7. commit
+
+```
+commit 86a2b81
+Author: ximing
+Date:   Wed Aug 12 00:35:13 2026 +0800
+
+    feat(core): tracing feature（cfg 门控零开销埋点，默认 off）（M4 阶段五 a）
+```
+
+commit 内容（`git status --short`）：
+- `crates/vane-core/Cargo.toml`（tracing optional dep + feature 定义）
+- `crates/vane-core/src/api/collection.rs`（search span + flush/merge_segments/dict 状态机埋点）
+- `crates/vane-core/src/vfs/page_cache.rs`（PageCache::read 命中/未命中 debug!）
+- `crates/vane-core/src/wal/mod.rs`（Wal::append debug!）
+- `Cargo.lock`（tracing v0.1.44 + tracing-attributes v0.1.31 + tracing-core v0.1.36 入 lock）
+
+**未触碰**：SPEC.md / CI yml（.github/workflows/ci.yml）/ fault.rs / crash_recovery.rs / vane-fuzz / proptest / cross_version_compat / segment/header.rs / Cargo.toml（根）/ 其他 M0-M3 冻结 pub API 文件。`git status` 确认 commit 只含上述 5 文件。
+
+## 8. 自审
+
+### 8.1 tracing crate 依赖链
+
+- tracing v0.1.44 → pin-project-lite + tracing-attributes（proc-macro）+ tracing-core v0.1.36 → once_cell。
+- 无 regex/tokio/prost/tonic/openssl/lindera/ndarray/wee_alloc/dashmap/parking_lot（cargo deny check bans ok 守护）。
+- 设计 §3.5 预判 `thread_local`+`cfg-if`；实测新版 tracing-core v0.1.36 改用 `once_cell`，更精简（thread_local 不在链）。`cfg-if` 来自 `sha2`（既有），非 tracing 引入。
+- 版本漂移风险（tracing 0.2 可能引黑名单）由 cargo deny check 守护——CI deny job 已配置。
+
+### 8.2 埋点散落度
+
+- 9 处埋点（`info_span!` 1 / `info!` 6 / `debug!` 2），分布在 4 个文件（api/collection.rs、vfs/page_cache.rs、wal/mod.rs）。
+- 集中在 §3.5 表关键路径（search/flush/merge/PageCache/Wal/词典状态机），未散乱全模块——符合设计 §3.5 "集中在 search/flush/merge 关键路径，不过度埋点"。
+- 每处埋点经 `#[cfg(feature = "tracing")]` 门控 + 单行注释标明 M4 §3.5 出处 + 指标名，便于 grep 审计与 future-off 时编译期消除验证。
+
+### 8.3 wasm 体积增量实测 vs 估算
+
+- 设计 §3.5 估算 +30-50KB gzip（启用 tracing 时）。
+- 实测 +15KB gzip（641277→656422，tracing on），**低于估算下限**——tracing 0.1.44 + tracing-core 0.1.36 + once_cell 较设计撰写时（~2025）更精简（tracing-core 改用 once_cell 替代 thread_local）。
+- tracing on 仍 ≤800KB（656422B ≤ 819200B，余量 162778B）——但 vane-wasm 永不启 tracing（守护红线），此增量仅 vane-ffi/vane-node native 可观测路径。
+- tracing off 体积不变（641275→641277 = +2B 构建噪声非 tracing）——**红线不变确认**。
+
+### 8.4 行为漂移
+
+- tracing feature off（默认）：所有埋点编译期消除，运行期零开销，行为与 Phase 2/1/3 完成态完全等价（322+ 单测 + 集成测试 0 回归确认）。
+- tracing feature on：埋点 emit 事件至 tracing dispatch，若无 subscriber 安装（如单元测试）则事件 emit 后丢弃——不影响检索/写入/合并逻辑。测试在 `--all-features`（含 tracing）下跑 0 failed 确认无行为漂移。
+- pub API 不变：埋点全在函数体内 `#[cfg(feature)]` 块，不改任何 pub fn 签名 / struct 定义 / 错误码 / 持久化格式。`tracing` feature 是纯新增能力开关（I-5），符合 SPEC v1.4 §14 I-5 释义。
+
+### 8.5 concerns
+
+1. **CI yml 未加 wasm32-size-tracing-on job**（per task scope 明示 "不碰 CI yml"——Phase 6 加 wasm32-size-tracing-on job 对比）。当前 tracing on wasm 体积验证（§4.3）仅本 report 记录，未进 CI 自动化守护。Phase 6 应加 `wasm32-size-tracing-on` job + `check-wasm-size.sh` 加 tracing on 对比断言。
+2. **`check-wasm-size.sh` 未加 `wasm-objdump -x vane_core.wasm \| grep -c tracing` 断言**（设计 §3.5 推荐加一行）。本 report 已手动验证 grep=0，但脚本未持久化此断言。Phase 6 加此一行（与 concern 1 同步）。
+3. **tracing-subscriber 不进 core**（设计取舍）：core 只 emit 事件，subscriber 是消费侧。vane-ffi/vane-node native 按需加 tracing-subscriber dev-dep + init——非本 phase 范围，defer 至绑定层演进。
+4. **`elapsed_us` 用 `web_time::Instant`**（跨平台，wasm32 用 performance.now()）——已用于 AutoCommitter，tracing 复用同一 Instant 源，无新依赖。但 tracing on wasm 时 `_search_start.elapsed()` 会调 performance.now()，理论上 wasm 检索延迟观测受 wasm 时钟精度限制（ms 级，足够 p50/p99）——非缺陷，记为 known-limitation。
+5. **`tracing::info_span!` RAII guard**：`_span` 在 `run_search` 入口绑定，函数返回时 drop 记录 span exit。早期返回（topK 超限/缺 text+vector/dim 不匹配）在 span 建立前——这些路径不经 span，是参数校验 fast-fail 的预期行为，非埋点缺口。span 建立后的 `?` 错误传播会 drop span 记录 exit（subscriber 可观测 span 持续期），但 `elapsed_us` done 事件仅在成功路径 emit——错误路径无 elapsed 事件。若需错误路径 elapsed，可改用 `tracing::instrument` 属性宏（需 tracing-attributes，已在 dep 但未用）——defer。
+
+## 9. 状态
+
+**DONE_WITH_CONCERNS**：tracing feature 落地 + 9 处埋点 + wasm off 体积不变（grep=0）+ deny 绿 + 全门禁绿。5 项 concerns 全属 Phase 6 CI/SPEC 修订范围（CI yml 加 tracing-on job + check-wasm-size.sh 加 grep 断言 + tracing-subscriber 绑定层 defer + elapsed_us wasm 时钟精度 known-limitation + 错误路径 elapsed defer），非本 phase 阻塞。
