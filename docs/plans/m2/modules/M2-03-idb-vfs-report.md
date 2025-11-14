# M2-03 IndexedDB 降级 VFS 实施报告

## 1. 实装概要

`crates/vane-wasm/src/vfs/idb.rs`（Create，feature = "idb"）实现 `IdbVfs`——OPFS 不可用时的浏览器降级 VFS 后端。复用 M2-02 `MemOverlay` 内核（文件表/区间/free list/双 meta_slot + CRC）零改动，差异仅在 `OverlayBackend` impl。

### 1.1 IdbBackend（内存 Vec 容器映像）

```rust
pub struct IdbBackend {
    data: RwLock<Vec<u8>>,   // 容器映像（Worker init 异步从 IDB 读取 blob 传入）
    dirty: AtomicBool,       // best-effort 标志
}
impl OverlayBackend for IdbBackend {
    fn read / write / size / truncate  // 操作内存 Vec（同步）
    fn flush() { dirty.store(true) }   // best-effort：标 dirty，不真正落盘
}
```

- `snapshot()` → 返回容器映像完整快照（JS 壳层 checkpoint tick put 回 IDB）
- `is_dirty()` / `clear_dirty()` → JS 壳层轮询 / checkpoint 完成清标志

### 1.2 IdbVfs（Vfs trait 8 方法委托 MemOverlay）

```rust
pub struct IdbVfs {
    overlay: MemOverlay,
    backend: Arc<IdbBackend>,
}
impl IdbVfs {
    pub fn from_blob(blob: Vec<u8>) -> Result<Self>;  // 新库传空 Vec
    pub fn schedule_checkpoint(&self);                 // 标 dirty
    pub fn snapshot(&self) -> Vec<u8>;                 // JS 壳层 checkpoint
    pub fn is_dirty(&self) -> bool;
    pub fn clear_dirty(&self);
}
impl Vfs for IdbVfs { /* 8 方法委托 MemOverlay（同步） */ }
```

## 2. OverlayBackend 复用（零改动 M2-02 overlay.rs）

`MemOverlay` + `OverlayBackend` trait + `persist_meta` + `read_meta_slot` + `compact_internal` 全部复用 M2-02 同一份代码。差异仅在 `OverlayBackend` impl：

| 后端 | flush 语义 | I-6 层级 |
|------|-----------|---------|
| `OpfsBackend`（M2-02） | `SyncAccessHandle.flush`（真落盘） | 等价原子 |
| `IdbBackend`（M2-03） | 标 dirty=true（不落盘） | 尽力持久化 |
| `MemoryBackend`（M2-02 测试） | no-op | 测试用 |

文件表/区间分配/双 meta_slot + CRC/free list/compaction shadow-write 全部共享——零重复实现。

## 3. sync best-effort 语义

`IdbVfs::sync(path)` 委托 `MemOverlay::sync`：
1. `persist_meta`（若 overlay state.dirty）：写非活跃 meta slot + superblock 到**内存 Vec**（容器映像一致）
2. `IdbBackend::flush`：标 `dirty=true`（**不真正落盘**）

JS 壳层（M2-04）异步 tick 轮询 `is_dirty()` → `snapshot()` → IDB `put` → `clear_dirty()`。

**不保证 sync 返回时已落盘 IDB**。崩溃可能丢最近未 checkpoint 的写入——降级场景可接受（REQUIREMENTS §4.1「浏览器存储非可靠存储，关键数据用 `export()` 快照导出」）。与 OPFS 主路径的 I-6 等价原子有意识地区分，文档明示。

## 4. worker.rs 最小探针占位

`idb.rs` 提供 `opfs_available() -> bool` stub（返 `true`）。M2-04 落实真实探针（`navigator.storage.getDirectory()` + feature 检测 + 写入能力 try/catch，Safari 历史 OPFS bug 缓解）：
- `true` → `OpfsVfs` 主路径
- `false` → 降级 `IdbVfs` + `console.warn`（不抛错，SPEC §10 E_UNSUPPORTED 消解）

不实装完整 Worker（M2-04 范畴）。本模块仅提供 IdbVfs 可被 M2-04 选用的接口 + 最小探针占位。

## 5. Cargo.toml / mod.rs 变更

