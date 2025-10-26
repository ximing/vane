//! 02-tombstone-merge：段合并（SPEC §7.3）。
//!
//! 物理清除 tombstone 文档，新段从零重建 HNSW 图（I-3）。倒排用 posting remap
//! 不重新分词（B-1）：从源段 InvertedIndexReader 读 term→postings，按新 docid
//! 重映射，重组 InvertedData → write_inverted。原文从源段 SegmentReader::text
//! 复用（00 前置）。标量重写经 `set_scalar` 从源段 ScalarReader 读出按新 docid
//! 重映射写入新段（Q-7，03-pre-filter 实装）。
//!
//! M1 全串行同步执行（R-4/R-6）：无 Executor/cfg，`step()` 处理一个源段全部数据。
//! 切片粒度留 M2 细化。

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap};

use crate::bm25::{write_inverted, InvertedData, InvertedIndexReader, Posting, TermPostings};
use crate::hnsw::{write_hnsw, HnswWriter};
use crate::segment::{ScalarReader, SegmentMeta, SegmentReader, SegmentWriter};
use crate::tokenizer::Tokenizer;
use crate::types::{Schema, TokenizerId};
use crate::vfs::Vfs;
use std::sync::Arc;

/// 合并候选段选择策略（SPEC §7.3：分层简化版，小段<1万优先，段数硬上限 10）。
///
/// 返回按优先级排序的待合并段 ULID 列表：
/// 优先 tombstone 比例高的段，其次 doc_count 小的段（小段优先）。
/// 调用方按需取全部（compact）或前 N 个（auto-merge）。
pub fn pick_merge_candidates(
    segments: &[Arc<SegmentReader>],
    tombstone_ratios: &[(String, f32)],
) -> Vec<String> {
    let ratio_of = |ulid: &str| -> f32 {
        tombstone_ratios
            .iter()
            .find(|(u, _)| u == ulid)
            .map(|(_, r)| *r)
            .unwrap_or(0.0)
    };
    let mut tagged: Vec<(Arc<SegmentReader>, f32, u32)> = segments
        .iter()
        .map(|r| (r.clone(), ratio_of(&r.meta().ulid), r.doc_count()))
        .collect();
    // 排序：tombstone 比例降序，再 doc_count 升序（小段优先）。
    tagged.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.2.cmp(&b.2))
    });
    tagged
        .into_iter()
        .map(|(r, _, _)| r.meta().ulid.clone())
        .collect()
}

/// 合并上下文（SPEC §7.3）。承载 VFS 与路径，跨 step 共享。
pub struct MergeContext<'a> {
    pub vfs: &'a Arc<dyn Vfs>,
    pub db_path: &'a str,
    pub segments_dir: &'a str,
}

/// 可切片增量合并任务（SPEC §7.3）。M1 同步执行（全串行，无 Executor）。
///
/// 每 `step()` 处理一个源段：读 vectors/idmap/stored/原文 → 跳过 tombstone →
/// SegmentWriter::add_doc + set_text（原文复用）+ HnswWriter::insert →
/// 倒排 posting remap（不重新分词，B-1）。全部源段处理完后 `finalize_merge`
/// 落盘 + 返回新段 meta。
pub struct MergeTask {
    source_ulids: Vec<String>,
    target_docid_base: u64,
    tokenizer_id: TokenizerId,
    schema: Schema,
    #[allow(dead_code)]
    tokenizer: Arc<dyn Tokenizer>,
    /// 调用方注入的内存 tombstone（ulid → 绝对 docid 位图）。
    /// 与源段 header.bin 的 tombstone 合并（取并集）。
    tombstones: HashMap<String, roaring::RoaringBitmap>,
    // 累积状态（跨 step）
    processed: usize,
    target_docid: u64,
    writer: Option<SegmentWriter>,
    /// term -> (new_docid -> tf)，跨源段累积。
    inv_terms: HashMap<String, HashMap<u64, u32>>,
    /// 按 new local docid 索引的字段长度。
    field_lengths: Vec<u32>,
    /// HnswWriter（首个有 vector 字段的段创建，跨源段累积）。
    hnsw_writer: Option<HnswWriter>,
}

