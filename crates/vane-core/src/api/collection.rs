//! SPEC §4.1 Collection 句柄：add/flush/search 编排。
//!
//! - WriteState（Mutex）护 buffer + auto-committer + docid 计数器
//! - snapshot（RwLock<Vec<Arc<SegmentReader>>>）护段快照，读路径零锁竞争
//! - I7：InvertedIndexReader 随段快照缓存，search 直接用，避免每次重开
//!
//! flush 编排：SegmentWriter + InvertedIndexBuilder + write_inverted + ManifestStore 原子切换。
//! flush 后向量与 BM25 在同一段快照同时可见（不变量 I-2）。

use crate::api::db::DbInner;
use crate::api::types::*;
use crate::bm25::{write_inverted, InvertedIndexBuilder, InvertedIndexReader};
use crate::fusion::{linear_fuse, minmax_normalize, rrf_fuse, FusionCandidate};
use crate::hnsw::{write_hnsw, HnswReader, HnswWriter};
use crate::persistence::{AutoCommitConfig, AutoCommitter, CollectionMeta, ManifestStore};
use crate::segment::{SegmentReader, SegmentWriter};
use crate::tokenizer::build_tokenizer;
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
    pub(crate) tokenizer: Arc<dyn crate::tokenizer::Tokenizer>,
    pub(crate) tokenizer_id: CoreTokenizerId,
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
    // 02-tombstone-merge：段 ULID → tombstone 位图（绝对 docid）。
    // delete 期更新内存位图（不修改段文件 I-1）；持久化经 WAL（04 计划）。
    // 查询期 search 把 tombstone 并入 filter 参数（02 手动并入；03 计划 compile_filter 统一）。
    tombstones: RwLock<HashMap<String, roaring::RoaringBitmap>>,
    // 02-tombstone-merge：compact 进行中标志（防重入；06 reindex 状态机复用）。
    compacting: Mutex<bool>,
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

