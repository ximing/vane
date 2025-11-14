//! MemOverlay 内核——后端无关的虚拟 FS overlay（M2-02 §3，路径 A）。
//!
//! 持有 `Arc<dyn OverlayBackend>`（字节 IO 抽象），在其上构建虚拟文件表：
//! 虚拟路径 `<db>/segments/seg_<ulid>/...` 映射到容器内 `(base, size)` 区间。
//! 文件表 + free list + 双 meta_slot + CRC 实现原子元数据切换（I-6 等价）。
//!
//! 后端无关：`MemoryBackend`（内存 Vec，测试用）与 `OpfsBackend`（SyncAccessHandle，
//! feature-gated）各 impl `OverlayBackend` 一次。M2-03 IdbVfs 复用本内核。

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use vane_core::types::{Result, VaneError};
use vane_core::vfs::Vfs;

use super::container::{
    meta_slot_offset, Extent, MetaSlot, Superblock, DATA_OFFSET, MAGIC, META_SLOT_SIZE,
};

// crc32 仅在 container.rs 内部使用，此处不需要。

/// RwLock poison 错误 → VaneError::Io。
fn poison_err<T>(_: std::sync::PoisonError<T>) -> VaneError {
    VaneError::Io("overlay state lock poisoned".into())
}

// ── OverlayBackend trait ───────────────────────────────────────────────────

/// 后端无关的字节 IO 抽象（M2-02 §3 Produces for）。
/// OpfsBackend（SyncAccessHandle）和 MemoryBackend（内存 Vec）各 impl 一次。
pub trait OverlayBackend: Send + Sync {
    /// 从 `off` 读到 `buf`，返回实际读取字节数。
    fn read(&self, off: u64, buf: &mut [u8]) -> Result<usize>;
    /// 把 `buf` 写到 `off`。
    fn write(&self, off: u64, buf: &[u8]) -> Result<()>;
    /// 刷盘（保证已写数据落盘）。
    fn flush(&self) -> Result<()>;
    /// 返回后端当前大小。
    fn size(&self) -> Result<u64>;
    /// 截断/扩展后端到 `sz`。
    fn truncate(&self, sz: u64) -> Result<()>;
}

// ── MemoryBackend（测试用，纯内存 Vec）──────────────────────────────────────

/// 内存后端（测试用）。后端数据为 `Vec<u8>`，flush 为 no-op。
/// 支持快照 / 截断 / 损坏注入用于崩溃恢复测试。
pub struct MemoryBackend {
    data: RwLock<Vec<u8>>,
}

impl MemoryBackend {
    pub fn new() -> Self {
        Self {
            data: RwLock::new(Vec::new()),
        }
    }

    /// 返回当前数据的完整快照（用于 round-trip / 崩溃模拟）。
    pub fn snapshot(&self) -> Vec<u8> {
        self.data.read().unwrap().clone()
    }

    /// 从快照恢复（用于跨 session 测试）。
    pub fn restore(&self, data: Vec<u8>) {
        *self.data.write().unwrap() = data;
    }

    /// 截断后端数据到 `sz`（模拟写一半崩溃：尾部数据丢失）。
    pub fn truncate_data(&self, sz: usize) {
        self.data.write().unwrap().truncate(sz);
    }

    /// 翻转指定偏移的一个字节（模拟 CRC 损坏）。
    pub fn corrupt_byte(&self, off: usize) {
        let mut d = self.data.write().unwrap();
        if off < d.len() {
            d[off] ^= 0xFF;
        }
    }
}

impl Default for MemoryBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl OverlayBackend for MemoryBackend {
    fn read(&self, off: u64, buf: &mut [u8]) -> Result<usize> {
        let d = self.data.read().unwrap();
        let off = off as usize;
        if off >= d.len() {
            return Ok(0);
        }
        let n = buf.len().min(d.len() - off);
        buf[..n].copy_from_slice(&d[off..off + n]);
        Ok(n)
    }

    fn write(&self, off: u64, buf: &[u8]) -> Result<()> {
        let mut d = self.data.write().unwrap();
        let off = off as usize;
        let end = off + buf.len();
        if d.len() < end {
            d.resize(end, 0);
        }
        d[off..end].copy_from_slice(buf);
        Ok(())
    }

    fn flush(&self) -> Result<()> {
        Ok(())
    }

    fn size(&self) -> Result<u64> {
        Ok(self.data.read().unwrap().len() as u64)
    }

    fn truncate(&self, sz: u64) -> Result<()> {
        self.data.write().unwrap().resize(sz as usize, 0);
        Ok(())
    }
}

// ── OverlayState（内部可变状态）─────────────────────────────────────────────

struct OverlayState {
    file_table: HashMap<String, Extent>,
    free_list: Vec<Extent>,
    container_size: u64,
    active_meta_slot: u8,
    generation: u64,
    /// 有未持久化的文件表变更（create/write_at/append 后置 true，persist 后 false）。
    dirty: bool,
}

// ── MemOverlay ─────────────────────────────────────────────────────────────

/// 虚拟 FS overlay：单容器 + 内存文件表 + 双 meta_slot + CRC + free list。
///
/// 实现 `Vfs` trait 的全部 8 方法（同步）。后端无关——通过 `OverlayBackend`
/// 抽象字节 IO，`OpfsBackend`/`MemoryBackend`/`IdbBackend` 各 impl 一次。
pub struct MemOverlay {
    backend: Arc<dyn OverlayBackend>,
    state: RwLock<OverlayState>,
}

impl MemOverlay {
    /// 打开 overlay：读 superblock + 双 meta_slot，取 generation 最大且 CRC 通过者
    /// 重建文件表。新库（backend size == 0）初始化空容器。
    pub fn open(backend: Arc<dyn OverlayBackend>) -> Result<Self> {
        let bsize = backend.size()?;
        if bsize == 0 {
            return Self::init_new(backend);
        }
        Self::recover(backend)
    }

    fn init_new(backend: Arc<dyn OverlayBackend>) -> Result<Self> {
        let state = OverlayState {
            file_table: HashMap::new(),
            free_list: Vec::new(),
            container_size: DATA_OFFSET,
            active_meta_slot: 0,
            generation: 0,
            dirty: false,
        };
        let overlay = Self {
            backend,
            state: RwLock::new(state),
        };
        // 直接写空 meta_slot_0 (generation=0) + superblock + flush
        // 不走 persist_meta（那会递增 generation 到 1 并写 slot 1）。
        let meta = MetaSlot {
            generation: 0,
            container_size: DATA_OFFSET,
            file_table: vec![],
            free_list: vec![],
        };
        let encoded = meta.encode()?;
        overlay.backend.write(meta_slot_offset(0), &encoded)?;
        let sb = Superblock {
            active_meta_slot: 0,
            container_size: DATA_OFFSET,
        };
        overlay.backend.write(0, &sb.encode())?;
        overlay.backend.flush()?;
        Ok(overlay)
    }

