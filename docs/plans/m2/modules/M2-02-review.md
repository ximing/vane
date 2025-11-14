# M2-02 OPFS VFS 评审报告

> 评审人：task reviewer（只读）
> 日期：2026-08-09
> 范围：M2-02 单 OPFS 容器 + MemOverlay + 双 meta_slot 崩溃恢复
> BASE a581d1f..HEAD 39650ea

## 0. 结论

**PASS_WITH_FINDINGS**

- B（阻塞）：0
- I（Issue）：3
- M（Major）：1
- Minor：4

core/Vfs 零改动已确认（`git diff a581d1f..39650ea -- crates/vane-core/` 为空）。Vfs trait 8 方法签名与 `crates/vane-core/src/vfs/mod.rs:5-13` 完全一致。MemOverlay impl Vfs 8 方法（overlay.rs:1015-1244）语义与 MemoryVfs/StdFsVfs 等价（conformance 测试同构覆盖）。崩溃恢复 I-6 等价性成立——双 meta_slot + CRC + generation 回退逻辑正确，3 时点测试覆盖（时点 B 模拟方式有偏差，见 I-3）。主要发现是 compaction 崩溃恢复间隙（M-1，非 I-6 违规但数据完整性隐患）。

---

## 1. Vfs trait 零改动 + 8 方法语义（重点 1）

**结论：PASS**

- `git diff a581d1f..39650ea -- crates/vane-core/` 输出空——core 零改动确认（I-5/I-8 守护）。
- Vfs trait 8 方法签名（`crates/vane-core/src/vfs/mod.rs:5-13`）与 `impl Vfs for MemOverlay`（overlay.rs:1015-1244）、`impl Vfs for OpfsVfs`（opfs.rs:606-638）完全一致。
- 语义对照 MemoryVfs（`vfs/memory.rs`）/ StdFsVfs（`vfs/std_fs.rs`）：
  - `create`：已存在报 Err——与 MemoryVfs:32-34、StdFsVfs:62-64 一致。
  - `read_at`：offset>=size 返回 0——与 MemoryVfs:45-47 一致。
  - `append`：返回旧 size——与 MemoryVfs:73 一致。
  - `rename`：覆盖目标——与 StdFsVfs:113-115（先删目标再 rename）一致。
  - `list`：前缀过滤 + 下一层分量 + sort + dedup——与 MemoryVfs:99-121 逐行等价。
- conformance 测试（overlay.rs:1344-1397 `run_conformance`）与 core `run_conformance_tests`（`vfs/tests.rs:5-59`）逻辑同构。见 I-2（drift 风险）。

**Minor 2（语义微差，非阻塞）**：MemOverlay::write_at/append 要求文件预先 create（overlay.rs:1047-1051、1121-1125），StdFsVfs::write_at/append 自动 create（`OpenOptions.create(true)`，std_fs.rs:82、95）。core 调用面（ManifestStore/SegmentBuilder/Wal）均先 create 再 write，不影响。MemOverlay::list 对无匹配前缀的 dir 返回空 Vec（overlay.rs:1229），StdFsVfs::list 对不存在 dir 返回 Err（read_dir 失败）。core recover 用 `let _ = list` 容错，不影响。

---

## 2. 崩溃恢复正确性（I-6 等价，重点 2 / 核心）

**结论：I-6 等价性成立。3 时点覆盖。1 个模拟偏差（I-3）。**

### 2.1 persist_meta 顺序分析（overlay.rs:1273-1310）

```
1. 写非活跃 meta slot（encoded bytes）
2. backend.flush()                              ← meta 落盘
3. 更新 in-memory state（active_meta_slot = inactive, generation++, dirty=false）
4. 写 superblock（active hint + container_size hint）
5. backend.flush()                              ← superblock 落盘
```

- recover（overlay.rs:836-889）**不依赖 superblock active hint**——始终读双 meta slot，取 `generation` 最大且 CRC 通过者（overlay.rs:855-874）。superblock 损坏时仍恢复（测试 `superblock_corruption_recovers_from_meta_slots` 覆盖）。
- 步骤 2 后、步骤 5 前崩溃：非活跃槽已落盘（generation 新、CRC 通过），superblock hint 旧——recover 取 max generation → 新槽。正确。
- 步骤 1 写一半崩溃：非活跃槽 CRC 失败——recover 回退活跃旧槽。正确。

### 2.2 manifest 原子性 3 时点（core save_atomic 序列）

core `ManifestStore::save_atomic`：`delete(tmp) → create(tmp) → write_at(tmp) → sync(tmp) → rename(tmp, manifest.json)`。

