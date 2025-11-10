# M2-02 OPFS VFS 后端

## 1. 目标
实现 `OpfsVfs`，作为 `vane_core::vfs::Vfs` trait（M0 冻结，`crates/vane-core/src/vfs/mod.rs:5-13`）的浏览器主后端。物理上整个 Db 是**单个 OPFS 容器文件 `vane.db`**，Worker init 异步获取该文件唯一的 `FileSystemSyncAccessHandle`（一次性 await），此后全部 Vfs 方法基于该同步句柄操作容器内字节区间。core 看到的 `<db>/segments/seg_<ulid>/vectors.bin` 等路径是**虚拟路径**，由 `OpfsVfs` 内存的虚拟 FS overlay（`MemOverlay`）映射到容器内 `(offset, size)` 区间。

**设计依据**：`docs/plans/m2/opfs-vfs-design.md`（路径 A，已评审）。该设计解除 reviewer B 阻塞 B-1：`FileSystemSyncAccessHandle` 仅提供同步字节 IO（read/write/flush/getSize/truncate/close），不提供目录操作；`create/rename/delete/list` 在纯同步 SyncAccessHandle 下无法实现。路径 A 用「单容器 + 内存 overlay」把目录/文件元操作全部搬到内存文件表，物理上只有单文件字节 IO，从而全部 Vfs 方法可同步实现。

SPEC 节号：§6.1（VFS trait + 四后端，签名零改动）、§6.2（逻辑目录布局，core 看到的虚拟路径）、§6.4（manifest 原子 rename，I-6）、§4.1（OPFS 主 + Dedicated Worker + core 同步 IO）。

**core / Vfs trait 零改动**：Vfs trait 8 方法签名不变；core 仍按 §6.2 目录布局写虚拟 path，OpfsVfs 内部映射；I-5（core 零平台分支）、I-8（binding 薄壳）保持。

## 2. 涉及文件
- **Create** `crates/vane-wasm/src/vfs/mod.rs`：Vfs 模块入口，re-export。
- **Create** `crates/vane-wasm/src/vfs/overlay.rs`：**`MemOverlay` 内核**（后端无关，M2-03 IDB 复用）——虚拟文件表 `HashMap<String, (base:u64, size:u64)>` + free list `Vec<(base,size)>` + 双元数据槽序列化/CRC + generation。提供 `trait OverlayBackend { read/write/flush/size/truncate }` 抽象，OpfsVfs 与 IdbVfs 各 impl 一次。
- **Create** `crates/vane-wasm/src/vfs/container.rs`：容器格式读写（superblock / 双 meta_slot / data area）+ CRC32（手写 8 行，避免新依赖）。
- **Create** `crates/vane-wasm/src/vfs/opfs.rs`：`OpfsVfs` struct（持有 `FileSystemSyncAccessHandle` + `MemOverlay`）+ `impl Vfs`（8 方法全同步）。
- **Modify** `crates/vane-wasm/Cargo.toml`：增 `web-sys`（feature `FileSystemSyncAccessHandle`/`FileSystemFileHandle`/`FileSystemDirectoryHandle`/`Storage`，**不启完整 `FileSystemAccess`**）+ `js-sys` dep；`[features] opfs = ["dep:web-sys", "dep:js-sys"]`。
- **Modify** `crates/vane-wasm/src/lib.rs`：`#[cfg(feature="opfs")] pub mod vfs;`。

> **不修改** `crates/vane-core/`（Vfs trait、core 调用面零改动）。

## 3. 接口契约
### Consumes from
- M0 `vane_core::vfs::Vfs` trait（M0 冻结签名，`vfs/mod.rs:5-13`，8 方法）：
  ```rust
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
  ```
- M0 `vane_core::types::{Result, VaneError}`（错误码映射：IO 失败 → `VaneError::Io`）。
- M2-01 vane-wasm cdylib + feature 体系。
- 设计依据：`docs/plans/m2/opfs-vfs-design.md`（路径 A）。

