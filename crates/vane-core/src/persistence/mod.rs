//! 持久化模块（SPEC §6.4 / §7.1）：
//! - Manifest 原子读写（临时文件 → sync → rename，不变量 I-6）
//! - AutoCommitter（计数 + 时间双触发）
//!
//! 本模块不直接读写段内容（段由 04-segment-format 产出），只管 manifest 指针。

use crate::tokenizer::{BuiltinTokenizer, UserDictEntry};
use crate::types::{Result, Schema, TokenizerId, VaneError};
use crate::vfs::Vfs;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

#[cfg(test)]
mod tests;

/// SPEC §6.2 manifest.json 结构。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub collections: std::collections::HashMap<String, CollectionMeta>,
}

impl Manifest {
    pub fn empty() -> Self {
        Self {
            version: 1,
            collections: std::collections::HashMap::new(),
        }
    }
}

/// 单个 collection 的持久化元数据（SPEC §6.2）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionMeta {
    pub schema: Schema,
    pub tokenizer_kind: BuiltinTokenizer,
    pub tokenizer_id: TokenizerId,
    pub user_dict: Vec<UserDictEntry>,
    pub segment_ulids: Vec<String>,
}

const MANIFEST_FILENAME: &str = "manifest.json";
const MANIFEST_TMP: &str = "manifest.json.tmp";

/// manifest.json 原子读写（SPEC §6.4）。
///
/// 封装 manifest 的加载与原子保存（临时文件 → sync → rename，不变量 I-6）。
/// 通过 Vfs trait 读写，core 不直接使用 std::fs。
pub struct ManifestStore {
    vfs: Arc<dyn Vfs>,
    db_path: String,
    // M4 Phase 4 fix（Bug 1）：序列化 manifest 原子保存。并发 save_atomic 共享固定
    // tmp 路径 `manifest.json.tmp`，delete/create/write_at/sync/rename 交错会互相覆写
    // tmp → manifest 损坏（E_CORRUPT）。save_lock 串行化原子的 manifest 切换；
    // 段文件写仍并发（各自 seg_<ulid>/ 目录，互不冲突）。private 字段，不改 pub API。
    // 共享实例（DbInner 持 Arc<ManifestStore>，CollectionInner 克隆同一 Arc）→ 跨
    // collection / 跨线程的 save_atomic 全部序列化，覆盖同一 db_path 的并发场景。
    save_lock: Mutex<()>,
}

impl ManifestStore {
    pub fn new(vfs: Arc<dyn Vfs>, db_path: &str) -> Self {
        Self {
            vfs,
            db_path: db_path.to_string(),
            save_lock: Mutex::new(()),
        }
    }

    fn manifest_path(&self) -> String {
        format!("{}/{}", self.db_path, MANIFEST_FILENAME)
    }

    fn tmp_path(&self) -> String {
        format!("{}/{}", self.db_path, MANIFEST_TMP)
    }

    /// 加载 manifest。
    ///
    /// 成功解析 `manifest.json` 返回 `Ok(Some)`。真正的新库返回 `Ok(None)`。
    /// 文件缺失但已有 `seg_*` 或非空 `wal.log` 时返回 `Err(Corrupt)`，禁止当成空库。
    /// 可解析的 `manifest.json.tmp` 经 `rename` 提升为 manifest。
    pub fn load(&self) -> Result<Option<Manifest>> {
        let entries = match self.vfs.list(&self.db_path) {
            Ok(entries) => entries,
            Err(e) if io_is_not_found(&e) => Vec::new(),
            Err(e) => return Err(e),
        };
        if entries.iter().any(|name| name == MANIFEST_FILENAME) {
            let buf = read_vfs_all(self.vfs.as_ref(), &self.manifest_path())?;
            // 空字节视同缺失，继续尝试 tmp / 持久数据判定。
            if !buf.is_empty() {
                return Ok(Some(parse_manifest(&buf, &self.db_path)?));
            }
        }
        self.recover_missing_manifest(&entries)
    }

    /// manifest 不在目录里（或读出为空）时的恢复。不删除段，不提升无法解析的 tmp。
    fn recover_missing_manifest(&self, entries: &[String]) -> Result<Option<Manifest>> {
        if entries.iter().any(|name| name == MANIFEST_TMP) {
            let buf = read_vfs_all(self.vfs.as_ref(), &self.tmp_path())?;
            if !buf.is_empty() {
                if let Ok(manifest) = serde_json::from_slice::<Manifest>(&buf) {
                    // 提升后 recover 会删掉 manifest 未列出的 seg_*。tmp 若是更旧的
                    // 空 manifest，提升等于授权清段。盖不住现有段就拒绝打开。
                    if !self.tmp_covers_on_disk_segments(&manifest)? {
                        return Err(self.missing_manifest_corrupt());
                    }
                    self.vfs.rename(&self.tmp_path(), &self.manifest_path())?;
                    return Ok(Some(manifest));
                }
                if self.has_durable_data()? {
                    return Err(self.missing_manifest_corrupt());
                }
                return Ok(None);
            }
        }
        if self.has_durable_data()? {
            Err(self.missing_manifest_corrupt())
        } else {
            Ok(None)
        }
    }

