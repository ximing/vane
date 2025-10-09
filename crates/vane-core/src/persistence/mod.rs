//! 持久化模块（SPEC §6.4 / §7.1）：
//! - Manifest 原子读写（临时文件 → sync → rename，不变量 I-6）
//! - AutoCommitter（计数 + 时间双触发）
//!
//! 本模块不直接读写段内容（段由 04-segment-format 产出），只管 manifest 指针。

use crate::tokenizer::{BuiltinTokenizer, UserDictEntry};
use crate::types::{Result, Schema, TokenizerId, VaneError};
use crate::vfs::Vfs;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

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
}

impl ManifestStore {
    pub fn new(vfs: Arc<dyn Vfs>, db_path: &str) -> Self {
        Self {
            vfs,
            db_path: db_path.to_string(),
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
        let m: Manifest = serde_json::from_slice(&buf)
            .map_err(|e| VaneError::Corrupt(format!("manifest parse: {}", e)))?;
        Ok(Some(m))
    }

    /// SPEC §6.4 原子切换：写临时文件 → sync → rename。
    /// 不变量 I-6：任何崩溃后 manifest 指向完整状态（rename 前崩溃 → 旧 manifest 完好；
    /// rename 是原子操作 → manifest 永远指向完整新状态或完整旧状态）。
    pub fn save_atomic(&self, manifest: &Manifest) -> Result<()> {
        let json = serde_json::to_vec(manifest)
            .map_err(|e| VaneError::Corrupt(format!("manifest serialize: {}", e)))?;
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
    pub fn add_segment(&self, collection: &str, ulid: &str) -> Result<()> {
        let mut m = self.load()?.unwrap_or_else(Manifest::empty);
        let col = m
            .collections
            .get_mut(collection)
            .ok_or_else(|| VaneError::NotFound(format!("collection not found: {}", collection)))?;
        if !col.segment_ulids.contains(&ulid.to_string()) {
            col.segment_ulids.push(ulid.to_string());
        }
        self.save_atomic(&m)
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
    last_flush: std::time::Instant,
}

impl AutoCommitter {
    pub fn new(config: AutoCommitConfig) -> Self {
        Self {
            config,
            docs_since_flush: 0,
            last_flush: std::time::Instant::now(),
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
        self.last_flush = std::time::Instant::now();
    }
}