### Produces for
```rust
// crates/vane-wasm/src/vfs/overlay.rs（后端无关内核，M2-03 复用）
pub trait OverlayBackend: Send + Sync {
    fn read(&self, off: u64, buf: &mut [u8]) -> Result<usize>;
    fn write(&self, off: u64, buf: &[u8]) -> Result<()>;
    fn flush(&self) -> Result<()>;
    fn size(&self) -> Result<u64>;
    fn truncate(&self, sz: u64) -> Result<()>;
}
pub struct MemOverlay { /* file_table + free_list + active_meta_slot + generation */ }
impl MemOverlay {
    pub fn open(backend: Arc<dyn OverlayBackend>) -> Result<Self>;  // 读 superblock + 活跃 meta_slot 重建文件表；新库初始化空容器
    pub fn create(&self, path: &str) -> Result<()>;                  // 内存表登记（区间首次 write 时分配）
    pub fn read_at(&self, path: &str, buf: &mut [u8], off: u64) -> Result<usize>;
    pub fn write_at(&self, path: &str, buf: &[u8], off: u64) -> Result<()>;
    pub fn append(&self, path: &str, buf: &[u8]) -> Result<u64>;
    pub fn sync(&self, path: &str) -> Result<()>;                    // 统一 flush（path 粒度去重连续 flush）
    pub fn rename(&self, from: &str, to: &str) -> Result<()>;        // 表项改挂 + 元数据落非活跃槽 + 翻转 active + flush
    pub fn delete(&self, path: &str) -> Result<()>;                  // 表项移除 + 区间进 free list + 元数据落盘
    pub fn list(&self, dir: &str) -> Result<Vec<String>>;            // 表 keys 前缀过滤返回下一层分量（与 MemoryVfs::list 语义一致 vfs/memory.rs:99）
}

// crates/vane-wasm/src/vfs/opfs.rs（feature = "opfs"）
pub struct OpfsVfs { overlay: MemOverlay, sah: RefCell<FileSystemSyncAccessHandle> }
impl OverlayBackend for OpfsVfsBackEnd { /* SyncAccessHandle.read/write/flush/getSize/truncate */ }
impl OpfsVfs {
    /// Worker init 异步获取 SyncAccessHandle 后传入（唯一 await 点在 Worker init）。
    pub fn from_handle(sah: web_sys::FileSystemSyncAccessHandle) -> Result<Self>;
}
impl vane_core::vfs::Vfs for OpfsVfs { /* 8 方法，全部委托 MemOverlay（同步） */ }
```
下游：M2-03（IdbVfs 复用 `MemOverlay` + `OverlayBackend`，仅换内存 Vec 后端 + 异步 checkpoint）；M2-04（Worker init 注入 Vfs 实例，§4.7 异步序列）；M2-12（export 直接读容器区间拼接，或 dump 整个容器 blob）。

### 容器格式（`container.rs`）
```
┌──────────────────────────────────────────────────────────┐
│ superblock (4 KB)                                        │
│  magic(4) | format_version(4 LE)                         │
│  active_meta_slot: u8 (0|1)                              │
│  meta_offset[2]: u64 LE | meta_size[2]: u64 LE           │
│  container_size: u64 LE                                  │
├──────────────────────────────────────────────────────────┤
│ meta_slot_0  (generation:u64 | crc32:u32 | file_table[]  │
│              | free_list[])  — 等大预留（各 256KB）       │
├──────────────────────────────────────────────────────────┤
│ meta_slot_1  (双槽，与 slot_0 等大预留)                   │
├──────────────────────────────────────────────────────────┤
│ data area                                                │
│  manifest.json / wal.log / segments/seg_<ulid>/{...}     │
│  （各文件一区间，按 append 分配序）                       │
└──────────────────────────────────────────────────────────┘
```
- 写入总是写**非活跃 meta_slot**，写完 + CRC 后翻转 `active_meta_slot`。
- `container_size` 记录数据区已分配末尾，用于 `append` 分配新区间。
- 新库 init：superblock 写 `active_meta_slot=0`、`meta_slot_0` 为空文件表，flush。

