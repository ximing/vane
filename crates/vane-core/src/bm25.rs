// 由 05-bm25 计划填充：倒排索引构建 + Block-Max WAND top-k + posting vbyte 编码
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use crate::tokenizer::Token;
use crate::types::{Result, ScoredDoc, VaneError, BM25_B, BM25_K1, FORMAT_VERSION, MAGIC};
use crate::vfs::Vfs;

// ---------------------------------------------------------------------------
// 常量
// ---------------------------------------------------------------------------

const BLOCK_SIZE: usize = 128;

// ---------------------------------------------------------------------------
// vbyte 编解码（LEB128 风格，低 7 bit 先行，高位 1 = 还有后续字节）
// ---------------------------------------------------------------------------

pub(crate) fn vbyte_encode(mut val: u32, out: &mut Vec<u8>) {
    loop {
        let b = (val & 0x7F) as u8;
        val >>= 7;
        if val == 0 {
            out.push(b);
            break;
        } else {
            out.push(b | 0x80);
        }
    }
}

pub(crate) fn vbyte_decode(buf: &[u8]) -> Option<(u32, usize)> {
    let mut val: u32 = 0;
    let mut shift: u32 = 0;
    for (i, &b) in buf.iter().enumerate() {
        val = val.checked_add(((b & 0x7F) as u32).checked_shl(shift)?)?;
        if b & 0x80 == 0 {
            return Some((val, i + 1));
        }
        shift += 7;
        if shift >= 35 {
            return None; // u32 溢出保护
        }
    }
    None
}

// ---------------------------------------------------------------------------
// LE 读取辅助（S7：模块级自由函数，非闭包）
// ---------------------------------------------------------------------------

fn read_u32_le(b: &[u8], off: &mut usize) -> Result<u32> {
    if *off + 4 > b.len() {
        return Err(VaneError::Corrupt("inverted.bin truncated u32".into()));
    }
    let v = u32::from_le_bytes(b[*off..*off + 4].try_into().unwrap());
    *off += 4;
    Ok(v)
}

fn read_u64_le(b: &[u8], off: &mut usize) -> Result<u64> {
    if *off + 8 > b.len() {
        return Err(VaneError::Corrupt("inverted.bin truncated u64".into()));
    }
    let v = u64::from_le_bytes(b[*off..*off + 8].try_into().unwrap());
    *off += 8;
    Ok(v)
}

fn read_f32_le(b: &[u8], off: &mut usize) -> Result<f32> {
    if *off + 4 > b.len() {
        return Err(VaneError::Corrupt("inverted.bin truncated f32".into()));
    }
    let v = f32::from_le_bytes(b[*off..*off + 4].try_into().unwrap());
    *off += 4;
    Ok(v)
}

// ---------------------------------------------------------------------------
// BM25 公式（SPEC §6.3 冻结）
// ---------------------------------------------------------------------------

/// IDF = ln((N - df + 0.5)/(df + 0.5) + 1)
/// score = IDF * (tf*(k1+1)) / (tf + k1*(1 - b + b*dl/avgdl))
fn bm25_term_score(tf: u32, dl: u32, df: u32, n: u64, avgdl: f32) -> f32 {
    if avgdl <= 0.0 || n == 0 {
        return 0.0;
    }
    let idf = (((n as f64 - df as f64 + 0.5) / (df as f64 + 0.5) + 1.0).ln()) as f32;
    let tf_f = tf as f32;
    let dl_f = dl as f32;
    let denom = tf_f + BM25_K1 * (1.0 - BM25_B + BM25_B * (dl_f / avgdl));
    idf * (tf_f * (BM25_K1 + 1.0)) / denom
}

// ---------------------------------------------------------------------------
// Posting / TermPostings / InvertedData
// ---------------------------------------------------------------------------

/// 单条 posting（内存态，绝对 docid）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Posting {
    pub docid: u64,
    pub tf: u32,
}

/// 单 term 的全部 posting（按 docid 升序）。
#[derive(Debug, Clone)]
pub struct TermPostings {
    pub doc_freq: u32,
    pub postings: Vec<Posting>,
}

/// 构建完成的内存倒排，供 write_inverted 消费。
pub struct InvertedData {
    pub docid_base: u64,
    pub doc_count: u64,
    pub avg_field_length: f32,
    /// index = docid - docid_base；长度 = doc_count
    pub field_lengths: Vec<u32>,
    /// term 字典序（BTreeMap 保证 write 时有序）
    pub terms: BTreeMap<String, TermPostings>,
}

// ---------------------------------------------------------------------------
// InvertedIndexBuilder
// ---------------------------------------------------------------------------

pub struct InvertedIndexBuilder {
    #[allow(dead_code)]
    doc_count_hint: usize,
    doc_count: u64,
    total_field_length: u64,
    docid_base: u64,
    docid_base_set: bool,
    field_lengths: Vec<u32>,
    /// term -> (docid -> tf)
    terms: HashMap<String, HashMap<u64, u32>>,
}

impl InvertedIndexBuilder {
    pub fn new(doc_count_hint: usize) -> Self {
        Self {
            doc_count_hint,
            doc_count: 0,
            total_field_length: 0,
            docid_base: 0,
            docid_base_set: false,
            field_lengths: Vec::with_capacity(doc_count_hint),
            terms: HashMap::new(),
        }
    }

