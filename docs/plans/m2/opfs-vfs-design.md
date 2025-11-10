# M2-02 OPFS VFS 架构设计（阻塞解除）

> 角色：M2 Phase One 架构设计 SubAgent
> 范围：只读研究 + 设计文档。不改代码 / Cargo / SPEC / 计划文件。
> 状态：草案，供编排者评审。

## 0. 结论速览

- **推荐路径：A（单 OPFS 容器文件 + 内存虚拟 FS overlay）**。
- **是否需 SPEC §6.1 修订：否**。Vfs trait 8 方法签名零改动，core 零改动，§6.1/§6.2/§6.4 语义全部保持。
- IDB 降级（M2-03）复用同一 overlay 内核，模型一致。
- M2-02 工作量上修（新增容器格式 + 双元数据区 + free list），M2-03 工作量下修（共享 overlay 抽象），M2-04 Worker init 改为异步序列。

---

## 1. 阻塞复述

`M2-02-opfs-vfs.md` 计划用 `FileSystemSyncAccessHandle` 同步实现 Vfs trait 全部 8 方法。reviewer B 指出：SyncAccessHandle 仅覆盖同步的字节 IO（read/write/flush/getSize/truncate/close），而 `create`（建文件/目录）、`rename`（manifest 原子切换）、`delete`（段清理）、`list`（recover 扫描）依赖 `FileSystemDirectoryHandle` 的 `getDirectoryHandle` / `getFileHandle` / `removeEntry` / `values`——这些**全部返回 Promise**，在 Worker wasm 同步调用栈内无法阻塞等待。

core 现有调用面（已读源码确认）：
- `persistence::ManifestStore::save_atomic`（`crates/vane-core/src/persistence/mod.rs:100-113`）：`delete(tmp)` → `create(tmp)` → `write_at(tmp)` → `sync(tmp)` → `rename(tmp, manifest.json)`。**rename 是 I-6 原子切换唯一原语**。
- `wal::Wal::open/append/truncate`（`crates/vane-core/src/wal/mod.rs:57-115`）：`create` / `append` / `sync` / `delete`。
- `wal::recover` + `cleanup_orphan_segment_dirs`（同文件 `:144-225`）：`list("segments")` 扫描孤儿段目录，递归 `delete`。
- `merge::delete_segment_dir`（`crates/vane-core/src/merge/mod.rs:312-333`）：`list` 递归 + `delete` 单文件。
- `segment::SegmentBuilder::finalize`（`crates/vane-core/src/segment/mod.rs:200-300`）：每段文件 `create` → `write_at` → `sync`（vectors.bin / stored.bin / idmap.bin / scalars.col / header.bin）。
- `SegmentReader::open`：`read_at` 多文件。

即 create / rename / delete / list 四个"目录/元操作"方法在 core 内被大量同步使用，且无法绕过（manifest 原子性、recover 孤儿段扫描、段目录递归删除均硬依赖）。

## 2. OPFS API 能力边界（核查 MDN + whatwg/fs 提案）

| API | 同步? | Worker 可用? | 说明 |
|---|---|---|---|
| `navigator.storage.getDirectory()` | **异步 Promise** | ✓ | 返回 OPFS root `FileSystemDirectoryHandle`。仅 init 用。 |
| `FileSystemDirectoryHandle.getFileHandle(name,{create})` | **异步 Promise** | ✓ | 返回 `FileSystemFileHandle`。建文件也走它。 |
| `FileSystemDirectoryHandle.getDirectoryHandle(name,{create})` | **异步 Promise** | ✓ | 建子目录。 |
| `FileSystemDirectoryHandle.removeEntry(name,{recursive})` | **异步 Promise** | ✓ | 删文件/目录。 |
| `FileSystemDirectoryHandle.values()/entries()/keys()` | **async iterator** | ✓ | 列目录，`for await...of`。 |
| `FileSystemFileHandle.createSyncAccessHandle()` | **异步 Promise** | ✓ 仅 Dedicated Worker | resolve 出 `FileSystemSyncAccessHandle`（独占写锁）。 |
| `FileSystemFileHandle.move(name)` | **异步 Promise** | ✓ | OPFS 内 rename/move，但仍是 async。 |
| `FileSystemSyncAccessHandle.read/write/flush/getSize/truncate/close` | **同步** | ✓ 仅 Worker | 唯一同步面。 |