impl MergeTask {
    /// 构造合并任务。
    /// `tokenizer` 仅为管线复用契约（M-2：06 reindex 传新 tokenizer 走重新分词）；
    /// 02 compact 走 posting remap 不重新分词，持 tokenizer 不切分。
    pub fn new(
        sources: Vec<String>,
        target_docid_base: u64,
        tokenizer_id: TokenizerId,
        schema: Schema,
        tokenizer: Arc<dyn Tokenizer>,
    ) -> Self {
        Self {
            source_ulids: sources,
            target_docid_base,
            tokenizer_id,
            schema,
            tokenizer,
            tombstones: HashMap::new(),
            processed: 0,
            target_docid: target_docid_base,
            writer: None,
            inv_terms: HashMap::new(),
            field_lengths: Vec::new(),
            hnsw_writer: None,
        }
    }

    /// 注入内存 tombstone（api compact 用：delete 期内存位图，未写 header.bin）。
    /// 与源段 header.bin 的 tombstone 取并集。属 M1 扩展方法，非契约签名变更。
    pub fn set_tombstones(&mut self, tombstones: HashMap<String, roaring::RoaringBitmap>) {
        self.tombstones = tombstones;
    }

    /// 执行一步合并；返回是否完成（true = 全部源段处理完）。
    pub fn step(&mut self, ctx: &MergeContext) -> crate::types::Result<bool> {
        if self.processed >= self.source_ulids.len() {
            return Ok(true);
        }
        let ulid = self.source_ulids[self.processed].clone();
        let seg_dir = format!("{}/seg_{}", ctx.segments_dir, ulid);
        let reader = SegmentReader::open(ctx.vfs, &seg_dir)?;
        let inv_reader = InvertedIndexReader::open(ctx.vfs, &seg_dir)?;
        // Q-7：加载源段 ScalarReader，标量按新 docid 重映射写入新段。
        let scalar_reader = ScalarReader::open(ctx.vfs, &seg_dir)?;
        // 收集该段有列的标量字段名（供下方逐 docid 重写）。
        let scalar_fields: Vec<String> = {
            // 通过 schema 枚举标量字段，再查 ScalarReader 是否有列。
            self.schema
                .fields
                .iter()
                .filter_map(|(n, d)| match d {
                    crate::types::FieldDef::Scalar { .. } => Some(n.clone()),
                    _ => None,
                })
                .filter(|n| scalar_reader.has_field(n))
                .collect()
        };

        // 合并 tombstone：header.bin 的 + 内存注入的（取并集）。
        let mut tombs = reader.meta().tombstones.clone();
        if let Some(extra) = self.tombstones.get(&ulid) {
            tombs |= extra;
        }
        let src_base = reader.meta().docid_base;
        let src_count = reader.doc_count() as u64;

        // 懒初始化 writer / hnsw_writer。
        if self.writer.is_none() {
            self.writer = Some(SegmentWriter::new(
                ctx.vfs.clone(),
                ctx.segments_dir,
                &self.schema,
                &self.tokenizer_id,
                self.target_docid_base,
            )?);
        }
        let writer = self.writer.as_mut().expect("writer initialized above");
        if self.hnsw_writer.is_none() {
            if let Ok((_, dim, metric)) = self.schema.vector_field() {
                if dim > 0 {
                    self.hnsw_writer = Some(HnswWriter::new(dim, metric, 16, 200));
                }
            }
        }

        let dim = reader.dim() as usize;
        let src_field_lengths = inv_reader.field_lengths();

        // docid 重映射：old_abs_docid -> new_abs_docid。
        let mut remap: HashMap<u64, u64> = HashMap::with_capacity(src_count as usize);

        for local in 0..src_count {
            let abs = src_base + local;
            // tombstone 存绝对 docid（u32）。超 u32 视为不命中（防御性，与 search 一致）。
            let tombed = if abs <= u32::MAX as u64 {
                tombs.contains(abs as u32)
            } else {
                false
            };
            if tombed {
                continue;
            }
            let external_id = match reader.external_id(local) {
                Some(e) => e,
                None => continue,
            };
            let vector: Option<&[f32]> = if dim > 0 {
                Some(&reader.vectors()[(local as usize) * dim..(local as usize + 1) * dim])
            } else {
                None
            };
            let stored_json = reader.stored_json(local).unwrap_or("{}");
            let new_local = writer.add_doc(external_id, vector, stored_json)?;
            let new_abs = self.target_docid_base + new_local;
            writer.set_text(reader.text(local).unwrap_or(""))?;
            // Q-7：标量重写——从源段 ScalarReader 读 local 的值，按 new_local 写入新段。
            for field in &scalar_fields {
                if let Some(sv) = scalar_reader.get(field, local as u32) {
                    writer.set_scalar(field, sv)?;
                }
            }
            if let Some(hw) = self.hnsw_writer.as_mut() {
                if let Some(v) = vector {
                    hw.insert(new_local as u32, v);
                }
            }
            // 字段长度（local 索引）。
            let fl = src_field_lengths.get(local as usize).copied().unwrap_or(0);
            self.field_lengths.push(fl);
            remap.insert(abs, new_abs);
            self.target_docid = new_abs + 1;
        }

        // 倒排 posting remap（B-1）：不重新分词，按 remap 重写 docid。
        for (term, te) in inv_reader.iter_terms() {
            let entry = self.inv_terms.entry(term.to_string()).or_default();
            for blk in &te.blocks {
                for p in &blk.postings {
                    if let Some(&new_docid) = remap.get(&p.docid) {
                        *entry.entry(new_docid).or_insert(0) += p.tf;
                    }
                }
            }
        }

        self.processed += 1;
        Ok(self.processed >= self.source_ulids.len())
    }