### manifest 原子性（I-6 等价，对 core 透明）
core `ManifestStore::save_atomic` 调用序列 `delete(tmp) → create(tmp) → write_at(tmp, json, 0) → sync(tmp) → rename(tmp, manifest.json)`。在 OpfsVfs 中：
1. `write_at(tmp, ..)`：manifest 字节写到 tmp 区间（若 tmp 不存在则 append 分配）。元数据表里 `tmp` 指向新字节，`manifest.json` 仍指向旧字节。
2. `sync(tmp)`：`SyncAccessHandle.flush()` —— tmp 字节落盘，元数据表仍指向 tmp（元数据未持久化翻转）。
3. `rename(tmp, manifest.json)`：内存表释放旧 `manifest.json` 区间（进 free list）→ `tmp` 区间改挂为 `manifest.json` → 新内存表序列化到**非活跃 meta_slot** + CRC → `SyncAccessHandle.write(meta_slot)` + 翻转 superblock `active_meta_slot` + `write(superblock)` + `flush()`。

**崩溃恢复**（测试覆盖）：
- 步骤 2 后、步骤 3 元数据落盘前崩溃：active meta_slot 仍是旧的（`tmp` 叫 tmp，`manifest.json` 指向旧字节，旧 manifest 完整）→ recover 读旧 manifest 完好。✓ I-6
- 步骤 3 元数据写一半崩溃：非活跃槽 CRC 校验失败 → recover 回退到 active 旧槽 → 旧 manifest 完好。✓ I-6
- 步骤 3 flush 后崩溃：新 meta_slot active，`manifest.json` 指向新字节，新 manifest 完整。✓ I-6
- superblock 翻转非单字节原子：recover 同时校验两 meta_slot CRC，取 generation 最大且 CRC 通过者为 active。等价双版本 + CRC 原子切换，不依赖单字节原子性。

`rename` 仍是原子切换唯一原语（OpfsVfs 内部用双 meta_slot + CRC 实现等价原子性，对 core 透明，I-6 等价）。

### sync 粒度
单容器下 `sync(path)` 统一 `SyncAccessHandle.flush()` 全容器。core 在 `SegmentBuilder::finalize` 对每段文件各调一次 `sync`（5 次 flush）。初版保守每次都 flush；优化（dirty 合并，连续 sync 去重为最后一次 flush）留作 M2 后期。OPFS 单文件 flush 成本低于多文件 fsync，预计可接受。

### 容器 compaction（初版 append-only + 全量 rewrite）
段删除（合并/compact 后旧段清除）留空洞。free list 管理空闲区间，后续 append 优先复用合适空洞（first-fit）。长期碎片化时触发 compaction：分配新容器区域 → 拷贝活跃区间 → 翻转。**初版 append-only + 阈值触发全量 rewrite（简单）**，碎片管理优化延后。core 不感知。