在 MemOverlay 中：
- `write_at(tmp)`：分配区间写数据，dirty=true，**未持久化元数据**。
- `sync(tmp)`：dirty → persist_meta（元数据落盘：tmp 存在，manifest.json → OLD，generation++）。
- `rename(tmp, manifest.json)`：表项改挂 + persist_meta（manifest.json → NEW，generation++）。

| 时点 | 测试 | recover 结果 | 判定 |
|---|---|---|---|
| sync(tmp) 后、rename 前 | `crash_recovery_after_sync_before_rename`（overlay.rs:1596） | active 旧槽：manifest.json → OLD | ✓ I-6 |
| rename 元数据写一半（CRC 损坏） | `crash_recovery_meta_write_partial`（overlay.rs:1621） | CRC 失败 → 回退旧槽：manifest.json → OLD | ✓ I-6（见 I-3） |
| rename flush 后 | `crash_recovery_after_rename_flush`（overlay.rs:1653） | 新槽 active：manifest.json → NEW | ✓ I-6 |

### 2.3 superblock 自损坏恢复

- recover 先尝试读 superblock magic（overlay.rs:838-846），但**仅作 `_sb_ok` 标记**，不阻断。
- 双 meta slot 任一 CRC 通过 → 取 max generation 恢复。
- 双槽都坏 → `Err(VaneError::Io)`（overlay.rs:864-873）——需 export 快照恢复。策略合理。
- 测试：`superblock_corruption_recovers_from_meta_slots`（overlay.rs:1772）+ `both_meta_slots_corrupt_returns_err`（overlay.rs:1792）覆盖。

### 2.4 manifest 原子性对 core 透明

core 仍调 `rename(tmp, manifest.json)`，MemOverlay 内部用双 meta_slot + CRC 实现等价原子切换。core 无感知。I-6 等价成立。

---

## 3. compaction（重点 3）

**M-1（Major）：compaction 崩溃恢复间隙——破坏性原地重写，persist_meta 前崩溃可损坏非 manifest 数据**

`compact_internal`（overlay.rs:937-984）步骤：
1. 读所有活跃文件数据到内存 `live: Vec<(String, Vec<u8>)>`
2. **从 DATA_OFFSET 起原地重写**（`self.backend.write(new_base, data)`，new_base 从 DATA_OFFSET 递增）——**覆盖旧数据区**
3. 更新 in-memory state（file_table 重映射、free_list 清空、container_size 缩减）
4. `backend.truncate(container_size)`

随后 `delete`（overlay.rs:1218）或 `compact`（overlay.rs:931）调 `persist_meta`。

**崩溃窗口**：步骤 2 已写数据（覆盖旧 extent 位置）、步骤 4 truncate 之后、`persist_meta` flush 之前崩溃：
- 旧 meta slot 仍 active（CRC 通过、generation N）——指向旧 extent
- 旧 extent 的字节已被 compact 步骤 2 覆盖写（新紧凑布局与旧布局重叠时）
- recover 读旧 meta slot → 旧 extent → **读到被覆盖的错误数据**

**示例**：旧布局 a.bin@DATA_OFFSET(4B)、b.bin@DATA_OFFSET+4(8B)、c.bin@DATA_OFFSET+12(2B)。compact 重写 a.bin@DATA_OFFSET(4B)、c.bin@DATA_OFFSET+4(2B)。若 compact 写 c.bin@DATA_OFFSET+4 后崩溃（覆盖旧 b.bin 前 2 字节），recover 读旧 meta slot 的 b.bin@DATA_OFFSET+4 → 读到 "CC"+残留 → 数据损坏。

**非 I-6 违规**：manifest 原子性不受影响（manifest 文件也在 compact 重写范围内，但 recover 读旧 meta slot 的 manifest extent 同样被覆盖——不过 manifest 原子性由 save_atomic 的 rename + 双 meta slot 保证，compact 不在 save_atomic 路径内。实际风险是**段文件 / WAL 数据损坏**）。

**缓解现状**：
- OPFS `write()` 不 `flush()` 不保证落盘——进程崩溃时 compact 的 write 可能未持久化，旧数据仍完好。但 OPFS 规范不保证 write 丢失，保守应视为可能落盘。
- 触发条件有限：仅 `delete` 后 free_ratio > 50% 才 compact。
- 测试 `compaction_full_rewrite_data_intact`（overlay.rs:1730）只覆盖 happy path，**未测 compact 中途崩溃**。

