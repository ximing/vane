pub mod header;
#[cfg(test)]
mod tests;
pub mod ulid;

use crate::types::{Result, ScalarKind, Schema, TokenizerId, VaneError};
use crate::vfs::Vfs;
use std::sync::{Arc, OnceLock};

/// 段元数据（SPEC §6.3）。写期由 SegmentWriter.finalize 产出，
/// 读期由 SegmentReader.open 从 header.bin 解码。
pub struct SegmentMeta {
    pub ulid: String,
    pub doc_count: u32,
    pub docid_base: u64,
    pub tokenizer_id: TokenizerId,
    /// tombstone 位图（SPEC §6.3）。M0 恒为空（delete 是 M1）。
    ///
    /// 存**绝对 docid**（u32 空间，与 WAL `WalRecord::AddTombstone.docids` 及
    /// 运行期 `CollectionInner.tombstones` 位图一致，M-minor-2 / M2 parked minor 2.1.6）。
    /// 段内 local docid 仅在 SegmentReader 边界处由 `docid_base` 转换。
    pub tombstones: roaring::RoaringBitmap,
}

/// stored.bin 写期单条记录（SPEC §6.2：原文 + JSON meta 分离存储）。
/// text 为 Option<String>：未调 set_text 时为 None，finalize 写入时落空串（text_len=0）。
struct StoredEntry {
    docid: u64,
    text: Option<String>,
    meta_json: String,
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
    stored: Vec<StoredEntry>,   // (local docid, text, meta_json)
    id_map: Vec<(u64, String)>, // (local docid, external_id)
    // 03-pre-filter：field_name -> (ScalarKind, Vec<Option<ScalarValue>>)，按 local docid 索引。
    // set_scalar 在 add_doc 后调用；未调 set_scalar 的 docid 该字段为 None。
    // finalize 写 scalars.col 列式块（SPEC §6.2）。
    scalars: std::collections::HashMap<String, (ScalarKind, Vec<Option<crate::api::ScalarValue>>)>,
    // 03-pre-filter：schema 快照（set_scalar 校验字段类型用；new 时 clone，不改 M0 签名）。
    schema_snapshot: Option<Schema>,
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
            scalars: std::collections::HashMap::new(),
            schema_snapshot: Some(schema.clone()),
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
            self.vectors
                .resize(self.vectors.len() + self.dim as usize, 0.0f32);
        }
        self.id_map.push((docid, external_id.to_string()));
        self.stored.push(StoredEntry {
            docid,
            text: None,
            meta_json: stored_json.to_string(),
        });
        Ok(docid)
    }

    /// 为最近一次 add_doc 的文档设置原文（SPEC §6.2 stored.bin 含原文）。
    /// 在 add_doc 之后、finalize 之前调用；重复调用覆盖。未调用则该文档 text_len=0。
    /// 不变量 I-1：仅修改写期 buffer，stored.bin 仍在 finalize 一次性写入（段不可变）。
    pub fn set_text(&mut self, text: &str) -> Result<()> {
        let entry = self
            .stored
            .last_mut()
            .ok_or_else(|| VaneError::Schema("set_text called before add_doc".into()))?;
        entry.text = Some(text.to_string());
        Ok(())
    }

    /// 为最近一次 add_doc 的文档设置标量字段值（SPEC §3.1/§6.2，03-pre-filter）。
    /// 在 add_doc 之后、finalize 之前调用；重复调用覆盖当前文档该字段值。
    /// 字段必须存在于 schema 且为 Scalar 类型，且 value 的变体与 ScalarKind 匹配，
    /// 否则 Err(Schema)。不调 set_scalar 的 docid 该字段为 None（filter 不命中）。
    /// 不变量 I-1：仅修改写期 buffer，scalars.col 仍在 finalize 一次性写入。
    pub fn set_scalar(&mut self, field: &str, value: crate::api::ScalarValue) -> Result<()> {
        if self.next_docid == 0 {
            return Err(VaneError::Schema("set_scalar called before add_doc".into()));
        }
        // 校验字段在 schema 且为 Scalar。
        let kind = self
            .schema_scalar_kind(field)
            .ok_or_else(|| VaneError::Schema(format!("field '{}' not a scalar field", field)))?;
        // 校验 value 变体与 ScalarKind 匹配。
        let ok = matches!(
            (&value, kind),
            (crate::api::ScalarValue::Int(_), ScalarKind::Int)
                | (crate::api::ScalarValue::Float(_), ScalarKind::Float)
                | (crate::api::ScalarValue::Bool(_), ScalarKind::Bool)
                | (crate::api::ScalarValue::Keyword(_), ScalarKind::Keyword)
        );
        if !ok {
            return Err(VaneError::Schema(format!(
                "scalar value kind mismatch for field '{}'",
                field
            )));
        }
        let local = (self.next_docid - 1) as usize;
        let col = self
            .scalars
            .entry(field.to_string())
            .or_insert_with(|| (kind, vec![None; self.next_docid as usize]));
        let vec = &mut col.1;
        // 补齐到当前 docid 长度（该字段首条 set_scalar 之前的 docid 为 None）。
        if vec.len() <= local {
            vec.resize(local + 1, None);
        }
        vec[local] = Some(value);
        Ok(())
    }

    /// 查 schema 中标量字段的 ScalarKind（非 Scalar 返回 None）。
    fn schema_scalar_kind(&self, field: &str) -> Option<ScalarKind> {
        self.schema_snapshot.as_ref().and_then(|s| {
            s.fields.iter().find_map(|(n, d)| {
                if n == field {
                    if let crate::types::FieldDef::Scalar { kind } = d {
                        Some(*kind)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
        })
    }

    pub fn finalize(self) -> Result<SegmentMeta> {
        let seg_dir = format!("{}/seg_{}", self.segments_dir, self.ulid);

        // 写 vectors.bin v2（SPEC §6.2 + M2-08：magic|version=2|dim(4 LE)|payload）。
        // v2 头 12 字节（含 dim 字段，M2-07 open 期预存 dim 不需读 payload）。
        // v1 旧段双模读（load_vectors 按 version 分支）。doc_count=0 时仍写头（空段合规）。
        let vpath = format!("{}/vectors.bin", seg_dir);
        self.vfs.create(&vpath)?;
        let mut vbytes = Vec::with_capacity(12 + self.vectors.len() * 4);
        vbytes.extend_from_slice(crate::types::MAGIC);
        vbytes.extend_from_slice(&crate::types::VECTORS_FORMAT_V2.to_le_bytes());
        vbytes.extend_from_slice(&self.dim.to_le_bytes());
        for f in &self.vectors {
            vbytes.extend_from_slice(&f.to_le_bytes());
        }
        self.vfs.write_at(&vpath, &vbytes, 0)?;
        self.vfs.sync(&vpath)?;

        // 写 stored.bin（SPEC §6.2 + M2-08：v2 zstd 块压缩 / v1 裸 JSON）。
        // v2 布局：magic|version=2|raw_payload_len(4 LE)|zstd_block_len(4 LE)|zstd_block。
        //   raw_payload = v1 body（count+entries），zstd 压缩（zstd-encode feature 写）。
        // v1 布局：magic|version=1|count|{entries}（裸 JSON，wasm 写 / 旧段兼容）。
        // 双模读（decode_stored 按 version 分支），不做原地迁移（I-6 corpus 兼容）。
        let spath = format!("{}/stored.bin", seg_dir);
        self.vfs.create(&spath)?;
        // 先构造 raw_payload（v1 body：count + entries）。
        let mut raw_payload = Vec::new();
        raw_payload.extend_from_slice(&(self.stored.len() as u32).to_le_bytes());
        for entry in &self.stored {
            raw_payload.extend_from_slice(&entry.docid.to_le_bytes());
            // text 为 None 时落空串（text_len=0 表示无原文）。
            let text_bytes = entry.text.as_deref().unwrap_or("").as_bytes();
            raw_payload.extend_from_slice(&(text_bytes.len() as u32).to_le_bytes());
            raw_payload.extend_from_slice(text_bytes);
            let meta_bytes = entry.meta_json.as_bytes();
            raw_payload.extend_from_slice(&(meta_bytes.len() as u32).to_le_bytes());
            raw_payload.extend_from_slice(meta_bytes);
        }
        let sbytes = encode_stored(&raw_payload);
        self.vfs.write_at(&spath, &sbytes, 0)?;
        self.vfs.sync(&spath)?;

        // 写 idmap.bin（docid → external_id，SPEC §3.2 映射表持久化落点）。
        let ipath = format!("{}/idmap.bin", seg_dir);
        self.vfs.create(&ipath)?;
        let mut ibytes = Vec::new();
        ibytes.extend_from_slice(crate::types::MAGIC);
        ibytes.extend_from_slice(&crate::types::IDMAP_FORMAT_V1.to_le_bytes());
        ibytes.extend_from_slice(&(self.id_map.len() as u32).to_le_bytes());
        for (docid, eid) in &self.id_map {
            ibytes.extend_from_slice(&docid.to_le_bytes());
            ibytes.extend_from_slice(&(eid.len() as u32).to_le_bytes());
            ibytes.extend_from_slice(eid.as_bytes());
        }
        self.vfs.write_at(&ipath, &ibytes, 0)?;
        self.vfs.sync(&ipath)?;

        // 03-pre-filter：写 scalars.col 列式块（SPEC §6.2）。
        // 格式：magic(4) | version(4 LE) | num_fields(4 LE) |
        //   { name_len(4 LE) | name_bytes | kind(1) | count(4 LE) |
        //     for each docid 0..count: present(1 byte) | [value if present] }
        // count = 段内 doc_count（dense，per-docid 槽位）。present=0 表示该 docid 未设值。
        let colpath = format!("{}/scalars.col", seg_dir);
        self.vfs.create(&colpath)?;
        let mut scbytes = Vec::new();
        scbytes.extend_from_slice(crate::types::MAGIC);
        scbytes.extend_from_slice(&crate::types::SCALARS_FORMAT_V1.to_le_bytes());
        // 收集已 set_scalar 的字段，按字段名排序保证写盘确定性。
        let mut scalar_fields: Vec<String> = self.scalars.keys().cloned().collect();
        scalar_fields.sort();
        scbytes.extend_from_slice(&(scalar_fields.len() as u32).to_le_bytes());
        let doc_count = self.next_docid as u32;
        for name in &scalar_fields {
            let (kind, vals) = &self.scalars[name];
            let name_bytes = name.as_bytes();
            scbytes.extend_from_slice(&(name_bytes.len() as u32).to_le_bytes());
            scbytes.extend_from_slice(name_bytes);
            scbytes.push(scalar_kind_to_u8(*kind));
            scbytes.extend_from_slice(&doc_count.to_le_bytes());
            // 确保列长度对齐 doc_count（未补齐的尾部填 None）。
            for i in 0..doc_count as usize {
                let v = vals.get(i).and_then(|x| x.as_ref());
                match v {
                    None => scbytes.push(0u8),
                    Some(sv) => {
                        scbytes.push(1u8);
                        write_scalar_value(sv, &mut scbytes);
                    }
                }
            }
        }
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

/// stored.bin 读期单条记录（SPEC §6.2：原文 + JSON meta）。
struct StoredReadEntry {
    text: String,
    meta_json: String,
}

/// 段读期句柄（SPEC §6.2）。M2-07 冷启动懒加载：open 仅读 header + idmap，
/// vectors/stored 改 `OnceLock` 首次访问按需加载（SPEC v1.2 §13.1 元数据 open<1s + 首次查询<3s）。
/// inverted.bin 由 05-bm25 的 InvertedIndexReader 通过 segment_dir() 单独读取，本结构不加载倒排。
///
/// 不变量 I-1（段不可变）：懒加载只读缓存，不写回段文件。
/// 并发安全：`OnceLock::get_or_init` 原子保证多线程首查只加载一次（std::sync，无新依赖）。
/// §4 IDL 签名冻结：`vectors(&self)->&[f32]` / `dim(&self)->u32` / `stored_json` / `text` 签名不变，
/// OnceLock 提供内部可变性，消费方（HnswReader::search / brute_search / reindex / merge）零改动。
pub struct SegmentReader {
    meta: SegmentMeta,
    vfs: Arc<dyn Vfs>,
    segment_dir: String,
    // M2-07：vectors 首次访问按需加载（OnceLock）。open 不读 vectors.bin payload。
    vectors: OnceLock<Vec<f32>>,
    // M2-07：dim 首次访问按需计算（OnceLock）。
    // v2 头含 dim 字段（offset 8..12，M2-08 写入）；v1 回退 `(payload_len)/doc_count/4`。
    // v1 回退时复用已加载的 vectors（若 vectors() 先于 dim() 调用），否则触发 vectors 加载。
    dim: OnceLock<u32>,
    // M2-07 fix round 1（I-1）：open 期廉价头探测预存的 v2 dim（Some 仅当 vectors.bin v2 头）。
    // v1（None）时 dim 走 OnceLock 闭包从 payload 反推。open 期已校验 magic+version，
    // dim() 直接信任预存值，消除访问期 read_at 静默吞错（M-3）。
    v2_header_dim: Option<u32>,
    id_map: std::collections::HashMap<u64, String>,
    // M2-07：stored 首次访问按需加载（OnceLock）。open 不读 stored.bin payload（仅头探测）。
    // key 为段内局部 docid（0 起，与 id_map 同一 key 空间）。
    stored: OnceLock<std::collections::HashMap<u64, StoredReadEntry>>,
}

/// 模块级辅助：循环 read_at 直到 EOF，拼出完整文件字节。
fn read_all(vfs: &dyn Vfs, path: &str) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    let mut off = 0u64;
    loop {
        let n = vfs.read_at(path, &mut tmp, off)?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        off += n as u64;
    }
    Ok(buf)
}

impl SegmentReader {
    /// M2-07 冷启动懒加载：open 仅读 header + id_map + 廉价头探测（SPEC v1.2 §13.1 元数据 open<1s）。
    /// vectors/stored payload 改 OnceLock 首次访问按需加载；dim 首次访问按需计算（v2 头含 dim；v1 回退）。
    ///
    /// fix round 1（I-1）：open 期对 vectors.bin / stored.bin 做**廉价头探测**（各读 ≤12 字节），
    /// 恢复 M0/M1 对 bad magic / unsupported version 的 loud `Err` 失败（payload 仍懒加载，不读）。
    /// 避免 vectors.bin-only 损坏被 `unwrap_or_default()` 静默吞成空结果。
    pub fn open(vfs: &Arc<dyn Vfs>, segment_dir: &str) -> Result<Self> {
        // 读 header
        let hpath = format!("{}/header.bin", segment_dir);
        let hbuf = read_all(vfs.as_ref(), &hpath)?;
        let meta = header::decode_header(&hbuf)?;

        // 读 id_map（小文件，open 时读）
        let id_map = Self::load_id_map(vfs.as_ref(), segment_dir)?;

        // fix round 1（I-1）：vectors.bin 头探测（doc_count>0 时）。
        // 只读前 12 字节：v1 头 8 字节（magic+version），v2 头 12 字节（magic+version+dim）。
        // 校验 magic + version ∈ {1, 2}，不匹配 loud `Err`（同 M0/M1 旧 open 语义）。
        // payload（154MB）不在 open 读，仍 lazy。doc_count==0 时 vectors.bin 仅 8 字节空段头，跳过探测。
        let v2_header_dim = if meta.doc_count > 0 {
            let vpath = format!("{}/vectors.bin", segment_dir);
            let mut hdr = [0u8; 12];
            let n = vfs.read_at(&vpath, &mut hdr, 0)?;
            if n < 8 || &hdr[0..4] != crate::types::MAGIC {
                return Err(VaneError::Corrupt("vectors.bin bad magic".into()));
            }
            let version = u32::from_le_bytes(hdr[4..8].try_into().unwrap());
            match version {
                v if v == crate::types::VECTORS_FORMAT_V1 => None, // v1：dim 懒加载从 payload 反推
                v if v == crate::types::VECTORS_FORMAT_V2 => {
                    // v2：dim 字段在 offset 8..12（M2-08 写入，本模块读）。
                    if n < 12 {
                        return Err(VaneError::Corrupt(
                            "vectors.bin v2 header truncated (need 12 bytes)".into(),
                        ));
                    }
                    Some(u32::from_le_bytes(hdr[8..12].try_into().unwrap()))
                }
                _ => {
                    return Err(VaneError::Version(format!(
                        "vectors.bin unsupported format_version: {} (expected {} or {})",
                        version,
                        crate::types::VECTORS_FORMAT_V1,
                        crate::types::VECTORS_FORMAT_V2
                    )));
                }
            }
        } else {
            None
        };

        // fix round 1（I-1）：stored.bin 头探测。只读前 8 字节（magic+version）。
        // M2-08：stored v2（zstd 块）双模——open 期接受 version ∈ {1, 2}，payload 仍懒加载。
        {
            let spath = format!("{}/stored.bin", segment_dir);
            let mut shdr = [0u8; 8];
            let n = vfs.read_at(&spath, &mut shdr, 0)?;
            if n < 8 || &shdr[0..4] != crate::types::MAGIC {
                return Err(VaneError::Corrupt("stored.bin bad magic".into()));
            }
            let sver = u32::from_le_bytes(shdr[4..8].try_into().unwrap());
            if sver != crate::types::STORED_FORMAT_V1 && sver != crate::types::STORED_FORMAT_V2 {
                return Err(VaneError::Version(format!(
                    "stored.bin unsupported format_version: {} (expected {} or {})",
                    sver,
                    crate::types::STORED_FORMAT_V1,
                    crate::types::STORED_FORMAT_V2
                )));
            }
        }

        // M2-07：vectors / stored / dim 延迟到首次访问（OnceLock）。头已校验，payload 仍 lazy。
        Ok(Self {
            meta,
            vfs: vfs.clone(),
            segment_dir: segment_dir.to_string(),
            vectors: OnceLock::new(),
            dim: OnceLock::new(),
            v2_header_dim,
            id_map,
            stored: OnceLock::new(),
        })
    }

    /// M2-07：从 vectors.bin 加载并解码全部向量（OnceLock 闭包用）。
    /// 支持 v1（8 字节头 magic|version=1|payload）与 v2（12 字节头 magic|version=2|dim|payload）。
    /// v2 头由 M2-08 finalize 写入；本模块只读。payload 起始偏移随 version 变化。
    fn load_vectors(vfs: &dyn Vfs, segment_dir: &str) -> Result<Vec<f32>> {
        let vpath = format!("{}/vectors.bin", segment_dir);
        let vbuf = read_all(vfs, &vpath)?;
        if vbuf.len() < 8 || &vbuf[0..4] != crate::types::MAGIC {
            return Err(VaneError::Corrupt("vectors.bin bad magic".into()));
        }
        let version = u32::from_le_bytes(vbuf[4..8].try_into().unwrap());
        // M2-08：字面量 2 替换为 VECTORS_FORMAT_V2 常量。v1=VECTORS_FORMAT_V1，v2 含 dim 头。
        let payload_off = match version {
            v if v == crate::types::VECTORS_FORMAT_V1 => 8,
            v if v == crate::types::VECTORS_FORMAT_V2 => 12,
            _ => {
                return Err(VaneError::Version(format!(
                    "vectors.bin unsupported format_version: {} (expected {} or {})",
                    version,
                    crate::types::VECTORS_FORMAT_V1,
                    crate::types::VECTORS_FORMAT_V2
                )));
            }
        };
        if vbuf.len() < payload_off {
            return Err(VaneError::Corrupt("vectors.bin truncated".into()));
        }
        Ok(vbuf[payload_off..]
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
            .collect())
    }

    fn load_stored(
        vfs: &dyn Vfs,
        segment_dir: &str,
    ) -> Result<std::collections::HashMap<u64, StoredReadEntry>> {
        let spath = format!("{}/stored.bin", segment_dir);
        let buf = read_all(vfs, &spath)?;
        decode_stored(&buf)
    }

    fn load_id_map(
        vfs: &dyn Vfs,
        segment_dir: &str,
    ) -> Result<std::collections::HashMap<u64, String>> {
        let ipath = format!("{}/idmap.bin", segment_dir);
        let buf = read_all(vfs, &ipath)?;
        decode_kv_map(&buf, "idmap")
    }

    pub fn meta(&self) -> &SegmentMeta {
        &self.meta
    }
    /// 返回段内全部向量连续排布（vectors.bin payload，SPEC §6.2）。
    /// M2-07：首次调用触发 OnceLock 加载（get_or_init），后续调用幂等返回同一 &[f32]。
    /// 多线程并发首查由 OnceLock 原子保证只加载一次。签名不变（§4 IDL 冻结）。
    pub fn vectors(&self) -> &[f32] {
        self.vectors
            .get_or_init(|| {
                Self::load_vectors(self.vfs.as_ref(), &self.segment_dir).unwrap_or_default()
            })
            .as_slice()
    }
    /// 返回向量维度。M2-07：首次调用按需计算（OnceLock）。
    /// - v2 头（version==2）：用 open 期预存的 `v2_header_dim`（已校验），不触发 vectors 加载、不再 read_at。
    /// - v1 回退（version==1）：dim = `vectors.len() / doc_count`；若 vectors 已加载则复用，
    ///   否则触发 vectors 加载（v1 无 dim 字段，必须从 payload 长度反推）。
    ///
    /// fix round 1（M-3）：open 期已校验 vectors.bin 头，dim() 信任预存值，消除访问期 read_at 静默吞错。
    /// doc_count==0 时返回 0。签名不变（§4 IDL 冻结）。
    pub fn dim(&self) -> u32 {
        *self.dim.get_or_init(|| {
            if self.meta.doc_count == 0 {
                return 0;
            }
            // v2：open 期预存 dim（已校验 magic+version），直接用，不读 vectors payload。
            if let Some(d) = self.v2_header_dim {
                return d;
            }
            // v1 回退：dim = vectors.len() / doc_count。
            // 若 vectors 已加载（vectors() 先于 dim() 调用），复用；否则触发加载。
            if let Some(v) = self.vectors.get() {
                return (v.len() / self.meta.doc_count as usize) as u32;
            }
            match Self::load_vectors(self.vfs.as_ref(), &self.segment_dir) {
                Ok(v) => {
                    let dim = (v.len() / self.meta.doc_count as usize) as u32;
                    let _ = self.vectors.set(v);
                    dim
                }
                // open 期已校验头，此处 Err 仅在段文件被外部篡改时发生（违反 I-1 不可变）；
                // 签名非 Result 无法上抛，返回 0 作兜底（消费方按空段处理）。
                Err(_) => 0,
            }
        })
    }
    pub fn doc_count(&self) -> u32 {
        self.meta.doc_count
    }
    pub fn external_id(&self, docid: u64) -> Option<&str> {
        self.id_map.get(&docid).map(|s| s.as_str())
    }

    /// 反查：external_id → 段内局部 docid（0 起）。
    /// 02-tombstone-merge delete 用（定位待删文档在段内的 local docid）。
    /// 属 M1 扩展，非 M0 冻结 API 破坏。
    pub fn local_docid_by_external(&self, external_id: &str) -> Option<u64> {
        self.id_map
            .iter()
            .find(|(_, eid)| eid.as_str() == external_id)
            .map(|(local, _)| *local)
    }
    /// 读取某文档的 stored.bin JSON（回填 Hit.fields，SPEC §6.2 stored.bin）。
    /// local_docid 为段内局部 docid（0 起，与 external_id 同一 key 空间）。
    /// M2-07：首次调用触发 stored OnceLock 加载；后续幂等。签名不变。
    /// 语义不变：仍返回 meta JSON（原文经 text() 读出）。
    pub fn stored_json(&self, local_docid: u64) -> Option<&str> {
        let map = self.stored.get_or_init(|| {
            Self::load_stored(self.vfs.as_ref(), &self.segment_dir).unwrap_or_default()
        });
        map.get(&local_docid).map(|e| e.meta_json.as_str())
    }

    /// 读取某文档的原文（SPEC §6.2 stored.bin 含原文）。
    /// local_docid 为段内局部 docid（0 起，与 external_id 同一 key 空间）。
    /// 无原文（text_len=0，写期未调 set_text）返回 Some("")；docid 不存在返回 None。
    /// M2-07：首次调用触发 stored OnceLock 加载；后续幂等。签名不变。
    /// 06-userdict-reindex 经此读原文用新分词器重建倒排；02-tombstone-merge 经此读原文写入新段。
    pub fn text(&self, local_docid: u64) -> Option<&str> {
        let map = self.stored.get_or_init(|| {
            Self::load_stored(self.vfs.as_ref(), &self.segment_dir).unwrap_or_default()
        });
        map.get(&local_docid).map(|e| e.text.as_str())
    }
    pub fn segment_dir(&self) -> &str {
        &self.segment_dir
    }
    pub fn vfs(&self) -> &Arc<dyn Vfs> {
        &self.vfs
    }
}

/// 解码 stored.bin / idmap.bin 共享的 KV 布局：
/// magic(4) | version(4 LE) | count(4 LE) | {docid(8 LE)|len(4 LE)|bytes}...
/// FA2：version 统一 LE；顺手加 version 校验（FF4 严格化的可接受轻量部分）。
fn decode_kv_map(buf: &[u8], label: &str) -> Result<std::collections::HashMap<u64, String>> {
    if buf.len() < 12 {
        return Ok(std::collections::HashMap::new());
    }
    if &buf[0..4] != crate::types::MAGIC {
        return Err(VaneError::Corrupt(format!("{} bad magic", label)));
    }
    let version = u32::from_le_bytes(buf[4..8].try_into().unwrap());
    if version != crate::types::IDMAP_FORMAT_V1 {
        return Err(VaneError::Version(format!(
            "{} unsupported format_version: {} (expected {})",
            label,
            version,
            crate::types::IDMAP_FORMAT_V1
        )));
    }
    let count = u32::from_le_bytes(buf[8..12].try_into().unwrap()) as usize;
    let mut pos = 12;
    let mut map = std::collections::HashMap::with_capacity(count);
    for _ in 0..count {
        if pos + 12 > buf.len() {
            break;
        }
        let docid = u64::from_le_bytes(buf[pos..pos + 8].try_into().unwrap());
        pos += 8;
        let len = u32::from_le_bytes(buf[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 4;
        if pos + len > buf.len() {
            return Err(VaneError::Corrupt(format!("{} entry truncated", label)));
        }
        let s = std::str::from_utf8(&buf[pos..pos + len])
            .map_err(|e| VaneError::Corrupt(format!("{} utf8: {}", label, e)))?
            .to_string();
        pos += len;
        map.insert(docid, s);
    }
    Ok(map)
}

/// 编码 stored.bin（SPEC §6.2 + M2-08）。
/// - zstd-encode feature：尝试 zstd 压缩 → 成功写 v2
///   `magic|version=2|raw_payload_len|zstd_block_len|zstd_block`；**失败回退写 v1**
///   `magic|version=1|raw_payload`（I-1：压缩失败降级 v1 而非写不可读 v2，文件始终可读）。
/// - 无 zstd-encode（wasm）：写 v1 `magic|version=1|raw_payload`（raw_payload = count+entries）。
///
/// raw_payload 在两模式下相同（v1 body），v2 仅外包 zstd 压缩 + 长度前缀。
fn encode_stored(raw_payload: &[u8]) -> Vec<u8> {
    #[cfg(feature = "zstd-encode")]
    {
        // zstd 压缩 raw_payload（level 3 默认，stored 小无需高压缩）。
        // I-1 fix：压缩失败回退写 v1（version=1 + raw_payload），而非写 version=2 + 未压缩
        // raw_payload 作 zstd_block（后者 ruzstd 解压失败 → 不可读 v2 文件）。
        match zstd::encode_all(raw_payload, 3) {
            Ok(zstd_block) => {
                let mut out = Vec::with_capacity(16 + zstd_block.len());
                out.extend_from_slice(crate::types::MAGIC);
                out.extend_from_slice(&crate::types::STORED_FORMAT_V2.to_le_bytes());
                out.extend_from_slice(&(raw_payload.len() as u32).to_le_bytes());
                out.extend_from_slice(&(zstd_block.len() as u32).to_le_bytes());
                out.extend_from_slice(&zstd_block);
                out
            }
            Err(_) => {
                // 压缩失败降级 v1：magic|version=1|raw_payload（decode_stored v1 路径可读）。
                let mut out = Vec::with_capacity(8 + raw_payload.len());
                out.extend_from_slice(crate::types::MAGIC);
                out.extend_from_slice(&crate::types::STORED_FORMAT_V1.to_le_bytes());
                out.extend_from_slice(raw_payload);
                out
            }
        }
    }
    #[cfg(not(feature = "zstd-encode"))]
    {
        // v1 裸 JSON：magic|version=1|raw_payload（wasm 写，浏览器 stored 小不压缩）。
        let mut out = Vec::with_capacity(8 + raw_payload.len());
        out.extend_from_slice(crate::types::MAGIC);
        out.extend_from_slice(&crate::types::STORED_FORMAT_V1.to_le_bytes());
        out.extend_from_slice(raw_payload);
        out
    }
}

/// 解码 stored.bin（SPEC §6.2 + M2-08：v1/v2 双模读取，不做原地迁移）。
/// - v1：magic|version=1|count(4 LE)|{entries}（裸 JSON，M0/M1 产物 + wasm 写）。
/// - v2：magic|version=2|raw_payload_len(4 LE)|zstd_block_len(4 LE)|zstd_block。
///   v2 解压 zstd_block → raw_payload（= v1 body count+entries）→ 复用 v1 entries 解析。
///
/// 读期 text 始终为 String（写期 Option 在 finalize 落空串），空串表示无原文。
fn decode_stored(buf: &[u8]) -> Result<std::collections::HashMap<u64, StoredReadEntry>> {
    if buf.len() < 8 {
        return Ok(std::collections::HashMap::new());
    }
    if &buf[0..4] != crate::types::MAGIC {
        return Err(VaneError::Corrupt("stored bad magic".into()));
    }
    let version = u32::from_le_bytes(buf[4..8].try_into().unwrap());
    match version {
        v if v == crate::types::STORED_FORMAT_V1 => {
            if buf.len() < 12 {
                return Ok(std::collections::HashMap::new());
            }
            let count = u32::from_le_bytes(buf[8..12].try_into().unwrap()) as usize;
            parse_stored_entries(count, &buf[12..])
        }
        v if v == crate::types::STORED_FORMAT_V2 => decode_stored_v2(buf),
        _ => Err(VaneError::Version(format!(
            "stored unsupported format_version: {} (expected {} or {})",
            version,
            crate::types::STORED_FORMAT_V1,
            crate::types::STORED_FORMAT_V2
        ))),
    }
}

/// 解码 stored.bin v2（SPEC §6.2 + M2-08：zstd 块压缩）。
/// 布局：magic|version=2|raw_payload_len(4 LE)|zstd_block_len(4 LE)|zstd_block。
/// 解压 zstd_block → raw_payload（= v1 body count+entries）→ 复用 entries 解析。
/// ruzstd 纯 Rust 解码器，wasm32 也可用（vane-wasm 启 zstd-decode 读 v2）。
#[cfg(feature = "zstd-decode")]
fn decode_stored_v2(buf: &[u8]) -> Result<std::collections::HashMap<u64, StoredReadEntry>> {
    use std::io::Read;
    if buf.len() < 16 {
        return Err(VaneError::Corrupt(
            "stored v2 header truncated (need 16 bytes)".into(),
        ));
    }
    let raw_len = u32::from_le_bytes(buf[8..12].try_into().unwrap()) as usize;
    let zstd_len = u32::from_le_bytes(buf[12..16].try_into().unwrap()) as usize;
    if buf.len() < 16 + zstd_len {
        return Err(VaneError::Corrupt("stored v2 zstd_block truncated".into()));
    }
    let mut decoder = ruzstd::streaming_decoder::StreamingDecoder::new(&buf[16..16 + zstd_len])
        .map_err(|e| VaneError::Corrupt(format!("stored v2 zstd decompress failed: {}", e)))?;
    let mut owned = Vec::with_capacity(raw_len);
    decoder
        .read_to_end(&mut owned)
        .map_err(|e| VaneError::Corrupt(format!("stored v2 zstd read failed: {}", e)))?;
    if owned.len() != raw_len {
        return Err(VaneError::Corrupt(format!(
            "stored v2 raw_payload_len mismatch: header {} actual {}",
            raw_len,
            owned.len()
        )));
    }
    if owned.len() < 4 {
        return Err(VaneError::Corrupt("stored v2 raw_payload too short".into()));
    }
    let count = u32::from_le_bytes(owned[0..4].try_into().unwrap()) as usize;
    parse_stored_entries(count, &owned[4..])
}

/// 无 zstd-decode feature 时 v2 stored 无法解码（wasm 无 zstd-encode 但有 zstd-decode，
/// 此分支仅在 vane-core 裸用且未启 zstd-decode 时命中——上游应启 zstd-decode 读 v2）。
#[cfg(not(feature = "zstd-decode"))]
fn decode_stored_v2(_buf: &[u8]) -> Result<std::collections::HashMap<u64, StoredReadEntry>> {
    Err(VaneError::Unsupported)
}

/// 解析 stored entries（v1/v2 共享 body 布局）：
/// {docid(8 LE) | text_len(4 LE) | text_bytes | meta_json_len(4 LE) | meta_json_bytes}...
fn parse_stored_entries(
    count: usize,
    body: &[u8],
) -> Result<std::collections::HashMap<u64, StoredReadEntry>> {
    let mut pos = 0;
    let mut map = std::collections::HashMap::with_capacity(count);
    for _ in 0..count {
        if pos + 8 > body.len() {
            return Err(VaneError::Corrupt("stored entry docid truncated".into()));
        }
        let docid = u64::from_le_bytes(body[pos..pos + 8].try_into().unwrap());
        pos += 8;
        // text
        if pos + 4 > body.len() {
            return Err(VaneError::Corrupt("stored entry text_len truncated".into()));
        }
        let text_len = u32::from_le_bytes(body[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 4;
        if pos + text_len > body.len() {
            return Err(VaneError::Corrupt(
                "stored entry text_bytes truncated".into(),
            ));
        }
        let text = std::str::from_utf8(&body[pos..pos + text_len])
            .map_err(|e| VaneError::Corrupt(format!("stored text utf8: {}", e)))?
            .to_string();
        pos += text_len;
        // meta_json
        if pos + 4 > body.len() {
            return Err(VaneError::Corrupt("stored entry meta_len truncated".into()));
        }
        let meta_len = u32::from_le_bytes(body[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 4;
        if pos + meta_len > body.len() {
            return Err(VaneError::Corrupt(
                "stored entry meta_bytes truncated".into(),
            ));
        }
        let meta_json = std::str::from_utf8(&body[pos..pos + meta_len])
            .map_err(|e| VaneError::Corrupt(format!("stored meta utf8: {}", e)))?
            .to_string();
        pos += meta_len;
        map.insert(docid, StoredReadEntry { text, meta_json });
    }
    Ok(map)
}

// ---------------------------------------------------------------------------
// 03-pre-filter：scalars.col 读期（ScalarReader）+ 编解码辅助
// ---------------------------------------------------------------------------

/// 标量列式块读期数据（SPEC §6.2 scalars.col）。
/// 偏离 README 契约：Vec 元素为 Option 以表达「该 docid 未设值」（filter 不命中）。
/// 属新增类型，非 M0 冻结签名变更。
pub enum ScalarColumn {
    Int(Vec<Option<i64>>),
    Float(Vec<Option<f64>>),
    Bool(Vec<Option<bool>>),
    Keyword(Vec<Option<String>>),
}

/// 标量列式块读期句柄（scalars.col，SPEC §6.2）。
pub struct ScalarReader {
    columns: std::collections::HashMap<String, ScalarColumn>,
}

impl ScalarReader {
    /// 从段目录加载 scalars.col。无 scalars.col 文件或空段返回空 reader（无列）。
    pub fn open(vfs: &Arc<dyn Vfs>, segment_dir: &str) -> Result<Self> {
        let path = format!("{}/scalars.col", segment_dir);
        let buf = match read_all_optional(vfs.as_ref(), &path) {
            Ok(Some(b)) => b,
            Ok(None) => {
                return Ok(Self {
                    columns: std::collections::HashMap::new(),
                })
            }
            Err(e) => return Err(e),
        };
        decode_scalars(&buf)
    }

    /// 读取某文档某字段的标量值。local_docid 为段内局部 docid（0 起）。
    /// 字段不存在或 docid 越界或该 docid 未设值返回 None。
    pub fn get(&self, field: &str, local_docid: u32) -> Option<crate::api::ScalarValue> {
        let col = self.columns.get(field)?;
        let i = local_docid as usize;
        match col {
            ScalarColumn::Int(v) => v.get(i).and_then(|x| x.map(crate::api::ScalarValue::Int)),
            ScalarColumn::Float(v) => v.get(i).and_then(|x| x.map(crate::api::ScalarValue::Float)),
            ScalarColumn::Bool(v) => v.get(i).and_then(|x| x.map(crate::api::ScalarValue::Bool)),
            ScalarColumn::Keyword(v) => v.get(i).and_then(|x| {
                x.as_ref()
                    .map(|s| crate::api::ScalarValue::Keyword(s.clone()))
            }),
        }
    }

    /// 字段是否存在（有列）。
    pub fn has_field(&self, field: &str) -> bool {
        self.columns.contains_key(field)
    }
}

/// read_all 但文件不存在时返回 Ok(None)（scalars.col 可缺失，兼容 M0 空段）。
fn read_all_optional(vfs: &dyn Vfs, path: &str) -> Result<Option<Vec<u8>>> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    let mut off = 0u64;
    let mut started = false;
    loop {
        let n = match vfs.read_at(path, &mut tmp, off) {
            Ok(n) => n,
            Err(crate::types::VaneError::Io(_)) if !started => return Ok(None),
            Err(e) => return Err(e),
        };
        if n == 0 {
            break;
        }
        started = true;
        buf.extend_from_slice(&tmp[..n]);
        off += n as u64;
    }
    if !started {
        // 文件存在但空 / 首读返回 0：视为不存在。
        return Ok(None);
    }
    Ok(Some(buf))
}

/// 解码 scalars.col。
fn decode_scalars(buf: &[u8]) -> Result<ScalarReader> {
    if buf.len() < 12 {
        return Ok(ScalarReader {
            columns: std::collections::HashMap::new(),
        });
    }
    if &buf[0..4] != crate::types::MAGIC {
        return Err(VaneError::Corrupt("scalars.col bad magic".into()));
    }
    let version = u32::from_le_bytes(buf[4..8].try_into().unwrap());
    if version != crate::types::SCALARS_FORMAT_V1 {
        return Err(VaneError::Version(format!(
            "scalars.col unsupported format_version: {} (expected {})",
            version,
            crate::types::SCALARS_FORMAT_V1
        )));
    }
    let num_fields = u32::from_le_bytes(buf[8..12].try_into().unwrap()) as usize;
    let mut pos = 12;
    let mut columns = std::collections::HashMap::with_capacity(num_fields);
    for _ in 0..num_fields {
        if pos + 4 > buf.len() {
            return Err(VaneError::Corrupt("scalars.col name_len truncated".into()));
        }
        let name_len = u32::from_le_bytes(buf[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 4;
        if pos + name_len > buf.len() {
            return Err(VaneError::Corrupt("scalars.col name truncated".into()));
        }
        let name = std::str::from_utf8(&buf[pos..pos + name_len])
            .map_err(|e| VaneError::Corrupt(format!("scalars.col name utf8: {}", e)))?
            .to_string();
        pos += name_len;
        if pos + 5 > buf.len() {
            return Err(VaneError::Corrupt(
                "scalars.col kind/count truncated".into(),
            ));
        }
        let kind = scalar_kind_from_u8(buf[pos])?;
        pos += 1;
        let count = u32::from_le_bytes(buf[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 4;
        let col = match kind {
            ScalarKind::Int => {
                let mut v: Vec<Option<i64>> = Vec::with_capacity(count);
                for _ in 0..count {
                    let (present, val, np) = read_opt_i64(buf, pos)?;
                    pos = np;
                    v.push(if present { Some(val) } else { None });
                }
                ScalarColumn::Int(v)
            }
            ScalarKind::Float => {
                let mut v: Vec<Option<f64>> = Vec::with_capacity(count);
                for _ in 0..count {
                    let (present, val, np) = read_opt_f64(buf, pos)?;
                    pos = np;
                    v.push(if present { Some(val) } else { None });
                }
                ScalarColumn::Float(v)
            }
            ScalarKind::Bool => {
                let mut v: Vec<Option<bool>> = Vec::with_capacity(count);
                for _ in 0..count {
                    if pos + 1 > buf.len() {
                        return Err(VaneError::Corrupt("scalars.col bool truncated".into()));
                    }
                    let present = buf[pos];
                    pos += 1;
                    if present == 1 {
                        if pos + 1 > buf.len() {
                            return Err(VaneError::Corrupt(
                                "scalars.col bool value truncated".into(),
                            ));
                        }
                        v.push(Some(buf[pos] != 0));
                        pos += 1;
                    } else {
                        v.push(None);
                    }
                }
                ScalarColumn::Bool(v)
            }
            ScalarKind::Keyword => {
                let mut v: Vec<Option<String>> = Vec::with_capacity(count);
                for _ in 0..count {
                    if pos + 1 > buf.len() {
                        return Err(VaneError::Corrupt(
                            "scalars.col keyword present truncated".into(),
                        ));
                    }
                    let present = buf[pos];
                    pos += 1;
                    if present == 1 {
                        if pos + 4 > buf.len() {
                            return Err(VaneError::Corrupt(
                                "scalars.col keyword len truncated".into(),
                            ));
                        }
                        let len =
                            u32::from_le_bytes(buf[pos..pos + 4].try_into().unwrap()) as usize;
                        pos += 4;
                        if pos + len > buf.len() {
                            return Err(VaneError::Corrupt(
                                "scalars.col keyword bytes truncated".into(),
                            ));
                        }
                        let s = std::str::from_utf8(&buf[pos..pos + len])
                            .map_err(|e| {
                                VaneError::Corrupt(format!("scalars.col keyword utf8: {}", e))
                            })?
                            .to_string();
                        pos += len;
                        v.push(Some(s));
                    } else {
                        v.push(None);
                    }
                }
                ScalarColumn::Keyword(v)
            }
        };
        columns.insert(name, col);
    }
    Ok(ScalarReader { columns })
}

fn read_opt_i64(buf: &[u8], pos: usize) -> Result<(bool, i64, usize)> {
    if pos + 1 > buf.len() {
        return Err(VaneError::Corrupt(
            "scalars.col int present truncated".into(),
        ));
    }
    let present = buf[pos];
    let pos = pos + 1;
    if present == 1 {
        if pos + 8 > buf.len() {
            return Err(VaneError::Corrupt("scalars.col int value truncated".into()));
        }
        let val = i64::from_le_bytes(buf[pos..pos + 8].try_into().unwrap());
        Ok((true, val, pos + 8))
    } else {
        Ok((false, 0, pos))
    }
}

fn read_opt_f64(buf: &[u8], pos: usize) -> Result<(bool, f64, usize)> {
    if pos + 1 > buf.len() {
        return Err(VaneError::Corrupt(
            "scalars.col float present truncated".into(),
        ));
    }
    let present = buf[pos];
    let pos = pos + 1;
    if present == 1 {
        if pos + 8 > buf.len() {
            return Err(VaneError::Corrupt(
                "scalars.col float value truncated".into(),
            ));
        }
        let val = f64::from_le_bytes(buf[pos..pos + 8].try_into().unwrap());
        Ok((true, val, pos + 8))
    } else {
        Ok((false, 0.0, pos))
    }
}

fn scalar_kind_to_u8(k: ScalarKind) -> u8 {
    match k {
        ScalarKind::Int => 0,
        ScalarKind::Float => 1,
        ScalarKind::Bool => 2,
        ScalarKind::Keyword => 3,
    }
}

fn scalar_kind_from_u8(b: u8) -> Result<ScalarKind> {
    match b {
        0 => Ok(ScalarKind::Int),
        1 => Ok(ScalarKind::Float),
        2 => Ok(ScalarKind::Bool),
        3 => Ok(ScalarKind::Keyword),
        _ => Err(VaneError::Corrupt(format!(
            "scalars.col unknown scalar kind byte: {}",
            b
        ))),
    }
}

/// 写单个 ScalarValue 到 buf（不带 present 标记，调用方先写 present=1）。
fn write_scalar_value(sv: &crate::api::ScalarValue, out: &mut Vec<u8>) {
    match sv {
        crate::api::ScalarValue::Int(i) => out.extend_from_slice(&i.to_le_bytes()),
        crate::api::ScalarValue::Float(f) => out.extend_from_slice(&f.to_le_bytes()),
        crate::api::ScalarValue::Bool(b) => out.push(if *b { 1 } else { 0 }),
        crate::api::ScalarValue::Keyword(s) => {
            let b = s.as_bytes();
            out.extend_from_slice(&(b.len() as u32).to_le_bytes());
            out.extend_from_slice(b);
        }
    }
}