**关键事实**：连"获取 SyncAccessHandle"本身都是异步的（`createSyncAccessHandle()` 返回 Promise）。因此任何"按 Vfs path → OPFS 文件"的多文件模型，在运行时新建段文件时都必须 await `getFileHandle` + `createSyncAccessHandle`，这在同步 Vfs 方法内无法完成。

**wasm 同步化 Promise 的可行性核查**：
- `wasm-bindgen-futures::spawn_local`：非阻塞，把 future 调度到事件循环，不阻塞当前调用栈 → 无法用于同步返回。
- `Atomics.wait + SharedArrayBuffer`：需另一个线程推进 Promise。wasm32-unknown-unknown 单线程；wasm threads 需 `--target-feature=+atomics,+bulk-memory` + SharedArrayBuffer，本质是 wasm threads 路线，与"不引 wasi"、单 Worker 简化架构相悖，且 OPFS Worker 通常单线程。**不可行**。
- 结论：在 wasm32-unknown-unknown 单线程 Worker 内，**无法把异步 Promise 桥成同步阻塞调用**。REQUIREMENTS §4.1 "core 保持同步 IO" 是硬约束，反过来要求 Vfs 后端的同步方法体内不得出现任何 await。

## 3. 路径评估

### 路径 A：单 OPFS 容器文件 + 内存虚拟 FS overlay（★ 推荐）

**模型**：整个 Db 物理上是一个 OPFS 文件 `vane.db`。Worker init 时异步获取该文件的 `FileSystemSyncAccessHandle`（一次性 await），之后所有 Vfs 方法用这同一个同步句柄操作容器内的字节区间。core 看到的 `<db>/segments/seg_<ulid>/vectors.bin` 等 path 是**虚拟路径**，由 OpfsVfs 内存的文件表映射到容器内的 `(offset, size)` 区间。

- `create(path)`：内存文件表登记 path（区间在首次 write 时分配，或立即分配零字节）。同步。
- `read_at(path, buf, off)`：查表得 `(base, size)`，`SyncAccessHandle.read(buf, base+off)`。同步。
- `write_at(path, buf, off)`：若 path 新建则 append 分配区间并更新表；`SyncAccessHandle.write(buf, base+off)`；必要时扩展容器（truncate 或 append 写）。同步。
- `append(path, buf)`：查表得 `size`，在 `base+size` 处 write 并扩展，返回旧 size。同步。
- `sync(path)`：`SyncAccessHandle.flush()`（单文件，path 粒度无意义，flush 全容器；可去重连续 flush）。同步。
- `rename(from, to)`：内存表把 from 的区间改挂到 to（先释放 to 旧区间），随后把新文件表序列化到非活跃元数据槽并 flush。同步。
- `delete(path)`：内存表移除 path，区间进 free list，序列化元数据并 flush。同步。
- `list(dir)`：遍历内存表 keys，按 dir 前缀过滤返回下一层分量（与 MemoryVfs::list 语义一致）。同步。

**Vfs trait 改动**：无。8 方法签名不变。
**core 改动**：零。core 仍按 §6.2 目录布局写虚拟 path，OpfsVfs 内部映射。
**manifest 原子性（I-6）**：见 §4.3，靠双元数据槽 + CRC 保证，不依赖 OPFS 目录 rename。
**与 §6.2 布局的张力**：逻辑布局不变（core 看到的就是 `<db>/segments/seg_<ulid>/...`），物理上是单文件内区间。无张力。
**体积**：web-sys subset 仅需 `Storage` + `FileSystemDirectoryHandle`（init）+ `FileSystemFileHandle` + `FileSystemSyncAccessHandle`，远小于完整 `FileSystemAccess`。预计增量 < 50KB gzip，800KB 门禁安全。

### 路径 B：目录模型 + JS 侧目录操作桥

保留 OPFS 多文件/多目录布局，binding 层在 Worker 内用异步 `FileSystemDirectoryHandle` 做目录操作，core 调 Vfs 时同步化。

**致命问题**：如 §2 核查，wasm32-unknown-unknown 单线程内无法把异步 Promise 桥成同步阻塞。`Atomics.wait` 需多线程，与单 Worker 架构 + "不引 wasi" 冲突。运行时新建段文件（`getFileHandle(create:true)` + `createSyncAccessHandle()`，两步异步）在同步 `Vfs::create` 内无法完成。

