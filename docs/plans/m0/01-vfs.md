# VFS 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development 执行本计划。步骤用 checkbox `- [ ]` 标记。

**Goal:** 实现 SPEC §6.1 VFS trait（M0 冻结签名）+ MemoryVfs + StdFsVfs + LRU PageCache，为全引擎提供唯一 IO 抽象层。
**Architecture:** `Vfs` trait 定义 8 个同步方法；MemoryVfs 用 `std::sync::RwLock<HashMap<String, Vec<u8>>>` 实现纯内存后端（零额外依赖、wasm32 安全）（测试/纯内存）；StdFsVfs 用 `std::fs` 实现 native 后端（仅此 crate 允许 std::fs，通过 cfg 隔离）；PageCache 是 read-through LRU。Memory + StdFs 跑同一测试套件（SPEC §6.1）。
**Tech Stack:** Rust std（std::sync::RwLock/Mutex，零额外依赖、wasm32 安全）。
**SPEC 引用:** §6.1 VFS trait（M0 冻结）、§6.4 崩溃恢复（rename 原子切换）、§13.3 core 禁 std::fs（StdFsVfs 是唯一例外，用 cfg 隔离在 vfs 模块内）、§14 I-5/I-6。
**前置依赖:** 00-workspace（VaneError, Result, PAGE_CACHE_DEFAULT_MB, PAGE_SIZE）。
**验收标准:**
- [ ] Vfs trait 8 方法签名与 SPEC §6.1 逐字一致
- [ ] MemoryVfs + StdFsVfs 跑同一 `vfs_conformance_tests` 套件全绿
- [ ] PageCache LRU 淘汰正确（capacity 满后淘汰最久未用）
- [ ] rename 原子切换：target 被 source 覆盖（不变量 I-6 基础）
- [ ] core crate 除 `vfs/std_fs.rs` 外无 `std::fs`（grep 验证）

## Global Constraints
- VFS trait 签名 M0 冻结（SPEC §6.1，REQUIREMENTS §3.5）；任何签名变更需 spec 修订。
- core crate 禁止 `std::fs`/`std::net`/mmap（§13.3）；**唯一例外**：`crates/vane-core/src/vfs/std_fs.rs` 内部使用 `std::fs`，通过 `#[cfg(not(target_arch = "wasm32"))]` 隔离。这是 SPEC §11 "cfg 只允许在 VFS/Executor" 的合法落点。
- 依赖黑名单：regex/tokio/prost/tonic/openssl/lindera/ndarray/wee_alloc（§13.3）。
- PageCache 默认 32MB，页 64KB（§6.1）。
- 四后端中 M0 实现 memory + std-fs；opfs/idb 为 M2（trait 已冻结）。

## File Structure
- `crates/vane-core/src/vfs/mod.rs` — trait Vfs + 公共 re-export
- `crates/vane-core/src/vfs/memory.rs` — MemoryVfs
- `crates/vane-core/src/vfs/std_fs.rs` — StdFsVfs（cfg 隔离 std::fs）
- `crates/vane-core/src/vfs/page_cache.rs` — PageCache LRU
- `crates/vane-core/src/vfs/tests.rs` — vfs_conformance_tests（MemoryVfs + StdFsVfs 共用）

## 任务清单（bite-sized TDD）

### Task 1: Vfs trait 定义 + MemoryVfs 基础方法
**Files:**
- Create: `crates/vane-core/src/vfs/mod.rs`, `crates/vane-core/src/vfs/memory.rs`
- Modify: 无（00-workspace 已在 lib.rs 一次性预声明全部 9 模块（含 `pub mod vfs;`），本计划不修改 lib.rs（B1 裁决））
- Modify: 无（00-workspace 已在 vane-core Cargo.toml 一次性加入全部依赖，本计划不修改 Cargo.toml（B1 裁决））

**Interfaces:**
- Consumes from 00-workspace: VaneError, Result
- Produces: `trait Vfs`（8 方法）、`MemoryVfs::new()`
- 后续 04-segment-format, 05-bm25, 07-api-core, 08-persistence 全部消费 Vfs trait

- [ ] **Step 1: 写失败测试** — 创建 `crates/vane-core/src/vfs/tests.rs`：
```rust
use super::memory::MemoryVfs;
use super::Vfs;

pub fn run_conformance_tests<V: Vfs>(vfs: &V) {
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
    // 根目录 list 不含 sub/ 下的文件
    let root_files = vfs.list(".").unwrap();
    assert!(!root_files.iter().any(|f| f.contains("x.bin") && f.contains("sub")));
}

#[test]
fn memory_vfs_conformance() {
    let vfs = MemoryVfs::new();
    run_conformance_tests(&vfs);
}
```