    /// 进度 0.0..1.0。
    pub fn progress(&self) -> f32 {
        if self.source_ulids.is_empty() {
            return 1.0;
        }
        self.processed as f32 / self.source_ulids.len() as f32
    }
}

/// 合并完成：落盘 vectors/stored/idmap/scalars/header + inverted + hnsw，返回新段 meta。
/// 新段 tombstone 恒为空（物理清除，SPEC §6.3）。
pub fn finalize_merge(
    mut task: MergeTask,
    ctx: &MergeContext,
) -> crate::types::Result<SegmentMeta> {
    let writer = task.writer.take().ok_or_else(|| {
        crate::types::VaneError::InvalidArg("finalize_merge with no steps".into())
    })?;
    let meta = writer.finalize()?;
    let seg_dir = format!("{}/seg_{}", ctx.segments_dir, meta.ulid);

    // 倒排：累积的 inv_terms -> InvertedData。
    let doc_count = task.field_lengths.len() as u64;
    let total_fl: u64 = task.field_lengths.iter().map(|&x| x as u64).sum();
    let avg_field_length = if doc_count == 0 {
        0.0
    } else {
        total_fl as f32 / doc_count as f32
    };
    let mut terms: BTreeMap<String, TermPostings> = BTreeMap::new();
    for (term, doc_tf) in task.inv_terms {
        let mut postings: Vec<Posting> = doc_tf
            .into_iter()
            .map(|(docid, tf)| Posting { docid, tf })
            .collect();
        postings.sort_by_key(|p| p.docid);
        let doc_freq = postings.len() as u32;
        terms.insert(term, TermPostings { doc_freq, postings });
    }
    let inv = InvertedData {
        docid_base: task.target_docid_base,
        doc_count,
        avg_field_length,
        field_lengths: std::mem::take(&mut task.field_lengths),
        terms,
    };
    write_inverted(ctx.vfs.as_ref(), &seg_dir, &inv)?;

    // HNSW：从零重建（I-3）。
    if let Some(hw) = task.hnsw_writer.take() {
        let graph = hw.build();
        if let Err(e) = write_hnsw(ctx.vfs.as_ref(), &seg_dir, &graph) {
            eprintln!(
                "[vane] hnsw write for merged segment {} failed: {} (fallback to brute)",
                meta.ulid, e
            );
        }
    }

    // 新段 tombstone 恒为空（物理清除）。
    Ok(SegmentMeta {
        ulid: meta.ulid,
        doc_count: meta.doc_count,
        docid_base: meta.docid_base,
        tokenizer_id: meta.tokenizer_id,
        tombstones: roaring::RoaringBitmap::new(),
    })
}

/// 递归删除段目录下全部文件（Vfs::delete 仅删单文件）。
/// core 禁 std::fs，经 Vfs::list 递归遍历删除。
pub fn delete_segment_dir(vfs: &dyn Vfs, seg_dir: &str) -> crate::types::Result<()> {
    fn collect(vfs: &dyn Vfs, dir: &str, out: &mut Vec<String>) -> crate::types::Result<()> {
        let entries = vfs.list(dir)?;
        for e in entries {
            let full = if dir.is_empty() {
                e.clone()
            } else {
                format!("{}/{}", dir.trim_end_matches('/'), e)
            };
            // 判断是否目录：list 该路径，非空且不报错视为目录。
            match vfs.list(&full) {
                Ok(sub) if !sub.is_empty() => collect(vfs, &full, out)?,
                _ => out.push(full),
            }
        }
        Ok(())
    }
    let mut paths = Vec::new();
    collect(vfs, seg_dir, &mut paths)?;
    for p in paths {
        // 忽略单个文件删除失败（可能已不存在），尽力清理。
        let _ = vfs.delete(&p);
    }
    Ok(())
}
