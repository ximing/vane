//! SPEC §4.1 / §15 快照导出（M2-12）。
//!
//! 单文件快照格式 `VANE_SNAP`：
//! ```text
//! magic(9)="VANE_SNAP" | version(4 LE)=1 | num_files(4 LE) |
//! { path_len(4 LE) | path_bytes | file_len(8 LE) | file_bytes }...
//! ```
//! 路径以相对 `db_path` 的 `/` 分量存储（如 `manifest.json`、`segments/seg_x/header.bin`、
//! `wal.log`），恢复时解包到任意 `db_path` 目录。
//!
//! - `write_snapshot`：只读遍历原库（manifest.json + segments/seg_*/全部文件 + wal.log），
//!   流式逐文件 `read_at` → `append` 写 dest 临时文件 → sync → rename（I-6 原子）。
//!   不修改原库（I-6 只读遍历 + 新文件写入）。
//! - `read_snapshot`：解包单文件快照到 `db_path` 目录（逐文件 delete→create→write_at→sync），
//!   供 `Db::open` 打开（P0-3 数据主权恢复闭环）。
//!
//! core 禁 std::fs；全部经 Vfs trait（list + read_at + append + create + write_at + sync + rename）。

use crate::types::{Result, VaneError};
use crate::vfs::Vfs;

/// 快照魔数（ASCII）。spec 标注 magic="VANE_SNAP"（9 字节）。
pub const SNAPSHOT_MAGIC: &[u8] = b"VANE_SNAP";
/// 快照格式版本。
pub const SNAPSHOT_VERSION: u32 = 1;

const MANIFEST_REL: &str = "manifest.json";
const WAL_REL: &str = "wal.log";
const SEGMENTS_DIR: &str = "segments";

// ---- 写入辅助：经 Vfs::append 流式写 dest 临时文件 ----

/// 把一段字节 append 到 dest 文件（dest 须已 create）。
fn append_bytes(vfs: &dyn Vfs, dest: &str, bytes: &[u8]) -> Result<()> {
    let _ = vfs.append(dest, bytes)?;
    Ok(())
}

/// 写 u32 LE。
fn append_u32(vfs: &dyn Vfs, dest: &str, v: u32) -> Result<()> {
    append_bytes(vfs, dest, &v.to_le_bytes())
}

/// 写 u64 LE。
fn append_u64(vfs: &dyn Vfs, dest: &str, v: u64) -> Result<()> {
    append_bytes(vfs, dest, &v.to_le_bytes())
}

/// 收集原库全部文件的相对路径（相对 db_path，`/` 分量）。
///
/// 遍历：manifest.json + segments/seg_*/全部文件 + wal.log（若存在）。
/// 递归固定 2 层（segments/seg_x/file）。不收集 manifest.json.tmp（瞬态）。
fn collect_files(vfs: &dyn Vfs, db_path: &str) -> Result<Vec<String>> {
    let mut rels: Vec<String> = Vec::new();
    rels.push(MANIFEST_REL.to_string());

    // segments/seg_<ulid>/ 全部文件
    let segments_dir = format!("{}/{}", db_path, SEGMENTS_DIR);
    if let Ok(seg_entries) = vfs.list(&segments_dir) {
        for seg_entry in seg_entries {
            // 仅处理 seg_ 前缀的段目录（与 wal::cleanup_orphan_segment_dirs 一致）。
            if !seg_entry.starts_with("seg_") {
                continue;
            }
            let seg_dir = format!("{}/{}", segments_dir, seg_entry);
            let files = match vfs.list(&seg_dir) {
                Ok(f) => f,
                Err(VaneError::Io(_)) => continue, // 段目录被并发清理（尽力）
                Err(e) => return Err(e),
            };
            for f in files {
                // 跳过可能的临时文件（保险；当前段写入无 tmp 残留）。
                if f.ends_with(".tmp") {
                    continue;
                }
                rels.push(format!("{}/{}/{}", SEGMENTS_DIR, seg_entry, f));
            }
        }
    }

    // wal.log（崩溃恢复完整性；可能不存在/为空）
    let wal_abs = format!("{}/{}", db_path, WAL_REL);
    if file_exists(vfs, &wal_abs)? {
        rels.push(WAL_REL.to_string());
    }

    Ok(rels)
}

/// 判断文件是否存在：经 0 字节 read_at 探测。
/// - MemoryVfs：文件存在（含空）→ Ok(0)；不存在 → Err(Io)。
/// - StdFsVfs：File::open 失败 → Err(Io)；成功 → read 空 buf → Ok(0)。
fn file_exists(vfs: &dyn Vfs, path: &str) -> Result<bool> {
    match vfs.read_at(path, &mut [], 0) {
        Ok(_) => Ok(true),
        Err(VaneError::Io(_)) => Ok(false),
        Err(e) => Err(e),
    }
}

