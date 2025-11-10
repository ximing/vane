# Vane M2 Phase One 聚焦复核 Round 2（fix 轮后 scoped re-review）

> 角色：聚焦复核 reviewer（只读，不跑 cargo）
> 范围：M2-02/03/04 overlay 链 + 签名修正点（M2-08/09/11/12）+ README 约束
> 日期：2026-08-09
> 结论：**PASS_WITH_FINDINGS**（0 阻塞 / 0 重要 / 5 次要）

---

## 0. 复核方法

逐项核查 fix 轮改动对齐实际源码（`crates/vane-core/`）+ 三计划 overlay 内核契约一致性 + 崩溃恢复覆盖 + core 零改动声明。每条发现附 file:line 证据。

---

## 1. overlay 链契约一致性（M2-02/03/04）

### 1.1 MemOverlay / OverlayBackend 签名一致性 ✓

三计划对 overlay 内核的契约描述一致：

| 契约项 | M2-02（定义） | M2-03（复用） | M2-04（注入） | 一致 |
|---|---|---|---|---|
| `OverlayBackend` 5 方法 | `read/write/flush/size/truncate`（M2-02 §3 L44-50） | 同（M2-03 §3 L33-36） | 不涉及 | ✓ |
| `MemOverlay` 8 方法 | `open/create/read_at/write_at/append/sync/rename/delete/list`（M2-02 §3 L52-62） | 「8 方法，委托 MemOverlay」（M2-03 §3 L41） | 不涉及 | ✓ |
| `OpfsVfs::from_handle(sah)` | M2-02 §3 L69 | — | M2-04 §3 L25 `OpfsVfs::from_handle(sah)` | ✓ |
| `IdbVfs::from_blob(blob)` | — | M2-03 §3 L39 | M2-04 §3 L30 `IdbVfs::from_blob(blob)` | ✓ |

### 1.2 文件表/区间/free list 语义一致性 ✓

- 文件表 `HashMap<String, (base:u64, size:u64)>`：M2-02 §2 L14 定义，M2-03 §3 L49「文件表/区间分配/双元数据/CRC 与 OpfsVfs 共享同一份代码」明示复用。✓
- free list `Vec<(base,size)>`：M2-02 §2 L14 定义，M2-03 复用（无重新定义）。✓
- `list(dir)` 语义：M2-02 §3 L61「表 keys 前缀过滤返回下一层分量（与 MemoryVfs::list 语义一致 `vfs/memory.rs:99`）」。核查 `crates/vane-core/src/vfs/memory.rs:99-121`：确实为前缀过滤 + `split('/').next()` 返回下一层分量 + sort + dedup。overlay 复刻此语义。✓
- `list` 语义与 core 递归调用兼容：`merge::delete_segment_dir`（`merge/mod.rs:312-333`）递归 `list` + `delete`；`wal::cleanup_orphan_segment_dirs`（`wal/mod.rs:201-221`）`list("segments")` → 逐段 `delete_segment_dir`。overlay 的 flat HashMap + 前缀过滤模拟目录层级，与 MemoryVfs 同构，递归逻辑正确。✓

### 1.3 双 meta_slot + CRC 覆盖 I-6 崩溃场景 ✓

M2-02 §3「manifest 原子性」描述 3 时点 + superblock 翻转非原子处理：

| 崩溃时点 | 恢复行为 | I-6 覆盖 | 核查 |
|---|---|---|---|
| 步骤 2 后（字节落盘，元数据未落盘） | active meta_slot 仍是旧的 → 旧 manifest 完整 | ✓ | M2-02 §3 L105 |
| 步骤 3 元数据写一半 | 非活跃槽 CRC 失败 → 回退 active 旧槽 | ✓ | M2-02 §3 L106 |
| 步骤 3 flush 后 | 新 meta_slot active + generation 最大 → 新 manifest 完整 | ✓ | M2-02 §3 L107 |
| superblock 翻转非单字节原子 | recover 同时校验两槽 CRC，取 generation 最大且 CRC 通过者 | ✓ | M2-02 §3 L108 |

`ManifestStore::save_atomic` 调用序列 `delete(tmp)→create(tmp)→write_at(tmp)→sync(tmp)→rename(tmp,manifest.json)` 见设计文档 `opfs-vfs-design.md:21`（核查 `persistence/mod.rs:100-113` 一致）。overlay `rename` 实现双 meta_slot 翻转等价原子切换，对 core 透明。✓

### 1.4 core / Vfs trait 零改动声明 ✓

- Vfs trait 8 方法签名：`crates/vane-core/src/vfs/mod.rs:5-13` 确认 `create/read_at/write_at/append/sync/rename/delete/list`。
- `OpfsVfs` impl Vfs 8 方法全委托 MemOverlay（M2-02 §3 L71）。✓
- `IdbVfs` impl Vfs 8 方法全委托 MemOverlay（M2-03 §3 L41）。✓
- 声明「不修改 `crates/vane-core/`」（M2-02 §2 L20）。✓

