//! SPEC §4.1 Collection 句柄：add/flush/search 编排。
//!
//! - WriteState（Mutex）护 buffer + auto-committer + docid 计数器
//! - snapshot（RwLock<Vec<Arc<SegmentReader>>>）护段快照，读路径零锁竞争
//! - I7：InvertedIndexReader 随段快照缓存，search 直接用，避免每次重开
//!
//! flush 编排：SegmentWriter + InvertedIndexBuilder + write_inverted + ManifestStore 原子切换。
//! flush 后向量与 BM25 在同一段快照同时可见（不变量 I-2）。

use crate::api::db::DbInner;
use crate::api::reindex::ReindexHandle;
use crate::api::types::*;
use crate::bm25::{write_inverted, InvertedIndexBuilder, InvertedIndexReader};
use crate::fusion::{linear_fuse, minmax_normalize, rrf_fuse, FusionCandidate};
use crate::hnsw::{write_hnsw, HnswReader, HnswWriter};
use crate::persistence::{AutoCommitConfig, AutoCommitter, CollectionMeta, ManifestStore};
use crate::segment::{SegmentReader, SegmentWriter};
use crate::tokenizer::{build_tokenizer, compute_tokenizer_id, BuiltinTokenizer, UserDictEntry};
use crate::types::{Result, Schema, TokenizerId as CoreTokenizerId, VaneError, RRF_K, TOPK_MAX};
use crate::vector::brute_search;
use crate::vfs::Vfs;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

pub struct Collection {
    pub(crate) inner: Arc<CollectionInner>,
}

impl Clone for Collection {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

pub(crate) struct CollectionInner {
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
    write_state: Mutex<WriteState>,
    snapshot: RwLock<Vec<Arc<SegmentReader>>>,
    // 段 ULID → 全局 docid 基址
    seg_offsets: RwLock<HashMap<String, u64>>,
    // I7：InvertedIndexReader 随段快照缓存，search 直接用，避免每次重开
    inverted_readers: RwLock<Vec<Arc<InvertedIndexReader>>>,
    // 01-hnsw：HnswReader 随段快照缓存。Option 因 M0 段无 hnsw.bin（Q-5 → fallback brute）。
    hnsw_readers: RwLock<Vec<Option<Arc<HnswReader>>>>,
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
    dict_state: RwLock<DictState>,
    // 06-userdict-reindex：暂存新词表（setUserDict 后；reindex 时消费）。
    pending_dict: RwLock<Vec<UserDictEntry>>,
    // 07-dict-distribution-node：collection 级 jieba 词典副本（从 DbInner 克隆 Arc）。
    // reindex 重建分词器时用（run_reindex 不持有 DbInner）。
    #[cfg(feature = "jieba")]
    jieba_dict: Option<std::sync::Arc<crate::tokenizer::jieba::JiebaDict>>,
}

struct WriteState {
    buffer: Vec<BufferedDoc>,
    auto_committer: AutoCommitter,
    next_docid: u64,
}

struct BufferedDoc {
    docid: u64, // 全局 docid
    external_id: String,
    text: Option<String>,
    vector: Option<Vec<f32>>,
    meta: Option<HashMap<String, ScalarValue>>,
}

/// M-minor-1（02 遗留）：compacting 标志 panic-safe Drop guard。
///
/// compact/reindex 期置 `compacting=true` 防重入；操作返回（含 panic）时 Drop 复位 false，
/// 避免一次 panic 致永久 E_BUSY。guard 不持有锁——仅在 drop 时重新获取锁复位（与原
/// 显式 finally 模式等价，但 panic-safe）。
struct CompactingGuard<'a> {
    flag: &'a Mutex<bool>,
}
impl Drop for CompactingGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut g) = self.flag.lock() {
            *g = false;
        }
    }
}

/// 构建 collection 分词器（07-dict-distribution-node）。
///
/// - `Standard` / `CjkBigram`：直接 `build_tokenizer`。
/// - `Jieba`：若 `jieba_dict` 可用（dict-zh feature 启用且 Db::open 加载成功）
///   → `build_jieba_tokenizer(dict, user_dict)`；否则 `build_tokenizer(Jieba)` 返回
///   `DictUnavailable`，由调用方（绑定层 convert.rs）降级 CjkBigram（Task 3）。
///
/// `jieba_dict` 参数类型在 jieba feature 关闭时退化为 `()`（JiebaDict 类型不存在），
/// 保证 wasm32 构建不引入 jieba 模块依赖。
#[cfg(feature = "jieba")]
fn build_collection_tokenizer(
    jieba_dict: Option<&std::sync::Arc<crate::tokenizer::jieba::JiebaDict>>,
    kind: BuiltinTokenizer,
    user_dict: &[UserDictEntry],
) -> Result<Arc<dyn crate::tokenizer::Tokenizer>> {
    if matches!(kind, BuiltinTokenizer::Jieba) {
        if let Some(dict) = jieba_dict {
            return Ok(Arc::<dyn crate::tokenizer::Tokenizer>::from(
                crate::tokenizer::build_jieba_tokenizer(dict.clone(), user_dict)?,
            ));
        }
    }
    Ok(Arc::<dyn crate::tokenizer::Tokenizer>::from(
        build_tokenizer(kind, user_dict)?,
    ))
}

/// 非 jieba feature 构建路径（wasm32 等）：直接 build_tokenizer（Jieba → DictUnavailable）。
#[cfg(not(feature = "jieba"))]
fn build_collection_tokenizer(
    _jieba_dict: Option<&()>,
    kind: BuiltinTokenizer,
    user_dict: &[UserDictEntry],
) -> Result<Arc<dyn crate::tokenizer::Tokenizer>> {
    Ok(Arc::<dyn crate::tokenizer::Tokenizer>::from(
        build_tokenizer(kind, user_dict)?,
    ))
}