impl CollectionInner {
    // I3 裁决：create_new 接收 auto_commit 参数（collection 级配置，SPEC §7.1）
    pub(crate) fn create_new(
        db: &DbInner,
        name: &str,
        meta: CollectionMeta,
        auto_commit: AutoCommitConfig,
    ) -> Result<Self> {
        let tokenizer: Arc<dyn crate::tokenizer::Tokenizer> =
            Arc::<dyn crate::tokenizer::Tokenizer>::from(build_tokenizer(
                meta.tokenizer_kind,
                &meta.user_dict,
            )?);
        let segments_dir = format!("{}/segments", db.db_path);
        Ok(Self {
            name: name.to_string(),
            schema: meta.schema,
            tokenizer,
            tokenizer_id: meta.tokenizer_id,
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
            tombstones: RwLock::new(HashMap::new()),
            compacting: Mutex::new(false),
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
            let base = reader.meta().docid_base;
            let count = reader.doc_count() as u64;
            offsets.insert(ulid.clone(), base);
            max_end = max_end.max(base + count);
            readers.push(reader);
            inv_readers.push(inv_reader);
            hnsw_readers.push(hnsw_reader);
        }
        inner.write_state.lock().unwrap().next_docid = max_end;
        *inner.snapshot.write().unwrap() = readers;
        *inner.seg_offsets.write().unwrap() = offsets;
        *inner.inverted_readers.write().unwrap() = inv_readers;
        *inner.hnsw_readers.write().unwrap() = hnsw_readers;
        Ok(inner)
    }
}

impl Collection {
    pub fn add(&self, docs: &[Doc]) -> Result<AddReport> {
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
        let mut writer = SegmentWriter::new(
            self.inner.vfs.clone(),
            &self.inner.segments_dir,
            &self.inner.schema,
            &self.inner.tokenizer_id,
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
            let global_docid = base_docid + local_docid;
            let tokens = doc
                .text
                .as_ref()
                .map(|t| self.inner.tokenizer.tokenize(t))
                .unwrap_or_default();
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

        // 更新 manifest（I-6 原子切换）
        let manifest_store = ManifestStore::new(self.inner.vfs.clone(), &self.inner.db_path);
        manifest_store.add_segment(&self.inner.name, &meta.ulid)?;

        // 更新段快照（Arc swap 语义：写锁替换 Vec）
        // I7：open 一次 InvertedIndexReader 并缓存
        let inv_reader = Arc::new(InvertedIndexReader::open(&self.inner.vfs, &seg_dir)?);
        {
            let mut snap = self.inner.snapshot.write().unwrap();
            let mut offsets = self.inner.seg_offsets.write().unwrap();
            let mut inv_readers = self.inner.inverted_readers.write().unwrap();
            let mut hnsw_readers = self.inner.hnsw_readers.write().unwrap();
            offsets.insert(meta.ulid.clone(), base_docid);
            snap.push(reader);
            inv_readers.push(inv_reader);
            hnsw_readers.push(hnsw_reader);
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
        let tokenizer_arc = self.inner.tokenizer.clone();
        // target_docid_base = 0（合并后新段从 0 起连续）。
        let mut task = crate::merge::MergeTask::new(
            source_ulids.clone(),
            0,
            self.inner.tokenizer_id.clone(),
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
        manifest_store.save_atomic(&manifest)?;

        // 更新内存快照。
        let new_reader = Arc::new(SegmentReader::open(&self.inner.vfs, &new_seg_dir)?);
        let new_inv = Arc::new(InvertedIndexReader::open(&self.inner.vfs, &new_seg_dir)?);
        let new_hnsw = match HnswReader::open(&self.inner.vfs, &new_seg_dir) {
            Ok(r) => Some(Arc::new(r)),
            Err(_) => None,
        };
        {
            let mut snap_w = self.inner.snapshot.write().unwrap();
            let mut offsets_w = self.inner.seg_offsets.write().unwrap();
            let mut inv_w = self.inner.inverted_readers.write().unwrap();
            let mut hnsw_w = self.inner.hnsw_readers.write().unwrap();
            let mut tomb_w = self.inner.tombstones.write().unwrap();
            let old_snap = std::mem::take(&mut *snap_w);
            let old_inv = std::mem::take(&mut *inv_w);
            let old_hnsw = std::mem::take(&mut *hnsw_w);
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
        }
        Ok(())
    }

    pub fn search(&self, query: &SearchQuery) -> Result<Vec<Hit>> {
        if query.top_k > TOPK_MAX {
            return Err(VaneError::InvalidArg(format!(
                "topK {} exceeds max {}",
                query.top_k, TOPK_MAX
            )));
        }
        // 01-hnsw Task 5：filter 编译由 03-pre-filter 实装；本计划不再 reject，
        // 透传 None 占位（03 接入后补编译位图 + 自适应回退判定）。
        // 02-tombstone-merge：tombstone 位图并入 filter（手动并入；03 计划 compile_filter 统一）。
        // 此处 filter_bm 为用户 filter（02 恒 None），tombstone 在每段循环内并入 alive_bm。
        let filter_bm: Option<&roaring::RoaringBitmap> = None;
        // mode 推断（S8：Auto 内部用，绑定层不暴露 "auto" 字符串）
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
        // 02-tombstone-merge：tombstone 位图（ulid → 绝对 docid），查询期并入 filter。
        let tombstones = self.inner.tombstones.read().unwrap();
        let topk = query.top_k as usize;
        let cand = topk * query.candidate_multiplier as usize;

        let mut vec_candidates: Vec<crate::types::ScoredDoc> = Vec::new();
        let mut text_candidates: Vec<crate::types::ScoredDoc> = Vec::new();

        // 自适应回退（SPEC §8.1）：filter 位图基数 < 2*topk → 暴力精确扫描。
        // M1 filter_bm=None（03 接入后补），此分支在 03 前不会触发。
        let force_brute = match filter_bm {
            Some(bm) => (bm.len() as usize) < 2 * topk,
            None => false,
        };

        // snap/inv_readers/hnsw_readers 在 flush/restore 中成对维护，zip 迭代对齐
        for ((reader, inv_reader), hnsw_reader) in
            snap.iter().zip(inv_readers.iter()).zip(hnsw_readers.iter())
        {
            let base = offsets.get(&reader.meta().ulid).copied().unwrap_or(0);
            // 02-tombstone-merge：构建本段 alive 位图 = [base, base+count) − tombstone。
            // tombstone 为 EXCLUSION 语义，search 的 filter 为 INCLUSION，故转 alive 集。
            // 03 计划正式 compile_filter 把用户 filter AND alive_bm 统一编译。
            let alive_bm: Option<roaring::RoaringBitmap> = {
                let seg_tombs = tombstones.get(&reader.meta().ulid);
                if let Some(tombs) = seg_tombs {
                    if tombs.is_empty() {
                        None
                    } else {
                        let mut bm = roaring::RoaringBitmap::new();
                        let start = base as u32;
                        let end = base + reader.doc_count() as u64;
                        if end <= u64::from(u32::MAX) {
                            bm.insert_range(start..(end as u32));
                            bm -= tombs;
                        }
                        Some(bm)
                    }
                } else {
                    None
                }
            };
            let alive_ref: Option<&roaring::RoaringBitmap> = alive_bm.as_ref();
            // 合并用户 filter（02 恒 None）与 alive：03 会做 AND 编译；此处取 alive（None 时透传 filter_bm）。
            let merged_filter: Option<&roaring::RoaringBitmap> = if alive_ref.is_some() {
                alive_ref
            } else {
                filter_bm
            };
            // vector 路
            if matches!(mode, SearchMode::Hybrid | SearchMode::Vector) {
                if let (Some(qv), Some(metric)) = (&query.vector, vf) {
                    let want = if matches!(mode, SearchMode::Hybrid) {
                        cand
                    } else {
                        topk
                    };
                    // 01-hnsw：有 HnswReader 且无需强制暴力 → HNSW 搜索；
                    // 否则 fallback brute_search（M0 段无 hnsw.bin / 低选择率回退 / 写失败）
                    let mut hits = if !force_brute {
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
                    let tokens = self.inner.tokenizer.tokenize(qt);
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

    /// M1 实装（02-tombstone-merge）：追加 tombstone（内存位图）。
    /// 查询期 search 把 tombstone 并入 filter 过滤；持久化经 WAL（04 计划）。
    /// 段不可变（I-1）：不修改段文件，仅更新内存位图。
    pub fn delete(&self, ids: &[String]) -> Result<u64> {
        let snap = self.inner.snapshot.read().unwrap();
        let offsets = self.inner.seg_offsets.read().unwrap();
        let mut tombstones = self.inner.tombstones.write().unwrap();
        let mut count: u64 = 0;
        // 构建 external_id → (ulid, abs_docid) 反查。doc 数通常不大；逐段 HashMap 查找。
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
                    let bm = tombstones.entry(reader.meta().ulid.clone()).or_default();
                    if bm.insert(abs as u32) {
                        count += 1;
                    }
                    break; // 一个 external_id 只可能存在于一个段
                }
            }
        }
        Ok(count)
    }

    /// M1 实装（02-tombstone-merge）：手动触发段合并（compact）。
    /// 物理清除 tombstone 文档，新段从零重建 HNSW 图（I-3）。
    /// 全串行同步执行（R-4/R-6）；E_BUSY 若 compact 进行中（06 reindex 状态机复用）。
    pub fn compact(&self) -> Result<()> {
        // 重入保护。
        {
            let mut guard = self.inner.compacting.lock().unwrap();
            if *guard {
                return Err(VaneError::Busy);
            }
            *guard = true;
        }
        // 作用域结束释放 guard，确保 panic 时不死锁——改用显式 finally 模式。
        let result = self.run_compact();
        *self.inner.compacting.lock().unwrap() = false;
        result
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
        self.merge_segments(source_ulids)
    }

    pub fn reindex(&self) -> Result<()> {
        Err(VaneError::Unsupported)
    }
}
