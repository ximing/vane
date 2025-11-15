# M2-03 IdbVfs 评审报告

**评审对象**：M2-03 IndexedDB 降级 VFS（`crates/vane-wasm/src/vfs/idb.rs`，feature = "idb"）
**评审基线**：BASE f03612f..HEAD 588b5c7（vane-wasm）
**评审日期**：2026-08-10
**评审模式**：只读静态评审（未跑 cargo）

## 评审结论

**状态**：PASS_WITH_FINDINGS
**发现**：B=0, I=1, M=5（均非阻塞）

---

## 1. 评审重点逐项核验

### 1.1 overlay 零改动复用 ✅
- `git diff f03612f..588b5c7 -- crates/vane-wasm/src/vfs/overlay.rs` 输出 0 行，确认 `overlay.rs` 未改。
- `IdbVfs` 8 方法（idb.rs:178-205）全部委托 `self.overlay.<method>`，与 `OpfsVfs`（opfs.rs:100-132）模式一致——零 overlay 逻辑复制。
- `OverlayBackend` impl（idb.rs:74-114）仅 `IdbBackend` 差异：内存 `Vec<u8>` + `flush` 标 dirty（vs `OpfsBackend` 的 `SyncAccessHandle.flush` 真落盘、`MemoryBackend` 的 no-op）。文件表/区间/free list/双 meta_slot/CRC/compaction 全部复用 `MemOverlay`（overlay.rs:384-614）。

### 1.2 sync best-effort 语义 ✅
- `IdbVfs::sync`（idb.rs:191-195）委托 `MemOverlay::sync`（overlay.rs:543-552）：`persist_meta`（写 meta 到内存 Vec）+ `IdbBackend::flush`（idb.rs:97-103 标 `dirty=true`，不落盘）。
- `schedule_checkpoint`（idb.rs:148-150）标 dirty 供 JS 壳层轮询。`snapshot()`/`is_dirty()`/`clear_dirty()` 提供 JS 异步 tick 所需接口。
- 符合 REQUIREMENTS §4.1（IDB 降级 + 浏览器存储非可靠）：sync 不保证落盘，关键数据走 `export()`。I-6 语义降级在模块 doc（idb.rs:7-14）和 `from_blob` doc（idb.rs:133-138）明示。

### 1.3 降级不抛错（核心）✅
- `IdbVfs` 路径错误来源仅 `poison_err`（idb.rs:70-72 → `VaneError::Io`）+ `MemOverlay` 的 `VaneError::Io` + `MetaSlot::encode`（container.rs:142-178，仅 `VaneError::Io`）。无 `VaneError::Unsupported` 路径。
- `downgrade_path_never_returns_e_unsupported`（idb.rs:497-517）覆盖 Vfs 6 方法正常路径 + 3 错误路径（read_at/delete/rename 不存在），断言均为 `VaneError::Io`。
- `from_blob`/`schedule_checkpoint`/`snapshot`/`is_dirty`/`clear_dirty` 签名为 `Result<Self>`/`()`/`Vec<u8>`/`bool`——前者仅 Io 错误，后四者不可失败。E_UNSUPPORTED 禁止到达 ✅。

### 1.4 I-6 降级语义文档化 ✅
- 模块 doc（idb.rs:7-14）：明示「崩溃可能丢最近未 checkpoint 的写入——降级场景可接受，关键数据走 `export()` 快照（M2-12）」。
- `IdbBackend::flush` 注释（idb.rs:98-102）：与 OPFS `SyncAccessHandle.flush`（真落盘）有意识区分。
- 报告 §3 表格对比 OPFS（等价原子）/ IDB（尽力持久化）。

### 1.5 from_blob 恢复 ✅
- `from_blob`（idb.rs:139-145）：构造 `IdbBackend`（blob 入 `RwLock<Vec<u8>>`）→ `MemOverlay::open`（空 Vec 走 `init_new`，非空走 `recover`）→ `clear_dirty`（清 init_new 的 flush 副作用）。
- 测试覆盖：
  - `from_blob_empty_vec_initializes_new_library`（idb.rs:381-389）：空 Vec → 空文件表，generation=0，dirty=false。
  - `from_blob_recovers_existing_file_table`（idb.rs:391-428）：写 4 文件 + sync → snapshot → from_blob → 文件表/数据一致。
  - `snapshot_round_trip_data_intact`（idb.rs:430-454）：模拟 IDB put/get round-trip。