    fn recover(backend: Arc<dyn OverlayBackend>) -> Result<Self> {
        // 尝试读 superblock（仅用于 magic 校验；active_meta_slot 仅为 hint）。
        let _sb_ok = {
            let mut sb_buf = vec![0u8; 64];
            let n = backend.read(0, &mut sb_buf)?;
            if n >= 8 {
                &sb_buf[0..4] == MAGIC
            } else {
                false
            }
        };
        // superblock 损坏时仍尝试读双 meta_slot——meta_slot 自带 CRC + generation，
        // 不依赖 superblock 正确性（聚焦复核 item 2：superblock 自损坏恢复）。

        // 读双 meta slot
        let meta0 = read_meta_slot(backend.as_ref(), 0)?;
        let meta1 = read_meta_slot(backend.as_ref(), 1)?;

        let (active, meta) = match (meta0, meta1) {
            (Some(m0), Some(m1)) => {
                if m0.generation >= m1.generation {
                    (0u8, m0)
                } else {
                    (1u8, m1)
                }
            }
            (Some(m0), None) => (0u8, m0),
            (None, Some(m1)) => (1u8, m1),
            (None, None) => {
                if !_sb_ok {
                    return Err(VaneError::Io(
                        "container corrupt: bad magic and no valid meta slot".into(),
                    ));
                }
                return Err(VaneError::Io(
                    "container corrupt: both meta slots invalid (CRC failure)".into(),
                ));
            }
        };

        let file_table: HashMap<String, Extent> = meta.file_table.into_iter().collect();
        let state = OverlayState {
            file_table,
            free_list: meta.free_list,
            container_size: meta.container_size,
            active_meta_slot: active,
            generation: meta.generation,
            dirty: false,
        };
        Ok(Self {
            backend,
            state: RwLock::new(state),
        })
    }

    // ── 非-Vfs 公开方法 ──────────────────────────────────────────────────

    /// 当前活跃 meta slot（测试 / 调试用）。
    pub fn active_meta_slot(&self) -> u8 {
        self.state.read().unwrap().active_meta_slot
    }

    /// 当前 generation（测试用）。
    pub fn generation(&self) -> u64 {
        self.state.read().unwrap().generation
    }

    /// container_size（测试用）。
    pub fn container_size(&self) -> u64 {
        self.state.read().unwrap().container_size
    }

    /// free list 快照（测试用）。
    pub fn free_list_snapshot(&self) -> Vec<Extent> {
        self.state.read().unwrap().free_list.clone()
    }

    /// 文件表快照（测试用）。
    pub fn file_table_snapshot(&self) -> Vec<(String, Extent)> {
        self.state
            .read()
            .unwrap()
            .file_table
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect()
    }

    /// 全量 rewrite compaction：读所有活跃区间 → 重写到紧凑数据区 → 回收碎片。
    /// core 不感知。触发条件：delete 后 free_space / total > 50%（`maybe_compact`）。
    /// 全量 rewrite compaction（shadow-write 模式，崩溃安全）。
    ///
    /// 写 compaction 产物到**容器尾部新区域**（旧数据暂不破坏），persist_meta
    /// 原子提交后旧区域进 free list。若崩溃在 persist 前/中 → 旧 meta 仍 active、
    /// 旧数据完整（未被覆盖）→ recover 回旧状态；若崩溃在 persist 后 → 新 meta
    /// active、新数据完整。两路径都安全（M-1 修复）。
    ///
    /// core 不感知。触发条件：delete 后 free_space / total > 50%（内嵌于 `delete`）。
    pub fn compact(&self) -> Result<()> {
        let mut state = self.state.write().map_err(poison_err)?;
        self.compact_internal(&mut state)?;
        Ok(())
    }

    /// compaction 内核（shadow-write + persist_meta，自带持久化）。
    /// 调用方无需再调 persist_meta。
    fn compact_internal(&self, state: &mut OverlayState) -> Result<()> {
        let old_cs = state.container_size;

        // 1. 读所有活跃文件数据到内存（备份——旧数据不被破坏）
        let mut live: Vec<(String, Vec<u8>)> = Vec::new();
        for (path, ext) in &state.file_table {
            if ext.size > 0 {
                let mut buf = vec![0u8; ext.size as usize];
                let n = self.backend.read(ext.base, &mut buf)?;
                if n != ext.size as usize {
                    return Err(VaneError::Io(format!(
                        "compaction read short: path={}, expected={}, got={}",
                        path, ext.size, n
                    )));
                }
                live.push((path.clone(), buf));
            } else {
                live.push((path.clone(), Vec::new()));
            }
        }

        // 2. shadow-write：写紧凑数据到容器尾部新区域 [old_cs, old_cs + live_size)
        //    旧数据区 [DATA_OFFSET, old_cs) 暂不破坏——崩溃安全的关键。
        let mut new_base = old_cs;
        let mut new_table: HashMap<String, Extent> = HashMap::new();
        for (path, data) in &live {
            if data.is_empty() {
                new_table.insert(path.clone(), Extent { base: 0, size: 0 });
            } else {
                self.backend.write(new_base, data)?;
                new_table.insert(
                    path.clone(),
                    Extent {
                        base: new_base,
                        size: data.len() as u64,
                    },
                );
                new_base += data.len() as u64;
            }
        }

        // 3. 更新 state：新 file_table 指向尾部新区域，旧区域进 free list
        state.file_table = new_table;
        state.free_list = vec![Extent {
            base: DATA_OFFSET,
            size: old_cs.saturating_sub(DATA_OFFSET),
        }];
        state.container_size = new_base;
        state.dirty = true;

        // 4. 原子提交（双 meta_slot + CRC + 翻转 + flush）
        //    崩溃在此之前 → 旧 meta active、旧数据完整 → recover 回旧状态。
        //    崩溃在此之后 → 新 meta active、新数据在尾部完整 → recover 到新状态。
        persist_meta(self.backend.as_ref(), state)?;
        Ok(())
    }