**结论**：不可行（除非引入 wasm threads + SharedArrayBuffer，违背架构约束）。

### 路径 C：Vfs trait 拆分 / 异步化

把 Vfs 拆为"同步字节 IO（SyncAccessHandle 覆盖）"+"async 目录元操作"，core 改用 async IO。

**致命问题**：
- 触及 §6.1 M0 冻结签名 → 需 SPEC 修订（⚠️）。
- 违背 REQUIREMENTS §4.1 "core 保持同步 IO……不为浏览器把 core 异步化"——这是 v1.1/v1.2 已冻结的硬约束。
- async 传染整个 core：`SegmentBuilder::finalize`、`ManifestStore::save_atomic`、`Wal::append`、`recover`、`delete_segment_dir`、合并器全部变 async，`Db::open/add/flush` 全链路 async，api-core IDL 语义剧变。代价不可接受。

**结论**：否决。即使作为候选，其代价（重写 core 控制流 + 违反显式约束）远超路径 A。

### 路径 D：其他

- **D1：IDB 作主存储**：IDB 无任何同步 API（连字节 IO 都没有），比 OPFS 更差。IDB 只能纯内存 overlay + 异步 checkpoint，sync 不可真正落盘 → 不满足 §4.1 "OPFS 主"定位。仅适合作降级（M2-03）。
- **D2：OPFS + WASI threads blocking**：引入 wasi/threads，违背"不引 wasi"。否决。

## 4. 推荐方案（路径 A）详细设计

### 4.1 模块边界

全部在 `vane-wasm` crate（非 core），满足 I-5（`cfg(target_arch="wasm32")` 仅 VFS impl）/ I-8（binding 薄壳，无检索逻辑）。

```
crates/vane-wasm/src/vfs/
├── mod.rs              // re-export
├── opfs.rs             // OpfsVfs: impl Vfs（8 方法，全同步）
├── overlay.rs          // MemOverlay 内核：虚拟文件表 + 区间分配 + free list + 双元数据
└── container.rs        // 容器格式读写（superblock / meta slot / data area）
```

`overlay.rs` 与后端无关，IDB 降级（M2-03）直接复用：定义 `trait OverlayBackend { fn read(&self, off: u64, buf: &mut [u8]) -> Result<usize>; fn write(&self, off: u64, buf: &[u8]) -> Result<()>; fn flush(&self) -> Result<()>; fn size(&self) -> Result<u64>; fn truncate(&self, sz: u64) -> Result<()>; }`，OpfsVfs（SyncAccessHandle）和 IdbVfs（内存 Vec + 异步 dump）各 impl 一次。

### 4.2 容器格式

```
┌──────────────────────────────────────────────────────────┐
│ superblock (4 KB)                                        │
│  - magic (4) | format_version (4 LE)                     │
│  - active_meta_slot: u8 (0|1)                            │
│  - meta_offset[2]: u64 LE                                │
│  - meta_size[2]:   u64 LE                                │
│  - container_size: u64 LE                                │
├──────────────────────────────────────────────────────────┤
│ meta_slot_0                                              │
│  - generation: u64 LE | crc32: u32 LE                    │
│  - file_table: { path_len:u16 LE | path:utf8 |           │
│                  base:u64 LE | size:u64 LE }[]           │
│  - free_list:  { base:u64 LE | size:u64 LE }[]           │
├──────────────────────────────────────────────────────────┤
│ meta_slot_1  (双槽，与 slot_0 等大预留)                    │
├──────────────────────────────────────────────────────────┤
│ data area                                                │
│  ┌─ manifest.json (区间) ─────────────────────┐          │
│  ├─ wal.log (区间) ───────────────────────────┤          │
│  └─ segments/seg_<ulid>/{header,vectors,...}  │          │
│     (各文件一区间，按分配序)                     │          │
└──────────────────────────────────────────────────────────┘
```

- 两个 meta_slot 等大预留（如各 256KB，足够 ~10k 文件表项）。写入总是写**非活跃槽**，写完 + CRC 后翻转 `active_meta_slot`。
- `container_size` 记录数据区已分配末尾，用于 `append` 分配新区间。
- 新库 init：superblock 写 `active_meta_slot=0`、`meta_slot_0` 为空文件表，flush。

