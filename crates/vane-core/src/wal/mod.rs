//! 04-wal：薄 WAL 元操作日志 + 崩溃恢复（SPEC §6.4/§6.2/§7.2）。
//!
//! WAL 文件 `<db>/wal.log`，JSON 行格式（每行一条 [`WalRecord`]）。`append` 后 `sync`
//! 保证崩溃前落盘。崩溃恢复见 [`recover`]：重放未提交的 tombstone/段增删，清理半成品段。
//!
//! # B-2 硬约束（计划裁决）
//!
//! **flush 路径不调 [`Wal::truncate`]**——否则 `flush→delete→flush→崩溃` 会丢失未消费的
//! `AddTombstone`（tombstone 仅存 WAL，02 不改 header.bin），致已删文档复活（数据损坏）。
//! **仅 compact/merge 成功 + manifest 切换后调 [`Wal::truncate`]**（此时 AddTombstone
//! 随旧段物理清除）。WAL 累积 AddSegment 记录直到 compact（ULID 字符串体积可忽略），
//! compact 后一次性清空。
//!
//! # M-minor-2：tombstone abs/local 语义（02 遗留）
//!
//! `WalRecord::AddTombstone.docids` 存**绝对 docid**（与运行期 `CollectionInner.tombstones`
//! 位图一致——delete 期写入的也是绝对 docid；filter/tombstone 运行期统一在绝对空间）。
//! [`recover`] 注入时直接用绝对 docid（roaring 存 u32，故 u64 截断到 u32，与 delete 一致）。
//! 段内 local docid 仅在 SegmentReader 边界处由 `docid_base` 转换，WAL 不涉及 local 语义。

use crate::persistence::Manifest;
use crate::types::{Result, VaneError};
use crate::vfs::Vfs;
use std::collections::HashMap;
use std::sync::Arc;

#[cfg(test)]
mod tests;

const WAL_FILENAME: &str = "wal.log";

/// WAL 记录类型（SPEC §6.4：仅段增删/tombstone 元操作）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum WalRecord {
    /// 新段添加（manifest 切换前 append）。半成品段（ULID 不在 manifest）在 [`recover`] 时清理。
    AddSegment { collection: String, ulid: String },
    /// 段删除（合并/compact 后旧段清除）。[`recover`] 时仅作信息记录（合并未完成则保留旧段）。
    DeleteSegment { collection: String, ulid: String },
    /// tombstone 追加（delete 即时记录，SPEC §7.2）。
    ///
    /// `docids` 为**绝对 docid**（M-minor-2：与运行期 tombstone 位图一致）。
    AddTombstone {
        collection: String,
        ulid: String,
        docids: Vec<u64>,
    },
}

/// 薄 WAL 句柄。无状态（每次 append 独立 sync），可随时丢弃/重建。
pub struct Wal {
    vfs: Arc<dyn Vfs>,
    path: String,
}

impl Wal {
    /// 打开（或首次创建）`<db>/wal.log`。幂等：文件已存在则保留（追加语义）。
    pub fn open(vfs: Arc<dyn Vfs>, db_path: &str) -> Result<Self> {
        let path = format!("{}/{}", db_path, WAL_FILENAME);
        // 幂等 create：已存在则忽略（Vfs::create 在已存在时返回 Io 错误，此处 best-effort）。
        let _ = vfs.create(&path);
        Ok(Self { vfs, path })
    }

    /// 追加一条记录（JSON 行，每行一条；append 后 sync 保证崩溃前落盘）。
    pub fn append(&self, record: &WalRecord) -> Result<()> {
        let mut line = serde_json::to_vec(record)
            .map_err(|e| VaneError::Corrupt(format!("wal serialize: {}", e)))?;
        line.push(b'\n');
        self.vfs.append(&self.path, &line)?;
        self.vfs.sync(&self.path)?;
        Ok(())
    }

    /// 读取全部记录（崩溃恢复用）。文件不存在（新库）返回空。
    pub fn read_all(&self) -> Result<Vec<WalRecord>> {
        let mut buf = Vec::new();
        let mut tmp = [0u8; 8192];
        let mut off = 0u64;
        loop {
            let n = match self.vfs.read_at(&self.path, &mut tmp, off) {
                Ok(n) => n,
                Err(VaneError::Io(_)) => return Ok(Vec::new()), // 文件不存在（新库）
                Err(e) => return Err(e),
            };
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
            off += n as u64;
        }
        let mut records = Vec::new();
        for line in buf.split(|b| *b == b'\n') {
            if line.is_empty() {
                continue;
            }
            let r: WalRecord = serde_json::from_slice(line)
                .map_err(|e| VaneError::Corrupt(format!("wal parse: {}", e)))?;
            records.push(r);
        }
        Ok(records)
    }