    pub fn add_document(&mut self, docid: u64, tokens: &[Token], field_length: u32) {
        if !self.docid_base_set {
            self.docid_base = docid;
            self.docid_base_set = true;
        }
        self.doc_count += 1;
        self.total_field_length += field_length as u64;
        self.field_lengths.push(field_length);

        // 同一文档内同 term 累积 tf
        for t in tokens {
            let entry = self.terms.entry(t.text.clone()).or_default();
            *entry.entry(docid).or_insert(0) += 1;
        }
    }

    pub fn build(self) -> InvertedData {
        let avg_field_length = if self.doc_count == 0 {
            0.0
        } else {
            self.total_field_length as f32 / self.doc_count as f32
        };

        let mut terms: BTreeMap<String, TermPostings> = BTreeMap::new();
        for (term, doc_tf) in self.terms {
            let mut postings: Vec<Posting> = doc_tf
                .into_iter()
                .map(|(docid, tf)| Posting { docid, tf })
                .collect();
            postings.sort_by_key(|p| p.docid);
            let doc_freq = postings.len() as u32;
            terms.insert(term, TermPostings { doc_freq, postings });
        }

        InvertedData {
            docid_base: self.docid_base,
            doc_count: self.doc_count,
            avg_field_length,
            field_lengths: self.field_lengths,
            terms,
        }
    }
}

// ---------------------------------------------------------------------------
// write_inverted（写 inverted.bin，SPEC §6.3 posting 布局）
// ---------------------------------------------------------------------------

/// 计算 block 内 max_score：遍历 block 内文档取真实 BM25 分最大值。
#[allow(clippy::too_many_arguments)]
fn block_max_score(
    postings: &[Posting],
    block_start: usize,
    block_len: usize,
    df: u32,
    n: u64,
    avgdl: f32,
    field_lengths: &[u32],
    docid_base: u64,
) -> f32 {
    let mut max = 0.0f32;
    for i in 0..block_len {
        let p = &postings[block_start + i];
        let local = (p.docid - docid_base) as usize;
        let dl = *field_lengths.get(local).unwrap_or(&0);
        let s = bm25_term_score(p.tf, dl, df, n, avgdl);
        if s > max {
            max = s;
        }
    }
    max
}

/// 写 inverted.bin 到段目录。
/// 格式：magic|version|docid_base|num_docs|avgdl|field_lengths|num_terms|{term...}
pub fn write_inverted(vfs: &dyn Vfs, segment_dir: &str, data: &InvertedData) -> Result<()> {
    let path = format!("{}/inverted.bin", segment_dir);
    vfs.create(&path)?;

    let mut buf: Vec<u8> = Vec::with_capacity(4096);
    // 头部
    buf.extend_from_slice(MAGIC);
    buf.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    buf.extend_from_slice(&data.docid_base.to_le_bytes());
    buf.extend_from_slice(&(data.doc_count as u32).to_le_bytes());
    buf.extend_from_slice(&data.avg_field_length.to_le_bytes());
    for &fl in &data.field_lengths {
        buf.extend_from_slice(&fl.to_le_bytes());
    }
    // 词典
    buf.extend_from_slice(&(data.terms.len() as u32).to_le_bytes());
    for (term, tp) in &data.terms {
        let term_bytes = term.as_bytes();
        let term_len = u16::try_from(term_bytes.len())
            .map_err(|_| VaneError::InvalidArg("term too long (>65535 bytes)".into()))?;
        buf.extend_from_slice(&term_len.to_le_bytes());
        buf.extend_from_slice(term_bytes);
        buf.extend_from_slice(&tp.doc_freq.to_le_bytes());

        // 切块
        let total = tp.postings.len();
        let num_blocks = if total == 0 {
            0
        } else {
            total.div_ceil(BLOCK_SIZE) as u32
        };
        buf.extend_from_slice(&num_blocks.to_le_bytes());

        let mut prev_docid: u64 = 0;
        let mut idx = 0;
        while idx < total {
            let block_len = BLOCK_SIZE.min(total - idx);
            let max_score = block_max_score(
                &tp.postings,
                idx,
                block_len,
                tp.doc_freq,
                data.doc_count,
                data.avg_field_length,
                &data.field_lengths,
                data.docid_base,
            );
            buf.extend_from_slice(&max_score.to_le_bytes());
            buf.push(block_len as u8);
            for j in 0..block_len {
                let p = &tp.postings[idx + j];
                let delta = (p.docid - prev_docid) as u32;
                vbyte_encode(delta, &mut buf);
                vbyte_encode(p.tf, &mut buf);
                prev_docid = p.docid;
            }
            idx += block_len;
        }
    }

    vfs.write_at(&path, &buf, 0)?;
    vfs.sync(&path)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// InvertedIndexReader + open
// ---------------------------------------------------------------------------

/// 加载到内存的 term 倒排（词典 + 块）。
#[derive(Debug, Clone)]
pub struct TermEntry {
    pub doc_freq: u32,
    pub idf: f32,
    /// 块数组：每块 max_score + 块内 posting（已解 vbyte，绝对 docid 还原）
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone)]
pub struct Block {
    pub max_score: f32,
    pub postings: Vec<Posting>, // 绝对 docid 升序
}