impl CollectionInner {
    // I3 裁决：create_new 接收 auto_commit 参数（collection 级配置，SPEC §7.1）
    pub(crate) fn create_new(
        db: &DbInner,
        name: &str,
        meta: CollectionMeta,
        auto_commit: AutoCommitConfig,
    ) -> Result<Self> {
        // M2-11 fix I-3：jieba_dict 在单次 read lock 内 snapshot，消除 TOCTOU 窗口。
        // 之前两次 read lock 间可被 set_jieba_dict 改写 → tokenizer 用 dict A、
        // CollectionInner 存 dict B → I-4 reindex 身份不一致。
        #[cfg(feature = "jieba")]
        let jieba_dict_snapshot: Option<
            std::sync::Arc<crate::tokenizer::jieba::JiebaDict>,
        > = db.jieba_dict.read().unwrap().clone();
        let tokenizer: Arc<dyn crate::tokenizer::Tokenizer> = {
            #[cfg(feature = "jieba")]
            {
                build_collection_tokenizer(
                    jieba_dict_snapshot.as_ref(),
                    meta.tokenizer_kind,
                    &meta.user_dict,
                )?
            }
            #[cfg(not(feature = "jieba"))]
            {
                build_collection_tokenizer(None, meta.tokenizer_kind, &meta.user_dict)?
            }
        };
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
            tombstones: RwLock::new(HashMap::new()),
            compacting: Mutex::new(false),
            dict_state: RwLock::new(DictState::Stable),
            pending_dict: RwLock::new(Vec::new()),
            #[cfg(feature = "jieba")]
            jieba_dict: jieba_dict_snapshot,
        })
    }

    pub(crate) fn restore_from_manifest(
        db: &DbInner,
        name: &str,
        meta: CollectionMeta,
        auto_commit: AutoCommitConfig,
    ) -> Result<Self> {
        let inner = Self::create_new(db, name, meta.clone(), auto_commit)?;
        let mut readers = Vec::new();
        let mut offsets = HashMap::new();
        let mut inv_readers = Vec::new();
        let mut hnsw_readers = Vec::new();
        let mut scalar_readers = Vec::new();
        // 读取每段 header.bin 已持久化的 docid_base（而非累加 doc_count 推断），
        // 更稳健：段顺序/非连续场景（M1 compaction）也能正确还原 offset。
        let mut max_end = 0u64;
        for ulid in &meta.segment_ulids {
            let seg_dir = format!("{}/segments/seg_{}", db.db_path, ulid);
            let reader = Arc::new(SegmentReader::open(&db.vfs, &seg_dir)?);
            // I7：同时 open InvertedIndexReader 缓存
            let inv_reader = Arc::new(InvertedIndexReader::open(&db.vfs, &seg_dir)?);
            // 01-hnsw（Q-5）：M0 段无 hnsw.bin → open 返回 Err → push None（fallback brute）
            let hnsw_reader = match HnswReader::open(&db.vfs, &seg_dir) {
                Ok(r) => Some(Arc::new(r)),
                Err(_) => None,
            };
            // 03-pre-filter：加载 ScalarReader（scalars.col）。
            let scalar_reader = Arc::new(crate::segment::ScalarReader::open(&db.vfs, &seg_dir)?);
            let base = reader.meta().docid_base;
            let count = reader.doc_count() as u64;
            offsets.insert(ulid.clone(), base);
            max_end = max_end.max(base + count);
            readers.push(reader);
            inv_readers.push(inv_reader);
            hnsw_readers.push(hnsw_reader);
            scalar_readers.push(scalar_reader);
        }
        inner.write_state.lock().unwrap().next_docid = max_end;
        *inner.snapshot.write().unwrap() = readers;
        *inner.seg_offsets.write().unwrap() = offsets;
        *inner.inverted_readers.write().unwrap() = inv_readers;
        *inner.hnsw_readers.write().unwrap() = hnsw_readers;
        *inner.scalar_readers.write().unwrap() = scalar_readers;
        Ok(inner)
    }
}

