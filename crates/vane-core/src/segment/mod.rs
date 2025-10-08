pub mod header;
pub mod ulid;
#[cfg(test)]
mod tests;

use crate::types::{Result, Schema, TokenizerId, VaneError};
use crate::vfs::Vfs;
use std::sync::Arc;

/// 段元数据（SPEC §6.3）。写期由 SegmentWriter.finalize 产出，
/// 读期由 SegmentReader.open 从 header.bin 解码。
pub struct SegmentMeta {
    pub ulid: String,
    pub doc_count: u32,
    pub docid_base: u64,
    pub tokenizer_id: TokenizerId,
    /// tombstone 位图（SPEC §6.3）。M0 恒为空（delete 是 M1）。
    pub tombstones: roaring::RoaringBitmap,
}

/// 段写期句柄（SPEC §6.2/§6.4）。构建 header.bin / vectors.bin /
/// stored.bin / scalars.col / idmap.bin；不写 inverted.bin（由 05-bm25 的
/// write_inverted 单独写）。finalize 消费 self（不变量 I-1：段不可变）。
pub struct SegmentWriter {
    vfs: Arc<dyn Vfs>,
    segments_dir: String,
    ulid: String,
    tokenizer_id: TokenizerId,
    docid_base: u64,
    next_docid: u64,
    vectors: Vec<f32>,
    dim: u32,
    stored: Vec<(u64, String)>, // (local docid, stored_json)
    id_map: Vec<(u64, String)>, // (local docid, external_id)
}

impl SegmentWriter {
    pub fn new(
        vfs: Arc<dyn Vfs>,
        segments_dir: &str,
        schema: &Schema,
        tokenizer_id: &TokenizerId,
        docid_base: u64,
    ) -> Result<Self> {
        let ulid = ulid::gen_ulid();
        let dim = schema.vector_field().map(|(_, d, _)| d).unwrap_or(0);
        // S3: new() 不预建目录（StdFsVfs::resolve 已 create_dir_all，finalize 才写文件）。
        Ok(Self {
            vfs,
            segments_dir: segments_dir.to_string(),
            ulid,
            tokenizer_id: tokenizer_id.clone(),
            docid_base,
            next_docid: 0,
            vectors: Vec::new(),
            dim,
            stored: Vec::new(),
            id_map: Vec::new(),
        })
    }

    pub fn docid_base(&self) -> u64 {
        self.docid_base
    }

    /// 返回段内局部 docid（从 0 起 u64 单调递增）。
    /// 全局 docid = docid_base + 返回值。
    pub fn add_doc(
        &mut self,
        external_id: &str,
        vector: Option<&[f32]>,
        stored_json: &str,
    ) -> Result<u64> {
        let docid = self.next_docid;
        self.next_docid += 1;
        if let Some(v) = vector {
            if self.dim == 0 {
                return Err(VaneError::Schema(
                    "vector provided but schema has no vector field".into(),
                ));
            }
            if v.len() as u32 != self.dim {
                return Err(VaneError::Schema(format!(
                    "vector dim mismatch: got {} expected {}",
                    v.len(),
                    self.dim
                )));
            }
            self.vectors.extend_from_slice(v);
        } else if self.dim > 0 {
            // S4: schema 有 vector 字段但 doc 未提供 vector → 填零向量。
            // 保证 docid i 的向量在 vectors[i*dim..]。
            self.vectors.resize(self.vectors.len() + self.dim as usize, 0.0f32);
        }
        self.id_map.push((docid, external_id.to_string()));
        self.stored.push((docid, stored_json.to_string()));
        Ok(docid)
    }

    pub fn finalize(self) -> Result<SegmentMeta> {
        let seg_dir = format!("{}/seg_{}", self.segments_dir, self.ulid);

        // 写 vectors.bin（f32 LE 连续）
        let vpath = format!("{}/vectors.bin", seg_dir);
        self.vfs.create(&vpath)?;
        let mut vbytes = Vec::with_capacity(self.vectors.len() * 4);
        for f in &self.vectors {
            vbytes.extend_from_slice(&f.to_le_bytes());
        }
        self.vfs.write_at(&vpath, &vbytes, 0)?;
        self.vfs.sync(&vpath)?;

        // 写 stored.bin：magic|version|count|{docid(8 LE)|len(4 LE)|json}...
        // I10: M0 写裸 JSON（zstd 块压缩延后 M1，format_version 不变）。
        let spath = format!("{}/stored.bin", seg_dir);
        self.vfs.create(&spath)?;
        let mut sbytes = Vec::new();
        sbytes.extend_from_slice(crate::types::MAGIC);
        sbytes.extend_from_slice(&crate::types::FORMAT_VERSION.to_be_bytes());
        sbytes.extend_from_slice(&(self.stored.len() as u32).to_le_bytes());
        for (docid, json) in &self.stored {
            sbytes.extend_from_slice(&docid.to_le_bytes());
            sbytes.extend_from_slice(&(json.len() as u32).to_le_bytes());
            sbytes.extend_from_slice(json.as_bytes());
        }
        self.vfs.write_at(&spath, &sbytes, 0)?;
        self.vfs.sync(&spath)?;

        // 写 idmap.bin（docid → external_id，SPEC §3.2 映射表持久化落点）。
        let ipath = format!("{}/idmap.bin", seg_dir);
        self.vfs.create(&ipath)?;
        let mut ibytes = Vec::new();
        ibytes.extend_from_slice(crate::types::MAGIC);
        ibytes.extend_from_slice(&crate::types::FORMAT_VERSION.to_be_bytes());
        ibytes.extend_from_slice(&(self.id_map.len() as u32).to_le_bytes());
        for (docid, eid) in &self.id_map {
            ibytes.extend_from_slice(&docid.to_le_bytes());
            ibytes.extend_from_slice(&(eid.len() as u32).to_le_bytes());
            ibytes.extend_from_slice(eid.as_bytes());
        }
        self.vfs.write_at(&ipath, &ibytes, 0)?;
        self.vfs.sync(&ipath)?;

        // S2: 写 scalars.col（空 stub：magic+version+0 字段）。
        // M0 filter 未实现，scalars 无消费方，写空保证段目录布局完整。
        let colpath = format!("{}/scalars.col", seg_dir);
        self.vfs.create(&colpath)?;
        let mut scbytes = Vec::new();
        scbytes.extend_from_slice(crate::types::MAGIC);
        scbytes.extend_from_slice(&crate::types::FORMAT_VERSION.to_be_bytes());
        scbytes.extend_from_slice(&0u32.to_le_bytes()); // 0 个标量字段
        self.vfs.write_at(&colpath, &scbytes, 0)?;
        self.vfs.sync(&colpath)?;

        let meta = SegmentMeta {
            ulid: self.ulid.clone(),
            doc_count: self.next_docid as u32,
            docid_base: self.docid_base,
            tokenizer_id: self.tokenizer_id.clone(),
            tombstones: roaring::RoaringBitmap::new(),
        };

        // 写 header.bin（I4: 含真实 docid_base）。
        let hpath = format!("{}/header.bin", seg_dir);
        self.vfs.create(&hpath)?;
        let hbytes = header::encode_header(&meta)?;
        self.vfs.write_at(&hpath, &hbytes, 0)?;
        self.vfs.sync(&hpath)?;

        // I-1: 段不可变——finalize 消费 self，编译期保证不可再调 add_doc。
        Ok(meta)
    }
}
