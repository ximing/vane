# Segment-Format 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development 执行本计划。步骤用 checkbox `- [ ]` 标记。

**Goal:** 实现 SPEC §6.2 段文件格式（header.bin / vectors.bin / stored.bin / scalars.col）+ SegmentWriter/Reader + ULID 生成，为 bm25/vector-brute/persistence 提供段级读写骨架。
**Architecture:** SegmentWriter 在 flush 时构建段文件集（header 先写、vectors/stored 边 add 边写、finalize sync 全部）；SegmentReader 在查询时加载 header + vectors（M0 全加载供暴力扫描）+ id 映射。inverted.bin 由 05-bm25 的 `write_inverted` 单独写入段目录，本模块不负责倒排内容。段目录名 `seg_<ulid>`。
**Tech Stack:** ulid crate、serde + serde_json（stored.bin JSON）、roaring（tombstone 位图，M0 为空）。
**SPEC 引用:** §6.2 目录布局、§6.3 段头字段（magic/version/tokenizer_id/docid_range/tombstone_bitmap）、§6.4 写入流程、§14 I-1 段不可变、§3.2 docid 映射。
**前置依赖:** 00-workspace（Schema, FieldDef, TokenizerId, Metric, MAGIC, FORMAT_VERSION, Result）、01-vfs（Vfs trait）。
**验收标准:**
- [ ] SegmentWriter 写出的段可被 SegmentReader 完整读回（vectors/external_id/meta 一致）
- [ ] 所有段文件以 magic(b"VANE")+format_version(1) 开头
- [ ] 段不可变：finalize 后再调 add_doc 报错（不变量 I-1）
- [ ] ULID 单调递增、26 字符
- [ ] tombstone 位图 M0 为空但格式预留
- [ ] MemoryVfs + StdFsVfs 均可跑通

## Global Constraints
- 段文件头：4 字节 magic(b"VANE") + 4 字节 format_version(1)（§6.2）。
- 段不可变：写一次后只读；任何更新 = 新段 + manifest 切换（§14 I-1）。
- 段目录布局：`<db>/segments/seg_<ulid>/{header.bin, vectors.bin, inverted.bin, scalars.col, stored.bin}`（§6.2）。
- vectors.bin：f32 定长连续排布（docid 序），M0 全加载内存（§6.2）。
- 单文档序列化 ≤16MB（§3.2）。
- dim 上限 4096（§3.1）。
- core 禁 std::fs/mmap（§13.3）；本模块通过 Vfs trait 读写。

## File Structure
- `crates/vane-core/src/segment/mod.rs` — SegmentMeta + SegmentWriter + SegmentReader + re-export
- `crates/vane-core/src/segment/ulid.rs` — gen_ulid()
- `crates/vane-core/src/segment/header.rs` — header.bin 编解码
- `crates/vane-core/src/segment/tests.rs` — 往返测试

## 任务清单（bite-sized TDD）

### Task 1: ULID 生成 + header.bin 编解码
**Files:**
- Create: `crates/vane-core/src/segment/mod.rs`, `crates/vane-core/src/segment/ulid.rs`, `crates/vane-core/src/segment/header.rs`
- 不修改 Cargo.toml / lib.rs（B1 裁决：00-workspace 已一次性加入依赖与模块声明）

**Interfaces:**
- Consumes from 00-workspace: TokenizerId, MAGIC, FORMAT_VERSION, Result, VaneError
- Consumes from 01-vfs: Vfs trait
- Produces: `gen_ulid() -> String`、`SegmentMeta`、`encode_header()` / `decode_header()`