- [ ] **Step 2: 跑测试确认失败** —
```bash
cargo test -p vane-core -- vfs 2>&1 | head -20
```
预期编译失败（vfs 模块未创建）。

- [ ] **Step 3: 最小实现** —

依赖确认（00-workspace 已加入，本计划不修改 Cargo.toml）：
```toml
# 参考（已由 00-workspace 加入，勿重复添加）：
# [dependencies]
# roaring = { workspace = true }
# sha2 = { workspace = true }
```
确认 00-workspace 已在 vane-core Cargo.toml 加入全部依赖（roaring/sha2/serde/serde_json/unicode-segmentation/rust-stemmers/ulid）。本模块**不引入 dashmap、不引入 parking_lot**（B2 裁决：并发原语统一 std::sync）。

`crates/vane-core/src/vfs/mod.rs`：
```rust
use crate::types::{Result, VaneError};

/// SPEC §6.1 VFS trait（M0 冻结签名）。
/// core 对 IO 的全部认知仅此接口。
pub trait Vfs: Send + Sync {
    fn create(&self, path: &str) -> Result<()>;
    fn read_at(&self, path: &str, buf: &mut [u8], offset: u64) -> Result<usize>;
    fn write_at(&self, path: &str, buf: &[u8], offset: u64) -> Result<()>;
    fn append(&self, path: &str, buf: &[u8]) -> Result<u64>;
    fn sync(&self, path: &str) -> Result<()>;
    fn rename(&self, from: &str, to: &str) -> Result<()>;
    fn delete(&self, path: &str) -> Result<()>;
    fn list(&self, dir: &str) -> Result<Vec<String>>;
}

pub mod memory;
#[cfg(not(target_arch = "wasm32"))]
pub mod std_fs;
pub mod page_cache;

#[cfg(test)]
mod tests;
```

`crates/vane-core/src/vfs/memory.rs`：
```rust
use crate::types::{Result, VaneError};
use crate::vfs::Vfs;
use std::collections::HashMap;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};

/// 纯内存 VFS 后端（测试/纯内存场景）。SPEC §6.1 四后端之一。
pub struct MemoryVfs {
    files: RwLock<HashMap<String, Vec<u8>>>,
    seq: AtomicU64,
}

impl MemoryVfs {
    pub fn new() -> Self {
        Self {
            files: RwLock::new(HashMap::new()),
            seq: AtomicU64::new(0),
        }
    }
}

impl Default for MemoryVfs {
    fn default() -> Self {
        Self::new()
    }
}

impl Vfs for MemoryVfs {
    fn create(&self, path: &str) -> Result<()> {
        let mut files = self.files.write().unwrap();
        if files.contains_key(path) {
            return Err(VaneError::Io(format!("file already exists: {}", path)));
        }
        files.insert(path.to_string(), Vec::new());
        Ok(())
    }

    fn read_at(&self, path: &str, buf: &mut [u8], offset: u64) -> Result<usize> {
        let files = self.files.read().unwrap();
        let file = files.get(path)
            .ok_or_else(|| VaneError::Io(format!("file not found: {}", path)))?;
        let off = offset as usize;
        if off >= file.len() {
            return Ok(0);
        }
        let avail = file.len() - off;
        let n = buf.len().min(avail);
        buf[..n].copy_from_slice(&file[off..off + n]);
        Ok(n)
    }

    fn write_at(&self, path: &str, buf: &[u8], offset: u64) -> Result<()> {
        let mut files = self.files.write().unwrap();
        let file = files.get_mut(path)
            .ok_or_else(|| VaneError::Io(format!("file not found: {}", path)))?;
        let off = offset as usize;
        let end = off + buf.len();
        if file.len() < end {
            file.resize(end, 0);
        }
        file[off..end].copy_from_slice(buf);
        Ok(())
    }

    fn append(&self, path: &str, buf: &[u8]) -> Result<u64> {
        let mut files = self.files.write().unwrap();
        let file = files.get_mut(path)
            .ok_or_else(|| VaneError::Io(format!("file not found: {}", path)))?;
        let offset = file.len() as u64;
        file.extend_from_slice(buf);
        Ok(offset)
    }

    fn sync(&self, _path: &str) -> Result<()> {
        Ok(()) // 内存无需 sync
    }

    fn rename(&self, from: &str, to: &str) -> Result<()> {
        let mut files = self.files.write().unwrap();
        let data = files.remove(from)
            .ok_or_else(|| VaneError::Io(format!("file not found: {}", from)))?;
        files.insert(to.to_string(), data);
        Ok(())
    }

    fn delete(&self, path: &str) -> Result<()> {
        let mut files = self.files.write().unwrap();
        files.remove(path)
            .ok_or_else(|| VaneError::Io(format!("file not found: {}", path)))?;
        Ok(())
    }

    fn list(&self, dir: &str) -> Result<Vec<String>> {
        // 按 dir 前缀过滤（与 StdFsVfs 语义一致）
        let prefix = if dir == "." || dir.is_empty() {
            String::new()
        } else {
            format!("{}/", dir.trim_end_matches('/'))
        };
        let files = self.files.read().unwrap();
        let mut out: Vec<String> = files.keys()
            .filter(|path| path.starts_with(&prefix))
            .map(|path| {
                // 返回相对 dir 的文件名（去掉前缀）
                if prefix.is_empty() {
                    path.clone()
                } else {
                    path[prefix.len()..].to_string()
                }
            })
            .collect();
        out.sort();
        Ok(out)
    }
}
```
> 注：`RwLock::read()/write()` 返回 `Result`，用 `.unwrap()` 处理 poison（M0 测试场景不会 poison）。