- `Cargo.toml`：`idb` feature 启用 `dep:web-sys` + `dep:js-sys` + `web-sys/IdbDatabase` + `web-sys/IdbObjectStore` + `web-sys/IdbTransaction`（供 M2-04 Worker 壳层 IDB put/get）。`IdbVfs` 本身不直接调用 IDB API（薄层：`from_blob` 接收 Worker 异步读取的 blob，`snapshot` 供 Worker 异步 put 回 IDB）。
- `vfs/mod.rs`：`#[cfg(feature = "idb")] pub mod idb;` + re-export `IdbBackend`/`IdbVfs`。

## 6. 自证门禁结果

| # | 门禁 | 结果 |
|---|------|------|
| 1 | `cargo check --target wasm32-unknown-unknown -p vane-wasm --features idb` | ✅ 通过 |
| 2 | `cargo test --workspace --all-features` | ✅ 437 passed（417 基线 + 20 IdbVfs 新增，0 回退） |
| 3 | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | ✅ clean |
| 4 | `cargo fmt --all -- --check` | ✅ clean |
| 5 | `bash scripts/check-no-std-fs.sh` | ✅ OK |
| 6 | `cargo deny check` | ✅ advisories/bans/licenses/sources ok |
| 7 | 体积门禁（idb feature gzip ≤800KB） | ✅ **348,746 bytes（≈341KB）** |
| 8 | IdbVfs Vfs 语义测试（8 方法，node） | ✅ conformance + 17 语义测试全绿 |
| 9 | from_blob 恢复测试（空 Vec + 既有 blob） | ✅ 2 测试绿 |
| 10 | 降级不抛错（sync best-effort + schedule_checkpoint + 不返 E_UNSUPPORTED） | ✅ 5 测试绿 |
| 11 | IDB put/get 薄层 wasm32 编译通过 | ✅ 编译通过（浏览器验证待 M2-04） |

## 7. 体积实测

| 口径 | gzip 大小 | 门禁 |
|------|----------|------|
| vane-wasm default（M2-01 deliverable） | 348,752 bytes | ≤800KB ✅ |
| vane-wasm --features idb | 348,746 bytes | ≤800KB ✅ |
| vane-core --export-all（保守上界） | 636,722 bytes | ≤800KB ✅ |

**idb 增量 ≈ 0 bytes**：`IdbVfs` 不直接调用 web-sys IDB API（薄层），启用的 IDB sub-features 未被引用，wasm-opt dead-code 消除后无体积增长。

## 8. 测试清单（20 新增，全 node 可跑）

Vfs 语义（与 OpfsVfs/MemoryVfs 等价）：
- `idb_vfs_conformance`（完整 8 方法契约）
- `idb_create_empty_file_read_returns_zero`
- `idb_write_at_and_read_at_roundtrip`
- `idb_append_twice_offsets_and_size`
- `idb_rename_content_moves_source_gone`
- `idb_delete_then_list_excludes`
- `idb_list_segments_returns_ulid_entries`
- `idb_read_nonexistent_returns_io_err`
- `idb_large_append_across_pages`
- `idb_write_at_beyond_size_grows_with_zero_fill`
- `idb_double_meta_slot_alternation`

from_blob 恢复：
- `from_blob_empty_vec_initializes_new_library`
- `from_blob_recovers_existing_file_table`
- `snapshot_round_trip_data_intact`

sync best-effort + 降级不抛错：
- `sync_marks_dirty_no_panic`
- `schedule_checkpoint_marks_dirty`
- `clear_dirty_after_checkpoint`
- `sync_on_empty_no_panic`
- `downgrade_path_never_returns_e_unsupported`
- `opfs_available_stub_returns_true`

## 9. 遗留

- **IDB 浏览器验证待 M2-04**：`IdbVfs` 的 Vfs 语义已由 node 测试覆盖（与 `MemoryBackend` 等价的内存 Vec backend）。IDB 实际 put/get 是 JS 异步薄层（Worker 壳层 postMessage 边界），浏览器端验证在 M2-04 Worker 接入后进行。web-sys IDB sub-features 已在 Cargo.toml 启用，wasm32 编译通过。
- **opfs_available() 真实探针待 M2-04**：当前 stub 返 true。M2-04 落实 `navigator.storage.getDirectory()` 探测 + OPFS 不可用切 IdbVfs + console.warn。
- **性能对比非门禁**（SPEC 测试 10）：IDB vs OPFS 写吞吐对比预期 IDB 慢 3~10 倍，文档明示降级折损——留待 M2-04 Worker 接入后浏览器实测。