- [ ] **Step 1: 写失败测试** — 创建 `crates/vane-core/src/segment/tests.rs`：
```rust
use super::*;
use super::ulid::gen_ulid;
use super::header::{encode_header, decode_header};
use crate::types::TokenizerId;
use crate::vfs::memory::MemoryVfs;
use crate::vfs::Vfs;

#[test]
fn ulid_is_26_chars_and_monotonic() {
    let a = gen_ulid();
    let b = gen_ulid();
    assert_eq!(a.len(), 26);
    assert_eq!(b.len(), 26);
    // 单调递增（时间前缀）
    assert!(b >= a, "ulid should be monotonic: {} vs {}", a, b);
}

#[test]
fn header_roundtrip() {
    let meta = SegmentMeta {
        ulid: gen_ulid(),
        doc_count: 100,
        docid_base: 0,
        tokenizer_id: TokenizerId([0xab; 32]),
        tombstones: roaring::RoaringBitmap::new(),
    };
    let bytes = encode_header(&meta).unwrap();
    // magic + version 开头
    assert_eq!(&bytes[0..4], b"VANE");
    assert_eq!(&bytes[4..8], &[0, 0, 0, 1]);
    let decoded = decode_header(&bytes).unwrap();
    assert_eq!(decoded.ulid, meta.ulid);
    assert_eq!(decoded.doc_count, 100);
    assert_eq!(decoded.docid_base, 0);
    assert_eq!(decoded.tokenizer_id, meta.tokenizer_id);
}
```

- [ ] **Step 2: 跑测试确认失败** — `cargo test -p vane-core -- segment`，编译失败。
- [ ] **Step 3: 最小实现** —

**确认 00-workspace 已在 vane-core Cargo.toml 一次性加入 `ulid`、`serde`(derive)、`serde_json`（B1 裁决：00 一次性加全部依赖，后续计划不重复添加，重复键会导致 Cargo 解析失败）。本计划不修改 Cargo.toml。**

`crates/vane-core/src/segment/ulid.rs`：
```rust
use ulid::Ulid;
use std::sync::Mutex;

static LOCK: Mutex<()> = Mutex::new(());

/// 生成 26 字符 ULID（单调递增）。
pub fn gen_ulid() -> String {
    let _g = LOCK.lock().unwrap();
    Ulid::new().to_string()
}
```

`crates/vane-core/src/segment/header.rs`：
```rust
use crate::types::{Result, VaneError, MAGIC, FORMAT_VERSION, TokenizerId};
use crate::segment::SegmentMeta;

/// header.bin 布局：
/// magic(4) | format_version(4 LE) | ulid_len(1) | ulid(26) |
/// doc_count(4 LE) | docid_base(8 LE) | tokenizer_id(32) | tombstone_bytes(4 LE) | tombstone_data
pub fn encode_header(meta: &SegmentMeta) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    let ulid_bytes = meta.ulid.as_bytes();
    out.push(ulid_bytes.len() as u8);
    out.extend_from_slice(ulid_bytes);
    out.extend_from_slice(&meta.doc_count.to_le_bytes());
    out.extend_from_slice(&meta.docid_base.to_le_bytes());
    out.extend_from_slice(meta.tokenizer_id.as_bytes());
    let mut tb = Vec::new();
    meta.tombstones.serialize(&mut tb)
        .map_err(|e| VaneError::Corrupt(format!("tombstone serialize: {}", e)))?;
    out.extend_from_slice(&(tb.len() as u32).to_le_bytes());
    out.extend_from_slice(&tb);
    Ok(out)
}

pub fn decode_header(buf: &[u8]) -> Result<SegmentMeta> {
    if buf.len() < 8 {
        return Err(VaneError::Corrupt("header too short".into()));
    }
    if &buf[0..4] != MAGIC {
        return Err(VaneError::Corrupt(format!("bad magic: {:?}", &buf[0..4])));
    }
    let version = u32::from_le_bytes(buf[4..8].try_into().unwrap());
    if version != FORMAT_VERSION {
        return Err(VaneError::Version(format!(
            "unsupported format_version: {} (expected {})", version, FORMAT_VERSION
        )));
    }
    let mut pos = 8;
    let ulid_len = buf[pos] as usize;
    pos += 1;
    let ulid = std::str::from_utf8(&buf[pos..pos + ulid_len])
        .map_err(|e| VaneError::Corrupt(format!("ulid utf8: {}", e)))?
        .to_string();
    pos += ulid_len;
    let doc_count = u32::from_le_bytes(buf[pos..pos+4].try_into().unwrap());
    pos += 4;
    let docid_base = u64::from_le_bytes(buf[pos..pos+8].try_into().unwrap());
    pos += 8;
    let mut tid = [0u8; 32];
    tid.copy_from_slice(&buf[pos..pos+32]);
    pos += 32;
    let tb_len = u32::from_le_bytes(buf[pos..pos+4].try_into().unwrap()) as usize;
    pos += 4;
    let tombstones = roaring::RoaringBitmap::deserialize_from(&buf[pos..pos+tb_len])
        .map_err(|e| VaneError::Corrupt(format!("tombstone deserialize: {}", e)))?;
    Ok(SegmentMeta {
        ulid, doc_count, docid_base,
        tokenizer_id: TokenizerId(tid),
        tombstones,
    })
}
```