00-workspace 已在 lib.rs 一次性预声明全部 9 模块（含 `pub mod vfs;`），本计划不修改 lib.rs（B1 裁决）。

- [ ] **Step 4: 跑测试确认通过** — `cargo test -p vane-core -- vfs`，memory_vfs_conformance 绿。
- [ ] **Step 5: Commit**
```bash
git add -A
git commit -m "feat(vfs): Vfs trait (§6.1 frozen) + MemoryVfs backend + conformance tests

"
```

### Task 2: StdFsVfs 后端（cfg 隔离 std::fs）
**Files:**
- Create: `crates/vane-core/src/vfs/std_fs.rs`

**Interfaces:**
- Consumes from 00-workspace: VaneError, Result
- Produces: `StdFsVfs::new() -> Self`
- 后续 07-api-core, 09-node-binding 消费（native 默认后端）

- [ ] **Step 1: 写失败测试** — 追加到 `crates/vane-core/src/vfs/tests.rs`：
```rust
#[cfg(not(target_arch = "wasm32"))]
mod std_fs_tests {
    use super::super::std_fs::StdFsVfs;
    use super::run_conformance_tests;
    use std::path::PathBuf;

    fn tmpdir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vane-vfs-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn std_fs_vfs_conformance() {
        let dir = tmpdir();
        let vfs = StdFsVfs::with_root(dir.to_str().unwrap());
        run_conformance_tests(&vfs);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
```