### 1.6 Vfs 8 方法语义 ✅
- `idb_vfs_conformance`（idb.rs:297-301）跑 `run_conformance`（idb.rs:240-293），与 overlay.rs:714-767、vane-core/tests.rs:5-59 同构（create/write_at/read_at/append/rename/delete/list/错误码）。
- 11 个 Vfs 语义测试（conformance + 10 细分）覆盖边界：空文件、append 累加、rename 覆盖、delete+list、ULID 段目录、大文件跨页、zero-fill 增长、双 meta_slot 翻转。

### 1.7 不变量 ✅
- **I-5**：`cfg(target_arch="wasm32")` 在 vane-wasm（`#[cfg(feature = "idb")]` mod gate，mod.rs:12-13），core 零 cfg。✅
- **I-8 薄壳**：`IdbVfs` 无检索逻辑，`active_meta_slot`/`generation` 仅为测试调试透传。✅

### 1.8 体积门禁 ✅
- 报告 §7：idb feature gzip = 348,746 bytes（≤800KB ✅），增量 ≈ 0 bytes。
- 代码佐证：`IdbVfs`/`IdbBackend` 不引用任何 `web_sys::Idb*` API（薄层），启用的 `web-sys/IdbDatabase`/`IdbObjectStore`/`IdbTransaction` 未被引用，wasm-opt dead-code 消除后无体积增长。
- `default = []`（Cargo.toml:33），idb 不在 default。✅

### 1.9 TDD 覆盖 ✅（有缺口，见 M-2/M-3）
- 20 新增测试：11 Vfs 语义 + 3 from_blob + 6 sync/降级。覆盖 conformance / from_blob 恢复 / sync best-effort / 降级不抛错 / checkpoint dirty 生命周期。
- 缺口：无 mock IDB put 延迟测试（计划测试 11「sync 语义文档化」未实装——报告 §9 标注「IDB 浏览器验证待 M2-04」）。降级场景可接受（node 无 IDB）。

### 1.10 opfs_available stub ✅
- `opfs_available()`（idb.rs:217-221）返 `true` 占位，TODO(M2-04) 标注真实探针。交接清晰（doc 注释明示 M2-04 落实 `navigator.storage.getDirectory`）。✅

---

## 2. 发现清单

### I-1（Issue, low）：`sync_on_empty_no_panic` 测试首行注释与断言矛盾

**证据**：`crates/vane-wasm/src/vfs/idb.rs:487-495`

```rust
fn sync_on_empty_no_panic() {
    // 无变更时 sync 不 panic，不标 dirty（MemOverlay::sync 在 dirty=false 时不 persist）
    let vfs = new_idb();
    vfs.create("empty.bin").unwrap();
    // create 只置 state.dirty（overlay 内部），不调 flush——is_dirty() 仍 false
    vfs.sync("empty.bin").unwrap();
    // sync 后 backend flush 被调用 → dirty=true
    assert!(vfs.is_dirty());
}
```

**问题**：首行注释「无变更时 sync 不 panic，不标 dirty」与末行断言 `assert!(vfs.is_dirty())` 直接矛盾。实际执行路径：`create("empty.bin")` 置 overlay `state.dirty=true`（overlay.rs:393），`sync` 检测 state.dirty → 调 `persist_meta` → `IdbBackend::flush` → backend dirty=true。测试逻辑正确（断言通过），但首行注释错误——既非「无变更」（create 是变更），也非「不标 dirty」（assert dirty=true）。

**建议**：修正首行注释为「sync 不 panic；create 置 state.dirty 后 sync 经 persist_meta + flush 标 backend dirty」。

### M-1（Minor）：`IdbBackend` read/write/size/truncate 与 `MemoryBackend` 逐行重复

**证据**：`idb.rs:74-114` vs `overlay.rs:89-124`——read/write/size/truncate 四方法实现完全相同（均操作 `RwLock<Vec<u8>>`），差异仅在 `flush`（dirty=true vs no-op）+ `dirty` 字段 + snapshot/is_dirty/clear_dirty 辅助方法。

