# M2-03 IndexedDB 降级 VFS

## 1. 目标
实现 `IdbVfs`，作为 OPFS 不可用时的浏览器降级 VFS 后端，实现 `vane_core::vfs::Vfs` trait（M0 冻结，`vfs/mod.rs:5-13`），适配层在 vane-wasm（不污染 core），降级不抛错（SPEC §6.1/§10 E_UNSUPPORTED 禁止到达：自动降级，REQUIREMENTS §4.1）。

**复用 M2-02 overlay 内核**（`MemOverlay` + `OverlayBackend`，后端无关）：底层后端换为内存 `Vec<u8>`（容器映像）+ 异步 checkpoint，sync 语义降级为 best-effort。文件表/区间分配/双元数据/CRC 与 OpfsVfs 共享同一份代码，差异仅在 `OverlayBackend` impl。

SPEC 节号：§6.1（idb 降级后端，适配层在 binding crate）、§10（E_UNSUPPORTED 仅无 OPFS 且未启用 idb 时，本模块消解）、REQUIREMENTS §4.1（OPFS 主 + IDB 降级；浏览器存储非可靠存储，关键数据用 `export()` 快照导出）。

## 2. 涉及文件
- **Create** `crates/vane-wasm/src/vfs/idb.rs`：`IdbVfs` struct + `impl OverlayBackend`（内存 Vec 后端）+ `impl Vfs`（委托 `MemOverlay`）+ 异步 checkpoint tick。
- **Modify** `crates/vane-wasm/Cargo.toml`：`[features] idb = ["dep:web-sys", "dep:js-sys"]`（web-sys feature `IdbDatabase`/`IdbObjectStore`/`IdbTransaction`）。
- **Modify** `crates/vane-wasm/src/vfs/mod.rs`：`#[cfg(feature="idb")] pub mod idb;`。
- **Modify** `crates/vane-wasm/src/worker.rs`（M2-04 协同）：init 探针 `opfs_available()` → OPFS 不可用切 IdbVfs（不抛错，console.warn）。
- **复用** `crates/vane-wasm/src/vfs/overlay.rs`（M2-02 产出，零改动）。

## 3. 接口契约
### Consumes from
- M0 `vane_core::vfs::Vfs` trait（`vfs/mod.rs:5-13`，8 方法）。
- M0 `vane_core::types::{Result, VaneError}`。
- **M2-02 `MemOverlay` + `OverlayBackend`**（overlay 内核，后端无关；M2-02 已产出）。
- M2-02 OpfsVfs（同 trait，降级路径）。

### Produces for
```rust
// crates/vane-wasm/src/vfs/idb.rs（feature = "idb"）
pub struct IdbVfs {
    overlay: MemOverlay,
    // backend = 内存 Vec<u8> 容器映像（Arc<RefCell<Vec<u8>>>）
    // dirty: AtomicBool
    // checkpoint handle：JS 壳层异步 tick 触发 IDB put
}
impl OverlayBackend for IdbBackEnd {
    // read/write/size/truncate 操作内存 Vec（同步）
    // flush 标 dirty=true（best-effort，不真正落盘）
}
impl IdbVfs {
    /// Worker init 异步从 IDB 读取容器 blob 到内存 Vec 后传入。
    pub fn from_blob(blob: Vec<u8>) -> Result<Self>;  // 新库传空 Vec
}
impl vane_core::vfs::Vfs for IdbVfs { /* 8 方法，委托 MemOverlay（同步） */ }

// 异步 checkpoint（JS 壳层 postMessage 触发）：
pub fn schedule_checkpoint(&self);  // 标 dirty，由 worker tick 异步 put 内存 blob 回 IDB
```

**sync 语义**：`sync(path)` best-effort —— 标 `dirty=true`，由 JS 壳层异步 tick（postMessage 触发）把内存 blob `put` 回 IDB。**不保证 sync 返回时已落盘**。这符合 REQUIREMENTS §4.1「IDB 降级」+「浏览器存储非可靠存储，关键数据用 `export()` 快照导出」+ §12.4 词典降级「不抛错」的语义层级。IDB 降级场景性能逊于 OPFS（写吞吐慢 3~10 倍），文档明示降级折损。