    /// 返回 backend 的 Arc 引用（测试用——例如直接操作后端模拟崩溃）。
    pub fn backend(&self) -> &Arc<dyn OverlayBackend> {
        &self.backend
    }

    // ── 内部辅助 ────────────────────────────────────────────────────────

    /// first-fit 分配：优先复用 free list 中 >= size 的空洞，否则在 container 尾部分配。
    fn allocate(state: &mut OverlayState, size: u64) -> u64 {
        if let Some(idx) = state.free_list.iter().position(|e| e.size >= size) {
            let ext = state.free_list.remove(idx);
            let base = ext.base;
            if ext.size > size {
                state.free_list.push(Extent {
                    base: base + size,
                    size: ext.size - size,
                });
            }
            base
        } else {
            let base = state.container_size;
            state.container_size += size;
            base
        }
    }
}

// ── Vfs trait impl ─────────────────────────────────────────────────────────

impl Vfs for MemOverlay {
    fn create(&self, path: &str) -> Result<()> {
        let mut state = self.state.write().map_err(poison_err)?;
        if state.file_table.contains_key(path) {
            return Err(VaneError::Io(format!("file already exists: {}", path)));
        }
        state
            .file_table
            .insert(path.to_string(), Extent { base: 0, size: 0 });
        state.dirty = true;
        Ok(())
    }

    fn read_at(&self, path: &str, buf: &mut [u8], offset: u64) -> Result<usize> {
        let ext = {
            let state = self.state.read().map_err(poison_err)?;
            state
                .file_table
                .get(path)
                .copied()
                .ok_or_else(|| VaneError::Io(format!("file not found: {}", path)))?
        };
        if offset >= ext.size {
            return Ok(0);
        }
        let avail = ext.size - offset;
        let n = (buf.len() as u64).min(avail) as usize;
        self.backend.read(ext.base + offset, &mut buf[..n])
    }

    fn write_at(&self, path: &str, buf: &[u8], offset: u64) -> Result<()> {
        let mut state = self.state.write().map_err(poison_err)?;
        let ext = state
            .file_table
            .get(path)
            .copied()
            .ok_or_else(|| VaneError::Io(format!("file not found: {}", path)))?;

        let new_end = offset + buf.len() as u64;

        if new_end <= ext.size {
            // 原地覆盖
            self.backend.write(ext.base + offset, buf)?;
        } else if ext.size == 0 {
            // 首次写：分配新区间
            let base = Self::allocate(&mut state, new_end);
            if offset > 0 {
                let zeros = vec![0u8; offset as usize];
                self.backend.write(base, &zeros)?;
            }
            self.backend.write(base + offset, buf)?;
            state.file_table.insert(
                path.to_string(),
                Extent {
                    base,
                    size: new_end,
                },
            );
        } else if ext.base + ext.size == state.container_size {
            // 尾部文件：原地扩展
            if offset > ext.size {
                let gap = (offset - ext.size) as usize;
                let zeros = vec![0u8; gap];
                self.backend.write(ext.base + ext.size, &zeros)?;
            }
            self.backend.write(ext.base + offset, buf)?;
            state.container_size += new_end - ext.size;
            state.file_table.insert(
                path.to_string(),
                Extent {
                    base: ext.base,
                    size: new_end,
                },
            );
        } else {
            // 重定位：分配新区间，拷贝旧数据，释放旧区间
            let new_base = Self::allocate(&mut state, new_end);
            if ext.size > 0 {
                let mut old_data = vec![0u8; ext.size as usize];
                self.backend.read(ext.base, &mut old_data)?;
                self.backend.write(new_base, &old_data)?;
            }
            if offset > ext.size {
                let gap = (offset - ext.size) as usize;
                let zeros = vec![0u8; gap];
                self.backend.write(new_base + ext.size, &zeros)?;
            }
            self.backend.write(new_base + offset, buf)?;
            if ext.size > 0 {
                state.free_list.push(ext);
            }
            state.file_table.insert(
                path.to_string(),
                Extent {
                    base: new_base,
                    size: new_end,
                },
            );
        }

        state.dirty = true;
        Ok(())
    }

    fn append(&self, path: &str, buf: &[u8]) -> Result<u64> {
        let mut state = self.state.write().map_err(poison_err)?;
        let ext = state
            .file_table
            .get(path)
            .copied()
            .ok_or_else(|| VaneError::Io(format!("file not found: {}", path)))?;

        let old_size = ext.size;
        let buf_len = buf.len() as u64;

        if ext.size == 0 {
            // 首次写：分配
            let base = Self::allocate(&mut state, buf_len);
            self.backend.write(base, buf)?;
            state.file_table.insert(
                path.to_string(),
                Extent {
                    base,
                    size: buf_len,
                },
            );
        } else if ext.base + ext.size == state.container_size {
            // 尾部：原地扩展
            self.backend.write(ext.base + ext.size, buf)?;
            state.container_size += buf_len;
            state.file_table.insert(
                path.to_string(),
                Extent {
                    base: ext.base,
                    size: old_size + buf_len,
                },
            );
        } else {
            // 重定位
            let new_size = old_size + buf_len;
            let new_base = Self::allocate(&mut state, new_size);
            let mut old_data = vec![0u8; old_size as usize];
            self.backend.read(ext.base, &mut old_data)?;
            self.backend.write(new_base, &old_data)?;
            self.backend.write(new_base + old_size, buf)?;
            state.free_list.push(ext);
            state.file_table.insert(
                path.to_string(),
                Extent {
                    base: new_base,
                    size: new_size,
                },
            );
        }

        state.dirty = true;
        Ok(old_size)
    }

    fn sync(&self, _path: &str) -> Result<()> {
        {
            let mut state = self.state.write().map_err(poison_err)?;
            if state.dirty {
                persist_meta(self.backend.as_ref(), &mut state)?;
            }
        }
        self.backend.flush()?;
        Ok(())
    }

    fn rename(&self, from: &str, to: &str) -> Result<()> {
        let mut state = self.state.write().map_err(poison_err)?;
        let from_ext = state
            .file_table
            .remove(from)
            .ok_or_else(|| VaneError::Io(format!("file not found: {}", from)))?;
        // 释放目标的旧区间（若存在）
        if let Some(to_ext) = state.file_table.remove(to) {
            if to_ext.size > 0 {
                state.free_list.push(to_ext);
            }
        }
        state.file_table.insert(to.to_string(), from_ext);
        persist_meta(self.backend.as_ref(), &mut state)?;
        Ok(())
    }