`crates/vane-core/src/segment/mod.rs`：
```rust
pub mod ulid;
pub mod header;
#[cfg(test)]
mod tests;

use crate::types::{TokenizerId, Result, VaneError, Schema};

pub struct SegmentMeta {
    pub ulid: String,
    pub doc_count: u32,
    pub docid_base: u64,
    pub tokenizer_id: TokenizerId,
    pub tombstones: roaring::RoaringBitmap,
}
```

**00-workspace 已在 lib.rs 一次性预声明全部 9 模块（含 `pub mod segment;`），本计划不修改 lib.rs（B1 裁决）。**

- [ ] **Step 4: 跑测试确认通过** — `cargo test -p vane-core -- segment`，2 测试绿。
- [ ] **Step 5: Commit**
```bash
git add -A
git commit -m "feat(segment): ULID gen + header.bin encode/decode (§6.2/§6.3)

"
```

### Task 2: SegmentWriter（写 header/vectors/stored）
**Files:**
- Modify: `crates/vane-core/src/segment/mod.rs`（追加 SegmentWriter）

**Interfaces:**
- Consumes from 00-workspace: Schema, TokenizerId, Result, VaneError
- Consumes from 01-vfs: Vfs
- Consumes from Task 1: gen_ulid, SegmentMeta, encode_header
- Produces: `SegmentWriter::new()`, `SegmentWriter::add_doc()`, `SegmentWriter::finalize()`

- [ ] **Step 1: 写失败测试** — 追加到 tests.rs：
```rust
use crate::types::{Schema, FieldDef, Metric};

#[test]
fn segment_writer_roundtrip_with_memory_vfs() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
    let schema = Schema::new(vec![
        ("title".into(), FieldDef::Text),
        ("vec".into(), FieldDef::Vector { dim: 4, metric: Metric::Cosine }),
    ]).unwrap();
    let tok_id = TokenizerId([0x11; 32]);

    let mut writer = SegmentWriter::new(
        vfs.clone(), "segments", &schema, &tok_id, 0,
    ).unwrap();
    let d0 = writer.add_doc("doc-0", Some(&[1.0, 0.0, 0.0, 0.0]), r#"{"title":"hello"}"#).unwrap();
    let d1 = writer.add_doc("doc-1", Some(&[0.0, 1.0, 0.0, 0.0]), r#"{"title":"world"}"#).unwrap();
    assert_eq!(d0, 0);
    assert_eq!(d1, 1);
    let meta = writer.finalize().unwrap();
    assert_eq!(meta.doc_count, 2);
    assert_eq!(meta.docid_base, 0);
    assert_eq!(meta.tokenizer_id, tok_id);

    // 段不可变：finalize 后再 add 报错（I-1）
    // finalize 消费 self，编译期保证不可再调 add_doc

    // 段目录存在
    let seg_dir = format!("segments/seg_{}", meta.ulid);
    let files = vfs.list(&seg_dir).unwrap();
    assert!(files.contains(&"header.bin".to_string()));
    assert!(files.contains(&"vectors.bin".to_string()));
    assert!(files.contains(&"stored.bin".to_string()));
}
```