**与 M2-02 契约一致**：overlay 内核共享（`MemOverlay` + `OverlayBackend`），文件表/区间分配/双元数据/CRC 同一份代码。差异仅在 `OverlayBackend`：
- OPFS = `SyncAccessHandle`（sync flush 真落盘，I-6 等价原子）
- IDB = 内存 `Vec<u8>`（sync 标 dirty，异步 dump；I-6 语义降级为「尽力持久化」，崩溃可能丢最近未 checkpoint 的写入——降级场景可接受，关键数据走 `export()`）

下游：M2-04（Worker init 探针选择 OPFS/IDB）。

## 4. TDD 测试清单
1. **Vfs 套件复用**：`IdbVfs` 跑 M0 Vfs 通用契约测试套件（同 M2-02 测试 1，wasm-bindgen-test in Worker）。
2. **降级不抛错**：`opfs_available() == false` 时 Worker init 切 `IdbVfs`，`vane_open` 返回 OK（不返 E_UNSUPPORTED，SPEC §10）。
3. `create` + `write_at` + `read_at` 回读一致（经内存 Vec + overlay）。
4. `append` 两次 → offset 正确，长度累加（append 分配新区间）。
5. `sync(path)` 标 dirty，不 panic；后续 checkpoint tick 把内存 blob put 回 IDB（mock 异步 tick 验证）。
6. `rename(from, to)`：源内容到 to，源删除（内存表改挂 + 元数据落非活跃槽；与 OpfsVfs 同路径）。
7. `delete` + `list` 一致（区间进 free list）。
8. **容器持久化**：`from_blob(blob)` 重建 → 文件表与原一致；checkpoint 后重新 `from_blob` 读回 → 数据一致。
9. **错误码**：`read_at` 不存在 → `Err(VaneError::Io)`（code -1）。
10. **性能文档化**（非门禁）：记录 IdbVfs vs OpfsVfs 写吞吐对比（预期 IDB 慢 3~10 倍），文档明示降级场景折损 + 「关键数据用 `export()` 快照导出」。
11. **sync 语义文档化**：断言 `sync` 返回后内存 Vec 已更新但 IDB 未必然落盘（mock IDB put 延迟）；`schedule_checkpoint` 后 IDB 落盘。

## 5. 验收标准
- Vfs 通用契约测试套件在 IdbVfs 上全绿（wasm-bindgen-test in Worker）。
- 降级路径不抛 E_UNSUPPORTED（SPEC §10 消解）。
- 体积：启用 `idb` feature 后 vane-wasm gzip ≤800KB。
- `cargo check --target wasm32-unknown-unknown -p vane-wasm --features idb` 通过。
- core 零改动；overlay 内核与 M2-02 共享（无重复实现）。
- sync best-effort 语义文档化（降级场景数据可靠性提示 + export 快照补救）。

## 6. 前置依赖
- M2-02（OPFS Vfs + `MemOverlay`/`OverlayBackend` 内核，本模块复用）。

## 7. 不变量覆盖
- **I-5**：`cfg(target_arch="wasm32")` 在 vane-wasm（VFS 适配层），core 零 cfg。
- **I-8 binding 薄壳**：IdbVfs 是 IO 适配层，无检索逻辑。行为测试在 core Vfs 套件。
- **降级不抛错**（SPEC §12.4/§10）：测试 2 守护。
- **core 同步 IO**：`Vfs` trait 8 方法均为同步签名；异步只在 IDB checkpoint tick（JS 壳层 postMessage 边界，REQUIREMENTS §4.1 允许）。测试 1 契约守护。
- **I-6 语义降级**（明示）：IDB 降级路径下 manifest 原子性为「尽力持久化」（内存表双 meta_slot + CRC 保证内存一致性，但崩溃可能丢未 checkpoint 的写入）；关键数据走 `export()` 快照（M2-12）。与 OPFS 主路径的 I-6 等价原子有意识地区分，文档明示。