    fn delete(&self, path: &str) -> Result<()> {
        let mut state = self.state.write().map_err(poison_err)?;
        let ext = state
            .file_table
            .remove(path)
            .ok_or_else(|| VaneError::Io(format!("file not found: {}", path)))?;
        if ext.size > 0 {
            state.free_list.push(ext);
        }
        // compaction 触发条件：free_space / (container_size - DATA_OFFSET) > 50%
        let free_space: u64 = state.free_list.iter().map(|e| e.size).sum();
        let total = state.container_size.saturating_sub(DATA_OFFSET);
        if total > 0 && (free_space as f64 / total as f64) > 0.5 {
            // compact_internal 自带 persist_meta（shadow-write + 原子提交）
            self.compact_internal(&mut state)?;
        } else {
            persist_meta(self.backend.as_ref(), &mut state)?;
        }
        Ok(())
    }

    fn list(&self, dir: &str) -> Result<Vec<String>> {
        let state = self.state.read().map_err(poison_err)?;
        let prefix = if dir == "." || dir.is_empty() {
            String::new()
        } else {
            format!("{}/", dir.trim_end_matches('/'))
        };
        let mut out: Vec<String> = state
            .file_table
            .keys()
            .filter(|path| path.starts_with(&prefix))
            .filter_map(|path| {
                path[prefix.len()..]
                    .split('/')
                    .next()
                    .map(|s| s.to_string())
            })
            .collect();
        out.sort();
        out.dedup();
        Ok(out)
    }
}

// ── 持久化辅助函数 ──────────────────────────────────────────────────────────

/// 读取 meta slot，返回 `Ok(None)` 表示无有效数据（空槽 / CRC 失败 / 不完整）。
fn read_meta_slot(backend: &dyn OverlayBackend, slot: u8) -> Result<Option<MetaSlot>> {
    let offset = meta_slot_offset(slot);
    // 先读 header（16 字节）
    let mut header = [0u8; 16];
    let n = backend.read(offset, &mut header)?;
    if n < 16 {
        return Ok(None);
    }
    let data_len = u32::from_le_bytes(header[8..12].try_into().unwrap()) as usize;
    if data_len == 0 || data_len + 16 > META_SLOT_SIZE as usize {
        return Ok(None);
    }
    // 读 payload
    let mut buf = vec![0u8; 16 + data_len];
    buf[..16].copy_from_slice(&header);
    let n2 = backend.read(offset + 16, &mut buf[16..])?;
    if n2 < data_len {
        return Ok(None);
    }
    MetaSlot::decode_from(&buf)
}

/// 持久化文件表到非活跃 meta_slot + CRC + 翻转 active + 写 superblock + flush。
/// 写完后 in-memory state 与持久化状态一致，dirty = false。
fn persist_meta(backend: &dyn OverlayBackend, state: &mut OverlayState) -> Result<()> {
    let inactive = 1 - state.active_meta_slot;
    let new_generation = state.generation + 1;

    let meta = MetaSlot {
        generation: new_generation,
        container_size: state.container_size,
        file_table: state
            .file_table
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect(),
        free_list: state.free_list.clone(),
    };
    let encoded = meta.encode()?;

    // 1. 写非活跃 meta slot
    let offset = meta_slot_offset(inactive);
    backend.write(offset, &encoded)?;
    // 2. flush（meta 落盘后再翻转）
    backend.flush()?;

    // 3. 更新 in-memory state
    state.active_meta_slot = inactive;
    state.generation = new_generation;
    state.dirty = false;

    // 4. 写 superblock（active_meta_slot hint + container_size hint）
    let sb = Superblock {
        active_meta_slot: state.active_meta_slot,
        container_size: state.container_size,
    };
    let sb_encoded = sb.encode();
    backend.write(0, &sb_encoded)?;
    backend.flush()?;

    Ok(())
}

// ── 崩溃恢复辅助（测试用）──────────────────────────────────────────────────

/// 损坏指定 meta slot（翻转 payload 字节，使 CRC 失败）。仅测试用。
#[cfg(test)]
pub(crate) fn corrupt_meta_slot(backend: &dyn OverlayBackend, slot: u8) {
    let offset = meta_slot_offset(slot);
    // 读取 payload 区第一个字节（header 之后），翻转后写回
    let mut byte = [0u8; 1];
    if backend.read(offset + 20, &mut byte).unwrap_or(0) > 0 {
        byte[0] ^= 0xFF;
        let _ = backend.write(offset + 20, &byte);
    }
}