- [ ] **Step 2: 跑测试确认失败** — `cargo test -p vane-core -- std_fs`，编译失败（std_fs 模块未实现）。
- [ ] **Step 3: 最小实现** — `crates/vane-core/src/vfs/std_fs.rs`：
```rust
use crate::types::{Result, VaneError};
use crate::vfs::Vfs;
use std::path::{Path, PathBuf};

/// Native 文件系统 VFS 后端。SPEC §6.1 四后端之一。
/// 这是 core crate 中唯一允许使用 std::fs 的模块（cfg 隔离，§13.3 例外）。
#[cfg(not(target_arch = "wasm32"))]
pub struct StdFsVfs {
    root: PathBuf,
}

#[cfg(not(target_arch = "wasm32"))]
impl StdFsVfs {
    pub fn new() -> Self {
        Self { root: PathBuf::new() }
    }

    pub fn with_root(root: &str) -> Self {
        Self { root: PathBuf::from(root) }
    }

    fn resolve(&self, path: &str) -> PathBuf {
        // 简化：路径相对于 root
        let p = self.root.join(path);
        // 确保父目录存在
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        p
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Default for StdFsVfs {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Vfs for StdFsVfs {
    fn create(&self, path: &str) -> Result<()> {
        let p = self.resolve(path);
        if p.exists() {
            return Err(VaneError::Io(format!("file already exists: {}", path)));
        }
        std::fs::File::create(&p).map_err(io_err)?;
        Ok(())
    }

    fn read_at(&self, path: &str, buf: &mut [u8], offset: u64) -> Result<usize> {
        use std::io::{Read, Seek, SeekFrom};
        let p = self.resolve(path);
        let mut f = std::fs::File::open(&p).map_err(io_err)?;
        f.seek(SeekFrom::Start(offset)).map_err(io_err)?;
        f.read(buf).map_err(io_err)
    }

    fn write_at(&self, path: &str, buf: &[u8], offset: u64) -> Result<()> {
        use std::io::{Seek, SeekFrom, Write};
        let p = self.resolve(path);
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .open(&p)
            .map_err(io_err)?;
        f.seek(SeekFrom::Start(offset)).map_err(io_err)?;
        f.write_all(buf).map_err(io_err)
    }

    fn append(&self, path: &str, buf: &[u8]) -> Result<u64> {
        use std::io::{Seek, SeekFrom, Write};
        let p = self.resolve(path);
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .open(&p)
            .map_err(io_err)?;
        let offset = f.seek(SeekFrom::End(0)).map_err(io_err)?;
        f.write_all(buf).map_err(io_err)?;
        Ok(offset)
    }

    fn sync(&self, path: &str) -> Result<()> {
        let p = self.resolve(path);
        let f = std::fs::File::open(&p).map_err(io_err)?;
        f.sync_all().map_err(io_err)
    }

    fn rename(&self, from: &str, to: &str) -> Result<()> {
        let pf = self.resolve(from);
        let pt = self.resolve(to);
        // 跨平台一致覆盖：先删目标（忽略不存在），再 rename
        let _ = std::fs::remove_file(&pt);
        std::fs::rename(&pf, &pt).map_err(io_err)
    }

    fn delete(&self, path: &str) -> Result<()> {
        let p = self.resolve(path);
        if p.is_dir() {
            std::fs::remove_dir_all(&p).map_err(io_err)
        } else {
            std::fs::remove_file(&p).map_err(io_err)
        }
    }

    fn list(&self, dir: &str) -> Result<Vec<String>> {
        let p = self.resolve(dir);
        let mut out = Vec::new();
        let entries = std::fs::read_dir(&p).map_err(io_err)?;
        for entry in entries {
            let entry = entry.map_err(io_err)?;
            if let Some(name) = entry.file_name().to_str() {
                out.push(name.to_string());
            }
        }
        Ok(out)
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn io_err(e: std::io::Error) -> VaneError {
    VaneError::Io(e.to_string())
}
```
> **S11: StdFsVfs::rename Windows 覆盖** — Windows 上 `std::fs::rename` 不覆盖已存在目标。为实现跨平台一致的覆盖语义（manifest 原子切换需要），StdFsVfs::rename 实现先 `delete(target)`（忽略 not-found 错误）再 `rename(from, to)`。conformance test 中 rename 覆盖已存在目标的用例在 Windows 上也通过。
- [ ] **Step 4: 跑测试确认通过** — `cargo test -p vane-core -- vfs`，两个后端测试全绿。
- [ ] **Step 5: Commit**
```bash
git add -A
git commit -m "feat(vfs): StdFsVfs backend (cfg-isolated std::fs, §13.3 exception)

"
```

### Task 3: PageCache LRU
**Files:**
- Create: `crates/vane-core/src/vfs/page_cache.rs`

**Interfaces:**
- Consumes from 00-workspace: PAGE_CACHE_DEFAULT_MB, PAGE_SIZE, VaneError, Result
- Consumes from Task 1: Vfs trait
- Produces: `PageCache::new(capacity_bytes, page_size)`、`PageCache::read(vfs, path, offset, len)`

