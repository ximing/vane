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
    pub(crate) tokenizer: Box<dyn crate::tokenizer::Tokenizer>,
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
        let tokenizer = build_tokenizer(meta.tokenizer_kind, &meta.user_dict)?;
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
        // 读取每段 header.bin 已持久化的 docid_base（而非累加 doc_count 推断），
        // 更稳健：段顺序/非连续场景（M1 compaction）也能正确还原 offset。
        let mut max_end = 0u64;
        for ulid in &meta.segment_ulids {
            let seg_dir = format!("{}/segments/seg_{}", db.db_path, ulid);
            let reader = Arc::new(SegmentReader::open(&db.vfs, &seg_dir)?);
            // I7：同时 open InvertedIndexReader 缓存
            let inv_reader = Arc::new(InvertedIndexReader::open(&db.vfs, &seg_dir)?);
            let base = reader.meta().docid_base;
            let count = reader.doc_count() as u64;
            offsets.insert(ulid.clone(), base);
            max_end = max_end.max(base + count);
            readers.push(reader);
            inv_readers.push(inv_reader);
        }
        inner.write_state.lock().unwrap().next_docid = max_end;
        *inner.snapshot.write().unwrap() = readers;
        *inner.seg_offsets.write().unwrap() = offsets;
        *inner.inverted_readers.write().unwrap() = inv_readers;
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

        // 更新 manifest（I-6 原子切换）
        let manifest_store = ManifestStore::new(self.inner.vfs.clone(), &self.inner.db_path);
        manifest_store.add_segment(&self.inner.name, &meta.ulid)?;

        // 更新段快照（Arc swap 语义：写锁替换 Vec）
        let reader = Arc::new(SegmentReader::open(&self.inner.vfs, &seg_dir)?);
        // I7：open 一次 InvertedIndexReader 并缓存
        let inv_reader = Arc::new(InvertedIndexReader::open(&self.inner.vfs, &seg_dir)?);
        {
            let mut snap = self.inner.snapshot.write().unwrap();
            let mut offsets = self.inner.seg_offsets.write().unwrap();
            let mut inv_readers = self.inner.inverted_readers.write().unwrap();
            offsets.insert(meta.ulid.clone(), base_docid);
            snap.push(reader);
            inv_readers.push(inv_reader);
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
        if query.filter.is_some() {
            return Err(VaneError::InvalidArg("filter not supported in M0".into()));
        }
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
        let topk = query.top_k as usize;
        let cand = topk * query.candidate_multiplier as usize;

        let mut vec_candidates: Vec<crate::types::ScoredDoc> = Vec::new();
        let mut text_candidates: Vec<crate::types::ScoredDoc> = Vec::new();

        // snap 与 inv_readers 在 flush/restore 中成对维护，zip 迭代对齐更稳健
        for (reader, inv_reader) in snap.iter().zip(inv_readers.iter()) {
            let base = offsets.get(&reader.meta().ulid).copied().unwrap_or(0);
            // vector 路
            if matches!(mode, SearchMode::Hybrid | SearchMode::Vector) {
                if let (Some(qv), Some(metric)) = (&query.vector, vf) {
                    let mut hits = brute_search(
                        reader.vectors(),
                        reader.dim(),
                        qv,
                        metric,
                        if matches!(mode, SearchMode::Hybrid) {
                            cand
                        } else {
                            topk
                        },
                        None,
                        base,
                    );
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
                        None,
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

    // I1 裁决：M0 占位
    pub fn delete(&self, _ids: &[String]) -> Result<u64> {
        Err(VaneError::Unsupported)
    }
    pub fn compact(&self) -> Result<()> {
        Err(VaneError::Unsupported)
    }
    pub fn reindex(&self) -> Result<()> {
        Err(VaneError::Unsupported)
    }
}
