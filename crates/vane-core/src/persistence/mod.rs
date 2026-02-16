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

    /// 加载 manifest。新库（manifest 不存在）返回 `Ok(None)`；
    /// 损坏返回 `Err(VaneError::Corrupt)`。
    pub fn load(&self) -> Result<Option<Manifest>> {
        let path = self.manifest_path();
        let mut buf = Vec::new();
        let mut tmp = [0u8; 8192];
        let mut off = 0u64;
        loop {
            let n = match self.vfs.read_at(&path, &mut tmp, off) {
                Ok(n) => n,
                Err(VaneError::Io(_)) => return Ok(None), // manifest 不存在（新库）
                Err(e) => return Err(e),
            };
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
            off += n as u64;
        }
        if buf.is_empty() {
            return Ok(None);
        }
        let m: Manifest = serde_json::from_slice(&buf).map_err(|e| {
            VaneError::Corrupt(format!(
                "manifest parse: {} (db={}, op=load manifest; 建议: 检查 manifest.json 完整性或从备份恢复)",
                e, self.db_path
            ))
        })?;
        Ok(Some(m))
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
            VaneError::Corrupt(format!(
                "manifest serialize: {} (db={}, op=save manifest; 建议: 重试或检查磁盘空间)",
                e, self.db_path
            ))
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
            VaneError::NotFound(format!(
                "collection not found: {} (db={}, seg={}, op=add_segment; 建议: 确认 collection 名称正确)",
                collection, self.db_path, ulid
            ))
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
/// 由 api-core 在 add 路径查询 `should_flush()`；触发后调用 `reset()`。
///
/// 双触发：`docs_since_flush >= max_docs` 或 `elapsed >= interval_ms`（先到先触发）。
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

    pub fn reset(&mut self) {
        self.docs_since_flush = 0;
        self.last_flush = web_time::Instant::now();
    }
}