- [ ] **Step 1: 写失败测试** — 追加到 `crates/vane-core/src/vfs/tests.rs`：
```rust
    use super::page_cache::PageCache;

    #[test]
    fn page_cache_read_through_and_lru_eviction() {
        let vfs = MemoryVfs::new();
        vfs.create("data.bin").unwrap();
        // 写 4 页数据，每页 64 字节（测试用小页）
        let page_size = 64usize;
        let mut data = vec![0u8; page_size * 4];
        for (i, b) in data.iter_mut().enumerate() {
            *b = (i / page_size) as u8; // page 0 = 0, page 1 = 1, ...
        }
        vfs.write_at("data.bin", &data, 0).unwrap();

        // capacity = 2 页
        let mut cache = PageCache::new(page_size * 2, page_size);
        // 读 page 0
        let r0 = cache.read(&vfs, "data.bin", 0, page_size).unwrap();
        assert_eq!(r0[0], 0);
        // 读 page 1
        let r1 = cache.read(&vfs, "data.bin", page_size as u64, page_size).unwrap();
        assert_eq!(r1[0], 1);
        // 读 page 2 → 淘汰 page 0
        let r2 = cache.read(&vfs, "data.bin", (page_size * 2) as u64, page_size).unwrap();
        assert_eq!(r2[0], 2);
        // 再读 page 0 → 应重新从 vfs 加载（缓存未命中）
        let r0b = cache.read(&vfs, "data.bin", 0, page_size).unwrap();
        assert_eq!(r0b[0], 0);
    }

    #[test]
    fn page_cache_invalidate() {
        let vfs = MemoryVfs::new();
        vfs.create("f.bin").unwrap();
        vfs.write_at("f.bin", &[1, 2, 3], 0).unwrap();
        let mut cache = PageCache::new(1024, 64);
        cache.read(&vfs, "f.bin", 0, 3).unwrap();
        // 修改底层文件后 invalidate
        vfs.write_at("f.bin", &[9, 9, 9], 0).unwrap();
        cache.invalidate("f.bin");
        let r = cache.read(&vfs, "f.bin", 0, 3).unwrap();
        assert_eq!(&r[..3], &[9, 9, 9]);
    }
```

- [ ] **Step 2: 跑测试确认失败** — `cargo test -p vane-core -- page_cache`，编译失败。
- [ ] **Step 3: 最小实现** — `crates/vane-core/src/vfs/page_cache.rs`：
```rust
use crate::types::Result;
use crate::vfs::Vfs;
use std::sync::Mutex;
use std::collections::HashMap;

/// SPEC §6.1 LRU 页缓存。read-through：未命中则从 Vfs 加载整页。
pub struct PageCache {
    inner: Mutex<Inner>,
    page_size: usize,
    capacity: usize, // 字节数
}

struct Inner {
    pages: HashMap<(String, u64), Vec<u8>>, // (path, page_index) -> data
    order: Vec<(String, u64)>,              // LRU 顺序（尾 = 最近用）
    capacity_bytes: usize,
    used_bytes: usize,
}

impl PageCache {
    pub fn new(capacity_bytes: usize, page_size: usize) -> Self {
        Self {
            inner: Mutex::new(Inner {
                pages: HashMap::new(),
                order: Vec::new(),
                capacity_bytes,
                used_bytes: 0,
            }),
            page_size,
            capacity: capacity_bytes,
        }
    }

    pub fn read(
        &self,
        vfs: &dyn Vfs,
        path: &str,
        offset: u64,
        len: usize,
    ) -> Result<Vec<u8>> {
        let mut result = vec![0u8; len];
        let mut remaining = len;
        let mut cur_off = offset;
        let mut out_off = 0usize;

        while remaining > 0 {
            let page_idx = cur_off / self.page_size as u64;
            let page_off = (cur_off % self.page_size as u64) as usize;
            let chunk = remaining.min(self.page_size - page_off);

            let page_data = {
                let mut inner = self.inner.lock().unwrap();
                if let Some(data) = inner.pages.get(&(path.to_string(), page_idx)) {
                    // 命中：移动到 LRU 尾
                    inner.touch(path.to_string(), page_idx);
                    data.clone()
                } else {
                    drop(inner);
                    // 未命中：从 vfs 加载整页
                    let mut page_buf = vec![0u8; self.page_size];
                    let page_start = page_idx * self.page_size as u64;
                    let n = vfs.read_at(path, &mut page_buf, page_start)?;
                    page_buf.truncate(n);
                    let mut inner = self.inner.lock().unwrap();
                    inner.put(path.to_string(), page_idx, page_buf.clone());
                    page_buf
                }
            };

            let copy_n = chunk.min(page_data.len().saturating_sub(page_off));
            if copy_n > 0 {
                result[out_off..out_off + copy_n]
                    .copy_from_slice(&page_data[page_off..page_off + copy_n]);
            }
            out_off += chunk;
            cur_off += chunk as u64;
            remaining -= chunk;
        }
        Ok(result)
    }

    pub fn invalidate(&self, path: &str) {
        let mut inner = self.inner.lock().unwrap();
        let keys_to_remove: Vec<(String, u64)> = inner
            .pages
            .keys()
            .filter(|(p, _)| p == path)
            .cloned()
            .collect();
        for k in keys_to_remove {
            if let Some(data) = inner.pages.remove(&k) {
                inner.used_bytes -= data.len();
            }
        }
        inner.order.retain(|(p, _)| p != path);
    }
}

impl Inner {
    fn touch(&mut self, path: String, page_idx: u64) {
        let key = (path, page_idx);
        if let Some(pos) = self.order.iter().position(|k| *k == key) {
            self.order.remove(pos);
        }
        self.order.push(key);
    }

    fn put(&mut self, path: String, page_idx: u64, data: Vec<u8>) {
        let key = (path, page_idx);
        let page_len = data.len();
        // 淘汰直到有空间
        while self.used_bytes + page_len > self.capacity_bytes && !self.order.is_empty() {
            let evict = self.order.remove(0);
            if let Some(d) = self.pages.remove(&evict) {
                self.used_bytes -= d.len();
            }
        }
        self.used_bytes += page_len;
        self.pages.insert(key, data);
        self.order.push(key);
    }
}
```
> 注：`std::sync::Mutex::lock` 返回 `Result<MutexGuard, _>`，用 `.unwrap()` 处理 poison（M0 测试场景不会 poison）。
> **S12: PageCache M0 无消费者标注** — PageCache M0 仅编译验证（实现就绪），07-api-core 的 OpenOptions.page_cache_mb 接线留 M1。M0 无消费者，避免死代码疑问。
- [ ] **Step 4: 跑测试确认通过** — `cargo test -p vane-core -- page_cache`，2 测试绿。
- [ ] **Step 5: Commit**
```bash
git add -A
git commit -m "feat(vfs): LRU PageCache with read-through + eviction (§6.1)

"
```