**建议**：compact 应写数据到**新区域**（append 到 container 尾部），persist_meta 翻转指向新区域后，再回收旧区域（truncate 或进 free list）。或用第三 meta slot 记录 compact 中间态。当前原地重写不满足崩溃安全。

---

## 4. free list（重点 4）

**结论：PASS**

- `allocate`（overlay.rs:994-1010）：first-fit，找到 >= size 的空洞则复用，剩余碎片回 free_list。否则在 container_size 尾部分配。正确。
- `delete`（overlay.rs:1208-1210）、`rename`（overlay.rs:1192-1195）、`write_at` 重定位（overlay.rs:1103-1105）、`append` 重定位（overlay.rs:1160）均正确释放旧区间进 free_list。
- 测试：`free_list_reuse_on_append`（overlay.rs:1679）+ `free_list_reuse_without_compaction`（overlay.rs:1705）覆盖 first-fit 复用 + container_size 不增长。正确。
- 碎片处理：first-fit 简单但够用，长期碎片由 compaction 回收。

---

## 5. OverlayBackend trait（重点 5）

**结论：PASS**

- `trait OverlayBackend`（overlay.rs:675-686）：5 方法（read/write/flush/size/truncate），后端无关抽象。清晰。
- `MemoryBackend`（overlay.rs:692-768）：`RwLock<Vec<u8>>`，flush no-op，支持 snapshot/restore/truncate_data/corrupt_byte（崩溃模拟）。原生可测。
- `OpfsBackend`（opfs.rs:531-584）：`FileSystemSyncAccessHandle` 薄封装，仅字节 IO，无逻辑。`unsafe impl Send/Sync`（wasm32 单线程，注释说明合理）。
- `OpfsVfs`（opfs.rs:592-638）：持有 `MemOverlay`，impl Vfs 8 方法全委托。薄壳（I-8）。

**I-1（Issue）：OpfsBackend::truncate 用 u32 限制容器 4GB**

opfs.rs:582：`self.sah.truncate_with_u32(sz as u32)`。`sz as u32` 截断高位，容器超 4GB 时截断位置错误。OPFS API `truncate(size)` 接受 number（f64），web-sys 提供 `truncate_with_f64`。应改用 f64。5 万文档验收场景（~250-2500 文件、向量数据 ~500MB）不触发，但扩展性限制。建议改 `truncate_with_f64(sz as f64)`。

---

## 6. 不变量（重点 6）

- **I-5**：core 零改动、零 cfg。OpfsVfs 在 vane-wasm crate。✓
- **I-6**：双 meta_slot + CRC 等价原子切换，对 core 透明。✓（见 §2）
- **I-8**：OpfsVfs/OpfsBackend 是 IO 适配层，无检索逻辑。✓

---

## 7. 体积（重点 7）

**结论：PASS**

- opfs feature 增量 2.2KB gzip（351,032 bytes total，343KB ≤ 800KB）。报告 §6 实测。
- web-sys subset：`FileSystemSyncAccessHandle` + `FileSystemReadWriteOptions` + `FileSystemFileHandle` + `FileSystemDirectoryHandle` + `Storage`（Cargo.toml:28-34）。未启完整 `FileSystemAccess`。feature-gated 正确（`opfs = ["dep:web-sys", "dep:js-sys"]`，default 不启）。
- 远低于预期 50KB。

---

## 8. meta slot 256KB 限制（重点 8）

**结论：合理，非风险**

- META_SLOT_SIZE = 256KB（container.rs:110）。每文件表项约 path_len + 18 字节开销。256KB / ~28B ≈ ~9k 项。
- 5 万文档验收：每段 ~5 文件（vectors/stored/idmap/scalars/header）。50 段 × 5 = 250 文件 + manifest + wal ≈ 260 项。即便 100 docs/段 = 500 段 × 5 = 2500 项。远低于 9k。
- 超限返回 `Err(VaneError::Io("meta slot overflow"))`（container.rs:228-234），可扩大 `META_SLOT_SIZE`。
- 判断：5 万文档边界内无风险。超大规模需动态扩容或分页 meta——非 M2 范围。

---

## 9. TDD 覆盖（重点 9）

**结论：覆盖充分，2 个缺口**

36 新增测试（overlay.rs + container.rs）覆盖：
- Vfs conformance（overlay.rs:1402）+ 8 方法专项（1408-1510）
- 嵌套虚拟路径（1488）、错误码（1501）、list 排序（1876）
- 容器 round-trip（1515）
- 双 meta_slot 翻转 + generation 递增（1557）
- 崩溃恢复 3 时点（1596/1621/1653）
- free list 复用（1679/1705）
- compaction happy path（1730）
- superblock 损坏恢复 + 双槽损坏 Err（1772/1792）
- 新库初始化（1814）
- 大文件 append（1828）、zero-fill grow（1843）

