//! 06-userdict-reindex：ReindexHandle + 重建逻辑（SPEC §7.4 / §4.1）。
//!
//! ReindexHandle 持有 `Arc<ReindexInner>`，inner 含 `Mutex<RebuildState>` +
//! Condvar。`progress()` 读进度；`wait()` 阻塞直到完成（native Condvar，WASM
//! 轮询——M1 同步执行下两者均立即返回）。
//!
//! M1 同步执行（R-4/R-6）：`reindex()` 同步完成重建后返回已完成的 handle
//! （progress=1.0, wait 立即返回）。后台化留 M2 Executor。

use crate::bm25::{write_inverted, InvertedIndexBuilder, InvertedIndexReader};
use crate::hnsw::{write_hnsw, HnswReader, HnswWriter};
use crate::persistence::{CollectionMeta, Manifest, ManifestStore};
use crate::segment::{ScalarReader, SegmentReader, SegmentWriter};
use crate::tokenizer::Tokenizer;
use crate::types::{Result, Schema, TokenizerId, VaneError};
use crate::vfs::Vfs;
use std::sync::{Arc, Condvar, Mutex};

/// SPEC §4.1 ReindexHandle（可轮询可阻塞）。
///
/// M2-11 fix：derive Clone——FFI 注册表需 clone 出 ReindexHandle 后释放锁再调
/// `wait()`，避免持读锁阻塞（I-4）。inner 是 Arc，clone 廉价。
#[derive(Clone)]
pub struct ReindexHandle {
    inner: Arc<ReindexInner>,
}

struct ReindexInner {
    state: Mutex<RebuildState>,
    condvar: Condvar,
}

struct RebuildState {
    progress: f32,
    done: bool,
    error: Option<VaneError>,
}

impl ReindexHandle {
    /// 构造一个已完成的 handle（M1 同步执行：reindex 完成后调用）。
    pub(crate) fn completed() -> Self {
        let inner = Arc::new(ReindexInner {
            state: Mutex::new(RebuildState {
                progress: 1.0,
                done: true,
                error: None,
            }),
            condvar: Condvar::new(),
        });
        Self { inner }
    }

    /// 构造一个携带错误的已完成 handle（reindex 失败时调用）。
    #[allow(dead_code)]
    pub(crate) fn failed(err: VaneError) -> Self {
        let inner = Arc::new(ReindexInner {
            state: Mutex::new(RebuildState {
                progress: 1.0,
                done: true,
                error: Some(err),
            }),
            condvar: Condvar::new(),
        });
        Self { inner }
    }

    /// 返回当前进度 0.0..1.0（M1 同步执行下完成后恒 1.0）。
    pub fn progress(&self) -> f32 {
        self.inner.state.lock().unwrap().progress
    }

    /// 阻塞直到 reindex 完成。M1 同步执行下立即返回。
    pub fn wait(&self) -> Result<()> {
        let mut state = self.inner.state.lock().unwrap();
        while !state.done {
            state = self.inner.condvar.wait(state).unwrap();
        }
        if let Some(e) = state.error.clone() {
            return Err(e);
        }
        Ok(())
    }
}

/// 单段重建产物：新段 ULID + meta + 新 InvertedIndexReader/HnswReader/ScalarReader。
pub(crate) struct ReindexedSegment {
    pub ulid: String,
    #[allow(dead_code)]
    pub doc_count: u32,
    pub docid_base: u64,
    pub inv_reader: Arc<InvertedIndexReader>,
    pub hnsw_reader: Option<Arc<HnswReader>>,
    pub scalar_reader: Arc<ScalarReader>,
    pub reader: Arc<SegmentReader>,
}