pub struct InvertedIndexReader {
    #[allow(dead_code)]
    vfs: Arc<dyn Vfs>,
    #[allow(dead_code)]
    segment_dir: String,
    docid_base: u64,
    doc_count: u64,
    avg_field_length: f32,
    field_lengths: Vec<u32>,
    /// 词典有序数组（按 term 字节序），二分查找
    terms: Vec<(String, TermEntry)>,
}

impl std::fmt::Debug for InvertedIndexReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InvertedIndexReader")
            .field("docid_base", &self.docid_base)
            .field("doc_count", &self.doc_count)
            .field("avg_field_length", &self.avg_field_length)
            .field("num_terms", &self.terms.len())
            .finish()
    }
}

impl InvertedIndexReader {
    pub fn open(vfs: &std::sync::Arc<dyn Vfs>, segment_dir: &str) -> Result<Self> {
        let path = format!("{}/inverted.bin", segment_dir);

        // 头部 8 字节：magic(4) + version(4)
        let mut header = [0u8; 8];
        let n = vfs.read_at(&path, &mut header, 0)?;
        if n < 8 {
            return Err(VaneError::Corrupt(crate::segment::seg_err(
                format!("inverted.bin truncated header: {}", n),
                segment_dir,
                "open inverted.bin",
            )));
        }
        if &header[0..4] != MAGIC {
            return Err(VaneError::Corrupt(crate::segment::seg_err(
                "inverted.bin bad magic",
                segment_dir,
                "open inverted.bin",
            )));
        }
        let version = u32::from_le_bytes(header[4..8].try_into().unwrap());
        if version != FORMAT_VERSION {
            return Err(VaneError::Version(crate::segment::seg_err(
                format!(
                    "inverted.bin version {} != supported {}",
                    version, FORMAT_VERSION
                ),
                segment_dir,
                "open inverted.bin",
            )));
        }

        // 读取剩余全部：循环 read_at 增量读取直到返回 0
        let mut blob: Vec<u8> = Vec::new();
        let mut offset: u64 = 8;
        let chunk = 64 * 1024;
        loop {
            let mut tmp = vec![0u8; chunk];
            let rn = vfs.read_at(&path, &mut tmp, offset)?;
            if rn == 0 {
                break;
            }
            blob.extend_from_slice(&tmp[..rn]);
            offset += rn as u64;
            if rn < chunk {
                break;
            }
        }

        let mut cur = 0usize;

        let docid_base = read_u64_le(&blob, &mut cur)?;
        let num_docs = read_u32_le(&blob, &mut cur)?;
        let avg_field_length = read_f32_le(&blob, &mut cur)?;

        let mut field_lengths = Vec::with_capacity(num_docs as usize);
        for _ in 0..num_docs {
            field_lengths.push(read_u32_le(&blob, &mut cur)?);
        }

        let num_terms = read_u32_le(&blob, &mut cur)?;
        let mut terms: Vec<(String, TermEntry)> = Vec::with_capacity(num_terms as usize);

        for _ in 0..num_terms {
            if cur + 2 > blob.len() {
                return Err(VaneError::Corrupt("inverted.bin truncated term_len".into()));
            }
            let term_len = u16::from_le_bytes(blob[cur..cur + 2].try_into().unwrap()) as usize;
            cur += 2;
            if cur + term_len > blob.len() {
                return Err(VaneError::Corrupt(
                    "inverted.bin truncated term_bytes".into(),
                ));
            }
            let term = String::from_utf8(blob[cur..cur + term_len].to_vec())
                .map_err(|e| VaneError::Corrupt(format!("inverted.bin term utf8: {}", e).into()))?;
            cur += term_len;

            let doc_freq = read_u32_le(&blob, &mut cur)?;
            let num_blocks = read_u32_le(&blob, &mut cur)?;

            let idf = if num_docs == 0 || doc_freq == 0 {
                0.0
            } else {
                (((num_docs as f64 - doc_freq as f64 + 0.5) / (doc_freq as f64 + 0.5) + 1.0).ln())
                    as f32
            };

            let mut blocks: Vec<Block> = Vec::with_capacity(num_blocks as usize);
            let mut prev_docid: u64 = 0;
            for _ in 0..num_blocks {
                let max_score = read_f32_le(&blob, &mut cur)?;
                if cur + 1 > blob.len() {
                    return Err(VaneError::Corrupt(
                        "inverted.bin truncated block_doc_count".into(),
                    ));
                }
                let block_doc_count = blob[cur] as usize;
                cur += 1;
                let mut postings: Vec<Posting> = Vec::with_capacity(block_doc_count);
                for _ in 0..block_doc_count {
                    let (delta, dn) = vbyte_decode(&blob[cur..])
                        .ok_or_else(|| VaneError::Corrupt("inverted.bin vbyte delta".into()))?;
                    cur += dn;
                    let (tf, tn) = vbyte_decode(&blob[cur..])
                        .ok_or_else(|| VaneError::Corrupt("inverted.bin vbyte tf".into()))?;
                    cur += tn;
                    prev_docid = prev_docid.checked_add(delta as u64).ok_or_else(|| {
                        VaneError::Corrupt("inverted.bin docid overflow".into())
                            .with_docid(prev_docid)
                    })?;
                    postings.push(Posting {
                        docid: prev_docid,
                        tf,
                    });
                }
                blocks.push(Block {
                    max_score,
                    postings,
                });
            }
            terms.push((
                term,
                TermEntry {
                    doc_freq,
                    idf,
                    blocks,
                },
            ));
        }

        Ok(Self {
            vfs: vfs.clone(),
            segment_dir: segment_dir.to_string(),
            docid_base,
            doc_count: num_docs as u64,
            avg_field_length,
            field_lengths,
            terms,
        })
    }

