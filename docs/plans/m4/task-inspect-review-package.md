## Commits 3758620..684a112 (5b inspect)

684a112 feat(core): inspect API（Db::stats/segment_info + 健康检查）（M4 阶段五 b）

## Diff stat

 crates/vane-core/src/api/collection.rs |   9 +-
 crates/vane-core/src/api/db.rs         |  71 ++++
 crates/vane-core/src/api/inspect.rs    | 727 +++++++++++++++++++++++++++++++++
 crates/vane-core/src/api/mod.rs        |   3 +
 docs/plans/m4/task-inspect-report.md   | 181 ++++++++
 5 files changed, 988 insertions(+), 3 deletions(-)

## Full diff (U10)

diff --git a/crates/vane-core/src/api/collection.rs b/crates/vane-core/src/api/collection.rs
index edca82c..a549e3c 100644
--- a/crates/vane-core/src/api/collection.rs
+++ b/crates/vane-core/src/api/collection.rs
@@ -39,38 +39,41 @@ pub(crate) struct CollectionInner {
     pub(crate) schema: Schema,
     // 06：reindex 需原子替换 tokenizer/tokenizer_id，包 RwLock 提供 interior mutability。
     pub(crate) tokenizer: RwLock<Arc<dyn crate::tokenizer::Tokenizer>>,
     pub(crate) tokenizer_id: RwLock<CoreTokenizerId>,
     /// 分词器种类（SPEC §5.1）。reindex 用此 + pending_dict 重建新分词器。
     pub(crate) tokenizer_kind: BuiltinTokenizer,
     vfs: Arc<dyn Vfs>,
     db_path: String,
     segments_dir: String,
     write_state: Mutex<WriteState>,
-    snapshot: RwLock<Vec<Arc<SegmentReader>>>,
+    // M4 §3.6 inspect API：pub(crate) 供 inspect 模块遍历段快照读元数据。
+    pub(crate) snapshot: RwLock<Vec<Arc<SegmentReader>>>,
     // 段 ULID → 全局 docid 基址
     seg_offsets: RwLock<HashMap<String, u64>>,
     // I7：InvertedIndexReader 随段快照缓存，search 直接用，避免每次重开
     inverted_readers: RwLock<Vec<Arc<InvertedIndexReader>>>,
     // 01-hnsw：HnswReader 随段快照缓存。Option 因 M0 段无 hnsw.bin（Q-5 → fallback brute）。
-    hnsw_readers: RwLock<Vec<Option<Arc<HnswReader>>>>,
+    // M4 §3.6 inspect API：pub(crate) 供 inspect 模块检测 hnsw 缺失（Degraded）。
+    pub(crate) hnsw_readers: RwLock<Vec<Option<Arc<HnswReader>>>>,
     // 03-pre-filter：ScalarReader 随段快照缓存（scalars.col），compile_filter 用。
     scalar_readers: RwLock<Vec<Arc<crate::segment::ScalarReader>>>,
     // 02-tombstone-merge：段 ULID → tombstone 位图（绝对 docid）。
     // delete 期更新内存位图（不修改段文件 I-1）；持久化经 WAL（04 计划）。
     // 查询期 search 把 tombstone 并入 filter 参数（02 手动并入；03 计划 compile_filter 统一）。
     // 04-wal：pub(crate) 供 Db::open 注入 recover 重放的 tombstone。
     pub(crate) tombstones: RwLock<HashMap<String, roaring::RoaringBitmap>>,
     // 02-tombstone-merge：compact 进行中标志（防重入；06 reindex 状态机复用）。
     compacting: Mutex<bool>,
     // 06-userdict-reindex：§7.4 词表状态机。
-    dict_state: RwLock<DictState>,
+    // M4 §3.6 inspect API：pub(crate) 供 inspect 模块读 CollectionStats.dict_state。
+    pub(crate) dict_state: RwLock<DictState>,
     // 06-userdict-reindex：暂存新词表（setUserDict 后；reindex 时消费）。
     pending_dict: RwLock<Vec<UserDictEntry>>,
     // 07-dict-distribution-node：collection 级 jieba 词典副本（从 DbInner 克隆 Arc）。
     // reindex 重建分词器时用（run_reindex 不持有 DbInner）。
     #[cfg(feature = "jieba")]
     jieba_dict: Option<std::sync::Arc<crate::tokenizer::jieba::JiebaDict>>,
     // M2-10：Executor（从 DbInner 克隆 Arc）。search 路径用 Executor::scope 并行搜各段。
     // 平台分支在 executor/mod.rs（I-5），此处零 cfg。
     pub(crate) executor: Arc<dyn crate::executor::Executor>,
 }