### 4.3 manifest 原子性（I-6）保证

core `ManifestStore::save_atomic` 调用序列：`delete(tmp)` → `create(tmp)` → `write_at(tmp, json, 0)` → `sync(tmp)` → `rename(tmp, manifest.json)`。

在 OpfsVfs 中：
1. `write_at(tmp, ..)`：把 manifest 字节写到 tmp 对应的容器区间（若 tmp 不存在则 append 分配）。此时元数据表里 `tmp` 指向新 manifest 字节，`manifest.json` 仍指向旧 manifest 字节。
2. `sync(tmp)`：`SyncAccessHandle.flush()` —— tmp 字节落盘，但**元数据表仍指向 tmp**（元数据未持久化翻转）。
3. `rename(tmp, manifest.json)`：
   - 内存表：释放旧 `manifest.json` 区间（进 free list），把 `tmp` 的区间改挂为 `manifest.json`。
   - 把新内存表序列化到**非活跃 meta_slot**，计算 CRC。
   - `SyncAccessHandle.write(meta_slot)` + 翻转 superblock `active_meta_slot` + `SyncAccessHandle.write(superblock)` + `flush()`。

崩溃分析：
- **步骤 2 后、步骤 3 元数据落盘前崩溃**：容器里 active meta_slot 仍是旧的（`tmp` 叫 tmp，`manifest.json` 指向旧 manifest 字节，旧 manifest 完整）→ recover 读旧 manifest 完好。✓ I-6
- **步骤 3 元数据写一半崩溃**：非活跃槽 CRC 校验失败 → recover 回退到 active 旧槽 → 旧 manifest 完好。✓ I-6
- **步骤 3 flush 后崩溃**：新 meta_slot active，`manifest.json` 指向新 manifest 字节，新 manifest 完整。✓ I-6

满足 I-6"任何崩溃后 manifest 指向完整状态"。rename 仍是原子切换唯一原语（OpfsVfs 内部用双 meta_slot + CRC 实现"原子"语义，对 core 透明）。

> 注：superblock 翻转本身不是单字节原子，但 superblock 很小且 `active_meta_slot` 是单字节；最坏情况下 recover 时同时校验两个 meta_slot 的 CRC，取 generation 最大且 CRC 通过者为 active。这等价于双版本 + CRC 的原子切换，不依赖单字节原子性。

### 4.4 与 core 现有 Vfs 用法的兼容性（逐项核验）

| core 调用点 | 用到的方法 | OpfsVfs 行为 | 兼容 |
|---|---|---|---|
| `ManifestStore::save_atomic` | create/write_at/sync/rename/delete | 见 §4.3 | ✓ |
| `Wal::open` | create（幂等，忽略已存在） | 内存表已存在则 best-effort 忽略（与 StdFsVfs 行为对齐：Wal::open 已 `let _ = create`） | ✓ |
| `Wal::append/read_all` | append/sync/read_at | 区间 append 语义 | ✓ |
| `Wal::truncate` | delete+create+sync | 释放旧区间 + 新建空区间 | ✓ |
| `recover::cleanup_orphan_segment_dirs` | `list("segments")` | 内存表前缀过滤返回 `seg_<ulid>` 列表 | ✓ |
| `merge::delete_segment_dir` | list 递归 + delete | 内存表递归 + 区间释放 | ✓ |
| `SegmentBuilder::finalize` | create+write_at+sync（每段文件） | 每文件一区间，sync 统一 flush 容器 | ✓ |
| `SegmentReader::open` | read_at | 区间 read | ✓ |

`MemoryVfs::list` 的"返回下一层分量"语义（`crates/vane-core/src/vfs/memory.rs:99-122`）必须在 OpfsVfs::list 复刻，以保证 `cleanup_orphan_segment_dirs` 和 `delete_segment_dir` 的递归逻辑正确。契约测试（M0 Vfs 通用套件）已覆盖此语义。

### 4.5 sync 粒度与性能