/// 读文件全部内容。文件不存在返回 Ok(None)。
fn read_file_full(vfs: &dyn Vfs, path: &str) -> Result<Option<Vec<u8>>> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    let mut off = 0u64;
    loop {
        let n = match vfs.read_at(path, &mut tmp, off) {
            Ok(n) => n,
            Err(VaneError::Io(_)) => return Ok(None),
            Err(e) => return Err(e),
        };
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        off += n as u64;
    }
    Ok(Some(buf))
}

/// 写快照到 dest（单文件）。
///
/// 流式：先收集路径（仅路径名，非内容）→ 写头 → 逐文件 read_at 全量 + append。
/// 原子：写 `<dest>.tmp` → sync → rename → dest（I-6）。中途失败 dest 不残留
/// （残留的是 `.tmp`，下次 export 覆盖前先 delete）。
pub fn write_snapshot(vfs: &dyn Vfs, db_path: &str, dest: &str) -> Result<()> {
    if dest.is_empty() {
        return Err(VaneError::InvalidArg("dest path is empty".into()));
    }
    let rels = collect_files(vfs, db_path)?;
    // manifest.json 必须存在（有效库）。
    let manifest_abs = format!("{}/{}", db_path, MANIFEST_REL);
    if !file_exists(vfs, &manifest_abs)? {
        return Err(VaneError::Corrupt(
            format!("manifest.json not found at {}", manifest_abs).into(),
        ));
    }

    let tmp = format!("{}.tmp", dest);
    // I-6：清理可能残留的 tmp（忽略不存在）。
    let _ = vfs.delete(&tmp);
    vfs.create(&tmp)?;

    // 头
    append_bytes(vfs, &tmp, SNAPSHOT_MAGIC)?;
    append_u32(vfs, &tmp, SNAPSHOT_VERSION)?;
    append_u32(vfs, &tmp, rels.len() as u32)?;

    // 逐文件：path_len | path | file_len | file_bytes
    for rel in &rels {
        let abs = format!("{}/{}", db_path, rel);
        let content = read_file_full(vfs, &abs)?.ok_or_else(|| {
            VaneError::Corrupt(format!("file vanished during snapshot: {}", abs).into())
        })?;
        append_u32(vfs, &tmp, rel.len() as u32)?;
        append_bytes(vfs, &tmp, rel.as_bytes())?;
        append_u64(vfs, &tmp, content.len() as u64)?;
        append_bytes(vfs, &tmp, &content)?;
    }

    vfs.sync(&tmp)?;
    // 原子切换（与 manifest save_atomic 同语义）。
    vfs.rename(&tmp, dest)?;
    Ok(())
}

/// 读快照（单文件）解包到 db_path 目录。
///
/// 逐文件 delete（忽略不存在）→ create → write_at → sync 还原 manifest.json +
/// segments/seg_*/... + wal.log。随后 `Db::open(vfs, db_path, opts)` 即可打开。
pub fn read_snapshot(vfs: &dyn Vfs, src: &str, db_path: &str) -> Result<()> {
    if src.is_empty() {
        return Err(VaneError::InvalidArg("src path is empty".into()));
    }
    let mut snap = Vec::new();
    let mut tmp = [0u8; 8192];
    let mut off = 0u64;
    loop {
        let n = match vfs.read_at(src, &mut tmp, off) {
            Ok(n) => n,
            Err(VaneError::Io(_)) => {
                return Err(VaneError::Io(format!("snapshot not found: {}", src).into()));
            }
            Err(e) => return Err(e),
        };
        if n == 0 {
            break;
        }
        snap.extend_from_slice(&tmp[..n]);
        off += n as u64;
    }

    // 解析头
    let mut pos = 0usize;
    if snap.len() < SNAPSHOT_MAGIC.len() {
        return Err(VaneError::Corrupt("snapshot too short for magic".into()));
    }
    if &snap[..SNAPSHOT_MAGIC.len()] != SNAPSHOT_MAGIC {
        return Err(VaneError::Corrupt("snapshot magic mismatch".into()));
    }
    pos += SNAPSHOT_MAGIC.len();

    let version = read_u32(&snap, &mut pos)?;
    if version != SNAPSHOT_VERSION {
        return Err(VaneError::Version(
            format!("unsupported snapshot version {}", version).into(),
        ));
    }
    let num_files = read_u32(&snap, &mut pos)?;

    for _ in 0..num_files {
        let path_len = read_u32(&snap, &mut pos)? as usize;
        if pos + path_len > snap.len() {
            return Err(VaneError::Corrupt("snapshot path truncated".into()));
        }
        let rel = std::str::from_utf8(&snap[pos..pos + path_len])
            .map_err(|e| VaneError::Corrupt(format!("snapshot path not utf-8: {}", e).into()))?
            .to_string();
        pos += path_len;
        let file_len = read_u64(&snap, &mut pos)? as usize;
        if pos + file_len > snap.len() {
            return Err(VaneError::Corrupt(
                format!("snapshot file content truncated: {}", rel).into(),
            ));
        }
        let content = &snap[pos..pos + file_len];
        pos += file_len;

        let abs = format!("{}/{}", db_path, rel);
        // 重新恢复（覆盖可能残留的同名文件）。
        let _ = vfs.delete(&abs);
        vfs.create(&abs)?;
        if !content.is_empty() {
            vfs.write_at(&abs, content, 0)?;
        }
        vfs.sync(&abs)?;
    }

    Ok(())
}