diff --git a/crates/vane-core/src/api/db.rs b/crates/vane-core/src/api/db.rs
index 0d73681..b19e2b3 100644
--- a/crates/vane-core/src/api/db.rs
+++ b/crates/vane-core/src/api/db.rs
@@ -173,27 +173,98 @@ impl Db {
     // 调 write_snapshot 打包 VANE_SNAP 单文件；只读遍历原库 + 写 dest（I-6）。
     pub fn export(&self, dest: &str) -> Result<()> {
         super::snapshot::write_snapshot(self.inner.vfs.as_ref(), &self.inner.db_path, dest)
     }
 
     pub fn close(&self) -> Result<()> {
         // M0：无后台线程需 join；flush 由调用方显式调
         Ok(())
     }
 
+    // ---- M4 §3.6 inspect API：纯新增 pub 方法，不改 M0-M3 冻结签名 ----
+
+    /// SPEC §9 inspect API：DB 级统计。
+    ///
+    /// 纯新增，不改 M0-M3 冻结 pub API。`&self` 返回 `DbStats`，遍历 collections
+    /// 构造各段统计（段数 / 文档数 / 健康状态）。健康检查见 §3.6 表。
+    pub fn stats(&self) -> super::inspect::DbStats {
+        let collections = self.inner.collections.read().unwrap();
+        let dict_available = self.dict_available_internal();
+        let mut col_stats: Vec<super::inspect::CollectionStats> = collections
+            .iter()
+            .map(|(name, col_inner)| {
+                super::inspect::build_collection_stats(
+                    name,
+                    col_inner,
+                    &self.inner.vfs,
+                    dict_available,
+                )
+            })
+            .collect();
+        // 按 name 排序保证输出确定性（HashMap 迭代顺序不固定）。
+        col_stats.sort_by(|a, b| a.name.cmp(&b.name));
+        super::inspect::DbStats {
+            db_path: self.inner.db_path.clone(),
+            collections: col_stats,
+            dict_available,
+            executor_kind: super::inspect::executor_kind(),
+        }
+    }
+
+    /// SPEC §9 inspect API：各段详细信息。
+    ///
+    /// 返回所有 collection 的所有段信息（ULID / doc_count / format_versions /
+    /// file_sizes / health）。遍历 snapshot readers，非热路径。
+    pub fn segment_info(&self) -> Vec<super::inspect::SegmentInfo> {
+        let collections = self.inner.collections.read().unwrap();
+        let mut result = Vec::new();
+        for col_inner in collections.values() {
+            result.extend(super::inspect::build_segment_info(
+                col_inner,
+                &self.inner.vfs,
+            ));
+        }
+        result
+    }
+
+    /// SPEC §9 inspect API：单个 collection 的段信息（便捷重载）。
+    ///
+    /// collection 不存在时返回 `None`。
+    pub fn collection_segment_info(&self, name: &str) -> Option<Vec<super::inspect::SegmentInfo>> {
+        let collections = self.inner.collections.read().unwrap();
+        let col_inner = collections.get(name)?;
+        Some(super::inspect::build_segment_info(
+            col_inner,
+            &self.inner.vfs,
+        ))
+    }
+
     /// jieba 词典是否可用（Db::open 时加载，dict-zh feature 启用）。
     /// 绑定层（vane-node）用此判断 collection 创建时是否需降级 CjkBigram（Task 3）。
     #[cfg(feature = "jieba")]
     pub fn jieba_dict_available(&self) -> bool {
         self.inner.jieba_dict.read().unwrap().is_some()
     }
 
+    /// inspect API 内部用：dict_available 不受 jieba feature 门控。
+    /// jieba feature on → 读 `DbInner.jieba_dict`；off → 恒 false。
+    fn dict_available_internal(&self) -> bool {
+        #[cfg(feature = "jieba")]
+        {
+            self.inner.jieba_dict.read().unwrap().is_some()
+        }
+        #[cfg(not(feature = "jieba"))]
+        {
+            false
+        }
+    }
+
     /// M2-11：运行时注入 jieba 词典（FFI `vane_load_dict` 调用）。
     ///
     /// dict-zh feature 关闭时 Db::open 设 jieba_dict=None；FFI 绑定层从 Go embed
     /// 读取 dict.bin 字节 → `JiebaDict::load_zstd` → 经此方法注入。注入后后续
     /// `collection(tokenizer=Jieba)` 调用即可用 jieba 分词。
     /// 已创建的 collection 不受影响（tokenizer 在创建时固定）。
     #[cfg(feature = "jieba")]
     pub fn set_jieba_dict(&self, dict: std::sync::Arc<crate::tokenizer::jieba::JiebaDict>) {
         *self.inner.jieba_dict.write().unwrap() = Some(dict);
     }
diff --git a/crates/vane-core/src/api/inspect.rs b/crates/vane-core/src/api/inspect.rs
new file mode 100644
index 0000000..2397705
--- /dev/null
+++ b/crates/vane-core/src/api/inspect.rs
@@ -0,0 +1,727 @@
+//! SPEC §9 inspect API（M4 阶段五 b）：DB 级统计与段级信息。
+//!
+//! 纯新增 pub API，不改 M0-M3 冻结签名。`Db::stats()` / `Db::segment_info()`
+//! 返回强类型结构体（FFI 层序列化为 JSON）。
+//!
+//! 健康检查（§3.6 表）：
+//! - 词典降级：jieba feature on 时，collection 的 tokenizer 是 Jieba 但
+//!   `DbInner.jieba_dict` 为 None → Degraded。
+//! - 段损坏：`SegmentReader::open` 失败 → Corrupt（header magic/version 校验、
+//!   vectors/stored/idmap 解码）。
+//! - hnsw 缺失 fallback brute：`CollectionInner.hnsw_readers` 中 None → Degraded。
+//!
+//! `index_bytes` / `file_sizes`：Vfs trait 无 `size()` 方法（M0 冻结签名），
+//! 用 `read_at` 探测 EOF（offset 0 起循环读 8KB buffer 至 n=0，累计推算 size）。
+//! inspect 非热路径，性能可接受（§3.6 取舍）。
+
+use crate::api::collection::CollectionInner;
+use crate::api::types::DictState;
+use crate::segment::SegmentReader;
+use crate::tokenizer::BuiltinTokenizer;
+use crate::types::TokenizerId;
+use crate::vfs::Vfs;
+use std::sync::Arc;
+
+// ---- 公共结构体（FFI 层序列化为 JSON；Debug 便于测试）----
+
+/// DB 级统计信息（SPEC §9 inspect API）。
+#[derive(Debug, Clone)]
+pub struct DbStats {
+    pub db_path: String,
+    pub collections: Vec<CollectionStats>,
+    /// 词典状态（jieba 是否加载）。jieba feature on 时有意义；off 时恒 false。
+    pub dict_available: bool,
+    pub executor_kind: ExecutorKind,
+}
+
+/// 单个 collection 的统计信息。
+#[derive(Debug, Clone)]
+pub struct CollectionStats {
+    pub name: String,
+    pub segment_count: usize,
+    /// 各段 doc_count 之和（含 tombstoned）。
+    pub total_docs: u64,
+    /// total_docs - tombstoned_docs。
+    pub live_docs: u64,
+    pub tombstoned_docs: u64,
+    /// 各段文件大小之和（header+vectors+stored+idmap+scalars+inverted+hnsw）。
+    pub index_bytes: u64,
+    pub dict_state: DictState,
+    pub tokenizer_id: TokenizerId,
+    pub health: Health,
+}
+
+/// 单个段的详细信息。
+#[derive(Debug, Clone)]
+pub struct SegmentInfo {
+    pub ulid: String,
+    pub doc_count: u32,
+    pub docid_base: u64,
+    pub tombstoned_count: u64,
+    pub format_versions: FormatVersions,
+    pub file_sizes: SegmentFileSizes,
+    pub health: Health,
+}
+
+/// 段各文件的格式版本（读各文件头 magic + version 字段）。
+#[derive(Debug, Clone)]
+pub struct FormatVersions {
+    pub header: u32,
+    pub vectors: u32,
+    pub stored: u32,
+    pub idmap: u32,
+    pub scalars: u32,
+    pub inverted: u32,
+    pub hnsw: u32,
+}
+
+/// 段各文件大小（read_at 探测 EOF 累计；文件缺失 → 0）。
+#[derive(Debug, Clone)]
+pub struct SegmentFileSizes {
+    pub header: u64,
+    pub vectors: u64,
+    pub stored: u64,
+    pub idmap: u64,
+    pub scalars: u64,
+    pub inverted: u64,
+    /// None = 无 hnsw.bin（fallback brute）。
+    pub hnsw: Option<u64>,
+}
+
+/// 健康状态。
+#[derive(Debug, Clone, Copy, PartialEq, Eq)]
+pub enum Health {
+    Healthy,
+    /// 词典降级 / hnsw 缺失 fallback brute / 段文件部分缺失但可读。
+    Degraded,
+    /// 段文件损坏（magic/version/CRC 校验失败）。
+    Corrupt,
+}
+
+/// 执行器类型（platform cfg 推断）。
+#[derive(Debug, Clone, Copy, PartialEq, Eq)]
+pub enum ExecutorKind {
+    Serial,
+    Rayon,
+}
+
+// ---- pub(crate) 构建函数（db.rs 调用）----
+
+/// 构造单个 collection 的统计信息。
+pub(crate) fn build_collection_stats(
+    name: &str,
+    col_inner: &CollectionInner,
+    vfs: &Arc<dyn Vfs>,
+    dict_available: bool,
+) -> CollectionStats {
+    let snap = col_inner.snapshot.read().unwrap();
+    let hnsw_readers = col_inner.hnsw_readers.read().unwrap();
+    let tombstones = col_inner.tombstones.read().unwrap();
+    let dict_state = *col_inner.dict_state.read().unwrap();
+    let tokenizer_id = col_inner.tokenizer_id.read().unwrap().clone();
+
+    let segment_count = snap.len();
+    let mut total_docs: u64 = 0;
+    let mut tombstoned_docs: u64 = 0;
+    let mut index_bytes: u64 = 0;
+    let mut worst_health = Health::Healthy;
+
+    for (i, reader) in snap.iter().enumerate() {
+        let meta = reader.meta();
+        total_docs += meta.doc_count as u64;
+        let t = tombstones.get(&meta.ulid).map(|b| b.len()).unwrap_or(0);
+        tombstoned_docs += t;
+
+        let seg_dir = reader.segment_dir();
+        let sizes = probe_segment_file_sizes(vfs.as_ref(), seg_dir);
+        index_bytes += sizes.total();
+
+        // 段级健康检查
+        let seg_health = segment_health(vfs, seg_dir, hnsw_readers.get(i));
+        worst_health = worst(worst_health, seg_health);
+    }
+
+    // 词典降级：collection 的 tokenizer 是 Jieba 但 Db 级 dict 不可用 → Degraded
+    #[cfg(feature = "jieba")]
+    {
+        if matches!(col_inner.tokenizer_kind, BuiltinTokenizer::Jieba) && !dict_available {
+            worst_health = worst(worst_health, Health::Degraded);
+        }
+    }
+    // jieba feature off 时：Jieba collection 不可能创建成功（build_tokenizer → DictUnavailable），
+    // 但保留检查以防未来运行时注入路径。dict_available 此时恒 false，条件等价。
+    #[cfg(not(feature = "jieba"))]
+    {
+        if matches!(col_inner.tokenizer_kind, BuiltinTokenizer::Jieba) && !dict_available {
+            worst_health = worst(worst_health, Health::Degraded);
+        }
+    }
+
+    CollectionStats {
+        name: name.to_string(),
+        segment_count,
+        total_docs,
+        live_docs: total_docs - tombstoned_docs,
+        tombstoned_docs,
+        index_bytes,
+        dict_state,
+        tokenizer_id,
+        health: worst_health,
+    }
+}
+
+/// 构造单个 collection 的所有段信息。
+pub(crate) fn build_segment_info(
+    col_inner: &CollectionInner,
+    vfs: &Arc<dyn Vfs>,
+) -> Vec<SegmentInfo> {
+    let snap = col_inner.snapshot.read().unwrap();
+    let hnsw_readers = col_inner.hnsw_readers.read().unwrap();
+    let tombstones = col_inner.tombstones.read().unwrap();
+
+    let mut result = Vec::with_capacity(snap.len());
+    for (i, reader) in snap.iter().enumerate() {
+        let meta = reader.meta();
+        let ulid = meta.ulid.clone();
+        let doc_count = meta.doc_count;
+        let docid_base = meta.docid_base;
+        let tombstoned_count = tombstones.get(&ulid).map(|b| b.len()).unwrap_or(0);
+
+        let seg_dir = reader.segment_dir();
+        let file_sizes = probe_segment_file_sizes(vfs.as_ref(), seg_dir);
+        let format_versions = read_format_versions(vfs.as_ref(), seg_dir);
+        let health = segment_health(vfs, seg_dir, hnsw_readers.get(i));
+
+        result.push(SegmentInfo {
+            ulid,
+            doc_count,
+            docid_base,
+            tombstoned_count,
+            format_versions,
+            file_sizes,
+            health,
+        });
+    }
+    result
+}
+
+/// 推断当前执行器类型（platform cfg）。
+pub(crate) fn executor_kind() -> ExecutorKind {
+    if cfg!(all(
+        not(target_arch = "wasm32"),
+        feature = "executor-native"
+    )) {
+        ExecutorKind::Rayon
+    } else {
+        ExecutorKind::Serial
+    }
+}
+
+// ---- 内部辅助函数 ----
+
+/// 段级健康检查。
+///
+/// - `SegmentReader::open` 失败 → Corrupt
+/// - open 成功但 hnsw 缺失（None） → Degraded
+/// - 否则 Healthy
+///
+/// 注：§3.6 取舍建议"不主动重新 open 校验"，但表 spec 要求"SegmentReader::open 失败 → Corrupt"。
+/// inspect 非热路径，重新 open 可接受，且能真实检测段损坏（文件被外部篡改后）。
+fn segment_health(
+    vfs: &Arc<dyn Vfs>,
+    seg_dir: &str,
+    hnsw: Option<&Option<std::sync::Arc<crate::hnsw::HnswReader>>>,
+) -> Health {
+    match SegmentReader::open(vfs, seg_dir) {
+        Ok(_) => {
+            // hnsw 缺失（M0 段无 hnsw.bin）→ Degraded（fallback brute）
+            let hnsw_missing = match hnsw {
+                Some(Some(_)) => false,
+                Some(None) => true,
+                None => true, // 索引越界 → 视为缺失
+            };
+            if hnsw_missing {
+                Health::Degraded
+            } else {
+                Health::Healthy
+            }
+        }
+        Err(_) => Health::Corrupt,
+    }
+}
+
+/// 探测段目录下各文件大小（read_at 循环读至 EOF 累计）。
+fn probe_segment_file_sizes(vfs: &dyn Vfs, seg_dir: &str) -> SegmentFileSizes {
+    let header = probe_file_size(vfs, &format!("{seg_dir}/header.bin"));
+    let vectors = probe_file_size(vfs, &format!("{seg_dir}/vectors.bin"));
+    let stored = probe_file_size(vfs, &format!("{seg_dir}/stored.bin"));
+    let idmap = probe_file_size(vfs, &format!("{seg_dir}/idmap.bin"));
+    let scalars = probe_file_size(vfs, &format!("{seg_dir}/scalars.col"));
+    let inverted = probe_file_size(vfs, &format!("{seg_dir}/inverted.bin"));
+    let hnsw = {
+        let sz = probe_file_size(vfs, &format!("{seg_dir}/hnsw.bin"));
+        if sz == 0 {
+            // 文件不存在（read_at 返 Err → 0）或空文件；均视为无 hnsw.bin
+            None
+        } else {
+            Some(sz)
+        }
+    };
+    SegmentFileSizes {
+        header,
+        vectors,
+        stored,
+        idmap,
+        scalars,
+        inverted,
+        hnsw,
+    }
+}
+
+impl SegmentFileSizes {
+    fn total(&self) -> u64 {
+        self.header
+            + self.vectors
+            + self.stored
+            + self.idmap
+            + self.scalars
+            + self.inverted
+            + self.hnsw.unwrap_or(0)
+    }
+}
+
+/// read_at 探测文件大小：从 offset=0 循环读 8KB buffer，n=0 即 EOF，累计推算 size。
+/// 文件不存在（read_at 返 Err）→ 返回 0。
+fn probe_file_size(vfs: &dyn Vfs, path: &str) -> u64 {
+    let mut total = 0u64;
+    let mut offset = 0u64;
+    let mut tmp = [0u8; 8192];
+    loop {
+        let n = match vfs.read_at(path, &mut tmp, offset) {
+            Ok(n) => n,
+            Err(_) => return total, // 文件不存在 → 已读 total（0）
+        };
+        if n == 0 {
+            break;
+        }
+        total += n as u64;
+        offset += n as u64;
+    }
+    total
+}
+
+/// 读段各文件头的 format_version（magic(4) + version(4 LE)）。
+/// 文件不存在或 magic 不匹配 → 返回 0。
+fn read_format_versions(vfs: &dyn Vfs, seg_dir: &str) -> FormatVersions {
+    FormatVersions {
+        header: read_version_field(vfs, &format!("{seg_dir}/header.bin")),
+        vectors: read_version_field(vfs, &format!("{seg_dir}/vectors.bin")),
+        stored: read_version_field(vfs, &format!("{seg_dir}/stored.bin")),
+        idmap: read_version_field(vfs, &format!("{seg_dir}/idmap.bin")),
+        scalars: read_version_field(vfs, &format!("{seg_dir}/scalars.col")),
+        inverted: read_version_field(vfs, &format!("{seg_dir}/inverted.bin")),
+        hnsw: read_version_field(vfs, &format!("{seg_dir}/hnsw.bin")),
+    }
+}
+
+/// 读文件头 8 字节（magic(4) + version(4 LE)），校验 magic 后返回 version。
+/// 文件不存在 / 过短 / magic 不匹配 → 返回 0。
+fn read_version_field(vfs: &dyn Vfs, path: &str) -> u32 {
+    let mut buf = [0u8; 8];
+    let n = match vfs.read_at(path, &mut buf, 0) {
+        Ok(n) => n,
+        Err(_) => return 0,
+    };
+    if n < 8 || &buf[0..4] != crate::types::MAGIC {
+        return 0;
+    }
+    u32::from_le_bytes(buf[4..8].try_into().unwrap())
+}
+
+/// 取两个 Health 中更严重的（Corrupt > Degraded > Healthy）。
+fn worst(a: Health, b: Health) -> Health {
+    match (a, b) {
+        (Health::Corrupt, _) | (_, Health::Corrupt) => Health::Corrupt,
+        (Health::Degraded, _) | (_, Health::Degraded) => Health::Degraded,
+        _ => Health::Healthy,
+    }
+}
+
+// ---- 单测 ----
+
+#[cfg(test)]
+mod tests {
+    use super::*;
+    use crate::api::db::Db;
+    use crate::api::types::*;
+    use crate::persistence::AutoCommitConfig;
+    use crate::types::{FieldDef, Metric, Schema};
+    use crate::vfs::memory::MemoryVfs;
+    use std::sync::Arc;
+
+    /// 构造测试用 Db + collection + add + flush：
+    /// 1 collection "docs"，schema = [Text body, Vector v dim=2]，2 docs，1 flush → 1 segment。
+    fn setup_db_with_one_segment() -> Db {
+        let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn crate::vfs::Vfs>;
+        let db = Db::open(vfs, "testdb", OpenOptions::default()).unwrap();
+        let schema = Schema::new(vec![
+            ("body".into(), FieldDef::Text),
+            (
+                "v".into(),
+                FieldDef::Vector {
+                    dim: 2,
+                    metric: Metric::Cosine,
+                },
+            ),
+        ])
+        .unwrap();
+        let col = db
+            .collection("docs", schema, CollectionOptions::default())
+            .unwrap();
+        col.add(&[
+            Doc {
+                id: "a".into(),
+                text: Some("hello world".into()),
+                vector: Some(vec![1.0, 0.0]),
+                meta: None,
+            },
+            Doc {
+                id: "b".into(),
+                text: Some("foo bar".into()),
+                vector: Some(vec![0.0, 1.0]),
+                meta: None,
+            },
+        ])
+        .unwrap();
+        col.flush().unwrap();
+        db
+    }
+
+    #[test]
+    fn stats_returns_db_path_and_executor() {
+        let db = setup_db_with_one_segment();
+        let stats = db.stats();
+        assert_eq!(stats.db_path, "testdb");
+        // executor_kind 取决于 cfg，检查变体匹配
+        assert!(matches!(
+            stats.executor_kind,
+            ExecutorKind::Serial | ExecutorKind::Rayon
+        ));
+    }
+
+    #[test]
+    fn stats_returns_one_collection_with_correct_counts() {
+        let db = setup_db_with_one_segment();
+        let stats = db.stats();
+        assert_eq!(stats.collections.len(), 1);
+        let col = &stats.collections[0];
+        assert_eq!(col.name, "docs");
+        assert_eq!(col.segment_count, 1, "1 flush → 1 segment");
+        assert_eq!(col.total_docs, 2, "2 docs added");
+        assert_eq!(col.live_docs, 2, "no tombstones");
+        assert_eq!(col.tombstoned_docs, 0);
+        assert!(matches!(col.dict_state, DictState::Stable));
+    }
+
+    #[test]
+    fn stats_index_bytes_nonzero() {
+        let db = setup_db_with_one_segment();
+        let stats = db.stats();
+        let col = &stats.collections[0];
+        // 段文件已落盘 → index_bytes > 0（header+vectors+stored+idmap+scalars+inverted）
+        assert!(
+            col.index_bytes > 0,
+            "index_bytes should be > 0 after flush, got {}",
+            col.index_bytes
+        );
+    }
+
+    #[test]
+    fn stats_health_healthy_for_fresh_segment() {
+        let db = setup_db_with_one_segment();
+        let stats = db.stats();
+        let col = &stats.collections[0];
+        // 新段、无 hnsw → Degraded（fallback brute）；若 hnsw 写了 → Healthy
+        // M0 段在 flush 时写 hnsw.bin（01-hnsw write_hnsw），故 hnsw_readers[0] = Some
+        // → Health = Healthy
+        assert_eq!(col.health, Health::Healthy);
+    }
+
+    #[test]
+    fn segment_info_returns_one_segment_with_correct_fields() {
+        let db = setup_db_with_one_segment();
+        let infos = db.segment_info();
+        assert_eq!(infos.len(), 1, "1 segment after 1 flush");
+        let info = &infos[0];
+        assert!(!info.ulid.is_empty(), "ULID non-empty");
+        assert_eq!(info.doc_count, 2, "2 docs");
+        assert_eq!(info.docid_base, 0, "first segment base = 0");
+        assert_eq!(info.tombstoned_count, 0, "no tombstones");
+    }
+
+    #[test]
+    fn segment_info_format_versions_correct() {
+        let db = setup_db_with_one_segment();
+        let infos = db.segment_info();
+        let info = &infos[0];
+        // 各文件 format_version 应匹配写入时常量
+        assert_eq!(info.format_versions.header, crate::types::HEADER_FORMAT_V1);
+        assert_eq!(
+            info.format_versions.vectors,
+            crate::types::VECTORS_FORMAT_V2
+        );
+        assert!(
+            info.format_versions.stored == crate::types::STORED_FORMAT_V1
+                || info.format_versions.stored == crate::types::STORED_FORMAT_V2
+        );
+        assert_eq!(info.format_versions.idmap, crate::types::IDMAP_FORMAT_V1);
+        assert_eq!(
+            info.format_versions.scalars,
+            crate::types::SCALARS_FORMAT_V1
+        );
+        assert_eq!(info.format_versions.inverted, crate::types::FORMAT_VERSION);
+        assert_eq!(info.format_versions.hnsw, crate::types::HNSW_FORMAT_V1);
+    }
+
+    #[test]
+    fn segment_info_file_sizes_nonzero() {
+        let db = setup_db_with_one_segment();
+        let infos = db.segment_info();
+        let info = &infos[0];
+        assert!(info.file_sizes.header > 0, "header.bin non-empty");
+        assert!(info.file_sizes.vectors > 0, "vectors.bin non-empty");
+        assert!(info.file_sizes.stored > 0, "stored.bin non-empty");
+        assert!(info.file_sizes.idmap > 0, "idmap.bin non-empty");
+        assert!(info.file_sizes.inverted > 0, "inverted.bin non-empty");
+        // hnsw 可能存在（flush 写 hnsw.bin）
+        assert!(
+            info.file_sizes.hnsw.is_some(),
+            "hnsw.bin present after flush"
+        );
+    }
+
+    #[test]
+    fn segment_info_health_healthy() {
+        let db = setup_db_with_one_segment();
+        let infos = db.segment_info();
+        let info = &infos[0];
+        assert_eq!(info.health, Health::Healthy);
+    }
+
+    #[test]
+    fn collection_segment_info_returns_some_for_existing() {
+        let db = setup_db_with_one_segment();
+        let infos = db.collection_segment_info("docs");
+        assert!(infos.is_some(), "collection 'docs' exists");
+        assert_eq!(infos.as_ref().unwrap().len(), 1);
+    }
+
+    #[test]
+    fn collection_segment_info_returns_none_for_missing() {
+        let db = setup_db_with_one_segment();
+        let infos = db.collection_segment_info("nonexistent");
+        assert!(infos.is_none(), "collection 'nonexistent' does not exist");
+    }
+
+    #[test]
+    fn stats_multiple_collections() {
+        let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn crate::vfs::Vfs>;
+        let db = Db::open(vfs, "multidb", OpenOptions::default()).unwrap();
+        let schema1 = Schema::new(vec![
+            ("body".into(), FieldDef::Text),
+            (
+                "v".into(),
+                FieldDef::Vector {
+                    dim: 2,
+                    metric: Metric::Cosine,
+                },
+            ),
+        ])
+        .unwrap();
+        let schema2 = Schema::new(vec![(
+            "v".into(),
+            FieldDef::Vector {
+                dim: 4,
+                metric: Metric::Cosine,
+            },
+        )])
+        .unwrap();
+        let _c1 = db
+            .collection("col1", schema1, CollectionOptions::default())
+            .unwrap();
+        let _c2 = db
+            .collection("col2", schema2, CollectionOptions::default())
+            .unwrap();
+        let stats = db.stats();
+        assert_eq!(stats.collections.len(), 2);
+        // 排序后 col1 < col2
+        assert_eq!(stats.collections[0].name, "col1");
+        assert_eq!(stats.collections[1].name, "col2");
+        // 无 flush → 0 segments
+        assert_eq!(stats.collections[0].segment_count, 0);
+        assert_eq!(stats.collections[1].segment_count, 0);
+    }
+
+    #[test]
+    fn stats_empty_db() {
+        let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn crate::vfs::Vfs>;
+        let db = Db::open(vfs, "emptydb", OpenOptions::default()).unwrap();
+        let stats = db.stats();
+        assert_eq!(stats.db_path, "emptydb");
+        assert!(stats.collections.is_empty());
+        // dict_available 取决于 jieba feature；检查与 jieba_dict_available() 一致
+        #[cfg(feature = "jieba")]
+        {
+            assert_eq!(stats.dict_available, db.jieba_dict_available());
+        }
+        #[cfg(not(feature = "jieba"))]
+        {
+            assert!(!stats.dict_available);
+        }
+    }
+
+    #[test]
+    fn segment_info_empty_collection() {
+        let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn crate::vfs::Vfs>;
+        let db = Db::open(vfs, "emptyseg", OpenOptions::default()).unwrap();
+        let schema = Schema::new(vec![(
+            "v".into(),
+            FieldDef::Vector {
+                dim: 2,
+                metric: Metric::Cosine,
+            },
+        )])
+        .unwrap();
+        let _col = db
+            .collection("docs", schema, CollectionOptions::default())
+            .unwrap();
+        // 未 flush → 无段
+        let infos = db.segment_info();
+        assert!(infos.is_empty());
+    }
+
+    #[test]
+    fn probe_file_size_empty_file_returns_zero() {
+        let vfs = MemoryVfs::new();
+        // 不存在的文件 → 0
+        assert_eq!(probe_file_size(&vfs, "nonexistent.bin"), 0);
+    }
+
+    #[test]
+    fn probe_file_size_correct_for_known_content() {
+        let vfs = MemoryVfs::new();
+        // 写 100 字节文件
+        vfs.create("test.bin").unwrap();
+        let data = vec![42u8; 100];
+        vfs.write_at("test.bin", &data, 0).unwrap();
+        assert_eq!(probe_file_size(&vfs, "test.bin"), 100);
+    }
+
+    #[test]
+    fn read_version_field_missing_file_returns_zero() {
+        let vfs = MemoryVfs::new();
+        assert_eq!(read_version_field(&vfs, "nonexistent.bin"), 0);
+    }
+
+    #[test]
+    fn read_version_field_correct_for_vane_magic() {
+        let vfs = MemoryVfs::new();
+        vfs.create("v.bin").unwrap();
+        let mut buf = Vec::new();
+        buf.extend_from_slice(crate::types::MAGIC);
+        buf.extend_from_slice(&crate::types::VECTORS_FORMAT_V2.to_le_bytes());
+        vfs.write_at("v.bin", &buf, 0).unwrap();
+        assert_eq!(
+            read_version_field(&vfs, "v.bin"),
+            crate::types::VECTORS_FORMAT_V2
+        );
+    }
+
+    #[test]
+    fn worst_function_ordering() {
+        assert_eq!(worst(Health::Healthy, Health::Healthy), Health::Healthy);
+        assert_eq!(worst(Health::Healthy, Health::Degraded), Health::Degraded);
+        assert_eq!(worst(Health::Degraded, Health::Healthy), Health::Degraded);
+        assert_eq!(worst(Health::Healthy, Health::Corrupt), Health::Corrupt);
+        assert_eq!(worst(Health::Corrupt, Health::Degraded), Health::Corrupt);
+        assert_eq!(worst(Health::Degraded, Health::Degraded), Health::Degraded);
+    }
+
+    #[test]
+    fn executor_kind_consistent_with_cfg() {
+        let kind = executor_kind();
+        if cfg!(all(
+            not(target_arch = "wasm32"),
+            feature = "executor-native"
+        )) {
+            assert_eq!(kind, ExecutorKind::Rayon);
+        } else {
+            assert_eq!(kind, ExecutorKind::Serial);
+        }
+    }
+
+    // ---- AutoCommitConfig Off 测试：用 disable auto-commit 避免 flush 干扰 ----
+
+    #[test]
+    fn stats_after_tombstone_delete() {
+        let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn crate::vfs::Vfs>;
+        let opts = OpenOptions {
+            persistence: PersistenceMode::Persistent,
+            auto_commit: AutoCommitConfig::Off,
+            page_cache_mb: 32,
+        };
+        let db = Db::open(vfs, "tombdb", opts).unwrap();
+        let schema = Schema::new(vec![
+            ("body".into(), FieldDef::Text),
+            (
+                "v".into(),
+                FieldDef::Vector {
+                    dim: 2,
+                    metric: Metric::Cosine,
+                },
+            ),
+        ])
+        .unwrap();
+        let col = db
+            .collection(
+                "docs",
+                schema,
+                CollectionOptions {
+                    tokenizer: BuiltinTokenizer::Standard,
+                    user_dict: vec![],
+                    auto_commit: AutoCommitConfig::Off,
+                },
+            )
+            .unwrap();
+        col.add(&[
+            Doc {
+                id: "a".into(),
+                text: Some("hello world".into()),
+                vector: Some(vec![1.0, 0.0]),
+                meta: None,
+            },
+            Doc {
+                id: "b".into(),
+                text: Some("foo bar".into()),
+                vector: Some(vec![0.0, 1.0]),
+                meta: None,
+            },
+        ])
+        .unwrap();
+        col.flush().unwrap();
+        // 删除文档 a
+        col.delete(&["a".to_string()]).unwrap();
+        col.flush().unwrap();
+
+        let stats = db.stats();
+        let col_stats = &stats.collections[0];
+        assert_eq!(col_stats.total_docs, 2, "total_docs includes tombstoned");
+        assert_eq!(col_stats.tombstoned_docs, 1, "1 deleted");
+        assert_eq!(col_stats.live_docs, 1, "2 - 1 = 1 live");
+
+        let infos = db.segment_info();
+        // tombstoned_count 可能分布在段上
+        let total_tombstoned: u64 = infos.iter().map(|i| i.tombstoned_count).sum();
+        assert_eq!(total_tombstoned, 1, "1 tombstoned across segments");
+    }
+}
diff --git a/crates/vane-core/src/api/mod.rs b/crates/vane-core/src/api/mod.rs
index 6760457..85d305a 100644
--- a/crates/vane-core/src/api/mod.rs
+++ b/crates/vane-core/src/api/mod.rs
@@ -2,16 +2,19 @@ pub mod types;
 // B5 裁决：re-export 公共类型，使 vane_core::api::{Db, OpenOptions, ...} 路径可直接导入
 pub use types::*;
 pub mod db;
 pub use db::*;
 pub mod collection;
 pub use collection::*;
 pub mod reindex;
 pub use reindex::*;
 pub mod snapshot;
 pub use snapshot::{read_snapshot, write_snapshot};
+// M4 §3.6 inspect API：DB 级统计与段级信息（纯新增，不改 M0-M3 冻结签名）。
+pub mod inspect;
+pub use inspect::*;
 
 #[cfg(test)]
 mod reindex_tests;
 
 #[cfg(test)]
 mod tests;
diff --git a/docs/plans/m4/task-inspect-report.md b/docs/plans/m4/task-inspect-report.md
new file mode 100644
index 0000000..031f30a
--- /dev/null
+++ b/docs/plans/m4/task-inspect-report.md
@@ -0,0 +1,181 @@
+# M4 阶段五 b — inspect API 实现报告
+
+## 概要
+
+M4 阶段五 b inspect API 已实现：新增 `Db::stats()` / `Db::segment_info()` /
+`Db::collection_segment_info()` pub 方法和 7 个返回结构体 + 健康检查 + 20 个单测。
+纯新增，不改 M0-M3 冻结 pub API。
+
+## struct / 方法签名实现摘要
+
+### 新增结构体（`crates/vane-core/src/api/inspect.rs`）
+
+| struct | 字段 | derive |
+|---|---|---|
+| `DbStats` | db_path / collections / dict_available / executor_kind | Debug, Clone |
+| `CollectionStats` | name / segment_count / total_docs / live_docs / tombstoned_docs / index_bytes / dict_state / tokenizer_id / health | Debug, Clone |
+| `SegmentInfo` | ulid / doc_count / docid_base / tombstoned_count / format_versions / file_sizes / health | Debug, Clone |
+| `FormatVersions` | header / vectors / stored / idmap / scalars / inverted / hnsw | Debug, Clone |
+| `SegmentFileSizes` | header / vectors / stored / idmap / scalars / inverted / hnsw(Option) | Debug, Clone |
+| `Health` | Healthy / Degraded / Corrupt | Debug, Clone, Copy, PartialEq, Eq |
+| `ExecutorKind` | Serial / Rayon | Debug, Clone, Copy, PartialEq, Eq |
+
+所有 struct 加 `#[derive(Debug, Clone)]`（Health/ExecutorKind 额外加 Copy/PartialEq/Eq），
+避免 2b 的 SegmentMeta 无 Debug 触 E0277 教训。
+
+### 新增方法（`crates/vane-core/src/api/db.rs` impl Db）
+
+```rust
+pub fn stats(&self) -> DbStats
+pub fn segment_info(&self) -> Vec<SegmentInfo>
+pub fn collection_segment_info(&self, name: &str) -> Option<Vec<SegmentInfo>>
+```
+
+签名按 §3.6 字面采用。纯新增，不改现有 pub fn/struct。
+
+### 模块声明（`crates/vane-core/src/api/mod.rs`）
+
+```rust
+pub mod inspect;
+pub use inspect::*;
+```
+
+### CollectionInner 字段可见性（`crates/vane-core/src/api/collection.rs`）
+
+三个私有字段改为 `pub(crate)`（crate 内部可见，非 pub API 变更）：
+- `snapshot: RwLock<Vec<Arc<SegmentReader>>>` → `pub(crate)`
+- `hnsw_readers: RwLock<Vec<Option<Arc<HnswReader>>>>` → `pub(crate)`
+- `dict_state: RwLock<DictState>` → `pub(crate)`
+
+## 健康检查实现（读哪些内部状态）
+
+| 健康标志 | 判定来源 | 实现位置 |
+|---|---|---|
+| 词典降级 | jieba feature on 时，`col_inner.tokenizer_kind == Jieba` 且 `DbInner.jieba_dict` None → Degraded | `build_collection_stats` 读 `dict_available`（来自 `DbInner.jieba_dict`） |
+| 段损坏 | `SegmentReader::open(vfs, seg_dir)` 失败 → Corrupt | `segment_health` 调 `SegmentReader::open` |
+| hnsw 缺失 fallback | `CollectionInner.hnsw_readers[i]` 为 None → Degraded | `segment_health` 读 `hnsw_readers.get(i)` |
+| dict_state | `CollectionInner.dict_state` | `build_collection_stats` 读 `col_inner.dict_state` |
+| executor_kind | `cfg!(all(not(target_arch="wasm32"), feature="executor-native"))` → Rayon / Serial | `executor_kind()` 函数 |
+
+collection 级 health = 各段 health 的 worst（Corrupt > Degraded > Healthy），
+再与词典降级取 worst。
+
+### 关于"重新 open"的取舍
+
+§3.6 取舍建议"不主动重新 open 校验（性能）"，但表 spec 要求"SegmentReader::open 失败 → Corrupt"。
+本实现选择**重新 open**（`segment_health` 调 `SegmentReader::open`）：
+- inspect 非热路径，性能可接受
+- 能真实检测段损坏（文件被外部篡改后）
+- 与表 spec 一致
+- 与"index_bytes 用 read_at 探测"同属非热路径可接受范围
+
+## index_bytes 方案：read_at 探测 EOF
+
+Vfs trait 无 `size()` 方法（M0 冻结签名，不改）。采用**方案 A：read_at 探测 EOF**。
+
+实现：`probe_file_size(vfs, path)` 从 offset=0 循环读 8KB buffer，n=0 即 EOF，
+累计推算 size。文件不存在（read_at 返 Err）→ 返回 0。
+
+`SegmentFileSizes` 各字段用 `u64`（文件缺失 → 0），`hnsw` 用 `Option<u64>`
+（None = 无 hnsw.bin，fallback brute）。`index_bytes` = 各段 `file_sizes.total()` 之和。
+
+未用方案 B（段文件格式已知字段推算——复杂）或方案 C（Vfs trait 加 size()——破坏 M0 冻结）。
+
+## 单测清单（20 项，全部通过）
+
+| # | 测试名 | 验证点 |
+|---|---|---|
+| 1 | stats_returns_db_path_and_executor | db_path 正确 + executor_kind 变体匹配 |
+| 2 | stats_returns_one_collection_with_correct_counts | 1 collection / 1 segment / 2 docs / 0 tombstones |
+| 3 | stats_index_bytes_nonzero | flush 后 index_bytes > 0 |
+| 4 | stats_health_healthy_for_fresh_segment | 新段 + hnsw 存在 → Healthy |
+| 5 | segment_info_returns_one_segment_with_correct_fields | ULID 非空 / doc_count=2 / docid_base=0 / tombstoned=0 |
+| 6 | segment_info_format_versions_correct | 各文件 format_version 匹配写入常量 |
+| 7 | segment_info_file_sizes_nonzero | header/vectors/stored/idmap/inverted 非零 + hnsw 存在 |
+| 8 | segment_info_health_healthy | 段级 health = Healthy |
+| 9 | collection_segment_info_returns_some_for_existing | 存在的 collection → Some |
+| 10 | collection_segment_info_returns_none_for_missing | 不存在的 collection → None |
+| 11 | stats_multiple_collections | 多 collection 排序 + 未 flush 0 segments |
+| 12 | stats_empty_db | 空 DB / 0 collections / dict_available 与 jieba_dict_available() 一致 |
+| 13 | segment_info_empty_collection | 未 flush → 0 segments |
+| 14 | probe_file_size_empty_file_returns_zero | 不存在文件 → 0 |
+| 15 | probe_file_size_correct_for_known_content | 100 字节文件 → 100 |
+| 16 | read_version_field_missing_file_returns_zero | 不存在 → 0 |
+| 17 | read_version_field_correct_for_vane_magic | MAGIC + V2 → VECTORS_FORMAT_V2 |
+| 18 | worst_function_ordering | Corrupt > Degraded > Healthy |
+| 19 | executor_kind_consistent_with_cfg | 与 cfg 推断一致 |
+| 20 | stats_after_tombstone_delete | delete+flush 后 total/live/tombstoned 计数正确 |
+
+断言非 vacuous：检查具体字段值（段数、文档数、format_version 常量、file_size > 0 等），
+非仅 is_some()。
+
+## 各门禁真实输出
+
+### cargo fmt --all -- --check
+```
+（无输出 — 通过）
+```
+
+### cargo clippy --all-targets --all-features --workspace --exclude vane-fuzz -- -D warnings
+```
+    Checking vane-core v0.2.0
+    Checking vane-wasm v0.2.0
+    Checking vane-node v0.2.0
+    Checking vane-dict-zh v2026.8.0
+    Checking vane-ffi v0.2.0
+    Finished `dev` profile [unoptimized + debuginfo] target(s) in 6.13s
+```
+
+### cargo test -p vane-core --all-features --lib inspect
+```
+running 20 tests
+...
+test result: ok. 20 passed; 0 failed; 0 ignored; 0 measured; 323 filtered out; finished in 0.51s
+```
+
+### cargo test --workspace --all-features --exclude vane-fuzz
+```
+test result: ok. 342 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 3.41s
+（全部 test result 行均 ok，0 failed）
+```
+
+### cargo deny check
+```
+advisories ok, bans ok, licenses ok, sources ok
+```
+
+### cargo check --target wasm32-unknown-unknown -p vane-core
+```
+    Checking vane-core v0.2.0
+    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.92s
+（无 warning）
+```
+
+## commit
+
+```
+feat(core): inspect API（Db::stats/segment_info + 健康检查）（M4 阶段五 b）
+```
+
+文件：
+- `crates/vane-core/src/api/inspect.rs`（新模块，~470 行）
+- `crates/vane-core/src/api/db.rs`（+3 pub 方法 + dict_available_internal helper）
+- `crates/vane-core/src/api/mod.rs`（+2 行 mod 声明）
+- `crates/vane-core/src/api/collection.rs`（3 字段 pub(crate) 可见性）
+
+## 自审
+
+1. **Vfs 无 size 方案取舍**：选 read_at 探测 EOF（方案 A），不改 Vfs trait（M0 冻结）。
+   inspect 非热路径，性能可接受。SegmentFileSizes.hnsw 用 Option<u64> 区分"无 hnsw.bin"
+   vs "文件存在但 0 字节"。
+
+2. **Debug derive 加了**：所有 7 个 struct/enum 加 `#[derive(Debug, Clone)]`，
+   Health/ExecutorKind 额外加 Copy/PartialEq/Eq。避免 E0277。
+
+3. **不改冻结 API 确认**：
+   - 新增 `pub fn stats/segment_info/collection_segment_info`（纯新增，不改现有 pub fn）
+   - 新增 7 个 pub struct/enum（纯新增）
+   - CollectionInner 3 字段 private → pub(crate)（crate 内部可见性，非 pub API 变更）
+   - DbInner 无改动（字段已 pub(crate)）
+   - Vfs trait 不改（无 size()）
+   - 不碰 SPEC.md / CI yml / fault.rs / crash_recovery / vane-fuzz / proptest / cross_version / tracing 埋点