impl Collection {
    pub fn add(&self, docs: &[Doc]) -> Result<AddReport> {
        // 06：Rebuilding 期写路径 E_BUSY（Q-6）。
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
                        "vector dim mismatch: got {} expected {}",
                        v.len(),
                        dim
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
                meta: doc.meta.clone(),
            });
            count += 1;
        }
        state.auto_committer.record_docs(count as u32);
        // I3：auto-commit 检查（消费 collection 级 AutoCommitConfig）
        let should = state.auto_committer.should_flush();
        drop(state);
        if should {
            // auto-commit flush 失败不再静默吞错：记录到 stderr 供排查。
            // 不改 AddReport pub API（加失败标志属 pub API 变更，交编排者裁决）；
            // M1 可引入 log crate 做结构化日志。
            if let Err(e) = self.flush() {
                eprintln!(
                    "[vane] auto-commit flush for collection '{}' failed: {}",
                    self.inner.name, e
                );
            }
        }
        Ok(AddReport {
            accepted: count,
            visible_after_flush: true,
        })
    }

    pub fn flush(&self) -> Result<()> {
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
        let base_docid = docs.first().map(|d| d.docid).unwrap_or(0);
        state.auto_committer.reset();
        drop(state);

        // 构建 SegmentWriter（I4/FF2 裁决：传入真实全局 docid 基址）
        let tok_id = self.inner.tokenizer_id.read().unwrap().clone();
        let mut writer = SegmentWriter::new(
            self.inner.vfs.clone(),
            &self.inner.segments_dir,
            &self.inner.schema,
            &tok_id,
            base_docid,
        )?;
        let mut inv_builder = InvertedIndexBuilder::new(docs.len());

        for doc in &docs {
            // I5 裁决：序列化真实 doc.meta 为 stored_json（非空 map）
            let stored_json = if let Some(meta) = &doc.meta {
                let mut map = serde_json::Map::new();
                for (k, v) in meta {
                    let val = match v {
                        ScalarValue::Int(i) => serde_json::json!(i),
                        ScalarValue::Float(f) => serde_json::json!(f),
                        ScalarValue::Bool(b) => serde_json::json!(b),
                        ScalarValue::Keyword(s) => serde_json::json!(s),
                    };
                    map.insert(k.clone(), val);
                }
                serde_json::to_string(&serde_json::Value::Object(map))
                    .unwrap_or_else(|_| "{}".into())
            } else {
                "{}".to_string()
            };
            // I4/FF2：add_doc 返回段内局部 docid（从 0 起），全局 = base + local
            let local_docid =
                writer.add_doc(&doc.external_id, doc.vector.as_deref(), &stored_json)?;
            // SPEC §6.2：原文持久化——add_doc 后 set_text 写入 doc.text（None 落空串）。
            // 06-userdict-reindex 经 SegmentReader::text 读原文用新分词器重建倒排；
            // 02-tombstone-merge 经此读原文写入新段。
            writer.set_text(doc.text.as_deref().unwrap_or(""))?;
            // 03-pre-filter：把 doc.meta 中的标量字段经 set_scalar 写入 scalars.col。
            // 仅写 schema 中声明为 Scalar 的字段；其余 meta 仍走 stored_json。
            if let Some(meta) = &doc.meta {
                for (field, val) in meta {
                    // schema 标量字段才写列式块（set_scalar 内部校验 kind）。
                    let is_scalar = self.inner.schema.fields.iter().any(|(n, d)| {
                        n == field && matches!(d, crate::types::FieldDef::Scalar { .. })
                    });
                    if is_scalar {
                        writer.set_scalar(field, val.clone())?;
                    }
                }
            }
            let global_docid = base_docid + local_docid;
            let tok = self.inner.tokenizer.read().unwrap();
            let tokens = doc
                .text
                .as_ref()
                .map(|t| tok.tokenize(t))
                .unwrap_or_default();
            drop(tok);
            let field_len = tokens.len() as u32;
            inv_builder.add_document(global_docid, &tokens, field_len);
        }

        let meta = writer.finalize()?;
        let seg_dir = format!("{}/seg_{}", self.inner.segments_dir, meta.ulid);
        let inverted = inv_builder.build();
        write_inverted(self.inner.vfs.as_ref(), &seg_dir, &inverted)?;

        // 01-hnsw：从 reader.vectors() 构建 HnswWriter → build → write_hnsw。
        // 段内不可变（I-1）：hnsw.bin 写一次，读期不修改（I-3 删除走 tombstone）。
        // 先 open SegmentReader 取 vectors/dim/metric，再写 hnsw.bin，最后复用同一 reader 入快照。
        let reader = Arc::new(SegmentReader::open(&self.inner.vfs, &seg_dir)?);
        let hnsw_reader = {
            let (dim, metric) = self
                .inner
                .schema
                .vector_field()
                .map(|(_, d, m)| (d, m))
                .unwrap_or((0, crate::types::Metric::Cosine));
            let vectors = reader.vectors();
            let doc_count = reader.doc_count();
            if dim > 0 && doc_count > 0 {
                let mut hw = HnswWriter::new(dim, metric, 16, 200);
                let d = dim as usize;
                for i in 0..doc_count {
                    let v = &vectors[(i as usize) * d..(i as usize + 1) * d];
                    hw.insert(i, v);
                }
                let graph = hw.build();
                // 写失败不阻塞 flush 主路径——HnswReader::open 会返回 Err → fallback brute。
                if let Err(e) = write_hnsw(self.inner.vfs.as_ref(), &seg_dir, &graph) {
                    eprintln!(
                        "[vane] hnsw write for segment {} failed: {} (fallback to brute)",
                        meta.ulid, e
                    );
                }
            }
            // open：若 hnsw.bin 缺失/损坏 → None（fallback brute_search，Q-5）
            match HnswReader::open(&self.inner.vfs, &seg_dir) {
                Ok(r) => Some(Arc::new(r)),
                Err(_) => None,
            }
        };

        // 04-wal：段文件集（header/vectors/inverted/hnsw/scalars）已全部 sync →
        // append AddSegment（SPEC §6.4：WAL → manifest rename）。B-2：flush 不 truncate。
        let wal = crate::wal::Wal::open(self.inner.vfs.clone(), &self.inner.db_path)?;
        wal.append(&crate::wal::WalRecord::AddSegment {
            collection: self.inner.name.clone(),
            ulid: meta.ulid.clone(),
        })?;

        // 更新 manifest（I-6 原子切换）
        let manifest_store = ManifestStore::new(self.inner.vfs.clone(), &self.inner.db_path);
        manifest_store.add_segment(&self.inner.name, &meta.ulid)?;

        // 更新段快照（Arc swap 语义：写锁替换 Vec）
        // I7：open 一次 InvertedIndexReader 并缓存
        let inv_reader = Arc::new(InvertedIndexReader::open(&self.inner.vfs, &seg_dir)?);
        // 03-pre-filter：缓存 ScalarReader。
        let scalar_reader = Arc::new(crate::segment::ScalarReader::open(
            &self.inner.vfs,
            &seg_dir,
        )?);
        {
            let mut snap = self.inner.snapshot.write().unwrap();
            let mut offsets = self.inner.seg_offsets.write().unwrap();
            let mut inv_readers = self.inner.inverted_readers.write().unwrap();
            let mut hnsw_readers = self.inner.hnsw_readers.write().unwrap();
            let mut scalar_readers = self.inner.scalar_readers.write().unwrap();
            offsets.insert(meta.ulid.clone(), base_docid);
            snap.push(reader);
            inv_readers.push(inv_reader);
            hnsw_readers.push(hnsw_reader);
            scalar_readers.push(scalar_reader);
        }
        // 02 Task 6：段数超 SEGMENT_MAX 自动合并小段（SPEC §3.3）。
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
        Ok(())
    }

    /// 当前段数（测试与诊断用）。
    pub fn segment_count(&self) -> usize {
        self.inner.snapshot.read().unwrap().len()
    }

    /// 选最小两段合并（auto-merge on exceeding SEGMENT_MAX，SPEC §3.3）。
    fn auto_merge_two_smallest(&self) -> Result<()> {
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
                    .get(&r.meta().ulid)
                    .map(|b| b.len() as f32)
                    .unwrap_or(0.0);
                (r.meta().ulid.clone(), t / total)
            })
            .collect();
        let picked = crate::merge::pick_merge_candidates(&snap, &ratios);
        if picked.len() < 2 {
            return Ok(());
        }
        let merge_two: Vec<String> = picked.into_iter().take(2).collect();
        self.merge_segments(merge_two)
    }

    /// 合并指定段 ULID 列表为单个新段，更新 manifest + 内存快照 + 删旧段。
    fn merge_segments(&self, source_ulids: Vec<String>) -> Result<()> {
        if source_ulids.is_empty() {
            return Ok(());
        }
        let offsets = self.inner.seg_offsets.read().unwrap().clone();
        let tombstones = self.inner.tombstones.read().unwrap().clone();
        let snap = self.inner.snapshot.read().unwrap().clone();
        // target_docid_base 选择（02-review B-2 修复）：
        // - compact 全合并（source 覆盖全部段，无保留段）：0（从 0 起合理）。
        // - partial auto-merge（合并 2/N 段）：max(保留段 base + count)，
        //   新段 docid 从所有保留段的最大 docid 之后开始，避免与任何保留段
        //   docid 空间重叠（否则 search 回填误命中、fusion 去重丢文档）。
        let is_full_merge = snap.iter().all(|r| source_ulids.contains(&r.meta().ulid));
        let target_docid_base = if is_full_merge {
            0
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
        let ctx = crate::merge::MergeContext {
            vfs: &self.inner.vfs,
            db_path: &self.inner.db_path,
            segments_dir: &self.inner.segments_dir,
        };
        while !task.step(&ctx)? {}
        let new_meta = crate::merge::finalize_merge(task, &ctx)?;
        let new_seg_dir = format!("{}/seg_{}", self.inner.segments_dir, new_meta.ulid);

        // partial merge 时推进 next_docid 到新段末尾之后，避免后续 flush 分配的
        // docid 与新段 [target_docid_base, target_docid_base + new_count) 重叠。
        // compact 全合并 base=0 不受影响（next_docid 保持 stale-high，详见 02-review 维度 8a）。
        if !is_full_merge {
            let new_end = target_docid_base + new_meta.doc_count as u64;
            let mut state = self.inner.write_state.lock().unwrap();
            if new_end > state.next_docid {
                state.next_docid = new_end;
            }
        }

        // 更新 manifest（I-6）。
        let manifest_store = ManifestStore::new(self.inner.vfs.clone(), &self.inner.db_path);
        let mut manifest = manifest_store
            .load()?
            .unwrap_or_else(crate::persistence::Manifest::empty);
        let col_meta = manifest
            .collections
            .get_mut(&self.inner.name)
            .ok_or_else(|| {
                VaneError::NotFound(format!("collection not in manifest: {}", self.inner.name))
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
                collection: self.inner.name.clone(),
                ulid: u.clone(),
            })?;
        }
        wal.append(&crate::wal::WalRecord::AddSegment {
            collection: self.inner.name.clone(),
            ulid: new_meta.ulid.clone(),
        })?;
        manifest_store.save_atomic(&manifest)?;

        // 更新内存快照。
        let new_reader = Arc::new(SegmentReader::open(&self.inner.vfs, &new_seg_dir)?);
        let new_inv = Arc::new(InvertedIndexReader::open(&self.inner.vfs, &new_seg_dir)?);
        let new_hnsw = match HnswReader::open(&self.inner.vfs, &new_seg_dir) {
            Ok(r) => Some(Arc::new(r)),
            Err(_) => None,
        };
        let new_scalar = Arc::new(crate::segment::ScalarReader::open(
            &self.inner.vfs,
            &new_seg_dir,
        )?);
        {
            let mut snap_w = self.inner.snapshot.write().unwrap();
            let mut offsets_w = self.inner.seg_offsets.write().unwrap();
            let mut inv_w = self.inner.inverted_readers.write().unwrap();
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
                    offsets_w.insert(
                        r.meta().ulid.clone(),
                        offsets.get(&r.meta().ulid).copied().unwrap_or(0),
                    );
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
                    offsets_w.remove(&r.meta().ulid);
                    tomb_w.remove(&r.meta().ulid);
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
        Ok(())
    }

    pub fn search(&self, query: &SearchQuery) -> Result<Vec<Hit>> {
        self.run_search(query, true)
    }

    /// 12-recall-regression 测试/bench 辅助：暴力双路+RRF 基线（绕过 HNSW）。
    ///
    /// SPEC §13.2-1 基线口径 = `brute_search`（vector 路）+ `InvertedIndexReader::search`
    /// （text 路）+ `rrf_fuse`（融合）。本方法复用 [`search`] 的 mode 推断 / dim 校验 /
    /// filter 编译 / Hit 回填逻辑，**仅** vector 路强制走 `brute_search`（跳过 HnswReader），
    /// 为 recall 回归提供 100% 召回的对照基线。非对外 IDL，标注 `#[doc(hidden)]`。
    #[doc(hidden)]
    pub fn search_brute_baseline(&self, query: &SearchQuery) -> Result<Vec<Hit>> {
        self.run_search(query, false)
    }

    /// 搜索主逻辑（`search` 与 `search_brute_baseline` 共享）。
    ///
    /// `allow_hnsw=true`：vector 路有 HnswReader 则走 HNSW（自适应回退时仍走 brute）。
    /// `allow_hnsw=false`：vector 路恒走 `brute_search`（基线口径，绕过 HNSW）。
    fn run_search(&self, query: &SearchQuery, allow_hnsw: bool) -> Result<Vec<Hit>> {
        if query.top_k > TOPK_MAX {
            return Err(VaneError::InvalidArg(format!(
                "topK {} exceeds max {}",
                query.top_k, TOPK_MAX
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
                        "search requires text or vector".into(),
                    ))
                }
            },
        };
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
            Some(metric)
        } else {
            None
        };

        let snap = self.inner.snapshot.read().unwrap();
        let offsets = self.inner.seg_offsets.read().unwrap();
        // I7：用缓存的 InvertedIndexReader，避免每次 search 重开
        let inv_readers = self.inner.inverted_readers.read().unwrap();
        // 01-hnsw：HnswReader 缓存（Option：M0 段无 hnsw.bin → None → fallback brute）
        let hnsw_readers = self.inner.hnsw_readers.read().unwrap();
        // 03-pre-filter：ScalarReader 缓存（scalars.col），compile_filter 用。
        let scalar_readers = self.inner.scalar_readers.read().unwrap();
        // 02-tombstone-merge：tombstone 位图（ulid → 绝对 docid），compile_filter 末尾排除。
        let tombstones = self.inner.tombstones.read().unwrap();
        let topk = query.top_k as usize;
        let cand = topk * query.candidate_multiplier as usize;

        // 03-pre-filter：编译全局 filter 位图（绝对 docid 空间）。
        // - 有用户 filter → compile_filter（含 tombstone 排除）。
        // - 无 filter 但有 tombstone → alive_bitmap（全量减 tombstone）。
        // - 无 filter 无 tombstone → None（M0 行为，最高效）。
        let has_tombstones = tombstones.values().any(|b| !b.is_empty());
        let filter_bm_owned: Option<roaring::RoaringBitmap> = if let Some(f) = &query.filter {
            // 构建按段对齐的 tombstone Arc 列表（compile_filter 契约要求）。
            let tb_arcs: Vec<Arc<roaring::RoaringBitmap>> = snap
                .iter()
                .map(|r| tombstones.get(&r.meta().ulid).cloned().unwrap_or_default())
                .map(Arc::new)
                .collect();
            Some(crate::filter::compile_filter(
                f,
                &self.inner.schema,
                &snap,
                &scalar_readers,
                &tb_arcs,
            )?)
        } else if has_tombstones {
            let tb_arcs: Vec<Arc<roaring::RoaringBitmap>> = snap
                .iter()
                .map(|r| tombstones.get(&r.meta().ulid).cloned().unwrap_or_default())
                .map(Arc::new)
                .collect();
            Some(crate::filter::alive_bitmap(&snap, &tb_arcs)?)
        } else {
            None
        };
        let filter_bm: Option<&roaring::RoaringBitmap> = filter_bm_owned.as_ref();

        let mut vec_candidates: Vec<crate::types::ScoredDoc> = Vec::new();
        let mut text_candidates: Vec<crate::types::ScoredDoc> = Vec::new();

        // 自适应回退（SPEC §8.1）：filter 位图基数 < 2*topk → 暴力精确扫描。
        let force_brute = match filter_bm {
            Some(bm) => crate::filter::should_fallback_brute(bm, topk),
            None => false,
        };

        // snap/inv_readers/hnsw_readers 在 flush/restore 中成对维护，zip 迭代对齐
        for ((reader, inv_reader), hnsw_reader) in
            snap.iter().zip(inv_readers.iter()).zip(hnsw_readers.iter())
        {
            let base = offsets.get(&reader.meta().ulid).copied().unwrap_or(0);
            // 03-pre-filter：filter_bm 已含 tombstone 排除，直接透传各段 search。
            let merged_filter: Option<&roaring::RoaringBitmap> = filter_bm;
            // vector 路
            if matches!(mode, SearchMode::Hybrid | SearchMode::Vector) {
                if let (Some(qv), Some(metric)) = (&query.vector, vf) {
                    let want = if matches!(mode, SearchMode::Hybrid) {
                        cand
                    } else {
                        topk
                    };
                    // 01-hnsw：有 HnswReader 且无需强制暴力 → HNSW 搜索；
                    // 否则 fallback brute_search（M0 段无 hnsw.bin / 低选择率回退 / 写失败 /
                    // search_brute_baseline 基线口径强制 brute）。
                    let use_hnsw = allow_hnsw && !force_brute;
                    let mut hits = if use_hnsw {
                        if let Some(hr) = hnsw_reader {
                            let ef = hr.ef_construction().max(want as u32 * 4) as usize;
                            // R-hnsw-vec：向量不进 hnsw.bin，由 SegmentReader.vectors() 传入共享单一副本。
                            hr.search(qv, want, ef, merged_filter, base, reader.vectors())
                        } else {
                            brute_search(
                                reader.vectors(),
                                reader.dim(),
                                qv,
                                metric,
                                want,
                                merged_filter,
                                base,
                            )
                        }
                    } else {
                        brute_search(
                            reader.vectors(),
                            reader.dim(),
                            qv,
                            metric,
                            want,
                            merged_filter,
                            base,
                        )
                    };
                    vec_candidates.append(&mut hits);
                }
            }
            // text 路
            if matches!(mode, SearchMode::Hybrid | SearchMode::Text) {
                if let Some(qt) = &query.text {
                    let tokens = self.inner.tokenizer.read().unwrap().tokenize(qt);
                    let mut hits = inv_reader.search(
                        &tokens,
                        if matches!(mode, SearchMode::Hybrid) {
                            cand
                        } else {
                            topk
                        },
                        merged_filter,
                    );
                    text_candidates.append(&mut hits);
                }
            }
        }

        // 归并多段 topK（取全局 topK/cand）
        vec_candidates.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        vec_candidates.truncate(if matches!(mode, SearchMode::Hybrid) {
            cand
        } else {
            topk
        });
        text_candidates.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        text_candidates.truncate(if matches!(mode, SearchMode::Hybrid) {
            cand
        } else {
            topk
        });

        // 融合
        let fused: Vec<crate::types::ScoredDoc> = match mode {
            SearchMode::Vector => vec_candidates,
            SearchMode::Text => text_candidates,
            SearchMode::Hybrid => match &query.fusion {
                FusionSpec::Rrf => {
                    let paths: Vec<Vec<FusionCandidate>> = vec![
                        vec_candidates
                            .iter()
                            .enumerate()
                            .map(|(i, d)| FusionCandidate {
                                docid: d.docid,
                                rank: i as u32,
                                score: d.score,
                            })
                            .collect(),
                        text_candidates
                            .iter()
                            .enumerate()
                            .map(|(i, d)| FusionCandidate {
                                docid: d.docid,
                                rank: i as u32,
                                score: d.score,
                            })
                            .collect(),
                    ];
                    rrf_fuse(&paths, RRF_K)
                }
                // I6 裁决：SPEC §4.2 M0 冻结 IDL 含 linear；§8.2 为显式选项非占位
                FusionSpec::Linear { alpha } => {
                    let vec_norm = minmax_normalize(&vec_candidates);
                    let text_norm = minmax_normalize(&text_candidates);
                    linear_fuse(&vec_norm, &text_norm, *alpha)
                }
            },
            SearchMode::Auto => unreachable!(),
        };

        // 回填 Hit：docid → external_id + stored meta
        // I5/F1：从对应段的 stored.bin 读取 doc.meta（local = sd.docid - base）
        let mut hits = Vec::with_capacity(fused.len());
        for sd in fused.iter().take(topk) {
            let mut found_id = None;
            let mut found_fields = None;
            for reader in snap.iter() {
                let base = offsets.get(&reader.meta().ulid).copied().unwrap_or(0);
                // checked_sub：sd.docid < base 时返回 None，跳过该段（更安全，
                // 避免 wrapping_sub 产生巨大 local 误命中脆弱 external_id 查找）
                let local = match sd.docid.checked_sub(base) {
                    Some(l) => l,
                    None => continue,
                };
                if let Some(eid) = reader.external_id(local) {
                    found_id = Some(eid.to_string());
                    if let Some(json) = reader.stored_json(local) {
                        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json) {
                            if let Some(obj) = parsed.as_object() {
                                let mut map = std::collections::HashMap::new();
                                for (k, v) in obj {
                                    map.insert(k.clone(), v.to_string());
                                }
                                if !map.is_empty() {
                                    found_fields = Some(map);
                                }
                            }
                        }
                    }
                    break;
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
        Ok(hits)
    }

    /// 当前段快照的 ULID 列表（测试与诊断用；01-hnsw Task 5 测试依赖）。
    pub fn segment_ulids(&self) -> Vec<String> {
        self.inner
            .snapshot
            .read()
            .unwrap()
            .iter()
            .map(|r| r.meta().ulid.clone())
            .collect()
    }

    /// 当前段快照 reader 列表（测试用；I-4 验证段头 tokenizer_id）。
    pub fn snapshot_readers(&self) -> Vec<Arc<SegmentReader>> {
        self.inner.snapshot.read().unwrap().clone()
    }

    /// 测试辅助：手动设置 dict_state（模拟 Rebuilding 窗口，M1 同步执行下窗口短）。
    #[doc(hidden)]
    pub fn set_state_for_test(&self, state: DictState) {
        *self.inner.dict_state.write().unwrap() = state;
    }

    /// M1 实装（02-tombstone-merge）：追加 tombstone（内存位图）。
    /// 查询期 search 把 tombstone 并入 filter 过滤；持久化经 WAL（04 计划）。
    /// 段不可变（I-1）：不修改段文件，仅更新内存位图。
    pub fn delete(&self, ids: &[String]) -> Result<u64> {
        // 06：Rebuilding 期写路径 E_BUSY（Q-6）。
        if *self.inner.dict_state.read().unwrap() == DictState::Rebuilding {
            return Err(VaneError::Busy);
        }
        let snap = self.inner.snapshot.read().unwrap();
        let offsets = self.inner.seg_offsets.read().unwrap();
        // 04-wal：先计算 (ulid, abs_docid) 对，append AddTombstone（SPEC §7.2 即时进 WAL），
        // 再更新内存位图。crash 在 WAL 后位图前 → reopen 时 recover 重放注入；
        // crash 在 WAL 前 → 位图也未改，一致。
        let mut by_ulid: HashMap<String, Vec<u64>> = HashMap::new();
        for id in ids {
            for reader in snap.iter() {
                let base = offsets.get(&reader.meta().ulid).copied().unwrap_or(0);
                // 段内 id_map 反查 local docid。
                let local = reader.local_docid_by_external(id);
                if let Some(l) = local {
                    let abs = base + l;
                    if abs > u32::MAX as u64 {
                        // roaring 存 u32；超限记不进来（与 search 一致），跳过。
                        continue;
                    }
                    by_ulid
                        .entry(reader.meta().ulid.clone())
                        .or_default()
                        .push(abs);
                    break; // 一个 external_id 只可能存在于一个段
                }
            }
        }
        drop(offsets);
        drop(snap);
        let wal = crate::wal::Wal::open(self.inner.vfs.clone(), &self.inner.db_path)?;
        for (ulid, docids) in &by_ulid {
            wal.append(&crate::wal::WalRecord::AddTombstone {
                collection: self.inner.name.clone(),
                ulid: ulid.clone(),
                docids: docids.clone(),
            })?;
        }
        // 更新内存位图（count 仅记 newly inserted，与原 02 语义一致）。
        let mut tombstones = self.inner.tombstones.write().unwrap();
        let mut count: u64 = 0;
        for (ulid, docids) in by_ulid {
            let bm = tombstones.entry(ulid).or_default();
            for d in docids {
                if bm.insert(d as u32) {
                    count += 1;
                }
            }
        }
        Ok(count)
    }

    /// M1 实装（02-tombstone-merge）：手动触发段合并（compact）。
    /// 物理清除 tombstone 文档，新段从零重建 HNSW 图（I-3）。
    /// 全串行同步执行（R-4/R-6）；E_BUSY 若 compact 进行中（06 reindex 状态机复用）。
    pub fn compact(&self) -> Result<()> {
        // 06：Rebuilding 期写路径 E_BUSY（Q-6）。
        if *self.inner.dict_state.read().unwrap() == DictState::Rebuilding {
            return Err(VaneError::Busy);
        }
        // 重入保护。M-minor-1：CompactingGuard 保证 panic 时复位标志。
        {
            let mut guard = self.inner.compacting.lock().unwrap();
            if *guard {
                return Err(VaneError::Busy);
            }
            *guard = true;
        }
        let _cg = CompactingGuard {
            flag: &self.inner.compacting,
        };
        self.run_compact()
    }

    fn run_compact(&self) -> Result<()> {
        let snap = self.inner.snapshot.read().unwrap().clone();
        // 0 段：直接返回。1 段且无 tombstone：无需重建。
        if snap.is_empty() {
            return Ok(());
        }
        if snap.len() == 1 {
            let has_tomb = self
                .inner
                .tombstones
                .read()
                .unwrap()
                .get(&snap[0].meta().ulid)
                .map(|b| !b.is_empty())
                .unwrap_or(false);
            if !has_tomb {
                return Ok(());
            }
        }
        let source_ulids: Vec<String> = snap.iter().map(|r| r.meta().ulid.clone()).collect();
        self.merge_segments(source_ulids)?;
        // 04-wal：compact 是唯一 truncate 调用点（B-2）。merge_segments 已 append
        // DeleteSegment(旧) + AddSegment(新) 并切换 manifest；此时所有旧段物理清除
        // （tombstone 随之清除），WAL 可一次性清空。
        let wal = crate::wal::Wal::open(self.inner.vfs.clone(), &self.inner.db_path)?;
        wal.truncate()?;
        Ok(())
    }

    /// §7.4：暂存新词表，进入 PendingReindex。新写入仍用旧分词身份（I-4）。
    ///
    /// - Rebuilding 期调用返回 E_BUSY（Q-6）。
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
        *self.inner.pending_dict.write().unwrap() = dict.to_vec();
        *state = DictState::PendingReindex;
        Ok(())
    }

    /// 查询当前词表状态（绑定层暴露 needsReindex）。
    pub fn dict_state(&self) -> DictState {
        *self.inner.dict_state.read().unwrap()
    }

    /// 当前生效的 TokenizerId（reindex 完成前为旧身份，完成后为新身份）。
    pub fn tokenizer_id(&self) -> CoreTokenizerId {
        self.inner.tokenizer_id.read().unwrap().clone()
    }

    /// §7.4：触发全量重建。从旧段 `SegmentReader::text` 读原文，用**新分词器**
    /// 重新 tokenize 重建倒排（vectors/hnsw 重写不变，段新 ULID）。旧段只读服务，
    /// 完成后原子切换。
    ///
    /// **签名变更**（R-2）：M0 为 `Result<()>`（占位），M1 落实为 SPEC §4.1
    /// `Result<ReindexHandle>`。M1 同步执行（R-4/R-6）：重建在调用内同步完成，
    /// 返回已完成的 handle（progress=1.0, wait 立即返回）。后台化留 M2 Executor。
    ///
    /// - 非 PendingReindex 状态：返回 InvalidArg（Stable 无待重建词表）。
    /// - compact 进行中：返回 E_BUSY。
    pub fn reindex(&self) -> Result<ReindexHandle> {
        // 互斥：compact 进行中 → E_BUSY。
        {
            let mut guard = self.inner.compacting.lock().unwrap();
            if *guard {
                return Err(VaneError::Busy);
            }
            *guard = true;
        }
        // M-minor-1：CompactingGuard 保证 panic 时复位标志。提前返回（状态校验失败）
        // 时 guard drop 复位；成功路径 run_reindex 返回后 guard drop 复位。
        let _cg = CompactingGuard {
            flag: &self.inner.compacting,
        };
        // 校验状态：必须 PendingReindex。
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
    }

    fn run_reindex(&self) -> Result<ReindexHandle> {
        // 构建新分词器 + 新 TokenizerId。
        let pending = self.inner.pending_dict.read().unwrap().clone();
        let new_tokenizer: Arc<dyn crate::tokenizer::Tokenizer> = {
            #[cfg(feature = "jieba")]
            {
                build_collection_tokenizer(
                    self.inner.jieba_dict.as_ref(),
                    self.inner.tokenizer_kind,
                    &pending,
                )?
            }
            #[cfg(not(feature = "jieba"))]
            {
                build_collection_tokenizer(None, self.inner.tokenizer_kind, &pending)?
            }
        };
        let new_tokenizer_id = compute_tokenizer_id(self.inner.tokenizer_kind, &pending);

        // 收集旧段 ULID + tombstones + offsets（快照，避免长锁）。
        let old_ulids = self.segment_ulids();
        let tombstones = self.inner.tombstones.read().unwrap().clone();
        let offsets_snap = self.inner.seg_offsets.read().unwrap().clone();

        // 逐段重建（M1 同步串行，R-4/R-6）。
        let mut new_segments: Vec<crate::api::reindex::ReindexedSegment> = Vec::new();
        for ulid in &old_ulids {
            let reindexed = crate::api::reindex::reindex_segment(
                &self.inner.vfs,
                &self.inner.segments_dir,
                ulid,
                &self.inner.schema,
                &new_tokenizer_id,
                &new_tokenizer,
            )?;
            new_segments.push(reindexed);
        }

        // 原子切换 manifest（I-6）：ULID 替换 + tokenizer_id/user_dict 更新。
        let manifest_store = ManifestStore::new(self.inner.vfs.clone(), &self.inner.db_path);
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
            wal.append(&crate::wal::WalRecord::AddSegment {
                collection: self.inner.name.clone(),
                ulid: new_u.clone(),
            })?;
        }
        for old_u in &old_ulids {
            wal.append(&crate::wal::WalRecord::DeleteSegment {
                collection: self.inner.name.clone(),
                ulid: old_u.clone(),
            })?;
        }
        for (i, old_u) in old_ulids.iter().enumerate() {
            if let Some(bm) = tombstones.get(old_u) {
                if !bm.is_empty() {
                    if let Some(new_u) = new_ulids.get(i) {
                        let docids: Vec<u64> = bm.iter().map(|d| d as u64).collect();
                        wal.append(&crate::wal::WalRecord::AddTombstone {
                            collection: self.inner.name.clone(),
                            ulid: new_u.clone(),
                            docids,
                        })?;
                    }
                }
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
            &manifest_store,
            &self.inner.name,
            &old_ulids,
            new_ulids.clone(),
            new_col_meta,
        )?;

        // 更新内存快照：旧段移除 → 新段插入。tombstone 按 ULID re-key
        // （docid 顺序不变，位图原值有效）。
        // I-4 原子性（06-review #1）：tokenizer/tokenizer_id 必须与 snapshot 段列表
        // 在同一写锁块内切换，杜绝「snapshot 已切到新段（新 TokenizerId）但
        // tokenizer 仍旧」的混排窗口。锁顺序与 search 读侧一致：
        // snapshot → offsets → inv → hnsw → scalar → tomb → tokenizer →
        // tokenizer_id，同序无死锁。
        {
            let mut snap_w = self.inner.snapshot.write().unwrap();
            let mut offsets_w = self.inner.seg_offsets.write().unwrap();
            let mut inv_w = self.inner.inverted_readers.write().unwrap();
            let mut hnsw_w = self.inner.hnsw_readers.write().unwrap();
            let mut scalar_w = self.inner.scalar_readers.write().unwrap();
            let mut tomb_w = self.inner.tombstones.write().unwrap();
            let mut tok_w = self.inner.tokenizer.write().unwrap();
            let mut tok_id_w = self.inner.tokenizer_id.write().unwrap();
            // 移除旧段（保留顺序，逐项过滤）。
            let old_snap = std::mem::take(&mut *snap_w);
            let old_inv = std::mem::take(&mut *inv_w);
            let old_hnsw = std::mem::take(&mut *hnsw_w);
            let old_scalar = std::mem::take(&mut *scalar_w);
            for (i, r) in old_snap.iter().enumerate() {
                if !old_ulids.contains(&r.meta().ulid) {
                    snap_w.push(r.clone());
                    offsets_w.insert(
                        r.meta().ulid.clone(),
                        offsets_snap.get(&r.meta().ulid).copied().unwrap_or(0),
                    );
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
                    offsets_w.remove(&r.meta().ulid);
                    tomb_w.remove(&r.meta().ulid);
                }
            }
            // 插入新段。
            for s in &new_segments {
                offsets_w.insert(s.ulid.clone(), s.docid_base);
                snap_w.push(s.reader.clone());
                inv_w.push(s.inv_reader.clone());
                hnsw_w.push(s.hnsw_reader.clone());
                scalar_w.push(s.scalar_reader.clone());
                // tombstone re-key：旧 ULID 的位图迁移到新 ULID（按顺序对应）。
            }
            // tombstone re-key：old_ulids[i] → new_ulids[i]。
            for (i, old_u) in old_ulids.iter().enumerate() {
                if let Some(bm) = tombstones.get(old_u) {
                    if let Some(new_u) = new_ulids.get(i) {
                        if !bm.is_empty() {
                            tomb_w.insert(new_u.clone(), bm.clone());
                        }
                    }
                }
            }
            // I-4：tokenizer/tokenizer_id 与 snapshot 段列表原子切换（同写锁块）。
            // 释放此块后，search 见到的 snapshot 与 tokenizer 必为同一身份。
            *tok_w = new_tokenizer;
            *tok_id_w = new_tokenizer_id;
        }

        // 删除旧段目录。
        for ulid in &old_ulids {
            let old_seg_dir = format!("{}/seg_{}", self.inner.segments_dir, ulid);
            let _ = crate::merge::delete_segment_dir(self.inner.vfs.as_ref(), &old_seg_dir);
        }

        // state → Stable。
        *self.inner.dict_state.write().unwrap() = DictState::Stable;
        Ok(ReindexHandle::completed())
    }
}