    pub fn doc_count(&self) -> u64 {
        self.doc_count
    }

    pub fn avg_field_length(&self) -> f32 {
        self.avg_field_length
    }

    /// 段 docid 基址（合并 posting remap 用，B-1）。
    pub fn docid_base(&self) -> u64 {
        self.docid_base
    }

    /// 段内每文档字段长度（index = local docid，合并 remap 用，B-1）。
    pub fn field_lengths(&self) -> &[u32] {
        &self.field_lengths
    }

    /// 迭代全部 term 及其 TermEntry（词典序）。
    /// 02-tombstone-merge 段合并 posting remap 用：读源段 postings 按新 docid 重写，
    /// 不重新分词（B-1）。属 M1 扩展，非 M0 冻结 API 破坏。
    pub fn iter_terms(&self) -> impl Iterator<Item = (&str, &TermEntry)> {
        self.terms.iter().map(|(t, e)| (t.as_str(), e))
    }

    /// 查找 term（二分，有序数组）。
    fn lookup(&self, term: &str) -> Option<&TermEntry> {
        self.terms
            .binary_search_by(|(t, _)| t.as_str().cmp(term))
            .ok()
            .map(|i| &self.terms[i].1)
    }

    fn field_length_of(&self, docid: u64) -> u32 {
        let local = docid.saturating_sub(self.docid_base) as usize;
        self.field_lengths.get(local).copied().unwrap_or(0)
    }
}

// ---------------------------------------------------------------------------
// Block-Max WAND search
// ---------------------------------------------------------------------------

/// 本地 OrderedFloat wrapper（避免引入 ordered-float crate）。
#[derive(Debug, Clone, Copy, PartialEq)]
struct OrderedFloat(f32);
impl Eq for OrderedFloat {}
impl PartialOrd for OrderedFloat {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for OrderedFloat {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0
            .partial_cmp(&other.0)
            .unwrap_or(std::cmp::Ordering::Equal)
    }
}

impl InvertedIndexReader {
    /// Block-Max WAND top-k（SPEC §8.1 text 模式）。
    pub fn search(
        &self,
        query_tokens: &[Token],
        topk: usize,
        filter: Option<&roaring::RoaringBitmap>,
    ) -> Vec<ScoredDoc> {
        if topk == 0 || query_tokens.is_empty() || self.doc_count == 0 {
            return Vec::new();
        }

        // query term 去重（保留首次出现）
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        let mut qterms: Vec<&TermEntry> = Vec::new();
        for t in query_tokens {
            if seen.insert(t.text.as_str()) {
                if let Some(te) = self.lookup(&t.text) {
                    qterms.push(te);
                }
            }
        }
        if qterms.is_empty() {
            return Vec::new();
        }

        // 每个 term 的指针：(block_idx, in_block_idx)
        let mut cursors: Vec<(usize, usize)> = vec![(0, 0); qterms.len()];
        // 最小堆（Reverse 包裹做 min-heap），存 (score, Reverse(docid))。
        // 并列分数时堆顶为最大 docid（先被淘汰），保留 docid 较小的——与暴力基线 tiebreak 一致。
        let mut heap: std::collections::BinaryHeap<
            std::cmp::Reverse<(OrderedFloat, std::cmp::Reverse<u64>)>,
        > = std::collections::BinaryHeap::with_capacity(topk + 1);

        loop {
            // 收集各 term 当前 docid
            let mut current: Vec<Option<u64>> = Vec::with_capacity(qterms.len());
            let mut alive = false;
            for (i, te) in qterms.iter().enumerate() {
                let (bi, ii) = cursors[i];
                if bi < te.blocks.len() {
                    let blk = &te.blocks[bi];
                    if ii < blk.postings.len() {
                        current.push(Some(blk.postings[ii].docid));
                        alive = true;
                        continue;
                    }
                }
                current.push(None);
            }
            if !alive {
                break;
            }

            // 候选 = min docid
            let candidate = current.iter().copied().flatten().min().unwrap();

            // 上界分 = Σ 各 term 当前 block 的 max_score（仅对当前 docid 所在 block）
            let mut upper = 0.0f32;
            for (i, te) in qterms.iter().enumerate() {
                let (bi, ii) = cursors[i];
                if bi < te.blocks.len() {
                    let blk = &te.blocks[bi];
                    if ii < blk.postings.len() && blk.postings[ii].docid == candidate {
                        upper += blk.max_score;
                    }
                }
            }

            // 剪枝判断
            let should_skip = heap.len() == topk && {
                let std::cmp::Reverse((top_score, _)) = *heap.peek().unwrap();
                upper <= top_score.0
            };

            if !should_skip {
                // filter 检查
                let pass = match filter {
                    None => true,
                    Some(f) => f.contains(candidate as u32),
                };
                if pass {
                    // 真实分
                    let mut score = 0.0f32;
                    for (i, te) in qterms.iter().enumerate() {
                        let (bi, ii) = cursors[i];
                        if bi < te.blocks.len() {
                            let blk = &te.blocks[bi];
                            if ii < blk.postings.len() && blk.postings[ii].docid == candidate {
                                let p = &blk.postings[ii];
                                let dl = self.field_length_of(p.docid);
                                score += bm25_term_score(
                                    p.tf,
                                    dl,
                                    te.doc_freq,
                                    self.doc_count,
                                    self.avg_field_length,
                                );
                            }
                        }
                    }
                    // 入堆
                    heap.push(std::cmp::Reverse((
                        OrderedFloat(score),
                        std::cmp::Reverse(candidate),
                    )));
                    if heap.len() > topk {
                        heap.pop();
                    }
                }
            }

            // 推进所有 term 当前 docid == candidate 的指针
            for (i, te) in qterms.iter().enumerate() {
                let (bi, ii) = cursors[i];
                if bi < te.blocks.len() {
                    let blk = &te.blocks[bi];
                    if ii < blk.postings.len() && blk.postings[ii].docid == candidate {
                        let mut nbi = bi;
                        let mut nii = ii + 1;
                        if nii >= blk.postings.len() {
                            nbi += 1;
                            nii = 0;
                        }
                        cursors[i] = (nbi, nii);
                    }
                }
            }
        }

        // 输出：堆转 Vec，按 score 降序
        let mut out: Vec<ScoredDoc> =
            heap.into_iter()
                .map(
                    |std::cmp::Reverse((OrderedFloat(score), std::cmp::Reverse(docid)))| {
                        ScoredDoc { docid, score }
                    },
                )
                .collect();
        out.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.docid.cmp(&b.docid))
        });
        out
    }
}