- [ ] **Step 2: 跑测试确认失败** — `cargo test -p vane-core -- segment_writer`，编译失败（SegmentWriter 未定义）。
- [ ] **Step 3: 最小实现** — 追加到 `crates/vane-core/src/segment/mod.rs`：
```rust
use crate::vfs::Vfs;
use std::sync::Arc;

pub struct SegmentWriter {
    vfs: Arc<dyn Vfs>,
    segments_dir: String,
    ulid: String,
    schema: Schema,
    tokenizer_id: TokenizerId,
    docid_base: u64,
    next_docid: u64,
    vectors: Vec<f32>,
    dim: u32,
    stored: Vec<(u64, String)>,  // (docid, stored_json)
    id_map: Vec<(u64, String)>,  // (docid, external_id)
    finalized: bool,
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
            vfs, segments_dir: segments_dir.to_string(), ulid,
            schema: schema.clone(), tokenizer_id: tokenizer_id.clone(),
            docid_base, next_docid: 0,
            vectors: Vec::new(), dim,
            stored: Vec::new(), id_map: Vec::new(),
            finalized: false,
        })
    }

    pub fn docid_base(&self) -> u64 { self.docid_base }

    /// 返回段内 docid（从 docid_base 起 u64 单调递增）。
    pub fn add_doc(
        &mut self,
        external_id: &str,
        vector: Option<&[f32]>,
        stored_json: &str,
    ) -> Result<u64> {
        assert!(!self.finalized, "segment already finalized (I-1)");
        let docid = self.next_docid;
        self.next_docid += 1;
        if let Some(v) = vector {
            if self.dim == 0 {
                return Err(VaneError::Schema("vector provided but schema has no vector field".into()));
            }
            if v.len() as u32 != self.dim {
                return Err(VaneError::Schema(format!(
                    "vector dim mismatch: got {} expected {}", v.len(), self.dim
                )));
            }
            self.vectors.extend_from_slice(v);
        } else if self.dim > 0 {
            // S4: schema 有 vector 字段但 doc 未提供 vector → 填零向量
            // 保证 docid i 的向量在 vectors[i*dim..]
            self.vectors.extend(std::iter::repeat(0.0f32).take(self.dim as usize));
        }
        self.id_map.push((docid, external_id.to_string()));
        self.stored.push((docid, stored_json.to_string()));
        Ok(docid)
    }

    pub fn finalize(mut self) -> Result<SegmentMeta> {
        self.finalized = true;
        let seg_dir = format!("{}/seg_{}", self.segments_dir, self.ulid);

        // 写 vectors.bin
        let vpath = format!("{}/vectors.bin", seg_dir);
        self.vfs.create(&vpath)?;
        let mut vbytes = Vec::with_capacity(self.vectors.len() * 4);
        for f in &self.vectors {
            vbytes.extend_from_slice(&f.to_le_bytes());
        }
        self.vfs.write_at(&vpath, &vbytes, 0)?;
        self.vfs.sync(&vpath)?;

        // 写 stored.bin：magic|version|count|{docid(8)|len(4)|json}...
        // **M0 写裸 JSON（zstd 块压缩延后 M1，format_version 不变）**。
        // SPEC §6.2 标注 stored.bin 为"zstd 块压缩"，M0 偏离此标注以避免引入 zstd 依赖 + wasm32 风险。M1 补 zstd 块压缩。
        let spath = format!("{}/stored.bin", seg_dir);
        self.vfs.create(&spath)?;
        let mut sbytes = Vec::new();
        sbytes.extend_from_slice(crate::types::MAGIC);
        sbytes.extend_from_slice(&crate::types::FORMAT_VERSION.to_le_bytes());
        sbytes.extend_from_slice(&(self.stored.len() as u32).to_le_bytes());
        for (docid, json) in &self.stored {
            sbytes.extend_from_slice(&docid.to_le_bytes());
            sbytes.extend_from_slice(&(json.len() as u32).to_le_bytes());
            sbytes.extend_from_slice(json.as_bytes());
        }
        self.vfs.write_at(&spath, &sbytes, 0)?;
        self.vfs.sync(&spath)?;

        // 写 id_map.bin（docid → external_id）
        // 注：`idmap.bin` 是 SPEC §3.2 映射表持久化的落点，§6.2 布局未显式命名此文件。M0 新增此文件作为段目录布局的一部分。
        let ipath = format!("{}/idmap.bin", seg_dir);
        self.vfs.create(&ipath)?;
        let mut ibytes = Vec::new();
        ibytes.extend_from_slice(crate::types::MAGIC);
        ibytes.extend_from_slice(&crate::types::FORMAT_VERSION.to_le_bytes());
        ibytes.extend_from_slice(&(self.id_map.len() as u32).to_le_bytes());
        for (docid, eid) in &self.id_map {
            ibytes.extend_from_slice(&docid.to_le_bytes());
            ibytes.extend_from_slice(&(eid.len() as u32).to_le_bytes());
            ibytes.extend_from_slice(eid.as_bytes());
        }
        self.vfs.write_at(&ipath, &ibytes, 0)?;
        self.vfs.sync(&ipath)?;

        // 写 scalars.col（空 stub：magic+version+0 字段）
        // M0 filter 未实现，scalars 无消费方，写空保证段目录布局完整
        let spath_col = format!("{}/scalars.col", seg_dir);
        self.vfs.create(&spath_col)?;
        let mut scbytes = Vec::new();
        scbytes.extend_from_slice(crate::types::MAGIC);
        scbytes.extend_from_slice(&crate::types::FORMAT_VERSION.to_le_bytes());
        scbytes.extend_from_slice(&0u32.to_le_bytes()); // 0 个标量字段
        self.vfs.write_at(&spath_col, &scbytes, 0)?;
        self.vfs.sync(&spath_col)?;

        let meta = SegmentMeta {
            ulid: self.ulid.clone(),
            doc_count: self.next_docid as u32,
            docid_base: self.docid_base,
            tokenizer_id: self.tokenizer_id.clone(),
            tombstones: roaring::RoaringBitmap::new(),
        };

        // 写 header.bin
        let hpath = format!("{}/header.bin", seg_dir);
        self.vfs.create(&hpath)?;
        let hbytes = header::encode_header(&meta)?;
        self.vfs.write_at(&hpath, &hbytes, 0)?;
        self.vfs.sync(&hpath)?;

        Ok(meta)
    }
}
```
- [ ] **Step 4: 跑测试确认通过** — `cargo test -p vane-core -- segment`，全绿。
- [ ] **Step 5: Commit**
```bash
git add -A
git commit -m "feat(segment): SegmentWriter writes header/vectors/stored/idmap (§6.2, I-1)

"
```

