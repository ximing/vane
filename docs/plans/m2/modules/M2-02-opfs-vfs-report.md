# M2-02 OPFS VFS 实装报告

> 状态：DONE
> commit：见 git log
> 日期：2026-08-09

## 1. overlay 设计实装

### 1.1 容器格式（container.rs）
单 OPFS 容器 `vane.db`，物理布局：

| 区域 | 偏移 | 大小 | 说明 |
|---|---|---|---|
| superblock | 0 | 4 KB | magic(4) + format_version(4) + active_meta_slot(1) + reserved(7) + meta_offset[2](16) + meta_size[2](16) + container_size(8) + reserved |
| meta_slot_0 | 4096 | 256 KB | generation(8) + data_len(4) + crc32(4) + payload[data_len] + padding |
| meta_slot_1 | 266240 | 256 KB | 双槽，等大预留 |
| data area | 528384 | 动态 | 文件区间，按分配序 |

- `DATA_OFFSET = 528384`（superblock + 双 meta slot）。
- meta slot payload 自包含：container_size + file_table[] + free_list[]。
- CRC32 手写（IEEE 802.3, polynomial 0xEDB88320），无新依赖。

### 1.2 文件表 + free list
- `file_table: HashMap<String, Extent>`——虚拟路径 → `(base, size)` 区间。
- `free_list: Vec<Extent>`——已释放区间，first-fit 复用。
- `container_size`——数据区已分配末尾，append 分配新区间。

### 1.3 双 meta_slot + CRC（I-6 原子性）
- `persist_meta`：写非活跃槽 → flush → 翻转 active → 写 superblock → flush。
- recover 始终校验双槽 CRC，取 generation 最大且 CRC 通过者（不依赖 superblock active hint）。
- `rename`/`delete` 持久化元数据；`sync` 在 dirty 时持久化；`create`/`write_at`/`append` 仅标记 dirty（不持久化）。

### 1.4 compaction（全量 rewrite）
- `compact_internal`：读所有活跃数据到内存 → 从 DATA_OFFSET 紧凑重写 → 截断后端 → 清空 free list。
- 触发条件：`delete` 后 `free_space / (container_size - DATA_OFFSET) > 50%`。
- 单次 persist_meta（compact_internal 不持久化，delete 统一持久化）——避免双 persist 导致 generation 跳跃。

### 1.5 sync 粒度
- `sync(path)`：dirty 时 persist_meta + flush；否则仅 flush。单容器统一 flush。

## 2. OverlayBackend trait + OpfsBackend/MemoryBackend

```rust
pub trait OverlayBackend: Send + Sync {
    fn read(&self, off: u64, buf: &mut [u8]) -> Result<usize>;
    fn write(&self, off: u64, buf: &[u8]) -> Result<()>;
    fn flush(&self) -> Result<()>;
    fn size(&self) -> Result<u64>;
    fn truncate(&self, sz: u64) -> Result<()>;
}
```

- **MemoryBackend**：`RwLock<Vec<u8>>`，flush no-op。支持 snapshot/truncate_data/corrupt_byte（崩溃模拟）。原生可测。
- **OpfsBackend**（feature = "opfs"）：`FileSystemSyncAccessHandle` 薄封装。`read_with_u8_array_and_options` / `write_with_u8_array_and_options`（`FileSystemReadWriteOptions.set_at(offset)`）。`unsafe impl Send/Sync`（wasm32 单线程安全）。

## 3. 五项次要处理

1. **Arc 循环**：MemOverlay 持有 `Arc<dyn OverlayBackend>`，OpfsBackend 不反向持有 MemOverlay。无循环。MemOverlay 用 `RwLock<OverlayState>` 做内部可变（满足 Vfs: Send + Sync）。
2. **superblock 自损坏恢复**：recover 不依赖 superblock active hint。magic 损坏时仍读双 meta slot。双槽 CRC 通过 → 取 max generation 恢复；双槽都坏 → `Err(VaneError::Io)`（需 export 快照恢复）。测试覆盖两种场景。
3. **compaction 机制**：`compact_internal`（不持久化）+ `delete` 统一 `persist_meta`。阈值 50%。无死循环（compaction 不触发 delete/rename）。
4. **VaneWorker Arc<dyn Vfs>**：OpfsVfs impl Vfs，可 `Arc<dyn Vfs>`（M2-04 注入）。
5. **header 行号**：实现以实际代码为准（container.rs 常量 + MetaSlot encode/decode）。

## 4. 崩溃恢复 3 时点

| 时点 | 模拟方式 | recover 结果 | 测试 |
|---|---|---|---|
| sync(tmp) 后、rename 前 | 不调 rename，丢弃 overlay | active 旧槽：manifest.json → OLD | `crash_recovery_after_sync_before_rename` ✓ |
| rename 元数据写一半（CRC 损坏） | rename 后 corrupt active meta slot | CRC 失败 → 回退旧槽：manifest.json → OLD | `crash_recovery_meta_write_partial` ✓ |
| rename flush 后 | rename 完成后丢弃 | 新槽 active：manifest.json → NEW | `crash_recovery_after_rename_flush` ✓ |

## 5. 自证门禁结果