// ===========================================================================
// 测试
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vbyte_roundtrip_basic() {
        let cases = [0u32, 1, 127, 128, 255, 16383, 16384, u32::MAX / 4, u32::MAX];
        for &val in cases.iter() {
            let mut buf = Vec::new();
            vbyte_encode(val, &mut buf);
            let (decoded, n) = vbyte_decode(&buf).expect("decode must succeed");
            assert_eq!(decoded, val, "val {} roundtrip", val);
            assert_eq!(n, buf.len(), "consumed bytes must match");
        }
    }

    #[test]
    fn vbyte_encode_known_bytes() {
        // 0 -> [0x00]; 127 -> [0x7F]; 128 -> [0x80, 0x01]
        let mut b = Vec::new();
        vbyte_encode(0, &mut b);
        assert_eq!(b, vec![0x00]);
        b.clear();
        vbyte_encode(127, &mut b);
        assert_eq!(b, vec![0x7F]);
        b.clear();
        vbyte_encode(128, &mut b);
        assert_eq!(b, vec![0x80, 0x01]);
    }

    #[test]
    fn vbyte_decode_truncated_returns_none() {
        // 0x80 表示还有后续字节，但 buf 截断
        assert_eq!(vbyte_decode(&[0x80]), None);
        assert_eq!(vbyte_decode(&[]), None);
    }

    #[test]
    fn vbyte_stream_decode_multiple() {
        let mut buf = Vec::new();
        for v in [5u32, 300, 1_000_000, 42] {
            vbyte_encode(v, &mut buf);
        }
        let mut off = 0;
        let expect = [5u32, 300, 1_000_000, 42];
        for &e in expect.iter() {
            let (v, n) = vbyte_decode(&buf[off..]).expect("decode");
            assert_eq!(v, e);
            off += n;
        }
        assert_eq!(off, buf.len());
    }
}

#[cfg(test)]
mod builder_tests {
    use super::*;
    use crate::tokenizer::Token;

    fn tok(text: &str) -> Token {
        Token {
            text: text.to_string(),
            position: 0,
        }
    }

    #[test]
    fn builder_single_doc_single_term() {
        let mut b = InvertedIndexBuilder::new(8);
        b.add_document(10, &[tok("rust"), tok("rust")], 2);
        let data = b.build();
        assert_eq!(data.doc_count, 1);
        assert_eq!(data.docid_base, 10);
        assert_eq!(data.avg_field_length, 2.0);
        assert_eq!(data.field_lengths, vec![2]);
        let p = data.terms.get("rust").unwrap();
        assert_eq!(p.doc_freq, 1);
        assert_eq!(p.postings, vec![Posting { docid: 10, tf: 2 }]);
    }

    #[test]
    fn builder_multi_doc_accumulates_tf_and_sorts() {
        let mut b = InvertedIndexBuilder::new(8);
        // doc 0: "a" x3, "b" x1
        b.add_document(0, &[tok("a"), tok("a"), tok("a"), tok("b")], 4);
        // doc 1: "a" x1, "c" x2
        b.add_document(1, &[tok("a"), tok("c"), tok("c")], 3);
        let data = b.build();
        assert_eq!(data.doc_count, 2);
        assert_eq!(data.avg_field_length, 3.5);
        assert_eq!(data.field_lengths, vec![4, 3]);

        let a = data.terms.get("a").unwrap();
        assert_eq!(a.doc_freq, 2);
        assert_eq!(
            a.postings,
            vec![Posting { docid: 0, tf: 3 }, Posting { docid: 1, tf: 1 },]
        );
        let b_term = data.terms.get("b").unwrap();
        assert_eq!(b_term.postings, vec![Posting { docid: 0, tf: 1 }]);
        let c = data.terms.get("c").unwrap();
        assert_eq!(c.postings, vec![Posting { docid: 1, tf: 2 }]);

        // BTreeMap 保证字典序
        let keys: Vec<_> = data.terms.keys().collect();
        assert_eq!(
            keys,
            vec![&"a".to_string(), &"b".to_string(), &"c".to_string()]
        );
    }