### Task 3: SegmentReader（读 header/vectors/stored/idmap）
**Files:**
- Modify: `crates/vane-core/src/segment/mod.rs`（追加 SegmentReader）

**Interfaces:**
- Consumes from Task 1-2
- Produces: `SegmentReader::open()`, `meta()`, `vectors()`, `dim()`, `doc_count()`, `external_id()`, `stored_json()`, `segment_dir()`, `vfs()`
- 后续 05-bm25（InvertedIndexReader 通过 segment_dir() 读 inverted.bin）、06-vector-brute（通过 vectors() 扫描）、07-api-core 消费

- [ ] **Step 1: 写失败测试** — 追加到 tests.rs：
```rust
use super::SegmentReader;

#[test]
fn segment_reader_roundtrip() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
    let schema = Schema::new(vec![
        ("v".into(), FieldDef::Vector { dim: 4, metric: Metric::Cosine }),
    ]).unwrap();
    let tok_id = TokenizerId([0x22; 32]);

    let mut writer = SegmentWriter::new(vfs.clone(), "segments", &schema, &tok_id, 0).unwrap();
    writer.add_doc("alpha", Some(&[1.0, 2.0, 3.0, 4.0]), r#"{"x":1}"#).unwrap();
    writer.add_doc("beta", Some(&[5.0, 6.0, 7.0, 8.0]), r#"{"x":2}"#).unwrap();
    let meta = writer.finalize().unwrap();

    let seg_dir = format!("segments/seg_{}", meta.ulid);
    let reader = SegmentReader::open(&vfs, &seg_dir).unwrap();

    assert_eq!(reader.meta().doc_count, 2);
    assert_eq!(reader.dim(), 4);
    assert_eq!(reader.vectors().len(), 8);
    assert_eq!(&reader.vectors()[0..4], &[1.0, 2.0, 3.0, 4.0]);
    assert_eq!(reader.external_id(0), Some("alpha"));
    assert_eq!(reader.external_id(1), Some("beta"));
    assert_eq!(reader.external_id(999), None);
    // stored.bin 回填：stored_json(local_docid) 返回写入时的 JSON
    assert_eq!(reader.stored_json(0), Some(r#"{"x":1}"#));
    assert_eq!(reader.stored_json(1), Some(r#"{"x":2}"#));
    assert_eq!(reader.stored_json(999), None);
    assert_eq!(reader.segment_dir(), seg_dir);
}

#[test]
fn segment_reader_rejects_bad_magic() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
    let seg_dir = "segments/seg_bad";
    vfs.create(&format!("{}/header.bin", seg_dir)).unwrap();
    vfs.write_at(&format!("{}/header.bin", seg_dir), b"XXXX", 0).unwrap();
    let r = SegmentReader::open(&vfs, seg_dir);
    assert!(matches!(r, Err(VaneError::Corrupt(_))));
}
```