| # | 门禁 | 结果 |
|---|---|---|
| 1 | `cargo check --target wasm32 -p vane-wasm --features opfs` | ✓ Finished |
| 2 | `cargo test --workspace --all-features` | ✓ 415 passed, 0 failed |
| 3 | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | ✓ Finished |
| 4 | `cargo fmt --all -- --check` | ✓ clean |
| 5 | `bash scripts/check-no-std-fs.sh` | ✓ OK |
| 6 | `cargo deny check` | ✓ advisories/bans/licenses/sources ok |
| 7 | 体积：wasm-opt -Oz → gzip | **351,032 bytes（343 KB）≤ 800 KB** ✓ |
| 8 | Overlay Vfs 语义测试（MemoryBackend） | ✓ 36 tests（含 conformance 套件） |
| 9 | 崩溃恢复 3 时点 | ✓ 3 tests |
| 10 | compaction 测试 | ✓ `compaction_full_rewrite_data_intact` + free list reuse |
| 11 | superblock 自损坏恢复 | ✓ `superblock_corruption_recovers_from_meta_slots` + `both_meta_slots_corrupt_returns_err` |
| 12 | OpfsBackend wasm32 编译 | ✓（薄层，浏览器手动验证待 M2-04） |

## 6. 体积实测

| 配置 | wasm-opt -Oz | gzip |
|---|---|---|
| 无 opfs（baseline） | — | 348,751 bytes (341 KB) |
| 有 opfs | — | 351,032 bytes (343 KB) |
| **opfs 增量** | — | **2,281 bytes (2.2 KB)** |

web-sys subset（FileSystemSyncAccessHandle + FileSystemReadWriteOptions + FileSystemFileHandle + FileSystemDirectoryHandle + Storage）增量 2.2 KB gzip，远低于预期 50 KB。

## 7. 测试摘要

vane-wasm 新增 36 个原生单元测试：
- Vfs 契约 conformance（8 方法全语义）
- create/read_at/write_at/append/sync/rename/delete/list 专项
- 嵌套虚拟路径、错误码、list 排序
- 容器 round-trip（close → reopen → 文件表一致）
- 双 meta_slot 翻转（generation 递增 + slot 交替）
- 崩溃恢复 3 时点
- free list 复用（first-fit，container_size 不增长）
- compaction（全量 rewrite + 数据一致 + round-trip）
- superblock 损坏恢复 + 双槽损坏 Err

## 8. 遗留

- **OpfsBackend 浏览器验证**：SyncAccessHandle 运行时行为（read/write/flush/truncate/getSize）需在真实浏览器 Worker 中验证（M2-04 Worker shell 接入后）。当前仅 wasm32 编译通过。
- **sync 性能优化**：初版每次 sync 都 flush（保守）。dirty 合并去重留作 M2 后期。
- **meta slot 容量**：256 KB 预留，约支持 ~10k 文件表项。超限返回 Err（可扩大 META_SLOT_SIZE）。

## 9. Fix Round 1（评审 PASS_WITH_FINDINGS 修复）

### M-1（数据完整性）：compaction shadow-write 崩溃安全

**问题**：原 `compact_internal` 从 DATA_OFFSET 原地破坏性重写数据区。若在 persist_meta 前崩溃，旧 meta slot 仍 active 但其 extent 指向的数据已被覆盖 → 段文件/WAL 数据损坏。

**修复（shadow-write 模式）**：
1. 读所有活跃数据到内存（备份）。
2. 写紧凑数据到**容器尾部新区域** `[old_cs, old_cs + live_size)`——旧数据区 `[DATA_OFFSET, old_cs)` 暂不破坏。
3. 更新 state：新 file_table 指向尾部，旧区域进 free_list，container_size = old_cs + live_size。
4. `persist_meta`（双槽翻转，原子提交）——新 meta 指向尾部新区域。
5. 旧区域在 free_list 中，后续 append 可 first-fit 复用。

**崩溃恢复**：
- 崩溃在 persist 前/中（CRC 失败）→ 旧 meta active、旧数据完整（未被覆盖）→ recover 回旧状态。
- 崩溃在 persist 后 → 新 meta active、新数据在尾部完整 → recover 到新状态。

**新增测试**：
- `compaction_crash_persist_fails_old_data_intact`：compaction persist CRC 损坏 → recover → 旧数据完整（a.bin + b.bin 均可读，delete 回滚）。
- `compaction_crash_after_persist_new_data_intact`：compaction 完成 → recover → 新数据在尾部完整，b.bin 已删除。

### I-1：OpfsBackend::truncate u32 → f64

**问题**：`truncate_with_u32` 限制容器上限 4GB。
**修复**：改用 `truncate_with_f64`（SyncAccessHandle.truncate 接受 f64/JS Number），无 4GB 限制。

### I-3：时点 B 测试语义修正

**问题**：原测试模拟「rename 后损坏 active 槽」而非计划描述的「写一半 inactive 槽」。
**修复**：改为记录 rename 前的 inactive 槽号，rename 后覆写该槽（inactive→active）payload 字节使 CRC 失败——语义为「persist_meta 写 inactive 槽中途截断」。recover 校验 CRC 失败 → 回退旧 active 槽 → 旧 manifest 完好。

### 门禁结果（fix round 1）

| # | 门禁 | 结果 |
|---|---|---|
| 1 | wasm32 check --features opfs | ✓ |
| 2 | workspace tests | ✓ 417 passed, 0 failed（+2 compaction 崩溃测试） |
| 3 | clippy -D warnings | ✓ |
| 4 | fmt --check | ✓ |
| 5 | no-std-fs | ✓ |
| 6 | cargo deny | ✓ |
| 7 | 体积 gzip | 351,027 bytes（343 KB）≤ 800 KB ✓ |

### 体积（fix 后）

opfs 增量仍 2.2 KB gzip（shadow-write 不影响 wasm 体积，仅运行时逻辑变更）。