    #[test]
    fn builder_empty_doc_count_hint_zero() {
        let b = InvertedIndexBuilder::new(0);
        let data = b.build();
        assert_eq!(data.doc_count, 0);
        assert!(data.terms.is_empty());
        assert!(data.avg_field_length.is_nan() || data.avg_field_length == 0.0);
    }
}

#[cfg(test)]
mod write_tests {
    use super::*;
    use crate::tokenizer::Token;
    use crate::vfs::memory::MemoryVfs;

    fn tok(text: &str) -> Token {
        Token {
            text: text.to_string(),
            position: 0,
        }
    }

    fn build_data() -> InvertedData {
        let mut b = InvertedIndexBuilder::new(8);
        // 3 docs, term "rust" 出现在 doc0(tf=2), doc1(tf=1), doc2(tf=3)
        b.add_document(0, &[tok("rust"), tok("rust"), tok("fast")], 3);
        b.add_document(1, &[tok("rust"), tok("fast")], 2);
        b.add_document(2, &[tok("rust"), tok("rust"), tok("rust"), tok("fast")], 4);
        b.build()
    }

    #[test]
    fn write_inverted_roundtrip_basic() {
        let vfs = MemoryVfs::new();
        let data = build_data();
        write_inverted(&vfs, "seg/test", &data).expect("write");

        let mut header = [0u8; 8];
        let n = vfs
            .read_at("seg/test/inverted.bin", &mut header, 0)
            .expect("read");
        assert_eq!(n, 8);
        assert_eq!(&header[0..4], b"VANE");
        assert_eq!(u32::from_le_bytes(header[4..8].try_into().unwrap()), 1);
    }

    #[test]
    fn write_inverted_creates_parent_dir() {
        let vfs = MemoryVfs::new();
        let data = build_data();
        write_inverted(&vfs, "seg/abc", &data).expect("write");
        let files = vfs.list("seg/abc").expect("list");
        assert!(files
            .iter()
            .any(|f| f == "inverted.bin" || f.ends_with("inverted.bin")));
    }

    #[test]
    fn write_inverted_empty_data() {
        let vfs = MemoryVfs::new();
        let b = InvertedIndexBuilder::new(0);
        let data = b.build();
        write_inverted(&vfs, "seg/empty", &data).expect("write empty");
        let mut header = [0u8; 8];
        vfs.read_at("seg/empty/inverted.bin", &mut header, 0)
            .expect("read");
        assert_eq!(&header[0..4], b"VANE");
    }
}

#[cfg(test)]
mod reader_tests {
    use super::*;
    use crate::tokenizer::Token;
    use crate::vfs::memory::MemoryVfs;

    fn tok(text: &str) -> Token {
        Token {
            text: text.to_string(),
            position: 0,
        }
    }

    fn build_and_write() -> (std::sync::Arc<dyn Vfs>, InvertedData) {
        let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
        let mut b = InvertedIndexBuilder::new(8);
        b.add_document(0, &[tok("rust"), tok("rust"), tok("fast")], 3);
        b.add_document(1, &[tok("rust"), tok("fast")], 2);
        b.add_document(2, &[tok("rust"), tok("rust"), tok("rust"), tok("fast")], 4);
        let data = b.build();
        write_inverted(vfs.as_ref(), "seg/test", &data).expect("write");
        (vfs, data)
    }

    #[test]
    fn open_reads_header_and_meta() {
        let (vfs, data) = build_and_write();
        let r = InvertedIndexReader::open(&vfs, "seg/test").expect("open");
        assert_eq!(r.doc_count(), data.doc_count);
        assert!((r.avg_field_length() - data.avg_field_length).abs() < 1e-5);
    }

    #[test]
    fn open_corrupt_magic_returns_corrupt() {
        let (vfs, _data) = build_and_write();
        // 破坏 magic
        vfs.write_at("seg/test/inverted.bin", b"XXXX", 0)
            .expect("write");
        let r = InvertedIndexReader::open(&vfs, "seg/test");
        match r {
            Err(VaneError::Corrupt(_)) => {}
            other => panic!("expected Corrupt, got {:?}", other.map_err(|e| e.name())),
        }
    }

    #[test]
    fn open_unsupported_version_returns_version() {
        let (vfs, _data) = build_and_write();
        vfs.write_at("seg/test/inverted.bin", &2u32.to_le_bytes(), 4)
            .expect("write");
        let r = InvertedIndexReader::open(&vfs, "seg/test");
        match r {
            Err(VaneError::Version(_)) => {}
            other => panic!("expected Version, got {:?}", other),
        }
    }
}

#[cfg(test)]
mod search_tests {
    use super::*;
    use crate::tokenizer::Token;
    use crate::types::ScoredDoc;
    use crate::vfs::memory::MemoryVfs;
    use roaring::RoaringBitmap;

    fn tok(text: &str) -> Token {
        Token {
            text: text.to_string(),
            position: 0,
        }
    }