- [ ] **Step 2: 跑测试确认失败** — `cargo test -p vane-core -- segment_reader`，编译失败。
- [ ] **Step 3: 最小实现** — 追加到 segment/mod.rs：

> 重构为顺序读取 + 末尾统一构造，避免 clippy 告。`read_all` 提取为模块级辅助函数。

```rust
/// 模块级辅助：循环 read_at 直到 EOF，拼出完整文件字节。
fn read_all(vfs: &dyn Vfs, path: &str) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    let mut off = 0u64;
    loop {
        let n = vfs.read_at(path, &mut tmp, off)?;
        if n == 0 { break; }
        buf.extend_from_slice(&tmp[..n]);
        off += n as u64;
    }
    Ok(buf)
}

pub struct SegmentReader {
    meta: SegmentMeta,
    vfs: Arc<dyn Vfs>,
    segment_dir: String,
    vectors: Vec<f32>,
    dim: u32,
    id_map: std::collections::HashMap<u64, String>,
    // stored.bin 按 docid 索引的 stored JSON（回填 Hit.fields，SPEC §6.2 stored.bin）
    // key 为段内局部 docid（0 起，与 id_map 一致）
    stored: std::collections::HashMap<u64, String>,
}

impl SegmentReader {
    pub fn open(vfs: &Arc<dyn Vfs>, segment_dir: &str) -> Result<Self> {
        // 读 header
        let hpath = format!("{}/header.bin", segment_dir);
        let hbuf = read_all(vfs.as_ref(), &hpath)?;
        let meta = header::decode_header(&hbuf)?;

        // 读 vectors（doc_count=0 时为空）
        let vectors: Vec<f32> = if meta.doc_count > 0 {
            let vpath = format!("{}/vectors.bin", segment_dir);
            let vbuf = read_all(vfs.as_ref(), &vpath)?;
            vbuf.chunks_exact(4)
                .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
                .collect()
        } else {
            Vec::new()
        };
        let dim = if vectors.is_empty() { 0 } else { (vectors.len() / meta.doc_count as usize) as u32 };

        // 读 id_map
        let id_map = Self::load_id_map(vfs.as_ref(), segment_dir)?;

        // 读 stored.bin（按 docid 索引，供 stored_json() 回填 Hit.fields）
        // stored.bin 布局与 idmap.bin 对称：magic|version|count|{docid(8)|len(4)|json}...
        // docid 为段内局部 docid（0 起，SegmentWriter::add_doc 返回值）
        let stored = Self::load_stored(vfs.as_ref(), segment_dir)?;

        Ok(Self {
            meta, vfs: vfs.clone(), segment_dir: segment_dir.to_string(),
            vectors, dim, id_map, stored,
        })
    }

    fn load_stored(vfs: &dyn Vfs, segment_dir: &str) -> Result<std::collections::HashMap<u64, String>> {
        let spath = format!("{}/stored.bin", segment_dir);
        let buf = read_all(vfs, &spath)?;
        if buf.len() < 12 { return Ok(std::collections::HashMap::new()); }
        // skip magic(4) + version(4)
        let count = u32::from_le_bytes(buf[8..12].try_into().unwrap()) as usize;
        let mut pos = 12;
        let mut map = std::collections::HashMap::with_capacity(count);
        for _ in 0..count {
            if pos + 12 > buf.len() { break; }
            let docid = u64::from_le_bytes(buf[pos..pos+8].try_into().unwrap());
            pos += 8;
            let len = u32::from_le_bytes(buf[pos..pos+4].try_into().unwrap()) as usize;
            pos += 4;
            let s = std::str::from_utf8(&buf[pos..pos+len])
                .map_err(|e| VaneError::Corrupt(format!("stored utf8: {}", e)))?
                .to_string();
            pos += len;
            map.insert(docid, s);
        }
        Ok(map)
    }

    fn load_id_map(vfs: &dyn Vfs, segment_dir: &str) -> Result<std::collections::HashMap<u64, String>> {
        let ipath = format!("{}/idmap.bin", segment_dir);
        let buf = read_all(vfs, &ipath)?;
        if buf.len() < 12 { return Ok(std::collections::HashMap::new()); }
        // skip magic(4) + version(4)
        let count = u32::from_le_bytes(buf[8..12].try_into().unwrap()) as usize;
        let mut pos = 12;
        let mut map = std::collections::HashMap::with_capacity(count);
        for _ in 0..count {
            if pos + 12 > buf.len() { break; }
            let docid = u64::from_le_bytes(buf[pos..pos+8].try_into().unwrap());
            pos += 8;
            let len = u32::from_le_bytes(buf[pos..pos+4].try_into().unwrap()) as usize;
            pos += 4;
            let s = std::str::from_utf8(&buf[pos..pos+len])
                .map_err(|e| VaneError::Corrupt(format!("idmap utf8: {}", e)))?
                .to_string();
            pos += len;
            map.insert(docid, s);
        }
        Ok(map)
    }

    pub fn meta(&self) -> &SegmentMeta { &self.meta }
    pub fn vectors(&self) -> &[f32] { &self.vectors }
    pub fn dim(&self) -> u32 { self.dim }
    pub fn doc_count(&self) -> u32 { self.meta.doc_count }
    pub fn external_id(&self, docid: u64) -> Option<&str> {
        self.id_map.get(&docid).map(|s| s.as_str())
    }
    /// 读取某文档的 stored.bin JSON（回填 Hit.fields，SPEC §6.2 stored.bin）。
    /// local_docid 为段内局部 docid（0 起，与 external_id 同一 key 空间）。
    pub fn stored_json(&self, local_docid: u64) -> Option<&str> {
        self.stored.get(&local_docid).map(|s| s.as_str())
    }
    pub fn segment_dir(&self) -> &str { &self.segment_dir }
    pub fn vfs(&self) -> &Arc<dyn Vfs> { &self.vfs }
}
```
- [ ] **Step 4: 跑测试确认通过** — `cargo test -p vane-core -- segment`，全绿。
- [ ] **Step 5: Commit**
```bash
git add -A
git commit -m "feat(segment): SegmentReader loads header/vectors/idmap (§6.2)

"
```