    /// tmp 里的 manifest 是否列出了磁盘上每一个 `seg_*`。没有段目录时为 true。
    fn tmp_covers_on_disk_segments(&self, manifest: &Manifest) -> Result<bool> {
        let seg_dir = format!("{}/segments", self.db_path);
        let entries = match self.vfs.list(&seg_dir) {
            Ok(entries) => entries,
            Err(e) if io_is_not_found(&e) => return Ok(true),
            Err(e) => return Err(e),
        };
        let listed: std::collections::HashSet<&str> = manifest
            .collections
            .values()
            .flat_map(|col| col.segment_ulids.iter().map(String::as_str))
            .collect();
        Ok(entries.iter().all(|name| match name.strip_prefix("seg_") {
            Some(ulid) => listed.contains(ulid),
            None => true,
        }))
    }

    /// `<db>/segments` 下有 `seg_*`，或 `wal.log` 至少能读到 1 字节。
    fn has_durable_data(&self) -> Result<bool> {
        let seg_dir = format!("{}/segments", self.db_path);
        match self.vfs.list(&seg_dir) {
            Ok(entries) => {
                if entries.iter().any(|name| name.starts_with("seg_")) {
                    return Ok(true);
                }
            }
            Err(e) if io_is_not_found(&e) => {}
            Err(e) => return Err(e),
        }
        let wal_path = format!("{}/wal.log", self.db_path);
        let mut byte = [0u8; 1];
        match self.vfs.read_at(&wal_path, &mut byte, 0) {
            Ok(n) if n >= 1 => Ok(true),
            Ok(_) => Ok(false),
            Err(e) if io_is_not_found(&e) => Ok(false),
            Err(e) => Err(e),
        }
    }

    fn missing_manifest_corrupt(&self) -> VaneError {
        VaneError::Corrupt(
            crate::types::ErrorContext::new(format!(
                "manifest.json missing with durable data (db={})",
                self.db_path
            ))
            .op("load manifest")
            .hint("不要删除已有段，请从 export 备份恢复"),
        )
    }

    /// SPEC §6.4 原子切换：写临时文件 → sync → rename。
    /// 不变量 I-6：任何崩溃后 manifest 指向完整状态（rename 前崩溃 → 旧 manifest 完好；
    /// rename 是原子操作 → manifest 永远指向完整新状态或完整旧状态）。
    ///
    /// M4 Phase 4 fix（Bug 1）：入口获取 save_lock，串行化并发原子切换。并发 save_atomic
    /// 共享固定 tmp 路径 `manifest.json.tmp`，无锁则 delete/create/write_at/sync/rename
    /// 交错覆写 → manifest 损坏（E_CORRUPT）。段文件写仍并发（各自 seg_<ulid>/ 目录）。
    pub fn save_atomic(&self, manifest: &Manifest) -> Result<()> {
        let _save_guard = self.save_lock.lock().unwrap();
        self.save_atomic_locked(manifest)
    }

    /// save_atomic 的落盘实现——调用者**必须**已持有 save_lock。
    ///
    /// 拆出私有方法是为了让 [`add_segment`] / [`update`] 在持锁的 load-modify-save
    /// 事务中复用落盘逻辑而不重入 save_lock（std::sync::Mutex 不可重入，重入死锁）。
    fn save_atomic_locked(&self, manifest: &Manifest) -> Result<()> {
        let json = serde_json::to_vec(manifest).map_err(|e| {
            VaneError::Corrupt(
                crate::types::ErrorContext::new(format!(
                    "manifest serialize: {} (db={})",
                    e, self.db_path
                ))
                .op("save manifest")
                .hint("重试或检查磁盘空间"),
            )
        })?;
        let tmp = self.tmp_path();
        let target = self.manifest_path();
        // I16 裁决：先清理可能残留的 tmp（忽略错误，tmp 可能不存在），处理上次崩溃残留。
        let _ = self.vfs.delete(&tmp);
        self.vfs.create(&tmp)?;
        self.vfs.write_at(&tmp, &json, 0)?;
        self.vfs.sync(&tmp)?;
        // 原子 rename 覆盖旧 manifest（MemoryVfs 直接覆盖；StdFsVfs 的 rename 落盘原子）。
        self.vfs.rename(&tmp, &target)?;
        Ok(())
    }