    /// 构建一个 10 文档语料，term 分布已知。
    fn setup() -> (std::sync::Arc<dyn Vfs>, InvertedIndexReader) {
        let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
        let mut b = InvertedIndexBuilder::new(16);
        // doc0: rust rust fast
        b.add_document(0, &[tok("rust"), tok("rust"), tok("fast")], 3);
        // doc1: rust fast
        b.add_document(1, &[tok("rust"), tok("fast")], 2);
        // doc2: rust rust rust fast
        b.add_document(2, &[tok("rust"), tok("rust"), tok("rust"), tok("fast")], 4);
        // doc3: slow
        b.add_document(3, &[tok("slow")], 1);
        // doc4: rust
        b.add_document(4, &[tok("rust")], 1);
        // doc5..doc9: empty-ish (fast x1 each)
        for d in 5..10 {
            b.add_document(d, &[tok("fast")], 1);
        }
        let data = b.build();
        write_inverted(vfs.as_ref(), "seg/test", &data).expect("write");
        let r = InvertedIndexReader::open(&vfs, "seg/test").expect("open");
        (vfs, r)
    }

    /// 暴力基线：全扫描所有 term posting，对每个文档累加 BM25 分。
    fn brute_baseline(
        reader: &InvertedIndexReader,
        query: &[Token],
        topk: usize,
        filter: Option<&RoaringBitmap>,
    ) -> Vec<ScoredDoc> {
        // 收集 query term（去重）
        let mut qterms: Vec<&str> = query.iter().map(|t| t.text.as_str()).collect();
        qterms.sort();
        qterms.dedup();

        let n = reader.doc_count();
        let avgdl = reader.avg_field_length();
        let mut scores: std::collections::HashMap<u64, f32> = std::collections::HashMap::new();
        for qt in &qterms {
            let te = match reader.lookup(qt) {
                Some(t) => t,
                None => continue,
            };
            for blk in &te.blocks {
                for p in &blk.postings {
                    if let Some(f) = filter {
                        if !f.contains(p.docid as u32) {
                            continue;
                        }
                    }
                    let dl = reader.field_length_of(p.docid);
                    let s = bm25_term_score(p.tf, dl, te.doc_freq, n, avgdl);
                    *scores.entry(p.docid).or_insert(0.0) += s;
                }
            }
        }
        let mut v: Vec<ScoredDoc> = scores
            .into_iter()
            .map(|(docid, score)| ScoredDoc { docid, score })
            .collect();
        v.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.docid.cmp(&b.docid))
        });
        v.truncate(topk);
        v
    }

    #[test]
    fn search_single_term_topk() {
        let (_vfs, r) = setup();
        let q = vec![tok("rust")];
        let got = r.search(&q, 3, None);
        let expect = brute_baseline(&r, &q, 3, None);
        assert_eq!(got.len(), expect.len());
        for (g, e) in got.iter().zip(expect.iter()) {
            assert_eq!(g.docid, e.docid, "docid mismatch");
            assert!(
                (g.score - e.score).abs() < 1e-5,
                "score mismatch {} vs {}",
                g.score,
                e.score
            );
        }
        // doc2 (tf=3, dl=4) 应排第一
        assert_eq!(got[0].docid, 2);
    }

    #[test]
    fn search_multi_term_and_sort() {
        let (_vfs, r) = setup();
        let q = vec![tok("rust"), tok("fast")];
        let got = r.search(&q, 5, None);
        let expect = brute_baseline(&r, &q, 5, None);
        assert_eq!(got.len(), expect.len());
        for (g, e) in got.iter().zip(expect.iter()) {
            assert_eq!(g.docid, e.docid);
            assert!((g.score - e.score).abs() < 1e-5);
        }
    }

    #[test]
    fn search_matches_brute_large_topk() {
        let (_vfs, r) = setup();
        let q = vec![tok("rust"), tok("fast"), tok("slow")];
        let got = r.search(&q, 100, None);
        let expect = brute_baseline(&r, &q, 100, None);
        assert_eq!(got.len(), expect.len());
        for (g, e) in got.iter().zip(expect.iter()) {
            assert_eq!(g.docid, e.docid);
            assert!((g.score - e.score).abs() < 1e-5);
        }
    }

    #[test]
    fn search_with_filter() {
        let (_vfs, r) = setup();
        let mut bmp = RoaringBitmap::new();
        bmp.insert(0u32);
        bmp.insert(2u32);
        bmp.insert(4u32);
        let q = vec![tok("rust")];
        let got = r.search(&q, 10, Some(&bmp));
        let expect = brute_baseline(&r, &q, 10, Some(&bmp));
        assert_eq!(got.len(), expect.len());
        for (g, e) in got.iter().zip(expect.iter()) {
            assert_eq!(g.docid, e.docid);
        }
        // 结果只含 0/2/4
        for g in &got {
            assert!(bmp.contains(g.docid as u32));
        }
    }

    #[test]
    fn search_blockmax_pruning_correctness_large() {
        // 构造 >128 文档验证跳块剪枝正确
        let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
        let mut b = InvertedIndexBuilder::new(300);
        for d in 0u64..200 {
            // term "common" 出现在所有文档；term "rare" 仅出现在 doc 150
            let mut toks = vec![tok("common")];
            if d == 150 {
                toks.push(tok("rare"));
                toks.push(tok("rare"));
            }
            b.add_document(d, &toks, toks.len() as u32);
        }
        let data = b.build();
        write_inverted(vfs.as_ref(), "seg/big", &data).expect("write");
        let r = InvertedIndexReader::open(&vfs, "seg/big").expect("open");

        let q = vec![tok("common"), tok("rare")];
        let got = r.search(&q, 5, None);
        let expect = brute_baseline(&r, &q, 5, None);
        assert_eq!(got.len(), expect.len());
        for (g, e) in got.iter().zip(expect.iter()) {
            assert_eq!(
                g.docid, e.docid,
                "docid mismatch got={} exp={}",
                g.docid, e.docid
            );
            assert!((g.score - e.score).abs() < 1e-4);
        }
        // doc150 因 rare 的 IDF 高，应排第一
        assert_eq!(got[0].docid, 150);
    }
}