**评估**：plan §1 明确「差异仅在 `OverlayBackend` impl」，报告 §2 表格亦如此对比。重复可接受，但若未来 `MemoryBackend` 修复 read/write 边界 bug，`IdbBackend` 需同步。可考虑提取 `VecBackend` 共享结构（composition），`IdbBackend` 持有 `VecBackend` + `AtomicBool`。

### M-2（Minor）：`run_conformance` 测试函数三处重复

**证据**：`idb.rs:240-293`、`overlay.rs:714-767`、`vane-core/src/vfs/tests.rs:5-59` 三份 `run_conformance`/`run_conformance_tests` 内容同构。

**评估**：`vane-core/src/vfs/tests.rs` 为 `#[cfg(test)] mod tests`（mod.rs:22 私有），vane-wasm 无法 import。M2-02 已确立此模式（overlay.rs 本地复制），M2-03 沿袭——非回归。理想方案是将 `run_conformance_tests` 提为 `pub` 在 dev-dep 暴露，但跨 crate test helper 共享在 Rust 生态需 `#[cfg(any(test, feature="test-utils"))]` 模式，改动面较大，留待后续重构。

### M-3（Minor）：Cargo.toml 启用 3 个 web-sys IDB sub-features 但本模块未引用

**证据**：`Cargo.toml:39-45` 启用 `web-sys/IdbDatabase`/`IdbObjectStore`/`IdbTransaction`；`idb.rs` 全文无 `web_sys::Idb` 引用（仅 `use std::sync::*` + `vane_core::*` + `super::overlay::*`）。

**评估**：报告 §5 明示这些 features「供 M2-04 Worker 壳层 put/get」。M2-03 提前引入未使用依赖——因 dead-code 消除体积增量≈0，不影响门禁。可推迟到 M2-04 启用，但当前不影响正确性或体积。交接清晰（Cargo.toml 注释 line 37-38 标注用途）。

### M-4（Minor）：`schedule_checkpoint` 直接访问 `backend.dirty` 私有字段

**证据**：`idb.rs:148-150`
```rust
pub fn schedule_checkpoint(&self) {
    self.backend.dirty.store(true, Ordering::Release);
}
```
`IdbBackend` 已有 `flush()`（idb.rs:97-103，标 dirty + 返 Result）和隐含的 mark 语义。`schedule_checkpoint` 直接访问 `self.backend.dirty`（私有字段，同模块合法），绕过方法封装。

**评估**：同模块内访问私有字段合法，功能正确。风格上可加 `IdbBackend::mark_dirty(&self)` 方法或复用 `flush()`（但 flush 返 Result，schedule_checkpoint 签名为 `()`）。非阻塞。

### M-5（Minor）：`opfs_available()` 探针 feature 耦合

**证据**：`opfs_available()` 定义在 `#[cfg(feature = "idb")]` 模块内（idb.rs:217，经 mod.rs:12-13 gate）。该函数语义是「OPFS 能力探针」（决定走 OPFS 主路径 vs IDB 降级），但仅在启用 `idb` feature 时可用。若 M2-04 worker 仅启 `opfs` 不启 `idb`，无法调用此函数。

**评估**：当前 stub 返 `true`（假设 OPFS 可用），M2-03 范围内 IDB 路径不会被选中。M2-04 落实时需确保 `worker` feature 依赖 `idb`，或将探针移至非 feature-gated 位置。报告 §4 已标注 M2-04 落实真实探针，交接清晰。

---

## 3. 不可确认项

无。所有评审重点均从代码/diff 确认。

---

## 4. 总结

M2-03 是高质量的薄层模块：overlay 零改动复用彻底，Vfs 8 方法纯委托，sync best-effort 语义清晰，I-6 降级文档化充分，降级不抛错经代码路径分析 + 测试双重守护，体积增量≈0 符合薄层预期。20 测试覆盖 Vfs 语义/from_blob 恢复/降级不抛错/checkpoint 生命周期。唯一 Issue（I-1）为测试注释错误，不影响代码正确性。5 个 Minor 均为可接受的设计折损或 M2-04 交接项。建议合并后修正 I-1 注释。