core 在 `SegmentBuilder::finalize` 中对每段文件各调一次 `sync`（5 次 flush）。单容器下 `sync(path)` 统一 `SyncAccessHandle.flush()` 全容器。优化：OpfsVfs 内部维护 `dirty: bool`，连续 sync 调用合并为一次 flush（最后一次真正 flush）。但 Vfs::sync 语义是"保证落盘"，保守起见初版每次都 flush，性能优化（dirty 合并）留作 M2 后期。OPFS 单文件 flush 成本低于多文件 fsync，预计可接受。

### 4.6 容器 compaction

段删除（合并/compact 后旧段清除）留空洞。free list 管理空闲区间，后续 append 优先复用合适空洞。长期碎片化时，OpfsVfs 内部触发 compaction：分配新容器区域 → 拷贝活跃区间 → 翻转。初版可用 append-only + 阈值触发全量 rewrite（简单），碎片管理优化延后。core 不感知。

### 4.7 Worker init 异步序列（与 M2-04 衔接）

```
Worker init（JS 异步上下文）:
  1. root = await navigator.storage.getDirectory()
  2. fh = await root.getFileHandle("vane.db", {create:true})
  3. sah = await fh.createSyncAccessHandle()        // 唯一同步句柄
  4. sah.read(superblock) → 重建内存文件表 / 新库初始化空容器
  5. OpfsVfs::from_handle(sah, file_table)          // 进入同步 Vfs 世界
  6. Db::open(Arc<OpfsVfs>, db_path)                // core 同步打开
```

步骤 1-4 在 JS/WASM binding 异步边界完成；步骤 5-6 进入 core 同步世界。异步性只存在于"主页面 ↔ Worker"边界（REQUIREMENTS §4.1 明确允许），core 内部全同步。

## 5. IDB 降级（M2-03）一致性

IDB 无任何同步 API。降级模型复用 §4.1 的 `overlay.rs` 内核：

- `IdbVfs` 持有内存 `Vec<u8>`（容器映像）+ `MemOverlay`（文件表/区间/free list，与 OpfsVfs 同一份代码）。
- Worker init 异步从 IDB 读取容器 blob 到内存 `Vec`。
- 运行时 Vfs 方法操作内存 `Vec`（read/write/append/rename/delete/list 全内存，同步）。
- `sync(path)`：best-effort —— 标记 `dirty: bool`，由 JS 壳层异步 tick（postMessage 触发）把内存 blob `put` 回 IDB。**不保证 sync 返回时已落盘**。
- 这符合 REQUIREMENTS §4.1 "IDB 降级"+ §4.1 "浏览器存储非可靠存储，关键数据用 `export()` 快照导出" + 中文降级"不抛错"的语义层级。

复用点：`MemOverlay`（文件表 + 区间分配 + 双元数据 + CRC）是后端无关的纯 Rust，OpfsVfs 与 IdbVfs 共享。差异仅在底层 `OverlayBackend`：OPFS = SyncAccessHandle（sync flush 真落盘），IDB = 内存 Vec（sync 标 dirty，异步 dump）。

**结论**：M2-03 模型与 M2-02 一致（同 overlay），工作量因共享抽象而下降。

## 6. 体积 / 性能 / 风险

### 体积
- web-sys subset：`Storage` + `FileSystemDirectoryHandle`（init）+ `FileSystemFileHandle` + `FileSystemSyncAccessHandle`。不启用完整 `FileSystemAccess`。
- 容器/overlay 逻辑纯 Rust，无新依赖（CRC32 用现成 `crc32fast` 或手写 8 行，倾向手写避免依赖）。
- 预计增量 < 50KB gzip。M2-01 体积预算（800KB 门禁）安全。

### 性能
- read_at：单 SyncAccessHandle.read，offset = 容器绝对偏移，与多文件等价。
- write_at/append：区间 write，append 时偶发扩展容器（truncate 或尾部 write）。
- sync：单文件 flush，优于多文件 fsync。
- list：内存表遍历，快于目录 IO。
- 大库：单 OPFS 文件无大小限制（OPFS 配额内，通常数 GB）。