### 1.5 append-only + rewrite compaction 可行性 ✓（无死循环）

- append-only：新区间在 `container_size` 末尾分配（M2-02 §3 L95）。✓
- free list first-fit 复用（M2-02 §3 L116, 测试 16）。✓
- compaction：阈值触发全量 rewrite，一次性操作，非循环（M2-02 §3 L116）。无死循环风险。✓

### 1.6 M2-03 best-effort sync + I-6 降级明示 ✓

- M2-03 §3 L47：`sync(path)` best-effort —— 标 dirty，不保证 sync 返回时已落盘。✓
- M2-03 §7 L84：I-6 语义降级（明示）—— 崩溃可能丢未 checkpoint 写入，关键数据走 `export()`。✓
- README §阶段性偏离 #1（L430）：同上文档化。✓

### 1.7 M2-04 init 序列完整性 ✓

M2-04 §3 L20-31 异步序列：
```
1. root = await navigator.storage.getDirectory()
2. fh = await root.getFileHandle("vane.db", {create:true})
3. sah = await fh.createSyncAccessHandle()
4. OpfsVfs::from_handle(sah)          // 内含 MemOverlay::open 重建文件表
5. Db::open(Arc<OpfsVfs>, db_path)
```
与设计文档 `opfs-vfs-design.md:188-196` §4.7 一致（设计文档 6 步，M2-04 将「读 superblock 重建文件表」折叠进 `OpfsVfs::from_handle`，由 `MemOverlay::open(backend)` 内部完成——M2-02 §3 L53 确认 `open` 读 superblock + 活跃 meta_slot 重建文件表）。✓

Safari 探测降级：M2-04 测试 4（L67）`opfs_available()==false` + 能力探测三件套（`getDirectory` 存在性 + `createSyncAccessHandle` 可用性 + 小写 round-trip 探针）。✓

---

## 2. 签名修正点核查

### 2.1 Db::open 首参 `vfs: Arc<dyn Vfs>` ✓

- 实际签名：`crates/vane-core/src/api/db.rs:35` → `pub fn open(vfs: Arc<dyn Vfs>, path: &str, opts: OpenOptions) -> Result<Self>`
- M2-11 §3 L27：`Db::open(vfs: Arc<dyn Vfs>, path: &str, opts: OpenOptions)`（`api/db.rs:35`，首参 `vfs: Arc<dyn Vfs>`）—— **对齐** ✓
- M2-04 §3 L25：`Db::open(Arc<OpfsVfs>, db_path)` —— 序列图简写（省略 opts），首参 `Arc<OpfsVfs>` 经 unsized coercion 到 `Arc<dyn Vfs>`，正确。✓
- README §M2-11 L325：`Db::open(vfs: Arc<dyn Vfs>, path, opts)` `api/db.rs:35` —— **对齐** ✓

### 2.2 M2-08 scalars/header/hnsw 行号 ✓（1 处行号偏差，次要）

| 计划引用 | 实际代码 | 核查 |
|---|---|---|
| `segment/mod.rs:652`（`fn decode_scalars`） | `segment/mod.rs:652` → `fn decode_scalars(buf: &[u8]) -> Result<ScalarReader>` | ✓ 精确 |
| `header.rs:21`（encode `to_le_bytes`） | `header.rs:21` → `out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());` | ✓ 精确 |
| `header.rs:46`（decode `version != FORMAT_VERSION`） | `header.rs:46` → `if version != FORMAT_VERSION {` | ✓ 精确 |
| `hnsw/mod.rs:534`（decode 校验） | `hnsw/mod.rs:534` → `if version != FORMAT_VERSION {` | ✓ 精确 |
| `hnsw/mod.rs:533`（version 读取） | `hnsw/mod.rs:533` → `let version = u32::from_le_bytes(buf[4..8].try_into().unwrap());` | ✓ 精确 |
| 计划描述「行 40 是 `buf.len() < 8` 长度检查」 | 实际 `if buf.len() < 8 {` 在 `header.rs:39`（行 40 是 `return Err(...)`） | ⚠️ 行号偏 1（次要 S-5） |

### 2.3 M2-09 `brute_search_sq8` 签名对齐 ✓

- 实际 `brute_search` 签名（`vector/mod.rs:101-109`）：
  ```rust
  pub fn brute_search(
      vectors: &[f32], dim: u32, query: &[f32],
      metric: Metric, topk: usize,
      filter: Option<&roaring::RoaringBitmap>,
      docid_base: u64,
  ) -> Vec<ScoredDoc>
  ```
  已含 `metric: Metric`（L105）和 `docid_base: u64`（L108）。