    /// **仅 compact/merge 成功 + manifest 切换后调用**（B-2 修复）。
    ///
    /// flush 路径**不**调 truncate——否则 `flush→delete→flush→崩溃` 会丢失未消费的
    /// `AddTombstone`（tombstone 仅存 WAL，02 不改 header.bin），致已删文档复活（数据损坏）。
    /// WAL 累积 AddSegment 记录直到 compact（ULID 字符串体积可忽略），compact 后一次性清空。
    ///
    /// 实现：delete（忽略不存在）+ create（空文件）+ sync。
    pub fn truncate(&self) -> Result<()> {
        let _ = self.vfs.delete(&self.path);
        self.vfs.create(&self.path)?;
        self.vfs.sync(&self.path)?;
        Ok(())
    }
}

/// 崩溃恢复产物：`collection → (ulid → 绝对 docid tombstone 位图)`。
/// 由 [`recover`] 返回，`Db::open` 注入各 `CollectionInner.tombstones`
/// （仅对 manifest 中仍存在的 ULID——recover 已过滤，调用方双重保险）。
pub type RecoveredTombstones = HashMap<String, HashMap<String, roaring::RoaringBitmap>>;

/// 崩溃恢复：`Db::open` 时调用。重放 WAL 未提交的 tombstone/段增删；半成品 segment
/// （ULID 不在 manifest）判定垃圾并清除。
///
/// 返回按 (collection, ulid) 聚合的 tombstone 位图（绝对 docid，M-minor-2）。
///
/// - **AddTombstone**：仅当 ULID 仍在 manifest 中时聚合到返回 map（否则段已被
///   compact/reindex 清除，tombstone 无意义）。
/// - **AddSegment**：ULID 不在 manifest → 半成品段（manifest 切换前崩溃），
///   `Vfs::delete` 段目录（经 `merge::delete_segment_dir` 递归删）。
/// - **DeleteSegment**：仅信息记录，不动作。若 ULID 仍在 manifest → 合并未完成，
///   保留旧段（恢复到合并前）；若 ULID 已不在 manifest → 已清除，无操作。
///
/// # 返回值偏离 README 契约
///
/// README § 04-wal 契约标注 `recover(...) -> Result<()>`，但 wal 模块不可依赖 api 模块
/// （`CollectionInner` 在 api 内，反向依赖形成环）。故 recover 返回聚合的 tombstone map，
/// 由 `Db::open` 注入。这是层ing 必要的偏离，已在 04-wal-report 记录。
pub fn recover(
    vfs: &Arc<dyn Vfs>,
    db_path: &str,
    manifest: &Manifest,
) -> Result<RecoveredTombstones> {
    let wal = Wal::open(vfs.clone(), db_path)?;
    let records = wal.read_all()?;
    let mut tombstones: RecoveredTombstones = HashMap::new();
    for rec in records {
        match rec {
            WalRecord::AddTombstone {
                collection,
                ulid,
                docids,
            } => {
                // 仅当该 ULID 仍在 manifest 中时才注入（否则段已被 compact/reindex 清除）。
                if !ulid_in_manifest(manifest, &collection, &ulid) {
                    continue;
                }
                let bm = tombstones
                    .entry(collection)
                    .or_default()
                    .entry(ulid)
                    .or_default();
                for d in docids {
                    // roaring 存 u32；与 delete 期 abs as u32 一致（M-minor-2）。
                    if d <= u32::MAX as u64 {
                        bm.insert(d as u32);
                    }
                }
            }
            WalRecord::AddSegment { collection, ulid } => {
                if !ulid_in_manifest(manifest, &collection, &ulid) {
                    // 半成品段：manifest 切换前崩溃，WAL 有记录但 manifest 未切换 → 清理。
                    let seg_dir = format!("{}/segments/seg_{}", db_path, ulid);
                    let _ = crate::merge::delete_segment_dir(vfs.as_ref(), &seg_dir);
                }
            }
            WalRecord::DeleteSegment { .. } => {
                // recover 不动作：合并未完成则保留旧段（manifest 仍指向它）；
                // 已完成则旧段已清除。两种情况均无需操作。
            }
        }
    }
    Ok(tombstones)
}

fn ulid_in_manifest(manifest: &Manifest, collection: &str, ulid: &str) -> bool {
    manifest
        .collections
        .get(collection)
        .map(|m| m.segment_ulids.iter().any(|u| u == ulid))
        .unwrap_or(false)
}