#[cfg(test)]
mod edge_tests {
    use super::*;
    use crate::tokenizer::Token;
    use crate::vfs::memory::MemoryVfs;

    fn tok(text: &str) -> Token {
        Token {
            text: text.to_string(),
            position: 0,
        }
    }

    #[test]
    fn search_empty_query_returns_empty() {
        let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
        let mut b = InvertedIndexBuilder::new(4);
        b.add_document(0, &[tok("a")], 1);
        let data = b.build();
        write_inverted(vfs.as_ref(), "seg/e", &data).expect("write");
        let r = InvertedIndexReader::open(&vfs, "seg/e").expect("open");
        let got = r.search(&[], 10, None);
        assert!(got.is_empty());
    }

    #[test]
    fn search_topk_zero_returns_empty() {
        let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
        let mut b = InvertedIndexBuilder::new(4);
        b.add_document(0, &[tok("a")], 1);
        let data = b.build();
        write_inverted(vfs.as_ref(), "seg/e", &data).expect("write");
        let r = InvertedIndexReader::open(&vfs, "seg/e").expect("open");
        let got = r.search(&[tok("a")], 0, None);
        assert!(got.is_empty());
    }

    #[test]
    fn search_single_doc() {
        let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
        let mut b = InvertedIndexBuilder::new(4);
        b.add_document(42, &[tok("hello"), tok("world")], 2);
        let data = b.build();
        write_inverted(vfs.as_ref(), "seg/e", &data).expect("write");
        let r = InvertedIndexReader::open(&vfs, "seg/e").expect("open");
        let got = r.search(&[tok("hello")], 5, None);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].docid, 42);
        assert!(got[0].score > 0.0);
    }

    #[test]
    fn search_topk_greater_than_doc_count() {
        let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
        let mut b = InvertedIndexBuilder::new(4);
        b.add_document(0, &[tok("a"), tok("b")], 2);
        b.add_document(1, &[tok("a")], 1);
        let data = b.build();
        write_inverted(vfs.as_ref(), "seg/e", &data).expect("write");
        let r = InvertedIndexReader::open(&vfs, "seg/e").expect("open");
        let got = r.search(&[tok("a")], 1000, None);
        assert_eq!(got.len(), 2); // 只有 2 个文档命中
    }

    #[test]
    fn search_query_term_not_in_index() {
        let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
        let mut b = InvertedIndexBuilder::new(4);
        b.add_document(0, &[tok("a")], 1);
        let data = b.build();
        write_inverted(vfs.as_ref(), "seg/e", &data).expect("write");
        let r = InvertedIndexReader::open(&vfs, "seg/e").expect("open");
        // "a" 在索引，"missing" 不在
        let got = r.search(&[tok("a"), tok("missing")], 10, None);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].docid, 0);
    }

    #[test]
    fn search_all_terms_missing_returns_empty() {
        let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
        let mut b = InvertedIndexBuilder::new(4);
        b.add_document(0, &[tok("a")], 1);
        let data = b.build();
        write_inverted(vfs.as_ref(), "seg/e", &data).expect("write");
        let r = InvertedIndexReader::open(&vfs, "seg/e").expect("open");
        let got = r.search(&[tok("zzz"), tok("yyy")], 10, None);
        assert!(got.is_empty());
    }

    #[test]
    fn search_filter_excludes_all() {
        let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
        let mut b = InvertedIndexBuilder::new(4);
        b.add_document(0, &[tok("a")], 1);
        b.add_document(1, &[tok("a")], 1);
        let data = b.build();
        write_inverted(vfs.as_ref(), "seg/e", &data).expect("write");
        let r = InvertedIndexReader::open(&vfs, "seg/e").expect("open");
        // 空位图：排除全部
        let bmp = roaring::RoaringBitmap::new();
        let got = r.search(&[tok("a")], 10, Some(&bmp));
        assert!(got.is_empty());
    }

    #[test]
    fn open_missing_file_returns_io_or_corrupt() {
        let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
        let r = InvertedIndexReader::open(&vfs, "seg/nonexistent");
        assert!(r.is_err());
    }

    #[test]
    fn write_then_open_docid_base_nonzero() {
        // docid 从 1000 起，验证 docid_base 往返
        let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
        let mut b = InvertedIndexBuilder::new(4);
        b.add_document(1000, &[tok("a")], 1);
        b.add_document(1001, &[tok("a"), tok("a")], 2);
        let data = b.build();
        assert_eq!(data.docid_base, 1000);
        write_inverted(vfs.as_ref(), "seg/b", &data).expect("write");
        let r = InvertedIndexReader::open(&vfs, "seg/b").expect("open");
        let got = r.search(&[tok("a")], 5, None);
        assert_eq!(got.len(), 2);
        assert!(got.iter().any(|s| s.docid == 1000));
        assert!(got.iter().any(|s| s.docid == 1001));
    }
}