**缺口 1（M-1 相关）**：compaction 崩溃恢复未测——compact_internal 中途崩溃（写数据后、persist_meta 前）后 recover 数据完整性。见 §3 M-1。

**缺口 2**：OpfsBackend 仅 wasm32 编译通过，无 wasm-bindgen-test in Worker 运行时验证。报告 §8 遗留承认（待 M2-04）。SyncAccessHandle read/write/flush/truncate/getSize 运行时行为未测。可接受（薄层 + M2-04 接入浏览器验证）。

**并发**：overlay 单线程 Worker 内同步（RwLock 实际无竞争），并发不适用。无需测。

---

## 10. Arc 循环 / 所有权（重点 10 / minor 1 复核）

**结论：无循环。PASS**

- `MemOverlay` 持有 `Arc<dyn OverlayBackend>`（overlay.rs:789）。
- `OpfsBackend` 持有 `FileSystemSyncAccessHandle`（opfs.rs:532），**不反向持有 MemOverlay**。
- `OpfsVfs` 持有 `MemOverlay`（owns，opfs.rs:593）。
- 所有权链：`OpfsVfs` → `MemOverlay` → `Arc<OpfsBackend>` → `FileSystemSyncAccessHandle`。单向，无环。✓

---

## 11. I-2（Issue）：conformance 测试是手工复制，非引用 core

overlay.rs:1344 `fn run_conformance<V: Vfs>` 是 core `run_conformance_tests`（`vfs/tests.rs:5`）的手工复制。core 的 `run_conformance_tests` 在 `#[cfg(test)] mod tests` 内（mod.rs:22），虽 `pub` 但仅 cfg(test) 可见，vane-wasm 无法 import。复制是当前结构下的必然选择，但存在 drift 风险（core conformance 演进时 overlay 副本不跟踪）。建议未来将 conformance 提为 vane-core 的 `pub` 非 cfg(test) 函数或独立 crate 供跨 crate 复用。非阻塞。

---

## 12. I-3（Issue）：崩溃恢复时点 B 测试模拟偏差

计划 §4 测试清单 T14 描述「元数据写一半崩溃（模拟非活跃槽 CRC 损坏）」。实际测试 `crash_recovery_meta_write_partial`（overlay.rs:1621-1650）的模拟方式是：rename **完成后**（已翻转 active 到新槽）再 `corrupt_meta_slot(active)` 损坏**新 active 槽**，recover 回退到旧 inactive 槽。

这与「写一半」语义不同：真正的「写一半」是 inactive 槽部分写入（CRC 失败）、active 旧槽完好。测试模拟的是「post-flip corruption」。两者 exercise 相同的 CRC 回退代码路径（recover 校验双槽 CRC + max generation），逻辑正确性等价，但模拟时点与计划描述有偏差。建议补充一个测试：rename 的 persist_meta 写非活跃槽到一半（截断 backend 模拟）后 recover，更贴合「写一半」语义。非阻塞。

---

## 13. Minor 汇总

- **Minor 1**：Arc 循环复核——无循环（见 §10）。
- **Minor 2**：write_at/append/list 与 StdFsVso 微差（见 §1），不影响 core 调用面。
- **Minor 3**：meta slot 256KB 合理（见 §8）。
- **Minor 4**：`compact` public 方法（overlay.rs:928）与 `delete` 内 compact 路径重复 `compact_internal + persist_meta`，无死循环（compact 不触发 delete/rename）。OK。

---

## 14. 发现汇总

| 级别 | # | 标题 | 证据 |
|---|---|---|---|
| M | 1 | compaction 崩溃恢复间隙——破坏性原地重写，persist_meta 前崩溃可损坏数据 | overlay.rs:937-984（compact_internal 步骤 2 原地覆盖）+ 1218（delete 调用链） |
| I | 1 | OpfsBackend::truncate 用 u32 限制 4GB | opfs.rs:582 |
| I | 2 | conformance 测试手工复制，drift 风险 | overlay.rs:1344 vs vfs/tests.rs:5 |
| I | 3 | 崩溃恢复时点 B 测试模拟偏差 | overlay.rs:1621-1650 vs 计划 T14 |
| Minor | 1-4 | Arc 无循环 / 语义微差 / meta slot 合理 / compact 无死循环 | 见各节 |

**无 B（阻塞）级发现。崩溃恢复 I-6 等价逻辑正确。**