fn read_u32(buf: &[u8], pos: &mut usize) -> Result<u32> {
    if *pos + 4 > buf.len() {
        return Err(VaneError::Corrupt("snapshot truncated reading u32".into()));
    }
    let v = u32::from_le_bytes([buf[*pos], buf[*pos + 1], buf[*pos + 2], buf[*pos + 3]]);
    *pos += 4;
    Ok(v)
}

fn read_u64(buf: &[u8], pos: &mut usize) -> Result<u64> {
    if *pos + 8 > buf.len() {
        return Err(VaneError::Corrupt("snapshot truncated reading u64".into()));
    }
    let v = u64::from_le_bytes([
        buf[*pos],
        buf[*pos + 1],
        buf[*pos + 2],
        buf[*pos + 3],
        buf[*pos + 4],
        buf[*pos + 5],
        buf[*pos + 6],
        buf[*pos + 7],
    ]);
    *pos += 8;
    Ok(v)
}

// ---- 测试 ----

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{CollectionOptions, Db, OpenOptions};
    use crate::types::{FieldDef, Metric, Schema};
    use crate::vfs::memory::MemoryVfs;
    use std::sync::Arc;

    fn schema_vec4() -> Schema {
        Schema::new(vec![
            ("body".into(), FieldDef::Text),
            (
                "v".into(),
                FieldDef::Vector {
                    dim: 4,
                    metric: Metric::Cosine,
                },
            ),
        ])
        .unwrap()
    }

    fn build_db(vfs: Arc<dyn Vfs>, db_path: &str) -> (Db, crate::api::Collection) {
        let db = Db::open(vfs.clone(), db_path, OpenOptions::default()).unwrap();
        let col = db
            .collection("docs", schema_vec4(), CollectionOptions::default())
            .unwrap();
        let docs = vec![
            crate::api::Doc {
                id: "a".into(),
                text: Some("hello world".into()),
                vector: Some(vec![1.0, 0.0, 0.0, 0.0]),
                meta: None,
            },
            crate::api::Doc {
                id: "b".into(),
                text: Some("foo bar".into()),
                vector: Some(vec![0.0, 1.0, 0.0, 0.0]),
                meta: None,
            },
        ];
        col.add(&docs).unwrap();
        col.flush().unwrap();
        (db, col)
    }

    #[test]
    fn snapshot_format_header() {
        let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
        let (db, _col) = build_db(vfs.clone(), "db");
        db.export("backup.vane").unwrap();

        let mut head = [0u8; 32];
        let n = vfs.read_at("backup.vane", &mut head, 0).unwrap();
        assert!(n >= SNAPSHOT_MAGIC.len() + 8);
        assert_eq!(&head[..SNAPSHOT_MAGIC.len()], SNAPSHOT_MAGIC);
        let version = u32::from_le_bytes([head[9], head[10], head[11], head[12]]);
        assert_eq!(version, 1);
        // num_files >= 1 (manifest) + segment files + wal
        let num_files = u32::from_le_bytes([head[13], head[14], head[15], head[16]]);
        assert!(num_files >= 1);
    }

    #[test]
    fn snapshot_includes_manifest_segments_wal() {
        let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
        let (db, _col) = build_db(vfs.clone(), "db");
        db.export("backup.vane").unwrap();

        // 解析快照，收集所有相对路径
        let mut snap = Vec::new();
        let mut tmp = [0u8; 8192];
        let mut off = 0u64;
        loop {
            let n = vfs.read_at("backup.vane", &mut tmp, off).unwrap();
            if n == 0 {
                break;
            }
            snap.extend_from_slice(&tmp[..n]);
            off += n as u64;
        }
        let mut pos = SNAPSHOT_MAGIC.len();
        let _version = u32::from_le_bytes(snap[pos..pos + 4].try_into().unwrap());
        pos += 4;
        let num_files = u32::from_le_bytes(snap[pos..pos + 4].try_into().unwrap());
        pos += 4;
        let mut rels = Vec::new();
        for _ in 0..num_files {
            let plen = u32::from_le_bytes(snap[pos..pos + 4].try_into().unwrap()) as usize;
            pos += 4;
            let rel = std::str::from_utf8(&snap[pos..pos + plen])
                .unwrap()
                .to_string();
            pos += plen;
            let flen = u64::from_le_bytes(snap[pos..pos + 8].try_into().unwrap()) as usize;
            pos += 8;
            pos += flen;
            rels.push(rel);
        }
        assert!(rels.iter().any(|r| r == "manifest.json"));
        assert!(rels.iter().any(|r| r == "wal.log"));
        assert!(rels.iter().any(|r| r.starts_with("segments/seg_")));
        assert!(rels.iter().any(|r| r.ends_with("/header.bin")));
        assert!(rels.iter().any(|r| r.ends_with("/vectors.bin")));
        assert!(rels.iter().any(|r| r.ends_with("/stored.bin")));
        // 不含 tmp
        assert!(rels.iter().all(|r| !r.ends_with(".tmp")));
    }

    #[test]
    fn export_read_snapshot_open_search_roundtrip() {
        // P0-3 数据主权闭环：原库 add/flush → export → read_snapshot 新路径 → open → search 一致
        let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
        let (db, col) = build_db(vfs.clone(), "orig");
        // 原库搜索
        let q = crate::api::SearchQuery {
            vector: Some(vec![1.0, 0.0, 0.0, 0.0]),
            top_k: 2,
            mode: crate::api::SearchMode::Vector,
            ..Default::default()
        };
        let orig_hits = col.search(&q).unwrap();
        assert_eq!(orig_hits[0].id, "a");

        // 导出
        db.export("backup.vane").unwrap();
        // 解包到新路径
        read_snapshot(vfs.as_ref(), "backup.vane", "restored").unwrap();
        // 打开恢复库
        let db2 = Db::open(vfs.clone(), "restored", OpenOptions::default()).unwrap();
        let col2 = db2
            .collection("docs", schema_vec4(), CollectionOptions::default())
            .unwrap();
        let restored_hits = col2.search(&q).unwrap();
        assert_eq!(restored_hits.len(), orig_hits.len());
        assert_eq!(restored_hits[0].id, orig_hits[0].id);
        assert_eq!(restored_hits[0].score, orig_hits[0].score);
        // 文本搜索一致性
        let tq = crate::api::SearchQuery {
            text: Some("hello".into()),
            top_k: 2,
            ..Default::default()
        };
        let orig_text = col.search(&tq).unwrap();
        let rest_text = col2.search(&tq).unwrap();
        assert_eq!(orig_text.len(), rest_text.len());
        assert_eq!(orig_text[0].id, rest_text[0].id);
    }

    #[test]
    fn empty_db_export() {
        let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
        let db = Db::open(vfs.clone(), "empty", OpenOptions::default()).unwrap();
        // 先 collection（写 manifest）但不 add → 空段库。
        let schema = Schema::new(vec![(
            "v".into(),
            FieldDef::Vector {
                dim: 2,
                metric: Metric::Cosine,
            },
        )])
        .unwrap();
        let _ = db
            .collection("c", schema, CollectionOptions::default())
            .unwrap();
        db.export("backup.vane").unwrap();
        // 解析 num_files
        let mut head = [0u8; 32];
        let n = vfs.read_at("backup.vane", &mut head, 0).unwrap();
        assert!(n >= 17);
        let num_files = u32::from_le_bytes([head[13], head[14], head[15], head[16]]);
        // 无段：仅 manifest.json + wal.log（wal.log 由 open 创建，可能为空但存在）
        assert!(num_files >= 1);
        assert!(num_files <= 2);
    }

    #[test]
    fn read_snapshot_rejects_bad_magic() {
        let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
        vfs.create("bad.vane").unwrap();
        vfs.write_at("bad.vane", b"NOTVANE!!!", 0).unwrap();
        let r = read_snapshot(vfs.as_ref(), "bad.vane", "out");
        assert!(matches!(r, Err(VaneError::Corrupt(_))));
    }

    #[test]
    fn write_snapshot_is_atomic_no_partial_dest() {
        // dest 不存在时 export 成功 → dest 存在；中途无 dest.tmp 残留
        let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
        let (db, _col) = build_db(vfs.clone(), "db");
        db.export("out.vane").unwrap();
        // dest 存在
        assert!(file_exists(vfs.as_ref(), "out.vane").unwrap());
        // tmp 已 rename 走，不残留
        assert!(!file_exists(vfs.as_ref(), "out.vane.tmp").unwrap());
    }
}