- M2-09 §3 L44 `brute_search_sq8` 签名：`(sq8: &[u8], dim: u32, query: &[f32], metric: Metric, topk: usize, filter: Option<&roaring::RoaringBitmap>, docid_base: u64) -> Vec<ScoredDoc>` —— 除首参 `sq8: &[u8]` 替代 `vectors: &[f32]` 外完全对齐。✓
- M2-09 §2 L16 调用点行号 `api/collection.rs:765,776` —— grep 确认 `collection.rs:765` 和 `:776` 两处 `brute_search(` 调用。✓

### 2.4 M2-12 read_snapshot / 恢复路径 / 递归 list ✓

- `read_snapshot(vfs, src, db_path)`：M2-12 §3 L29 —— 解包单文件快照到 db_path 目录。✓
- 恢复路径：export → `read_snapshot` → `Db::open`（M2-12 §3 L36-39）。✓
- 递归 list：M2-12 §3 L41 —— `list("segments")` → 逐段 `list("segments/seg_<ulid>")` → 逐文件 `read_at`。递归层数固定 2 层（segments/seg_xxx/file）。与实际段目录结构（扁平文件，无子目录）一致。✓

---

## 3. README 约束核查

### 3.1 jieba 约束放宽与 M2 Prompt 一致性 ✓

README L385：
> vane-wasm default features 不启 dict-zh（红线）：**永不启用 dict-zh**（dict-zh 捆绑 vane-dict-zh 词典数据进产物，红线）；**jieba feature（仅算法代码 DAT/HMM/seg，无词典数据）可在 vane-wasm 非 default 启用，但须通过 800KB 门禁实测**（A-I5 放宽 M1 约束……M2 Prompt 明确「含 jieba 代码、不含词典数据」）

- dict-zh 红线（捆绑词典数据）永不启。✓
- jieba feature（仅算法代码）可启用须过门禁。✓
- 与 M2 Prompt「含 jieba 代码、不含词典数据」一致。✓
- M2-04 §3 L34 同步：`jieba feature（仅算法代码 DAT/HMM/seg，无词典数据）在 vane-wasm 非 default 启用`。✓

### 3.2 体积预算累计管理表登记完整性 ✓

README L399-409 体积评估表登记的依赖：

| 依赖 | 引入模块 | 登记行 | 覆盖 |
|---|---|---|---|
| wasm-bindgen | M2-01 | L401 | ✓ |
| web-sys / js-sys | M2-02/03/04（opfs/idb/worker） | L402 | ✓ |
| wasm-bindgen-futures | M2-04 | L403 | ✓ |
| ruzstd | M2-08 | L404 | ✓ |
| zstd | M2-08 | L405 | ✓ |
| rayon | M2-10 | L406 | ✓ |
| jieba 算法代码 | M2-04（feature 启用） | L407 | ✓ |
| overlay 内核 | M2-02 | L408 | ✓ |
| cbindgen | M2-11 | L409 | ✓ |

M2-02 CRC32 手写（M2-02 §2 L15「手写 8 行，避免新依赖」），不引入 crc32fast——与 overlay 内核「纯 Rust 无依赖」一致。✓
M2-05/06/07/09/12/13/14 无新依赖——均无登记需求。✓

---

## 4. 发现清单

### 阻塞（Blocking）：0

### 重要（Important）：0

### 次要（Minor）：5

---

#### S-1 [次要] M2-02 `OpfsVfs` struct 草图与 `OverlayBackend` impl 目标不一致

**文件**：`docs/plans/m2/modules/M2-02-opfs-vfs.md` §3 L65-66
**证据**：
- L65：`pub struct OpfsVfs { overlay: MemOverlay, sah: RefCell<FileSystemSyncAccessHandle> }`
- L66：`impl OverlayBackend for OpfsVfsBackEnd { /* SyncAccessHandle.read/write/flush/getSize/truncate */ }`

**问题**：`OpfsVfsBackEnd` 未在任何处定义。若 `OpfsVfs` 自身 impl `OverlayBackend`，则 `MemOverlay` 持有 `Arc<dyn OverlayBackend>` = `Arc<OpfsVfs>`，而 `OpfsVfs` 又持有 `MemOverlay` → Arc 循环引用 → 内存泄漏（永不 drop）。正确设计应拆分：`OpfsBackend { sah }` impl `OverlayBackend`，`OpfsVfs { overlay: MemOverlay }` 仅持 overlay（overlay 持 `Arc<OpfsBackend>`）。

**影响**：内部实现细节，不影响 Vfs trait 契约或 core。实现者可自行修正，但计划草图误导。
**建议**：将 struct 草图改为 `OpfsVfs { overlay: MemOverlay }` + 独立 `OpfsBackend { sah: RefCell<FileSystemSyncAccessHandle> }` impl `OverlayBackend`，与 M2-03 `IdbBackEnd` 模式对齐。