### 风险
1. **容器格式复杂度**：双 meta_slot + CRC + free list 是迷你 FS。缓解：初版 append-only + 全量 rewrite compaction，free list 先做最简 first-fit，后续优化。
2. **superblock 翻转非单字节原子**：缓解：recover 同时校验两槽 CRC，取 generation 最大且 CRC 通过者。已在 §4.3 注明。
3. **sync 统一 flush 的性能**：段 finalize 5 次 flush。缓解：dirty 合并（M2 后期）。
4. **容器损坏恢复**：双 meta_slot + CRC 回退；数据区文件本身有 magic + format_version（§6.2），单文件损坏可定位。
5. **独占锁**：单 SyncAccessHandle 对单文件独占，Worker 单线程无并发问题。
6. **OPFS 兼容性**：Safari 历史上有 OPFS 写入 bug（搜索结果提及）。缓解：M2-04 Worker shell 做能力探测，OPFS 不可用则降级 IDB（M2-03）。

## 7. 对计划的影响

### M2-02（OPFS VFS 后端）
- 从"多文件 SyncAccessHandle per path"改为"单容器 + 内存 overlay"。
- 新增工作：容器格式（container.rs）、overlay 内核（overlay.rs）、双 meta_slot + CRC、free list、init 异步序列。
- 测试：M0 Vfs 通用契约套件在 OpfsVfs 上跑（wasm-bindgen-test in Worker）；新增容器格式 round-trip 测试、崩溃恢复测试（模拟步骤 3 各时点崩溃）。
- 体积验收：web-sys subset 实测登记。

### M2-03（IDB 降级）
- 复用 `overlay.rs` 内核，工作量下降。
- 新增：`OverlayBackend` trait + IdbVfs impl + 异步 checkpoint tick（JS 壳层）。
- sync 语义：best-effort（与 OPFS 真落盘区别），文档声明。

### M2-04（Worker shell）
- Worker init 改为 §4.7 异步序列：getDirectory → getFileHandle → createSyncAccessHandle → 重建文件表 → Db::open。
- OPFS 能力探测：失败则降级 IdbVfs。
- 异步性严格限于 init + "主页面 ↔ Worker" postMessage 边界，core 内部全同步。

### M2-12（export 快照）
- export 可直接读容器区间拼接，或 dump 整个容器 blob。简化。

## 8. SPEC 修订判定

**不需要修订 §6.1。**

- Vfs trait 8 方法签名：零改动。
- §6.1 四后端：仍四后端（std-fs / opfs / idb / memory），OpfsVfs 仍是 opfs 后端，只是物理实现是单容器。
- §6.2 目录布局：描述的是**逻辑布局**（core 看到的 path），OpfsVfs 内部映射到容器区间，布局语义不变。可加一行非规范性注记"OPFS 后端物理上为单容器文件，逻辑布局由 Vfs impl 内部映射"，但非必须。
- §6.4 manifest 原子 rename：rename 仍是原子切换唯一原语（OpfsVfs 用双 meta_slot + CRC 实现等价原子性），语义不变。
- §4.1 core 同步 IO：OpfsVfs 全同步，不破坏。
- I-5 / I-8：OpfsVfs 在 vane-wasm crate，core 零 cfg、零改动。

若编排者希望在 SPEC 显式记录"OPFS 单容器实现"以避免后续误解，可在 §6.1 后追加一条非规范性注记（非修订，不触及冻结签名）。建议注记草案：

> *实现注记（非规范性）*：OPFS 后端（`opfs`）在 Worker 内以单个 OPFS 容器文件 + 内存虚拟文件表实现 Vfs trait。Worker init 异步获取 `FileSystemSyncAccessHandle`（唯一 await 点），此后全部 Vfs 方法基于该同步句柄操作容器内字节区间，虚拟路径映射由 Vfs impl 内部维护。manifest 原子切换（I-6）由容器内双元数据槽 + CRC 等价实现，对 core 透明。此注记不改变 §6.1 签名或 §6.2 逻辑布局。

## 9. 参考资料

- [FileSystemSyncAccessHandle — MDN](https://developer.mozilla.org/en-US/docs/Web/API/FileSystemSyncAccessHandle)
- [FileSystemDirectoryHandle — MDN](https://developer.mozilla.org/en-US/docs/Web/API/FileSystemDirectoryHandle)
- [WICG AccessHandle 提案](https://raw.githubusercontent.com/WICG/file-system-access/8427e246da4ca4683c32884fb5c982d8c30011ce/AccessHandle.md)
- [whatwg/fs move 提案](https://raw.githubusercontent.com/whatwg/fs/main/proposals/MovingNonOpfsFiles.md)