## 4. TDD 测试清单
1. **Vfs 套件复用**：`OpfsVfs` 跑 M0 既有 Vfs 通用测试套件（`crates/vane-core/src/vfs/tests.rs` 或同构 `vfs_contract` 测试，Memory+StdFs 已跑过）——证明 OPFS 后端满足 trait 契约。Wasm 端用 wasm-bindgen-test 在 Worker 跑。
2. `create(path)` 创建文件后 `read_at(path, .., 0)` 返回 0 字节（空文件，内存表登记）。
3. `write_at(path, &[..], 0)` + `read_at` 回读一致（容器区间 write/read）。
4. `append(path, &buf)` 两次 → 文件长度 = 两段之和，返回第一次/第二次起始 offset（append 分配新区间）。
5. `sync(path)` 不 panic（SyncAccessHandle.flush）。
6. `rename(from, to)`：源文件内容出现在 to，源不存在（内存表改挂 + 元数据落盘 + flush）。
7. `delete(path)` 后 `list(dir)` 不含该文件（区间进 free list）。
8. `list("segments")` 返回 `seg_<ulid>` 列表（与 `MemoryVfs::list` `vfs/memory.rs:99` 「返回下一层分量」语义一致；`cleanup_orphan_segment_dirs` / `delete_segment_dir` 递归逻辑正确）。
9. **目录嵌套（虚拟）**：`create("segments/seg_x/header.bin")` 自动登记中间路径（内存表前缀，无物理目录创建）。
10. **错误码**：`read_at` 不存在路径 → `Err(VaneError::Io(..))`（code -1，SPEC §10 E_IO）。
11. **容器格式 round-trip**：写若干文件 → close → 重新 `from_handle` 打开 → superblock + meta_slot 解析 → 文件表与原一致（base/size 全对）。
12. **双 meta_slot 翻转**：连续多次 rename/delete → 每次 active_meta_slot 翻转 0↔1，两槽交替写入，generation 单调递增。
13. **崩溃恢复：步骤 2 后崩溃**（模拟 `rename` 前 flush 完字节但未落元数据）：recover 读 active 旧槽 → 旧 manifest 完整，新 tmp 字节被 free list 回收。✓ I-6
14. **崩溃恢复：元数据写一半崩溃**（模拟非活跃槽 CRC 损坏）：recover 校验 CRC 失败 → 回退 active 旧槽 → 旧 manifest 完好。✓ I-6
15. **崩溃恢复：superblock 翻转后崩溃**（模拟翻转后 active 指向新槽）：recover 读新槽（CRC 通过 + generation 最大）→ 新 manifest 完整。✓ I-6
16. **free list 复用**：delete 释放区间 → 后续 append 优先复用该区间（first-fit），`container_size` 不增长。
17. **compaction（全量 rewrite）**：碎片率超阈值 → 触发 rewrite → 活跃区间拷贝到新区域 → 文件表重映射 → 旧区域回收 → 数据一致（round-trip 验证）。
18. **core 调用面兼容**（集成测试）：用 OpfsVfs 跑 `ManifestStore::save_atomic` / `Wal::open+append+truncate` / `recover::cleanup_orphan_segment_dirs` / `merge::delete_segment_dir` / `SegmentBuilder::finalize` / `SegmentReader::open` 全路径，行为与 StdFsVfs/MemoryVfs 等价。

## 5. 验收标准
- Vfs 通用契约测试套件在 OpfsVfs 上全绿（wasm-bindgen-test in Worker）。
- 崩溃恢复测试（13/14/15）全绿：任何崩溃时点后 manifest 指向完整状态（I-6 等价）。
- 体积：启用 `opfs` feature 后 vane-wasm gzip 仍 ≤800KB（web-sys subset 体积实测登记：`Storage` + `FileSystemDirectoryHandle` + `FileSystemFileHandle` + `FileSystemSyncAccessHandle`，不启完整 `FileSystemAccess`）。
- `cargo check --target wasm32-unknown-unknown -p vane-wasm --features opfs` 通过。
- core 零改动（Vfs trait 不污染 core，I-5/I-8）；`crates/vane-core/` git diff 为空。
- 容器 compaction（全量 rewrite）可触发且数据一致。

## 6. 前置依赖
- M2-01（vane-wasm cdylib + feature 体系）。
- 设计文档 `opfs-vfs-design.md`（已评审，路径 A）。

## 7. 不变量覆盖
- **I-5**：`cfg(target_arch="wasm32")` 允许在 VFS impl（SPEC §11/I-5）。OpfsVfs 在 vane-wasm crate（非 core），core 零 cfg、零改动。测试 1+18 守护。
- **I-8 binding 薄壳**：OpfsVfs 是 IO 适配层，无检索逻辑。行为测试在 core Vfs 套件。测试 1 守护。
- **I-6 manifest 原子性**（等价）：双 meta_slot + CRC 实现原子切换，对 core 透明（core 仍调 `rename`）。测试 13/14/15 守护。
- **core 同步 IO**（REQUIREMENTS §4.1）：SyncAccessHandle 在 Worker 内同步；唯一 await 在 Worker init（`createSyncAccessHandle()`），core 内部全同步。测试 1 守护。
- **Vfs trait 冻结签名**：8 方法签名零改动。验收「core git diff 为空」守护。