---

#### S-2 [次要] M2-02 superblock 自身损坏恢复未显式处理

**文件**：`docs/plans/m2/modules/M2-02-opfs-vfs.md` §3 L104-108
**证据**：崩溃恢复覆盖 meta_slot CRC（3 时点），但未涉及 superblock 本身损坏（`meta_offset[2]`/`meta_size[2]`/`container_size` 不可读）。`MemOverlay::open`（L53）「读 superblock + 活跃 meta_slot 重建文件表」依赖 superblock 可读。

**问题**：若 superblock 写入时崩溃（部分字节已写），`meta_offset`/`meta_size` 可能不一致。meta_slot 虽在固定偏移（4KB / 4KB+256KB），但计划未声明「meta_slot 偏移为常量，superblock 损坏时可硬编码定位」。

**影响**：superblock 很小（4KB），OPFS 单次 write 原子性覆盖该范围的概率高；风险低。但计划应显式声明恢复策略。
**建议**：在 §3 容器格式或崩溃恢复节加注：「meta_slot 偏移为格式常量（slot_0=4KB, slot_1=4KB+256KB），recover 不依赖 superblock 的 meta_offset 字段，直接按常量定位两槽并取 generation 最大 + CRC 通过者」。

---

#### S-3 [次要] M2-02 compaction「全量 rewrite」机制欠细

**文件**：`docs/plans/m2/modules/M2-02-opfs-vfs.md` §3 L115-116
**证据**：「分配新容器区域 → 拷贝活跃区间 → 翻转。初版 append-only + 阈值触发全量 rewrite（简单），碎片管理优化延后」。

**问题**：单 OPFS 文件无法 truncate 中间空洞。若旧活跃区间在新区域之前，compaction 后旧区域进 free list 但文件物理大小不缩。真正缩文件需将活跃区间拷贝到文件头部 → truncate 尾部，但这是原地覆盖活跃数据，风险高。计划未说明 compaction 是「拷贝到尾部 + 旧区进 free list（不缩文件）」还是「拷贝到头部 + truncate（缩文件）」。

**影响**：计划已标注「初版简单 + 优化延后」，不阻塞 M2 交付（append-only 可工作）。仅 compaction 效果不确定。
**建议**：明确初版 compaction 为「拷贝活跃区间到新尾部区域 + 旧区进 free list + meta 翻转（不 truncate，文件不缩，仅碎片整理）」，文件缩减延后。

---

#### S-4 [次要] M2-04 `VaneWorker` struct 草图 `vfs: Box<dyn Vfs>` 类型应为 `Arc`

**文件**：`docs/plans/m2/modules/M2-04-worker-shell.md` §3 L39
**证据**：`pub struct VaneWorker { /* db: Option<Db>, collections: HashMap<u32, Collection>, vfs: Box<dyn Vfs> */ }`

**问题**：`Db::open` 首参为 `Arc<dyn Vfs>`（`api/db.rs:35`），`DbInner.vfs` 字段为 `Arc<dyn Vfs>`（`api/db.rs:18`）。Worker 持有 `Box<dyn Vfs>` 无法传给 `Db::open`（需 Arc）。且 `Db` 已内部持有 `Arc<dyn Vfs>`，Worker 无需重复持有（export 等场景可从 `DbInner` 取或 Worker 另持 `Arc`）。

**影响**：仅注释草图（`/* ... */`），非 definitive 签名。但类型误导。
**建议**：改为 `vfs: Arc<dyn Vfs>` 或删除该字段注释（Db 已持 Vfs）。

---

#### S-5 [次要] M2-08 header.rs 行号描述偏差 1

**文件**：`docs/plans/m2/modules/M2-08-stored-zstd.md` §2 L32
**证据**：计划描述「行 40 是 `buf.len() < 8` 长度检查」。

**实际**：`crates/vane-core/src/segment/header.rs:39` → `if buf.len() < 8 {`，行 40 是 `return Err(VaneError::Corrupt(...))`。

**影响**：修改目标行号（:21 encode、:46 decode）均精确正确，仅上下文描述行号偏 1。不影响实现。
**建议**：将「行 40」改「行 39」。

---

## 5. 无法确认项

无。全部复核范围内的声明均已通过源码核查或计划交叉引用确认。

---

## 6. 结论

**PASS_WITH_FINDINGS**：fix 轮改动整体对齐实际代码，overlay 链三计划契约一致，双 meta_slot + CRC 覆盖 I-6 三时点崩溃场景，core/Vfs trait 零改动声明成立，签名修正点全部对齐，README jieba 约束与 M2 Prompt 一致，体积预算表登记完整。5 条次要发现均为实现细节或描述偏差，不阻塞开工。