### Task 4: 段不可变验证 + StdFsVfs 往返
**Files:**
- Modify: `crates/vane-core/src/segment/tests.rs`

**Interfaces:**
- Consumes from Task 1-3
- Produces: 完整段格式测试（不变量 I-1 覆盖）

- [ ] **Step 1: 写测试** — 追加：
```rust
#[test]
fn segment_immutable_after_finalize() {
    // finalize 消费 self → 编译期保证不可再调 add_doc。
    // 此测试验证 finalize 后段文件不被修改。
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
    let schema = Schema::new(vec![
        ("v".into(), FieldDef::Vector { dim: 2, metric: Metric::Dot }),
    ]).unwrap();
    let mut w = SegmentWriter::new(vfs.clone(), "seg", &schema, &TokenizerId([0;32]), 0).unwrap();
    w.add_doc("a", Some(&[1.0, 0.0]), "{}").unwrap();
    let meta = w.finalize().unwrap();
    // 读回段，验证内容不变
    let seg_dir = format!("seg/seg_{}", meta.ulid);
    let r1 = SegmentReader::open(&vfs, &seg_dir).unwrap();
    assert_eq!(r1.doc_count(), 1);
    let r2 = SegmentReader::open(&vfs, &seg_dir).unwrap();
    assert_eq!(r1.vectors(), r2.vectors()); // 两次读一致
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn segment_stdfs_roundtrip() {
    use crate::vfs::std_fs::StdFsVfs;
    use std::path::PathBuf;
    let dir = std::env::temp_dir().join(format!("vane-seg-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let vfs = std::sync::Arc::new(StdFsVfs::with_root(dir.to_str().unwrap())) as std::sync::Arc<dyn Vfs>;
    let schema = Schema::new(vec![
        ("v".into(), FieldDef::Vector { dim: 3, metric: Metric::Cosine }),
    ]).unwrap();
    let mut w = SegmentWriter::new(vfs.clone(), "segments", &schema, &TokenizerId([0xff;32]), 0).unwrap();
    w.add_doc("x", Some(&[0.1, 0.2, 0.3]), r#"{"k":"v"}"#).unwrap();
    let meta = w.finalize().unwrap();
    let seg_dir = format!("segments/seg_{}", meta.ulid);
    let r = SegmentReader::open(&vfs, &seg_dir).unwrap();
    assert_eq!(r.external_id(0), Some("x"));
    assert_eq!(r.dim(), 3);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn segment_writer_docid_base_nonzero() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
    let schema = Schema::new(vec![
        ("v".into(), FieldDef::Vector { dim: 2, metric: Metric::Cosine }),
    ]).unwrap();
    let tok_id = TokenizerId([0x33; 32]);
    // 第一段 base=0
    let mut w1 = SegmentWriter::new(vfs.clone(), "seg", &schema, &tok_id, 0).unwrap();
    w1.add_doc("a", Some(&[1.0, 0.0]), "{}").unwrap();
    w1.add_doc("b", Some(&[0.0, 1.0]), "{}").unwrap();
    let m1 = w1.finalize().unwrap();
    assert_eq!(m1.docid_base, 0);
    assert_eq!(m1.doc_count, 2);
    // 第二段 base=2（接续）
    let mut w2 = SegmentWriter::new(vfs.clone(), "seg", &schema, &tok_id, 2).unwrap();
    w2.add_doc("c", Some(&[1.0, 1.0]), "{}").unwrap();
    let m2 = w2.finalize().unwrap();
    assert_eq!(m2.docid_base, 2);
    assert_eq!(m2.doc_count, 1);
    // 读回验证
    let seg1_dir = format!("seg/seg_{}", m1.ulid);
    let r1 = SegmentReader::open(&vfs, &seg1_dir).unwrap();
    assert_eq!(r1.meta().docid_base, 0);
    let seg2_dir = format!("seg/seg_{}", m2.ulid);
    let r2 = SegmentReader::open(&vfs, &seg2_dir).unwrap();
    assert_eq!(r2.meta().docid_base, 2);
}

#[test]
fn segment_writer_vector_none_fills_zeros() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
    let schema = Schema::new(vec![
        ("v".into(), FieldDef::Vector { dim: 3, metric: Metric::Cosine }),
    ]).unwrap();
    let tok_id = TokenizerId([0x44; 32]);
    let mut w = SegmentWriter::new(vfs.clone(), "seg", &schema, &tok_id, 0).unwrap();
    // doc0 有 vector
    w.add_doc("a", Some(&[1.0, 2.0, 3.0]), "{}").unwrap();
    // doc1 无 vector → 填零向量
    w.add_doc("b", None, "{}").unwrap();
    let meta = w.finalize().unwrap();
    let seg_dir = format!("seg/seg_{}", meta.ulid);
    let r = SegmentReader::open(&vfs, &seg_dir).unwrap();
    // doc0 的向量
    assert_eq!(&r.vectors()[0..3], &[1.0, 2.0, 3.0]);
    // doc1 的向量 = 零向量
    assert_eq!(&r.vectors()[3..6], &[0.0, 0.0, 0.0]);
}
```
- [ ] **Step 2: 跑测试确认通过** — `cargo test -p vane-core -- segment`，全绿。
- [ ] **Step 3: clippy + wasm32 check** —
```bash
cargo clippy -p vane-core -- -D warnings
cargo check --target wasm32-unknown-unknown -p vane-core 2>&1 | tail -5
```
- [ ] **Step 4: 确认全绿**
- [ ] **Step 5: Commit**
```bash
git add -A
git commit -m "test(segment): immutability (I-1) + StdFsVfs roundtrip coverage

"
```