### Task 4: 跨后端一致性套件 + sync 验证
**Files:**
- Modify: `crates/vane-core/src/vfs/tests.rs`（补充 sync + append offset 跨页测试）

**Interfaces:**
- Consumes from Task 1-3
- Produces: 完整 vfs_conformance_tests（覆盖 SPEC §6.1 全部 8 方法）

- [ ] **Step 1: 写失败测试** — 追加到 tests.rs：
```rust
    #[test]
    fn memory_vfs_sync_is_noop() {
        let vfs = MemoryVfs::new();
        vfs.create("s.bin").unwrap();
        vfs.write_at("s.bin", b"data", 0).unwrap();
        // sync 不报错
        vfs.sync("s.bin").unwrap();
    }

    #[test]
    fn memory_vfs_large_append_across_pages() {
        let vfs = MemoryVfs::new();
        vfs.create("big.bin").unwrap();
        let chunk = vec![42u8; 100_000];
        let off1 = vfs.append("big.bin", &chunk).unwrap();
        assert_eq!(off1, 0);
        let off2 = vfs.append("big.bin", &chunk).unwrap();
        assert_eq!(off2, 100_000);
        // 读回校验
        let mut buf = vec![0u8; 50];
        vfs.read_at("big.bin", &mut buf, 99_990).unwrap();
        assert!(buf.iter().all(|&b| b == 42));
    }
```

- [ ] **Step 2: 跑测试确认通过**（实现已在 Task 1-3 完成，此处验证覆盖）— `cargo test -p vane-core -- vfs`。
- [ ] **Step 3: 最终验证** —
```bash
cargo test -p vane-core
cargo clippy -p vane-core -- -D warnings
cargo check --target wasm32-unknown-unknown -p vane-core 2>&1 | tail -5
./scripts/check-no-std-fs.sh
```
wasm32 check 应通过（std_fs.rs 被 cfg 排除）。grep 脚本应只允许 `vfs/std_fs.rs` 出现 std::fs（脚本需更新排除该文件，或确认脚本 grep `crates/vane-core/src/` 排除 `vfs/std_fs.rs`）。

更新 `scripts/check-no-std-fs.sh` 排除合法文件：
```bash
#!/usr/bin/env bash
set -euo pipefail
# §13.3: core 禁 std::fs/std::net/mmap，唯一例外 vfs/std_fs.rs
if grep -rn --include='*.rs' 'std::fs\|std::net\|mmap' crates/vane-core/src/ \
    | grep -v 'crates/vane-core/src/vfs/std_fs.rs'; then
    echo "FAIL: forbidden IO usage outside vfs/std_fs.rs" >&2
    exit 1
fi
echo "OK"
```
> **B7: 单一事实源** — 本脚本是 `check-no-std-fs.sh` 的**单一事实源**（B7 裁决）。00-workspace 与 10-ci-gates 均引用此产出，不重复创建。
- [ ] **Step 4: 确认全绿**
- [ ] **Step 5: Commit**
```bash
git add -A
git commit -m "test(vfs): cross-backend conformance + sync/large-append coverage

"
```