/// 用新分词器重建单段倒排索引（B-1/00：从原文重新分词，非 posting remap）。
///
/// - 段新 ULID（I-1 段不可变）。
/// - 原文从旧段 `SegmentReader::text` 读出，用新分词器重新 tokenize →
///   `InvertedIndexBuilder::add_document`。
/// - vectors/hnsw/idmap/stored/scalars 经 SegmentWriter 重写（同 docid 顺序，
///   物理保留所有文档含 tombstone——tombstone 位图随后按新 ULID 复用）。
/// - 不跳过 tombstone 文档：reindex 只换分词身份，不做物理清除（与 compact 区分）。
///   docid 顺序不变 → tombstone 位图（绝对 docid）对新段同样有效，仅需 re-key ULID。
pub(crate) fn reindex_segment(
    vfs: &Arc<dyn Vfs>,
    segments_dir: &str,
    old_ulid: &str,
    schema: &Schema,
    new_tokenizer_id: &TokenizerId,
    new_tokenizer: &Arc<dyn Tokenizer>,
) -> Result<ReindexedSegment> {
    let old_seg_dir = format!("{}/seg_{}", segments_dir, old_ulid);
    let reader = SegmentReader::open(vfs, &old_seg_dir)?;
    let scalar_reader = ScalarReader::open(vfs, &old_seg_dir)?;

    // 收集 schema 中的标量字段名（供逐 docid 重写）。
    let scalar_fields: Vec<String> = schema
        .fields
        .iter()
        .filter_map(|(n, d)| match d {
            crate::types::FieldDef::Scalar { .. } => Some(n.clone()),
            _ => None,
        })
        .filter(|n| scalar_reader.has_field(n))
        .collect();

    let doc_count = reader.doc_count();
    let docid_base = reader.meta().docid_base;
    let dim = reader.dim() as usize;

    // 新段 writer（新 ULID、新 tokenizer_id、同 docid_base）。
    let mut writer = SegmentWriter::new(
        vfs.clone(),
        segments_dir,
        schema,
        new_tokenizer_id,
        docid_base,
    )?;
    let mut inv_builder = InvertedIndexBuilder::new(doc_count as usize);

    for local in 0..doc_count as u64 {
        let external_id = reader.external_id(local).unwrap_or("");
        let vector: Option<&[f32]> = if dim > 0 {
            Some(&reader.vectors()[(local as usize) * dim..(local as usize + 1) * dim])
        } else {
            None
        };
        let stored_json = reader.stored_json(local).unwrap_or("{}");
        let local_id = writer.add_doc(external_id, vector, stored_json)?;
        // 原文写入新段（供未来再次 reindex）。
        let text = reader.text(local).unwrap_or("");
        writer.set_text(text)?;
        // 标量重写（Q-7：按 local docid 读源段，写入新段同一 local docid）。
        for field in &scalar_fields {
            if let Some(sv) = scalar_reader.get(field, local as u32) {
                writer.set_scalar(field, sv)?;
            }
        }
        // 用新分词器重新 tokenize 原文（B-1：非 posting remap）。
        let tokens = new_tokenizer.tokenize(text);
        let field_len = tokens.len() as u32;
        let global_docid = docid_base + local_id;
        inv_builder.add_document(global_docid, &tokens, field_len);
    }

    let meta = writer.finalize()?;
    let new_seg_dir = format!("{}/seg_{}", segments_dir, meta.ulid);
    let inverted = inv_builder.build();
    write_inverted(vfs.as_ref(), &new_seg_dir, &inverted)?;

    // HNSW 重建（vectors 与分词无关，但段新 ULID 需重写 hnsw.bin；
    // 同 docid 顺序插入 → 功能等价图。计划允许「复制或重写」，此处重写）。
    let hnsw_reader = {
        if let Ok((_, vdim, metric)) = schema.vector_field() {
            if vdim > 0 && doc_count > 0 {
                let mut hw = HnswWriter::new(vdim, metric, 16, 200);
                let d = vdim as usize;
                // 从新段 reader 取 vectors（已 finalize 落盘）。
                let new_reader_tmp = SegmentReader::open(vfs, &new_seg_dir)?;
                let vectors = new_reader_tmp.vectors();
                for i in 0..doc_count {
                    let v = &vectors[(i as usize) * d..(i as usize + 1) * d];
                    hw.insert(i, v);
                }
                let graph = hw.build();
                if let Err(e) = write_hnsw(vfs.as_ref(), &new_seg_dir, &graph) {
                    eprintln!(
                        "[vane] hnsw write for reindexed segment {} failed: {} (fallback to brute)",
                        meta.ulid, e
                    );
                }
            }
        }
        match HnswReader::open(vfs, &new_seg_dir) {
            Ok(r) => Some(Arc::new(r)),
            Err(_) => None,
        }
    };

    let new_reader = Arc::new(SegmentReader::open(vfs, &new_seg_dir)?);
    let inv_reader = Arc::new(InvertedIndexReader::open(vfs, &new_seg_dir)?);
    let new_scalar = Arc::new(ScalarReader::open(vfs, &new_seg_dir)?);

    Ok(ReindexedSegment {
        ulid: meta.ulid,
        doc_count: meta.doc_count,
        docid_base: meta.docid_base,
        inv_reader,
        hnsw_reader,
        scalar_reader: new_scalar,
        reader: new_reader,
    })
}

/// 更新 manifest 中的 collection meta（段 ULID 替换 + tokenizer_id/user_dict 更新）。
pub(crate) fn update_manifest_after_reindex(
    manifest_store: &ManifestStore,
    col_name: &str,
    old_ulids: &[String],
    new_ulids: Vec<String>,
    new_meta: CollectionMeta,
) -> Result<()> {
    let mut manifest = manifest_store.load()?.unwrap_or_else(Manifest::empty);
    let col = manifest.collections.get_mut(col_name).ok_or_else(|| {
        VaneError::NotFound(format!(
            "collection not in manifest: {} (op=reindex; 建议: 确认 collection 已创建)",
            col_name
        ))
    })?;
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
}