    /// 在指定 collection 的 segment_ulids 中追加一个 ULID（去重），并原子保存。
    ///
    /// M4 Phase 4 fix（Bug 1）：load-modify-save 须在同一 save_lock 内完成，否则
    /// 并发 add_segment 的 load 互相看不到对方未保存的修改 → lost-update（一方段 ULID 丢失）。
    pub fn add_segment(&self, collection: &str, ulid: &str) -> Result<()> {
        let _save_guard = self.save_lock.lock().unwrap();
        let mut m = self.load()?.unwrap_or_else(Manifest::empty);
        let col = m.collections.get_mut(collection).ok_or_else(|| {
            VaneError::NotFound(
                crate::types::ErrorContext::new(format!(
                    "collection not found: {} (db={})",
                    collection, self.db_path
                ))
                .seg(ulid)
                .op("add_segment")
                .hint("确认 collection 名称正确"),
            )
        })?;
        if !col.segment_ulids.contains(&ulid.to_string()) {
            col.segment_ulids.push(ulid.to_string());
        }
        self.save_atomic_locked(&m)
    }

    /// 在 save_lock 保护下执行 load→f→save 原子事务（M4 Phase 4 fix，Bug 1）。
    ///
    /// 用于 `merge_segments` / `update_manifest_after_reindex` / `Db::collection` 等
    /// 需要自定义 load-modify-save 序列的路径：整个事务在持锁期间完成，杜绝并发
    /// lost-update 与 tmp 覆盖。`f` 修改 manifest（并可在此期间做 WAL append——
    /// WAL → manifest 的 §6.4 顺序保持）。`pub(crate)`：同 crate 内调用，不扩 pub API。
    pub(crate) fn update<F: FnOnce(&mut Manifest) -> Result<()>>(&self, f: F) -> Result<()> {
        let _save_guard = self.save_lock.lock().unwrap();
        let mut m = self.load()?.unwrap_or_else(Manifest::empty);
        f(&mut m)?;
        self.save_atomic_locked(&m)
    }
}

fn read_vfs_all(vfs: &dyn Vfs, path: &str) -> Result<Vec<u8>> {
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

fn parse_manifest(buf: &[u8], db_path: &str) -> Result<Manifest> {
    serde_json::from_slice(buf).map_err(|e| {
        VaneError::Corrupt(
            crate::types::ErrorContext::new(format!("manifest parse: {} (db={})", e, db_path))
                .op("load manifest")
                .hint("检查 manifest.json 完整性或从备份恢复"),
        )
    })
}

/// list/read 失败时，只有看起来像「文件不存在」的 Io 才当成缺失。
fn io_is_not_found(err: &VaneError) -> bool {
    crate::vfs::io_error_is_not_found(err)
}

/// SPEC §7.1 auto-commit 配置。默认 `On { interval_ms=1000, max_docs=1000 }`。
#[derive(Debug, Clone)]
pub enum AutoCommitConfig {
    Off,
    On { interval_ms: u32, max_docs: u32 },
}

impl Default for AutoCommitConfig {
    fn default() -> Self {
        AutoCommitConfig::On {
            interval_ms: 1000,
            max_docs: 1000,
        }
    }
}

/// SPEC §7.1 auto-commit 触发器：无状态计数器 + 时间戳。
/// `add()` 查 `should_flush()`（`max_docs` 或 interval 已到）。
/// 后台线程查 `interval_delay()`，到点再 flush，不靠下一次 `add()`。
/// 触发 flush 后调用 `reset()`。
pub struct AutoCommitter {
    config: AutoCommitConfig,
    docs_since_flush: u32,
    last_flush: web_time::Instant,
}

impl AutoCommitter {
    pub fn new(config: AutoCommitConfig) -> Self {
        Self {
            config,
            docs_since_flush: 0,
            last_flush: web_time::Instant::now(),
        }
    }

    pub fn record_docs(&mut self, n: u32) {
        self.docs_since_flush = self.docs_since_flush.saturating_add(n);
    }

    pub fn should_flush(&self) -> bool {
        match &self.config {
            AutoCommitConfig::Off => false,
            AutoCommitConfig::On {
                interval_ms,
                max_docs,
            } => {
                if self.docs_since_flush == 0 {
                    return false;
                }
                if self.docs_since_flush >= *max_docs {
                    return true;
                }
                let elapsed = self.last_flush.elapsed().as_millis() as u32;
                elapsed >= *interval_ms
            }
        }
    }

    /// 距 interval 触发还要多久。
    ///
    /// `None`：Off，或 buffer 里没有文档。`Some(0)`：interval 已到。
    /// 不看 `max_docs`。那条仍由 `add()` 里的 `should_flush()` 处理，
    /// 避免 flush 失败后后台线程立刻重试、把一次性故障吃掉。
    pub fn interval_delay(&self) -> Option<std::time::Duration> {
        match &self.config {
            AutoCommitConfig::Off => None,
            AutoCommitConfig::On { interval_ms, .. } => {
                if self.docs_since_flush == 0 {
                    return None;
                }
                let interval = std::time::Duration::from_millis(u64::from(*interval_ms));
                let elapsed = self.last_flush.elapsed();
                if elapsed >= interval {
                    Some(std::time::Duration::ZERO)
                } else {
                    Some(interval.saturating_sub(elapsed))
                }
            }
        }
    }

    pub fn reset(&mut self) {
        self.docs_since_flush = 0;
        self.last_flush = web_time::Instant::now();
    }
}