// ══════════════════════════════════════════════════════════════════════════
// 测试
// ══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use vane_core::vfs::Vfs;

    // ── 辅助 ────────────────────────────────────────────────────────────

    fn new_overlay() -> (MemOverlay, Arc<MemoryBackend>) {
        let backend = Arc::new(MemoryBackend::new());
        let overlay = MemOverlay::open(backend.clone()).unwrap();
        (overlay, backend)
    }

    /// 通用 Vfs 契约测试（与 vane-core/tests.rs::run_conformance_tests 同构）。
    fn run_conformance<V: Vfs>(vfs: &V) {
        // create + write_at + read_at
        vfs.create("a.bin").unwrap();
        vfs.write_at("a.bin", b"hello", 0).unwrap();
        let mut buf = [0u8; 5];
        let n = vfs.read_at("a.bin", &mut buf, 0).unwrap();
        assert_eq!(n, 5);
        assert_eq!(&buf, b"hello");

        // append 返回起始 offset
        let off = vfs.append("a.bin", b" world").unwrap();
        assert_eq!(off, 5);
        let mut buf2 = [0u8; 11];
        vfs.read_at("a.bin", &mut buf2, 0).unwrap();
        assert_eq!(&buf2, b"hello world");

        // write_at 覆盖
        vfs.write_at("a.bin", b"HELLO", 0).unwrap();
        let mut buf3 = [0u8; 5];
        vfs.read_at("a.bin", &mut buf3, 0).unwrap();
        assert_eq!(&buf3, b"HELLO");

        // list
        vfs.create("b.bin").unwrap();
        let files = vfs.list(".").unwrap();
        assert!(files.contains(&"a.bin".to_string()));
        assert!(files.contains(&"b.bin".to_string()));

        // rename 原子覆盖
        vfs.create("c.bin").unwrap();
        vfs.write_at("c.bin", b"replaced", 0).unwrap();
        vfs.rename("a.bin", "c.bin").unwrap();
        let mut buf4 = [0u8; 8];
        vfs.read_at("c.bin", &mut buf4, 0).unwrap();
        assert_eq!(&buf4, b"HELLO wo"); // a.bin 的前 8 字节覆盖 c.bin

        // delete
        vfs.delete("c.bin").unwrap();
        assert!(vfs.read_at("c.bin", &mut [0u8; 1], 0).is_err());

        // read 不存在文件报错
        assert!(vfs.read_at("nonexistent", &mut [0u8; 1], 0).is_err());

        // list 按 dir 过滤
        vfs.create("sub/x.bin").unwrap();
        vfs.create("sub/y.bin").unwrap();
        let sub_files = vfs.list("sub").unwrap();
        assert!(sub_files.contains(&"x.bin".to_string()));
        assert!(sub_files.contains(&"y.bin".to_string()));
        let root_files = vfs.list(".").unwrap();
        assert!(!root_files
            .iter()
            .any(|f| f.contains("x.bin") && f.contains("sub")));
    }

    // ── 门禁 8: Vfs 语义测试 ───────────────────────────────────────────

    #[test]
    fn overlay_vfs_conformance() {
        let (overlay, _) = new_overlay();
        run_conformance(&overlay);
    }

    #[test]
    fn create_empty_file_read_returns_zero() {
        // T2: create(path) 后 read_at(path, .., 0) 返回 0 字节
        let (overlay, _) = new_overlay();
        overlay.create("empty.bin").unwrap();
        let mut buf = [0u8; 10];
        let n = overlay.read_at("empty.bin", &mut buf, 0).unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn write_at_and_read_at_roundtrip() {
        // T3: write_at + read_at 回读一致
        let (overlay, _) = new_overlay();
        overlay.create("f.bin").unwrap();
        overlay.write_at("f.bin", b"hello world", 0).unwrap();
        let mut buf = [0u8; 11];
        let n = overlay.read_at("f.bin", &mut buf, 0).unwrap();
        assert_eq!(n, 11);
        assert_eq!(&buf, b"hello world");
    }

    #[test]
    fn append_twice_offsets_and_size() {
        // T4: append 两次 → 文件长度 = 两段之和，返回起始 offset
        let (overlay, _) = new_overlay();
        overlay.create("log.bin").unwrap();
        let off1 = overlay.append("log.bin", b"AAAA").unwrap();
        assert_eq!(off1, 0);
        let off2 = overlay.append("log.bin", b"BBBB").unwrap();
        assert_eq!(off2, 4);
        let mut buf = [0u8; 8];
        overlay.read_at("log.bin", &mut buf, 0).unwrap();
        assert_eq!(&buf, b"AAAABBBB");
    }

    #[test]
    fn sync_no_panic() {
        // T5: sync 不 panic
        let (overlay, _) = new_overlay();
        overlay.create("s.bin").unwrap();
        overlay.write_at("s.bin", b"data", 0).unwrap();
        overlay.sync("s.bin").unwrap();
    }

    #[test]
    fn rename_content_moves_source_gone() {
        // T6: rename 内容移动，源不存在
        let (overlay, _) = new_overlay();
        overlay.create("from.bin").unwrap();
        overlay.write_at("from.bin", b"content", 0).unwrap();
        overlay.rename("from.bin", "to.bin").unwrap();
        let mut buf = [0u8; 7];
        overlay.read_at("to.bin", &mut buf, 0).unwrap();
        assert_eq!(&buf, b"content");
        assert!(overlay.read_at("from.bin", &mut [0u8; 1], 0).is_err());
    }

    #[test]
    fn delete_then_list_excludes() {
        // T7: delete 后 list 不含该文件
        let (overlay, _) = new_overlay();
        overlay.create("del.bin").unwrap();
        overlay.write_at("del.bin", b"x", 0).unwrap();
        overlay.delete("del.bin").unwrap();
        let files = overlay.list(".").unwrap();
        assert!(!files.contains(&"del.bin".to_string()));
    }

    #[test]
    fn list_segments_returns_ulid_entries() {
        // T8: list("segments") 返回 seg_<ulid> 列表
        let (overlay, _) = new_overlay();
        overlay.create("segments/seg_01abc/header.bin").unwrap();
        overlay.create("segments/seg_02def/header.bin").unwrap();
        overlay.create("manifest.json").unwrap();
        let segs = overlay.list("segments").unwrap();
        assert_eq!(segs, vec!["seg_01abc", "seg_02def"]);
    }

    #[test]
    fn nested_virtual_paths() {
        // T9: create("segments/seg_x/header.bin") 自动登记中间路径
        let (overlay, _) = new_overlay();
        overlay.create("segments/seg_x/header.bin").unwrap();
        overlay.create("segments/seg_x/vectors.bin").unwrap();
        let segs = overlay.list("segments").unwrap();
        assert_eq!(segs, vec!["seg_x"]);
        let files = overlay.list("segments/seg_x").unwrap();
        assert!(files.contains(&"header.bin".to_string()));
        assert!(files.contains(&"vectors.bin".to_string()));
    }

    #[test]
    fn read_nonexistent_returns_err() {
        // T10: read_at 不存在路径 → Err(VaneError::Io)
        let (overlay, _) = new_overlay();
        let result = overlay.read_at("nope", &mut [0u8; 1], 0);
        assert!(result.is_err());
        match result {
            Err(vane_core::types::VaneError::Io(_)) => {}
            other => panic!("expected VaneError::Io, got {:?}", other.map(|_| ())),
        }
    }

    // ── 门禁 9: 崩溃恢复 3 时点 ───────────────────────────────────────

    #[test]
    fn container_round_trip() {
        // T11: 写若干文件 → 重新 open → 文件表与原一致
        let backend = Arc::new(MemoryBackend::new());
        {
            let overlay = MemOverlay::open(backend.clone()).unwrap();
            overlay.create("manifest.json").unwrap();
            overlay
                .write_at("manifest.json", b"manifest_data", 0)
                .unwrap();
            overlay.create("wal.log").unwrap();
            overlay.append("wal.log", b"entry1").unwrap();
            overlay.append("wal.log", b"entry2").unwrap();
            overlay.create("segments/seg_01/header.bin").unwrap();
            overlay
                .write_at("segments/seg_01/header.bin", b"hdr", 0)
                .unwrap();
            overlay.sync("manifest.json").unwrap(); // persist meta
        }
        // 重新打开
        let overlay2 = MemOverlay::open(backend.clone()).unwrap();
        let mut buf = [0u8; 13];
        let n = overlay2.read_at("manifest.json", &mut buf, 0).unwrap();
        assert_eq!(n, 13);
        assert_eq!(&buf, b"manifest_data");

        let mut wal_buf = [0u8; 12];
        let n = overlay2.read_at("wal.log", &mut wal_buf, 0).unwrap();
        assert_eq!(n, 12);
        assert_eq!(&wal_buf, b"entry1entry2");

        let mut hdr_buf = [0u8; 3];
        let n = overlay2
            .read_at("segments/seg_01/header.bin", &mut hdr_buf, 0)
            .unwrap();
        assert_eq!(n, 3);
        assert_eq!(&hdr_buf, b"hdr");

        let segs = overlay2.list("segments").unwrap();
        assert_eq!(segs, vec!["seg_01"]);
    }

    #[test]
    fn double_meta_slot_alternation() {
        // T12: 连续多次 rename/delete → active_meta_slot 翻转 0↔1，generation 递增
        let (overlay, _) = new_overlay();
        let gen0 = overlay.generation();
        let slot0 = overlay.active_meta_slot();

        overlay.create("a.bin").unwrap();
        overlay.write_at("a.bin", b"A", 0).unwrap();
        overlay.rename("a.bin", "b.bin").unwrap();
        let gen1 = overlay.generation();
        let slot1 = overlay.active_meta_slot();
        assert_eq!(gen1, gen0 + 1);
        assert_ne!(slot0, slot1);

        overlay.rename("b.bin", "c.bin").unwrap();
        let gen2 = overlay.generation();
        let slot2 = overlay.active_meta_slot();
        assert_eq!(gen2, gen1 + 1);
        assert_eq!(slot2, slot0); // 翻回

        overlay.delete("c.bin").unwrap();
        let gen3 = overlay.generation();
        let slot3 = overlay.active_meta_slot();
        assert_eq!(gen3, gen2 + 1);
        assert_eq!(slot3, slot1);
    }

    /// 辅助：模拟 manifest 原子保存序列（save_atomic 的核心步骤）。
    /// 返回 crash 点描述用于断言。
    fn setup_manifest_atomic(overlay: &MemOverlay) {
        // 初始 manifest
        overlay.create("manifest.json").unwrap();
        overlay
            .write_at("manifest.json", b"OLD_MANIFEST", 0)
            .unwrap();
        overlay.sync("manifest.json").unwrap(); // persist meta: manifest.json → OLD
    }

    #[test]
    fn crash_recovery_after_sync_before_rename() {
        // T13: 崩溃恢复——步骤 2 后（sync tmp 后，rename 前）
        // recover 应读到旧 manifest 完好
        let backend = Arc::new(MemoryBackend::new());
        {
            let overlay = MemOverlay::open(backend.clone()).unwrap();
            setup_manifest_atomic(&overlay);

            // 写 tmp + sync（meta 持久化：manifest.json → OLD, tmp → NEW）
            overlay.create("manifest.tmp").unwrap();
            overlay
                .write_at("manifest.tmp", b"NEW_MANIFEST", 0)
                .unwrap();
            overlay.sync("manifest.tmp").unwrap();
            // CRASH: 不调 rename，直接丢弃 overlay
        }
        // recover
        let overlay2 = MemOverlay::open(backend.clone()).unwrap();
        let mut buf = [0u8; 12];
        let n = overlay2.read_at("manifest.json", &mut buf, 0).unwrap();
        assert_eq!(n, 12);
        assert_eq!(&buf, b"OLD_MANIFEST"); // 旧 manifest 完好 ✓
    }

    #[test]
    fn crash_recovery_meta_write_partial() {
        // T14: 崩溃恢复——persist_meta 写 inactive 槽中途截断（CRC 失败）
        // 模拟：rename 的 persist_meta 写 inactive 槽时数据不完整。
        // recover 校验 CRC 失败 → 回退旧 active 槽 → 旧 manifest 完好。
        let backend = Arc::new(MemoryBackend::new());
        let inactive_slot;
        {
            let overlay = MemOverlay::open(backend.clone()).unwrap();
            setup_manifest_atomic(&overlay);

            // 写 tmp + sync
            overlay.create("manifest.tmp").unwrap();
            overlay
                .write_at("manifest.tmp", b"NEW_MANIFEST", 0)
                .unwrap();
            overlay.sync("manifest.tmp").unwrap();

            // rename 会 persist_meta 到 inactive 槽。记录 inactive 槽号。
            inactive_slot = 1 - overlay.active_meta_slot();
            overlay.rename("manifest.tmp", "manifest.json").unwrap();
            // rename 完成：inactive 槽（现已翻转为 active）有新 meta
        }
        // 模拟 persist_meta 写 inactive 槽中途截断：覆写该槽 payload 字节使 CRC 失败
        let slot_offset = meta_slot_offset(inactive_slot);
        backend
            .write(slot_offset + 20, &[0xDE, 0xAD, 0xBE, 0xEF])
            .unwrap();

        // recover：该槽 CRC 失败 → 回退到另一槽（sync 时的状态）
        let overlay2 = MemOverlay::open(backend.clone()).unwrap();
        let mut buf = [0u8; 12];
        let n = overlay2.read_at("manifest.json", &mut buf, 0).unwrap();
        assert_eq!(n, 12);
        assert_eq!(&buf, b"OLD_MANIFEST"); // 旧 manifest 完好 ✓
    }

    #[test]
    fn crash_recovery_after_rename_flush() {
        // T15: 崩溃恢复——步骤 3 flush 后（rename 完成）
        // recover 读新槽 → 新 manifest 完整
        let backend = Arc::new(MemoryBackend::new());
        {
            let overlay = MemOverlay::open(backend.clone()).unwrap();
            setup_manifest_atomic(&overlay);

            overlay.create("manifest.tmp").unwrap();
            overlay
                .write_at("manifest.tmp", b"NEW_MANIFEST", 0)
                .unwrap();
            overlay.sync("manifest.tmp").unwrap();
            overlay.rename("manifest.tmp", "manifest.json").unwrap();
            // CRASH: rename 完成后丢弃
        }
        let overlay2 = MemOverlay::open(backend.clone()).unwrap();
        let mut buf = [0u8; 12];
        let n = overlay2.read_at("manifest.json", &mut buf, 0).unwrap();
        assert_eq!(n, 12);
        assert_eq!(&buf, b"NEW_MANIFEST"); // 新 manifest 完整 ✓
    }

    // ── 门禁 10: compaction 测试 ──────────────────────────────────────

    #[test]
    fn free_list_reuse_on_append() {
        // T16: delete 释放区间 → 后续 append 优先复用（first-fit），container_size 不增长
        let (overlay, _) = new_overlay();
        // 写一个 100 字节文件
        overlay.create("a.bin").unwrap();
        overlay.write_at("a.bin", &[0x41; 100], 0).unwrap();
        overlay.sync("a.bin").unwrap();
        let cs_after_sync = overlay.container_size();

        // 删除 a.bin → 释放 100 字节区间
        // free_ratio = 100% > 50% → 触发 compaction（shadow-write）
        // 无活跃文件 → shadow-write 写 0 字节到尾部，旧区域进 free list
        overlay.delete("a.bin").unwrap();
        let cs_after_delete = overlay.container_size();
        // shadow-write：container_size 不变（无数据写到尾部），旧区域在 free list
        assert_eq!(cs_after_delete, cs_after_sync);
        assert_eq!(overlay.free_list_snapshot().len(), 1);

        // 重新写文件——复用 free list 中的 100B 区间
        overlay.create("b.bin").unwrap();
        overlay.append("b.bin", &[0x42; 50]).unwrap();
        let cs_after_b = overlay.container_size();
        // container_size 不增长——复用了 free list ✓
        assert_eq!(cs_after_b, cs_after_sync);
    }

    #[test]
    fn free_list_reuse_without_compaction() {
        // T16 变体：delete 后 free_ratio < 50% → 不触发 compaction，但 append 仍复用 free list
        let (overlay, _) = new_overlay();
        // 写两个文件：a.bin (100B) + b.bin (200B)
        overlay.create("a.bin").unwrap();
        overlay.write_at("a.bin", &[0x41; 100], 0).unwrap();
        overlay.create("b.bin").unwrap();
        overlay.write_at("b.bin", &[0x42; 200], 0).unwrap();
        overlay.sync("b.bin").unwrap();
        let cs_before = overlay.container_size();

        // 删除 a.bin → free 100B, total = 300B, ratio = 33% < 50% → 不 compact
        overlay.delete("a.bin").unwrap();
        assert_eq!(overlay.container_size(), cs_before); // container_size 不变
        assert_eq!(overlay.free_list_snapshot().len(), 1); // free list 有 1 项

        // append 到 b.bin——b.bin 在尾部，原地扩展（不复用 free list）
        // 但创建新文件 c.bin 并 append——应复用 free list 中的 100B 区间
        overlay.create("c.bin").unwrap();
        overlay.append("c.bin", &[0x43; 50]).unwrap();
        let cs_after = overlay.container_size();
        assert_eq!(cs_after, cs_before); // container_size 不增长——复用了 free list ✓
    }

    #[test]
    fn compaction_full_rewrite_data_intact() {
        // T17: 碎片率超阈值 → 触发 shadow-write compaction → 数据一致
        let (overlay, _) = new_overlay();
        // 写 3 个文件
        overlay.create("a.bin").unwrap();
        overlay.write_at("a.bin", b"AAAA", 0).unwrap();
        overlay.create("b.bin").unwrap();
        overlay.write_at("b.bin", b"BBBBBBBB", 0).unwrap();
        overlay.create("c.bin").unwrap();
        overlay.write_at("c.bin", b"CC", 0).unwrap();
        overlay.sync("a.bin").unwrap();
        let cs_before = overlay.container_size();

        // 删除 b.bin（8B）→ free 8B, total = 14B, ratio = 57% > 50% → 触发 compaction
        overlay.delete("b.bin").unwrap();
        let cs_after = overlay.container_size();
        // shadow-write：活跃数据 a.bin(4B)+c.bin(2B)=6B 写到尾部 [cs_before, cs_before+6)
        // container_size = cs_before + 6（增长 6B，旧区域在 free list）
        assert_eq!(cs_after, cs_before + 6);

        // 数据一致（在新位置）
        let mut buf = [0u8; 4];
        overlay.read_at("a.bin", &mut buf, 0).unwrap();
        assert_eq!(&buf, b"AAAA");
        let mut buf2 = [0u8; 2];
        overlay.read_at("c.bin", &mut buf2, 0).unwrap();
        assert_eq!(&buf2, b"CC");

        // free list 有旧区域（shadow-write 不破坏旧数据，旧区域可复用）
        assert!(!overlay.free_list_snapshot().is_empty());

        // round-trip：重新打开后数据仍一致
        let backend = overlay.backend().clone();
        let overlay2 = MemOverlay::open(backend).unwrap();
        let mut buf3 = [0u8; 4];
        overlay2.read_at("a.bin", &mut buf3, 0).unwrap();
        assert_eq!(&buf3, b"AAAA");
    }

    // ── 门禁 10a: compaction 崩溃安全（M-1 修复）─────────────────────

    #[test]
    fn compaction_crash_persist_fails_old_data_intact() {
        // compaction shadow-write 完成，但 persist_meta 写入损坏（CRC 失败）
        // → recover 回退到旧 meta → 旧数据完整（未被 shadow-write 破坏）
        let backend = Arc::new(MemoryBackend::new());
        {
            let overlay = MemOverlay::open(backend.clone()).unwrap();
            overlay.create("a.bin").unwrap();
            overlay.write_at("a.bin", b"AAAA", 0).unwrap();
            overlay.create("b.bin").unwrap();
            overlay.write_at("b.bin", b"BBBBBBBB", 0).unwrap();
            overlay.sync("a.bin").unwrap(); // 旧 meta 持久化
        }
        // 执行 delete → 触发 compaction（shadow-write + persist_meta）
        {
            let overlay = MemOverlay::open(backend.clone()).unwrap();
            overlay.delete("b.bin").unwrap(); // compaction 完成
                                              // compaction 的 persist_meta 写了新 meta（generation 递增）
            let active = overlay.active_meta_slot();
            // 模拟崩溃：新 meta CRC 损坏（persist 写一半）
            corrupt_meta_slot(backend.as_ref(), active);
        }
        // recover：新 meta CRC 失败 → 回退到旧 meta（sync 时的状态）
        let overlay2 = MemOverlay::open(backend.clone()).unwrap();
        // 旧 meta 有 a.bin + b.bin（delete 未提交）
        let mut buf_a = [0u8; 4];
        overlay2.read_at("a.bin", &mut buf_a, 0).unwrap();
        assert_eq!(&buf_a, b"AAAA"); // 旧数据完整 ✓
        let mut buf_b = [0u8; 8];
        overlay2.read_at("b.bin", &mut buf_b, 0).unwrap();
        assert_eq!(&buf_b, b"BBBBBBBB"); // b.bin 仍在（delete 回滚）✓
    }

    #[test]
    fn compaction_crash_after_persist_new_data_intact() {
        // compaction 完成（persist_meta 成功）→ recover → 新数据在尾部完整
        let backend = Arc::new(MemoryBackend::new());
        {
            let overlay = MemOverlay::open(backend.clone()).unwrap();
            overlay.create("a.bin").unwrap();
            overlay.write_at("a.bin", b"AAAA", 0).unwrap();
            overlay.create("b.bin").unwrap();
            overlay.write_at("b.bin", b"BBBBBBBB", 0).unwrap();
            overlay.sync("a.bin").unwrap();
            overlay.delete("b.bin").unwrap(); // compaction 完成
        }
        // recover：新 meta active，数据在尾部
        let overlay2 = MemOverlay::open(backend.clone()).unwrap();
        let mut buf = [0u8; 4];
        overlay2.read_at("a.bin", &mut buf, 0).unwrap();
        assert_eq!(&buf, b"AAAA"); // 新数据完整 ✓
        assert!(overlay2.read_at("b.bin", &mut [0u8; 1], 0).is_err()); // b.bin 已删除 ✓
                                                                       // free list 有旧区域
        assert!(!overlay2.free_list_snapshot().is_empty());
    }

    // ── 门禁 11: superblock 自损坏恢复 ─────────────────────────────────

    #[test]
    fn superblock_corruption_recovers_from_meta_slots() {
        // superblock magic 损坏 → 仍从 meta slot 恢复
        let backend = Arc::new(MemoryBackend::new());
        {
            let overlay = MemOverlay::open(backend.clone()).unwrap();
            overlay.create("a.bin").unwrap();
            overlay.write_at("a.bin", b"data", 0).unwrap();
            overlay.sync("a.bin").unwrap();
        }
        // 损坏 superblock magic（前 4 字节）
        backend.write(0, b"XXXX").unwrap();

        let overlay2 = MemOverlay::open(backend.clone()).unwrap();
        let mut buf = [0u8; 4];
        let n = overlay2.read_at("a.bin", &mut buf, 0).unwrap();
        assert_eq!(n, 4);
        assert_eq!(&buf, b"data"); // 从 meta slot 恢复 ✓
    }

    #[test]
    fn both_meta_slots_corrupt_returns_err() {
        // 双 meta slot 都坏 → Err（策略：不可恢复，需 export 快照恢复）
        let backend = Arc::new(MemoryBackend::new());
        {
            let overlay = MemOverlay::open(backend.clone()).unwrap();
            overlay.create("a.bin").unwrap();
            overlay.write_at("a.bin", b"data", 0).unwrap();
            overlay.sync("a.bin").unwrap();
            // 再写一次到 slot 1
            overlay.create("b.bin").unwrap();
            overlay.write_at("b.bin", b"x", 0).unwrap();
            overlay.sync("b.bin").unwrap();
        }
        // 损坏两个 meta slot
        corrupt_meta_slot(backend.as_ref(), 0);
        corrupt_meta_slot(backend.as_ref(), 1);

        let result = MemOverlay::open(backend);
        assert!(result.is_err(), "both meta slots corrupt should return Err");
    }

    #[test]
    fn new_container_initializes_empty() {
        // 新库（backend size == 0）→ 初始化空容器
        let backend = Arc::new(MemoryBackend::new());
        let overlay = MemOverlay::open(backend.clone()).unwrap();
        assert_eq!(overlay.container_size(), DATA_OFFSET);
        assert!(overlay.list(".").unwrap().is_empty());
        assert_eq!(overlay.generation(), 0);
        // backend 已写入 superblock + meta_slot_0（紧凑编码，不含 padding）
        assert!(backend.size().unwrap() >= 4096 + 16);
    }

    // ── 大文件 / 多 append 测试 ────────────────────────────────────────

    #[test]
    fn large_append_across_pages() {
        // 与 MemoryVfs::memory_vfs_large_append_across_pages 等价
        let (overlay, _) = new_overlay();
        overlay.create("big.bin").unwrap();
        let chunk = vec![42u8; 100_000];
        let off1 = overlay.append("big.bin", &chunk).unwrap();
        assert_eq!(off1, 0);
        let off2 = overlay.append("big.bin", &chunk).unwrap();
        assert_eq!(off2, 100_000);
        let mut buf = vec![0u8; 50];
        overlay.read_at("big.bin", &mut buf, 99_990).unwrap();
        assert!(buf.iter().all(|&b| b == 42));
    }

    #[test]
    fn write_at_beyond_size_grows_with_zero_fill() {
        // write_at 在 offset > size 处写入 → gap 填零
        let (overlay, _) = new_overlay();
        overlay.create("f.bin").unwrap();
        overlay.write_at("f.bin", b"hello", 0).unwrap(); // size = 5
        overlay.write_at("f.bin", b"X", 10).unwrap(); // size = 11, gap [5..10] = 0
        let mut buf = [0u8; 11];
        overlay.read_at("f.bin", &mut buf, 0).unwrap();
        assert_eq!(&buf[0..5], b"hello");
        assert_eq!(&buf[5..10], &[0, 0, 0, 0, 0]);
        assert_eq!(buf[10], b'X');
    }

    #[test]
    fn create_already_exists_errors() {
        let (overlay, _) = new_overlay();
        overlay.create("a.bin").unwrap();
        assert!(overlay.create("a.bin").is_err());
    }

    #[test]
    fn delete_nonexistent_errors() {
        let (overlay, _) = new_overlay();
        assert!(overlay.delete("nope").is_err());
    }

    #[test]
    fn rename_nonexistent_source_errors() {
        let (overlay, _) = new_overlay();
        assert!(overlay.rename("nope", "dst").is_err());
    }

    #[test]
    fn list_root_sorted() {
        let (overlay, _) = new_overlay();
        overlay.create("c.bin").unwrap();
        overlay.create("a.bin").unwrap();
        overlay.create("b.bin").unwrap();
        let files = overlay.list(".").unwrap();
        assert_eq!(files, vec!["a.bin", "b.bin", "c.bin"]);
    }
}
