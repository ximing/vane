# M4 阶段零：测试基础设施设计

> 来源：Phase 0 只读设计 SubAgent（Plan agent / opus）产出，编排者落档。
> 完整 M4 计划见 `docs/plans/m4/M4-PLAN.md`。ledger 见 `PROGRESS.md`。

## 1. 概述 + 设计目标

本设计为 Vane M4（生产门槛）阶段零的只读蓝图，覆盖六项测试基础设施与可观测性骨架：

1. **FaultVfs 故障注入 VFS** —— 在 `Vfs` trait 上做包装实现，精确模拟 IO 错误 / 部分写 / 写后丢 / ENOSPC / 延迟乱序，并在 `persist_meta` 翻转前/后、WAL flush 前/后、merge persist 前/后注入失败，验证崩溃恢复一致性。
2. **cargo-fuzz 集成** —— 检索 / 持久化 / 合并 / 词典 fuzz targets + CI 短跑 + 定期长跑。
3. **proptest** —— property-based 不变量（检索排序稳定 / persist round-trip / merge 不丢文档）。
4. **跨版本兼容 fixture** —— v0.1 旧格式 fixture + 当前版本读取/迁移测试。
5. **tracing feature 骨架** —— `cfg(feature="tracing")` 埋点（零开销，I-5 能力开关），不启用时 wasm 体积不变。
6. **inspect API** —— `Db::stats()` / `Db::segment_info()` 新增 pub API（不改 M0-M3 冻结签名）。

**设计目标（优先级 MoSCoW）**：
- Must：测试安全铁律（不破坏宿主机）、core 禁 std::fs、依赖黑名单、不改冻结 pub API、wasm 800KB gzip 红线。
- Should：精确故障注入点位、CI 友好可复现、fuzz/proptest 不污染生产构建。
- Could：loom 竞态、延迟乱序故障、tracing 指标 dashboard。
- Won't：内置 embedding/GPU/SQL/分布式（不碰）。

**核心约束**：FaultVfs 是 `cfg(test)`/dev-feature，不进生产二进制；tracing 是 `cfg(feature)` 能力开关；inspect API 纯新增；SPEC 修订需用户批准。

---

## 2. 现状摸底

### 2.1 Vfs trait 方法表（`crates/vane-core/src/vfs/mod.rs`）

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

**关键性质**：
- **同步 API**：8 个方法全部同步（非 async），返回 `Result<()>` / `Result<usize>` / `Result<u64>`。
- **trait 对象友好**：`Send + Sync`，core 内全部经 `Arc<dyn Vfs>` 持有（`DbInner.vfs: Arc<dyn Vfs>`，`CollectionInner.vfs: Arc<dyn Vfs>`，`ManifestStore.vfs: Arc<dyn Vfs>`，`Wal.vfs: Arc<dyn Vfs>`）。
- **错误类型**：`VaneError::Io(String)`（code -1）。
- **现有实现**：`MemoryVfs`（纯内存 HashMap）、`StdFsVfs`（native 唯一 std::fs 模块，`cfg(not(target_arch="wasm32"))`）、`PageCache`（read-through LRU，`&mut self`，非 Vfs impl）。
- **路径语义**：相对路径，`StdFsVfs::resolve` join root；`MemoryVfs::list` 按 dir 前缀过滤下一层分量。
- **PageCache 的 `read(&mut self, vfs: &dyn Vfs, ...)`**：缓存层在 Vfs 之上，FaultVfs 透明包装不破坏 PageCache 语义。

### 2.2 persist_meta / WAL flush / merge persist 关键点文件位置

| 关键点 | 文件位置 | 调用序列 |
|---|---|---|
| **manifest 原子切换**（persist_meta） | `crates/vane-core/src/persistence/mod.rs:100` `ManifestStore::save_atomic` | `delete(tmp)` → `create(tmp)` → `write_at(tmp, json, 0)` → `sync(tmp)` → `rename(tmp, target)` |
| **WAL append+sync** | `crates/vane-core/src/wal/mod.rs:65` `Wal::append` | `serde_json::to_vec` → `vfs.append(path, line)` → `vfs.sync(path)` |
| **WAL truncate** | `crates/vane-core/src/wal/mod.rs:110` `Wal::truncate` | `delete(path)` → `create(path)` → `sync(path)`（仅 compact/merge 成功 + manifest 切换后调） |
| **flush 段物化** | `crates/vane-core/src/api/collection.rs:303` `Collection::flush` | `SegmentWriter::finalize()`（写 header/vectors/stored/idmap/scalars）→ `write_inverted(seg_dir, &inverted)` → `write_hnsw(seg_dir, &graph)` → `Wal::append(AddSegment)` → `ManifestStore::add_segment` → 内存快照 swap |
| **merge persist** | `crates/vane-core/src/merge/mod.rs:252` `finalize_merge` | `writer.finalize()`（新段 header/vectors/stored/idmap/scalars）→ `write_inverted(seg_dir, &inv)` → `write_hnsw(seg_dir, &graph)` → 返回 new_meta |
| **merge manifest 切换** | `crates/vane-core/src/api/collection.rs:498` `merge_segments` | `Wal::append(DeleteSegment×N)` → `Wal::append(AddSegment new)` → `manifest_store.save_atomic(&manifest)` → 内存快照 swap → `delete_segment_dir(旧段)` |
| **崩溃恢复入口** | `crates/vane-core/src/wal/mod.rs:144` `recover` | `Wal::read_all()` → 重放 AddTombstone / AddSegment(半成品清理) / DeleteSegment(信息记录) → `cleanup_orphan_segment_dirs` |

**注入点精确位置分析**（FaultVfs 设计核心依据）：
- **manifest 翻转前** = `save_atomic` 的 `rename(tmp, target)` **之前**的任意一步（`create`/`write_at`/`sync`）失败 → manifest 未切换，旧 manifest 完好，tmp 残留（下次 `save_atomic` 先 `delete(tmp)` 处理，I16）。
- **manifest 翻转后** = `rename` 返回**之后**，`save_atomic` 返回 `Ok(())` 之后的代码路径失败 → manifest 已指向新状态，旧段或孤儿段需经 `recover` 清理。
- **WAL flush 前** = `Wal::append` 的 `vfs.append(path, line)` 失败 → 记录未落盘，崩溃恢复时看不到此 AddSegment。
- **WAL flush 后** = `Wal::append` 的 `vfs.sync(path)` 失败或返回后崩溃 → 取决于 OS 是否已落盘；FaultVfs 可模拟"写后丢"（sync 返回 Ok 但实际未持久化）。
- **merge persist 前** = `finalize_merge` 的 `write_inverted` / `write_hnsw` 失败 → 新段文件半成品，`merge_segments` 后续 `manifest_store.save_atomic` 不会执行（`?` 传播），旧段保留。
- **merge manifest 切换前** = `merge_segments` 的 `Wal::append(DeleteSegment/AddSegment)` 或 `save_atomic` 失败 → manifest 未切换，新段孤儿（recover 清理）。
- **merge manifest 切换后** = `save_atomic` 返回后，内存快照 swap 前/后崩溃 → 重启后 manifest 指向新段，旧段 ULID 已不在 manifest → recover 时旧段目录需清理（但 WAL 有 DeleteSegment 记录，recover DeleteSegment 仅信息记录不动作；目录扫描清理孤儿）。

### 2.3 现有测试布局

| 类别 | 位置 | 说明 |
|---|---|---|
| 单元测试 | `crates/vane-core/src/**/tests.rs`（cfg(test) 模块） | 各模块内联，如 `vfs/tests.rs`、`segment/tests.rs`、`merge/tests.rs`、`persistence/tests.rs`、`wal/tests.rs`、`api/tests.rs`、`api/reindex_tests.rs` |
| 集成测试 | `crates/vane-core/tests/*.rs` | 16 个集成测试文件 |
| 集成测试目录 | `crates/vane-core/tests/` | `cold_start_gate.rs` / `corpus_compat.rs` / `hnsw_recall.rs` / `jieba_compat.rs` / `million_scale.rs` / `ndcg_wiki.rs` / `ndcg_wiki_zh.rs` / `pre_filter.rs` / `recall.rs` / `recall_fixture.rs` / `recall_regression.rs` / `text_persistence.rs` / `tombstone_merge.rs` / `userdict_reindex.rs` / `wal_crash.rs` |
| fixture | `crates/vane-core/tests/fixtures/` | `jieba_200.txt`、`wiki_zh/`（词典/语料 fixture，非段格式 fixture） |
| bench | `crates/vane-core/benches/` | `hybrid_search`、`batch_add`、`cold_start`（criterion，dev-dep） |
| **现有崩溃测试** | `tests/wal_crash.rs` | 已验证 SPEC §6.4 WAL 崩溃恢复，但用 **MemoryVfs + 手工状态**，非故障注入 VFS |
| **现有 corpus compat** | `tests/corpus_compat.rs` | 用 **StdFsVfs + tempdir** 重新生成 → close → reopen 验证；注释明确「真实历史版本 golden fixture 待首个正式发布后补」 |
| **fuzz/proptest** | 无 | 全工作区无 cargo-fuzz / proptest 痕迹 |

### 2.4 CI 现状 job 清单（`.github/workflows/ci.yml`）

现有 16 jobs（按 `needs` 链）：
1. `fmt`（rustfmt check）
2. `clippy`（needs fmt，-D warnings）
3. `test`（needs clippy，cargo test --workspace --all-features）
4. `recall`（needs test，recall smoke + recall_regression §13.2-1）
5. `wasm32-check`（needs test，core+wasm check wasm32 + clippy wasm + check-no-std-fs.sh 双保险）
6. `deny`（needs wasm32-check，cargo-deny 0.19.9 + 依赖黑名单）
7. `corpus-compat`（needs test，corpus_compat §13.3）
8. `cold-start`（needs test，cold_start_gate --release --ignored §13.1）
9. `wasm32-size`（needs test，check-wasm-size.sh §13.2-3 800KB 红线）
10. `dict-size`（needs test，check-dict-size.sh）
11. `dict-hash`（needs test，check-dict-hash.sh §12.3 三渠道哈希）
12. `jieba-compat`（needs test，jieba_compat --features dict-zh --release §13.2-2①）
13. `ndcg-wiki`（needs test，ndcg_wiki + ndcg_wiki_zh --features dict-zh --release §13.2-2②）
14. `go-host`（needs test，vane-ffi + go build/test/demo）
15. `go-cross`（needs go-host，zig cc 4 平台交叉编译）
16. `wasm-recall`（needs test，run-wasm-recall.sh §8.4 双变体）

**触发**：push（main，paths-ignore website/docs-site）+ PR。**无 cron / workflow_dispatch 长跑 job**。

**deny.toml 黑名单**：regex（wrappers 限 napi-derive-backend + criterion）/ tokio / prost / tonic / openssl / lindera / ndarray / wee_alloc / dashmap / parking_lot。

---

## 3. 六项逐项设计

### 3.1 FaultVfs 故障注入 VFS

#### 方案

**FaultVfs = 包装任意 inner Vfs 的 trait impl**，注入可控故障。`cfg(test)` + 可选 dev-feature `fault-injection`（供 vane-ffi 集成测试用，但默认不启用）。包装 `MemoryVfs`（核心崩溃恢复测试主力）或 `StdFsVfs`（tempdir，验证真实 fs 行为对齐）。

**核心机制**：在每次 Vfs 方法调用前，查询 **故障规则表**（path/op/offset 匹配），命中则按规则返错 / 部分写 / 写后丢 / ENOSPC / 延迟。

#### 关键 API / struct 签名

```rust
// crates/vane-core/src/vfs/fault.rs（新增，cfg(test) 或 feature="fault-injection"）

use crate::types::{Result, VaneError};
use crate::vfs::Vfs;
use std::sync::{Arc, Mutex};

/// 故障规则。按 (path_pattern, op) 匹配，触发后消费（one-shot）或持久（每次命中）。
#[derive(Debug, Clone)]
pub enum Fault {
    /// 指定 op 在指定 path 返 VaneError::Io(msg)。one_shot=true 时仅触发一次。
    IoError { op: VfsOp, path_pattern: String, msg: String, one_shot: bool },
    /// write_at/append 写 N 字节后返 Err（模拟中途失败）。FaultVfs 先写前 N 字节再返错。
    PartialWrite { op: VfsOp, path_pattern: String, bytes_before_fail: usize },
    /// sync 返 Ok 但实际未持久化（MemoryVfs 模拟"写后丢"：标记文件 dirty，重启模拟丢弃）。
    /// 仅对 StdFsVfs 有意义（MemoryVfs sync 本是 noop）；StdFsVfs 模式下 sync 真 fsync 但下次 read 返旧数据。
    LostWrite { path_pattern: String },
    /// write_at/append 返 VaneError::Io("ENOSPC")，不写真字节。
    Enospc { op: VfsOp, path_pattern: String },
    /// 注入延迟（ms）。仅测试并发时序，不影响正确性。
    Delay { op: VfsOp, path_pattern: String, ms: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VfsOp { Create, ReadAt, WriteAt, Append, Sync, Rename, Delete, List }

/// 故障注入 VFS。包装 inner Vfs，按规则表注入故障。
pub struct FaultVfs {
    inner: Arc<dyn Vfs>,
    faults: Mutex<Vec<Fault>>,  // 已注册故障规则；命中 one_shot 后移除
    /// LostWrite 模拟：标记"path 的最后一次 sync 被丢弃"。下次 open/recover 时模拟。
    lost_writes: Mutex<Vec<String>>,
}

impl FaultVfs {
    pub fn new(inner: Arc<dyn Vfs>) -> Self {
        Self { inner, faults: Mutex::new(Vec::new()), lost_writes: Mutex::new(Vec::new()) }
    }
    pub fn wrap_memory() -> Self { Self::new(Arc::new(MemoryVfs::new())) }

    /// 注册故障规则（链式）。测试用：`vfs.inject(Fault::IoError{...})`。
    pub fn inject(&self, fault: Fault) -> &Self {
        self.faults.lock().unwrap().push(fault);
        self
    }

    /// 检查是否命中故障，命中返 Some(故障效果)。
    fn check_fault(&self, op: VfsOp, path: &str) -> Option<FaultAction> { /* ... */ }
}

enum FaultAction {
    ReturnErr(VaneError),
    PartialWrite(usize),  // 写前 N 字节后返 Err
    Enospc,
    DelayMs(u64),
    MarkLostWrite,
}

impl Vfs for FaultVfs {
    fn create(&self, path: &str) -> Result<()> {
        if let Some(a) = self.check_fault(VfsOp::Create, path) {
            return apply_action_create(a, path);  // ReturnErr / DelayMs
        }
        self.inner.create(path)
    }
    fn read_at(&self, path: &str, buf: &mut [u8], offset: u64) -> Result<usize> {
        if let Some(a) = self.check_fault(VfsOp::ReadAt, path) { /* ... */ }
        self.inner.read_at(path, buf, offset)
    }
    fn write_at(&self, path: &str, buf: &[u8], offset: u64) -> Result<()> {
        if let Some(FaultAction::PartialWrite(n)) = self.check_fault(VfsOp::WriteAt, path) {
            // 先写前 n 字节（可能不足 n，取 min(n, buf.len())）
            let n = n.min(buf.len());
            self.inner.write_at(path, &buf[..n], offset)?;
            return Err(VaneError::Io(format!("partial write at {} ({} bytes)", path, n)));
        }
        if let Some(FaultAction::Enospc) = self.check_fault(VfsOp::WriteAt, path) {
            return Err(VaneError::Io(format!("ENOSPC: write_at {} (simulated)", path)));
        }
        self.inner.write_at(path, buf, offset)
    }
    fn append(&self, path: &str, buf: &[u8]) -> Result<u64> { /* 类似 write_at */ }
    fn sync(&self, path: &str) -> Result<()> {
        if let Some(FaultAction::MarkLostWrite) = self.check_fault(VfsOp::Sync, path) {
            self.lost_writes.lock().unwrap().push(path.to_string());
            return Ok(());  // 假装 sync 成功
        }
        self.inner.sync(path)
    }
    fn rename(&self, from: &str, to: &str) -> Result<()> {
        // rename 是 manifest 原子切换关键 —— check_fault(VfsOp::Rename, from) 或 (to)
        self.inner.rename(from, to)
    }
    fn delete(&self, path: &str) -> Result<()> { /* ... */ }
    fn list(&self, dir: &str) -> Result<Vec<String>> { /* ... */ }
}
```

#### 精确注入失败机制（核心难点）

**问题**：M4-PLAN 要求在 `persist_meta` 翻转前/后、WAL flush 前/后、merge persist 前/后**精确注入**。仅靠 path_pattern 匹配够吗？

**机制设计**（三层精度，递进）：

**层 1：path + op 匹配（基础，够用 80% 场景）**：
- manifest 翻转前注入：`Fault::IoError { op: VfsOp::Sync, path_pattern: "*/manifest.json.tmp", ... }` 或 `op: VfsOp::Rename, path_pattern: "*/manifest.json.tmp"`。tmp 路径由 `ManifestStore::tmp_path()` 生成 = `{db_path}/manifest.json.tmp`，pattern 用 glob 或前缀匹配。
- WAL flush 前注入：`Fault::IoError { op: VfsOp::Append, path_pattern: "*/wal.log", ... }`。
- merge persist 前注入：`Fault::IoError { op: VfsOp::WriteAt, path_pattern: "*/segments/seg_*/inverted.bin", ... }` 或 `*/hnsw.bin`。

**层 2：调用计数器（精确第 N 次调用）**：
- `FaultVfs` 维护 `call_count: HashMap<(VfsOp, path_pattern), u32>`。
- `Fault::IoError { trigger_on_nth: u32, ... }` —— 仅在第 N 次匹配调用时触发。测试可精确控制「第 2 次 save_atomic 的 sync 失败」「第 3 次 wal append 失败」。
- 解决 path 匹配无法区分"翻转前 vs 后"的问题：翻转前是 tmp 的 sync，翻转后是 target 的 rename（已无 sync），用 op + path + 计数器组合即可区分。

**层 3：hook 点位标记（可选，仅当层 1+2 不够时启用）**：
- 在 `ManifestStore::save_atomic` / `Wal::append` / `finalize_merge` / `merge_segments` 内部，在关键步骤前后调 `FaultVfs::mark_checkpoint(name)`。
- `Fault::AtCheckpoint { checkpoint: &str, fault: Fault }` —— 仅在标记 checkpoint 后的下次匹配调用触发。
- **取舍**：层 3 需要在生产代码加 `cfg(test)` hook 调用，污染面广，**不推荐**。层 1+2（path + op + 计数器）已覆盖所有 M4 需求的注入点。**推荐层 1+2，不实现层 3**。

**注入点映射表**（FaultVfs 层 1+2 实现的精确点位）：

| M4 需求注入点 | FaultVfs 规则 | 验证恢复后状态 |
|---|---|---|
| manifest 翻转前（sync tmp 失败） | `IoError{op:Sync, path:"*.json.tmp", one_shot:true}` | manifest 未切换，旧完好，tmp 残留下次清理（I16） |
| manifest 翻转前（rename 失败） | `IoError{op:Rename, path:"*.json.tmp", one_shot:true}` | 同上；rename 在 StdFsVfs 是原子，FaultVfs 可模拟返错但 inner 未实际 rename |
| manifest 翻转后（save_atomic 返回 Ok 后，后续代码崩） | 测试在 `save_atomic` 返回后 drop FaultVfs / 不 drop，重开 Db | manifest 指向新段，旧段孤儿 → recover 清理 |
| WAL flush 前（append 失败） | `IoError{op:Append, path:"*/wal.log", one_shot:true}` | AddSegment 未记录；若 manifest 已切换则段可见但无 WAL 记录（不影响）；若未切换则孤儿 |
| WAL flush 后（sync 失败） | `IoError{op:Sync, path:"*/wal.log", one_shot:true}` | append 已写但未持久化；模拟"写后丢"用 LostWrite |
| WAL 写后丢（sync Ok 但未落盘） | `LostWrite{path:"*/wal.log"}` | 重启后 WAL 不含此记录 → 模拟未确认事务丢失 |
| merge persist 前（write_inverted 失败） | `IoError{op:WriteAt, path:"*/segments/seg_*/inverted.bin"}` | finalize_merge `?` 传播，merge_segments 不到 save_atomic，旧段保留 |
| merge manifest 切换前（save_atomic 失败） | `IoError{op:Rename, path:"*.json.tmp"}` 注入到 merge_segments 的 save_atomic | manifest 未切换，新段孤儿 |
| merge manifest 切换后 | save_atomic 返回后重开 Db | manifest 指向新段，旧段目录孤儿 → recover 清理 |
| 部分写（write_at 写 N 字节后失败） | `PartialWrite{op:WriteAt, path:"*/header.bin", bytes_before_fail:8}` | header.bin 写 8 字节（magic+version）后失败 → decode_header 校验失败 → Corrupt |
| ENOSPC（磁盘满） | `Enospc{op:WriteAt, path:"*"}` 或特定段文件 | write_at/append 返 ENOSPC，不损已有数据 |

#### 文件位置

- `crates/vane-core/src/vfs/fault.rs` —— FaultVfs impl + Fault enum + VfsOp。
- `crates/vane-core/src/vfs/mod.rs` —— 加 `#[cfg(any(test, feature="fault-injection"))] pub mod fault;`。
- `crates/vane-core/Cargo.toml` `[features]` 加 `fault-injection = []`（dev/optional，默认不启用）。
- 崩溃恢复测试：`crates/vane-core/tests/crash_recovery.rs`（新集成测试文件）。

#### 取舍

- **MemoryVfs 为主 vs StdFsVfs 为主**：MemoryVfs 为主力（快、无真 fs 副作用、CI 友好）；StdFsVfs + tempdir 用于验证「真实 fs 行为与 MemoryVfs 一致」（少量 conformance 对齐测试）。**推荐 MemoryVfs 主力**。
- **层 3 hook 不实现**：避免污染生产代码，层 1+2 足够覆盖。
- **dev-feature `fault-injection` vs 纯 `cfg(test)`**：`cfg(test)` 够用于 vane-core 内联测试；dev-feature 仅在 vane-ffi 集成测试需 FaultVfs 时启用。**推荐 `cfg(test)` 为主 + `feature="fault-injection"` 作为 dev/optional feature 供下游测试**。
- **LostWrite 在 MemoryVfs 的语义**：MemoryVfs sync 本是 noop，LostWrite 需特殊处理 —— FaultVfs 记录 lost_writes 列表，但 MemoryVfs 数据不会真丢；模拟"重启"需测试 drop FaultVfs + 重新构造 MemoryVfs（但 inner 数据丢失）。**推荐**：LostWrite 主要用于 StdFsVfs + tempdir 场景（sync 返 Ok 但测试 reopen 时模拟未落盘 —— 实际 StdFsVfs 已 fsync，难以真模拟丢写，故 LostWrite 退化为"sync 失败"注入 + 测试手动构造"未 sync 的数据"场景）。**LostWrite 列为 Could，非 Must**。

#### 风险

- **path pattern 匹配语义**：glob vs 前缀 vs 正则。**推荐前缀匹配 + 通配 `*`**（自研轻量 matcher，不引 regex 黑名单）。
- **FaultVfs 包装 PageCache 的透明性**：PageCache 持 `&dyn Vfs`，FaultVfs 透明包装不破坏。但 PageCache 的 `read(&mut self, vfs: &dyn Vfs, ...)` 调用方需传 FaultVfs 引用。**无风险**。
- **one_shot 故障的并发安全**：`faults: Mutex<Vec<Fault>>`，命中后 remove。并发场景下 Mutex 保护，但并发测试可能竞争移除同一 fault。**推荐**：并发测试不用 one_shot，用持久 fault + 计数器。
- **StdFsVfs 的 rename 原子性**：FaultVfs 模拟 rename 失败时，inner StdFsVfs 可能已实际 rename（取决于 FaultVfs 在调 inner 前 check）。**推荐**：check_fault 在调 inner 前执行，返错则不调 inner，保证 inner 状态不变。

---

### 3.2 cargo-fuzz 集成

#### 方案

**crate 布局取舍**：

| 选项 | 优点 | 缺点 |
|---|---|---|
| A. `crates/vane-fuzz`（独立 crate） | 与 vane-core 解耦；cargo-fuzz 标准布局；不污染 vane-core Cargo.toml | 新增 crate，workspace members +1；需 `path` 依赖 vane-core |
| B. `crates/vane-core/fuzz/`（vane-core 子目录） | cargo-fuzz 默认布局；无需新 crate | vane-core Cargo.toml 需加 `[dev-dependencies] cargo-fuzz`；但 cargo-fuzz 是 libfuzzer-sys 绑定，非普通 dev-dep |

**推荐 A（`crates/vane-fuzz` 独立 crate）**：
- cargo-fuzz 0.12+ 要求 `[[bin]]` target + `libfuzzer-sys`，独立 crate 隔离 libfuzzer C 依赖，**绝不污染 vane-core/wasm/ffi 生产构建**。
- workspace `Cargo.toml` `members` 加 `"crates/vane-fuzz"`，但加 `default-members` 排除 vane-fuzz（`cargo test --workspace` 不跑 fuzz）。
- vane-fuzz `Cargo.toml`：`[dependencies] vane-core = { path = "../vane-core" }` + `libfuzzer-sys = "0.4"`（cargo-fuzz 0.12+ 自动管理）。
- **libfuzzer-sys 传递依赖检查**：libfuzzer 是 C++ libFuzzer，无 regex/tokio/prost/tonic/openssl/lindera/ndarray/wee_alloc/dashmap/parking_lot 传递依赖。**不触黑名单**。但需 cargo-deny 验证（vane-fuzz 不进 wasm32-check 的 cargo check 范围，因 vane-fuzz 不在 wasm target）。

#### cargo-fuzz 配置

```toml
# crates/vane-fuzz/Cargo.toml
[package]
name = "vane-fuzz"
version = "0.0.0"
edition = "2021"
publish = false  # 不发布

[dependencies]
vane-core = { path = "../vane-core" }
libfuzzer-sys = "0.4"

# fuzz targets
[[bin]]
name = "brute_search_fuzz"
path = "fuzz_targets/brute_search_fuzz.rs"

[[bin]]
name = "hnsw_search_fuzz"
path = "fuzz_targets/hnsw_search_fuzz.rs"

[[bin]]
name = "persist_roundtrip_fuzz"
path = "fuzz_targets/persist_roundtrip_fuzz.rs"

[[bin]]
name = "merge_fuzz"
path = "fuzz_targets/merge_fuzz.rs"

[[bin]]
name = "dict_load_fuzz"
path = "fuzz_targets/dict_load_fuzz.rs"
```

**fuzz targets 设计**（每个独立 `#[no_mangle] extern "C" fn`）：

| target | 输入 | 不变量 |
|---|---|---|
| `brute_search_fuzz` | 随机文档集 + 随机 query（text/vector/topK） | 暴力检索不 panic、topK 合法、score 非NaN |
| `hnsw_search_fuzz` | 随机文档集 + query | HNSW 不 panic、recall 与暴力一致（小规模） |
| `persist_roundtrip_fuzz` | 随机文档集 + add + flush + reopen | round-trip 数据一致、external_id 全回填 |
| `merge_fuzz` | 多段文档 + delete + compact | merge 不丢文档（除 tombstone）、docid 连续 |
| `dict_load_fuzz` | 畸形词典字节 | 降级 bigram 不抛错（M2-04 铁律） |

#### CI workflow 草案

**fuzz-smoke（push/PR，每 target 60s）**：
```yaml
  fuzz-smoke:
    needs: test
    runs-on: ubuntu-latest
    timeout-minutes: 15
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@nightly  # cargo-fuzz 需 nightly
      - uses: Swatinem/rust-cache@v2
      - name: Install cargo-fuzz
        run: cargo install cargo-fuzz --locked
      - name: Fuzz smoke (60s per target)
        run: |
          for target in brute_search_fuzz hnsw_search_fuzz persist_roundtrip_fuzz merge_fuzz dict_load_fuzz; do
            cargo fuzz run $target -- -max_total_time=60 -max_len=4096
          done
        working-directory: crates/vane-fuzz
```

**fuzz-long（cron + workflow_dispatch）**：
```yaml
  fuzz-long:
    on:
      schedule:
        - cron: '0 3 * * 0'  # 每周日 03:00 UTC
      workflow_dispatch:
    runs-on: ubuntu-latest
    timeout-minutes: 60
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@nightly
      - uses: Swatinem/rust-cache@v2
      - name: Install cargo-fuzz
        run: cargo install cargo-fuzz --locked
      - name: Fuzz long (10min per target)
        run: |
          for target in brute_search_fuzz hnsw_search_fuzz persist_roundtrip_fuzz merge_fuzz dict_load_fuzz; do
            cargo fuzz run $target -- -max_total_time=600 -max_len=65536 || true
          done
        working-directory: crates/vane-fuzz
      - name: Upload crash artifacts
        if: failure()
        uses: actions/upload-artifact@v4
        with:
          name: fuzz-crash
          path: crates/vane-fuzz/fuzz/artifacts/
```

#### 取舍

- **nightly 依赖**：cargo-fuzz 需 nightly（-Z sanitizer）。CI 用 `dtolnay/rust-toolchain@nightly`。**风险**：nightly 可能 break；pin 特定 nightly 版本（如 `nightly-2026-07-01`）。
- **default-members 排除 vane-fuzz**：workspace `Cargo.toml` 加 `default-members = ["crates/vane-core", "crates/vane-ffi", "crates/vane-node", "crates/vane-dict-zh", "crates/vane-wasm"]`，确保 `cargo test --workspace` 隐式等价 `cargo test --workspace`（默认不含 vane-fuzz）。但 `--workspace` 显式包含所有 members。**推荐**：vane-fuzz 的 `Cargo.toml` 加 `cargo-fuzz` 仅在 `[[bin]]` 隐式依赖，不进 `[dependencies]` of vane-core。`cargo test --workspace --all-features` 仍会尝试编译 vane-fuzz —— 需 vane-fuzz 在 `default-members` 外且 CI test job 用 `default-members` 范围或 `--exclude vane-fuzz`。**推荐 test job 改为 `cargo test --workspace --all-features --exclude vane-fuzz`**。

#### 风险

- **libfuzzer-sys 传递依赖触黑名单**：需 cargo-deny 验证。预判：不触（libFuzzer 是 C++，Rust 绑定仅 cc + libfuzzer-sys 本身）。
- **wasm 体积**：vane-fuzz 不进 wasm 构建（wasm32-check 的 cargo check 范围 -p vane-core / -p vane-wasm，不含 vane-fuzz）。**无风险**。
- **nightly break**：fuzz-long 用 `|| true` 容错，crash 上传 artifact 人工分析。fuzz-smoke 失败则阻断 PR（要求修复）。

---

### 3.3 proptest

#### 方案

`proptest` 作为 vane-core dev-dep，在集成测试中用。**proptest 传递依赖检查**：proptest 依赖 `bit-set` / `byteorder` / `num_cpus`（build）/ `rustc-ar` / `lazy_static`，**不触黑名单**（regex/tokio/prost/tonic/openssl/lindera/ndarray/wee_alloc/dashmap/parking_lot）。但 proptest 的 `regex` 依赖需确认 —— proptest 0.x 不直接依赖 regex。**预判不触**，cargo-deny 验证。

#### Strategy 设计

```rust
// crates/vane-core/tests/proptest_invariants.rs

use proptest::prelude::*;

/// 随机文档生成 strategy
fn arb_doc(dim: u32) -> impl Strategy<Value = Doc> {
    (
        "[a-z]{1,16}",  // id
        "[a-z ]{0,256}",  // text
        prop::collection::vec(prop::num::f32::ANY, (dim as usize)..=(dim as usize)),  // vector
    ).prop_map(|(id, text, vec)| Doc {
        id: id.into(),
        text: Some(text),
        vector: Some(vec),
        meta: None,
    })
}

fn arb_doc_batch(dim: u32, max_docs: usize) -> impl Strategy<Value = Vec<Doc>> {
    prop::collection::vec(arb_doc(dim), 1..max_docs)
}

fn arb_query(dim: u32) -> impl Strategy<Value = SearchQuery> {
    (
        option::of("[a-z ]{0,64}"),  // text
        option::of(prop::collection::vec(prop::num::f32::ANY, (dim as usize)..=(dim as usize))),
        1u32..100,  // topK
        prop_oneof![Just(SearchMode::Vector), Just(SearchMode::Text), Just(SearchMode::Hybrid)],
    ).prop_map(|(text, vec, top_k, mode)| SearchQuery {
        text, vector: vec, top_k, mode, fusion: FusionSpec::Rrf, filter: None, candidate_multiplier: 3,
    })
}
```

#### 不变量断言

| 不变量 | 测试骨架 | 验证 |
|---|---|---|
| **检索排序稳定合法** | `proptest! { fn search_returns_stable_topk(docs in arb_batch, q in arb_query) { add+flush+search → assert topK 合法、score 非NaN、相同 query 二次检索结果一致 } }` | topK ≤ 结果数、score 单调递减、二次 query 结果一致 |
| **persist round-trip 一致** | `proptest! { fn persist_roundtrip(docs in arb_batch) { add+flush+close+reopen+search → 结果与关闭前基线一致 } }` | external_id 全回填、stored_json 一致、search 结果集相同 |
| **merge 不丢文档** | `proptest! { fn merge_preserves_live_docs(docs in arb_batch, delete_ids in arb_ids) { add multi-flush + delete + compact → 活文档全可见、tombstoned 文档不可见、docid 连续 } }` | compact 前后活文档集合相同、deleted 不可见、无重复 docid |

#### 文件位置

- `crates/vane-core/tests/proptest_invariants.rs`（新集成测试）。
- `crates/vane-core/Cargo.toml` `[dev-dependencies]` 加 `proptest = "1"`。

#### CI

- `test` job 已覆盖（`cargo test --workspace --all-features --exclude vane-fuzz` 会跑 proptest_invariants）。proptest 默认跑 256 cases，CI 友好。
- 可选 proptest-long（cron，10k cases）—— **Could，非 Must**。

#### 取舍 / 风险

- **proptest 与 fuzz 的职责边界**：fuzz 找 panic/crash（未定义行为），proptest 找逻辑不变量（定义但错误的行为）。**推荐两者并存**，不重叠。
- **f32 NaN 风险**：随机 vector 含 NaN 会使 score 排序异常。**推荐**：strategy 过滤 NaN（`.prop_filter("no_nan", |v| v.iter().all(|x| !x.is_nan()))`）或测试内显式处理。
- **proptest 失败 reproducibility**：proptest 自动 persist failing seed 到 `proptest-regressions/`。**推荐**：提交 `proptest-regressions/` 目录确保 CI 复现。

---

### 3.4 跨版本兼容 fixture

#### 方案取舍

| 选项 | 优点 | 缺点 |
|---|---|---|
| A. 提交仓库 `fixtures/v0.1/` | 可控、可审计、离线可复现、CI 无需 build 旧版本 | 二进制 fixture 体积、维护成本、格式变更需同步更新 |
| B. CI 用 v0.1.0 tag 现场生成 | 无二进制提交、自动跟 tag、fixture 永远是真实 release | CI 复杂、需 checkout 双 tag、build 慢、tag 不存在则 job 不可用 |
| C. 混合：仓库提交小 fixture + CI 现场生成大 fixture | 小 fixture 快速验证、大 fixture 现场生成避免体积 | 两套机制维护成本 |

**推荐 A（提交仓库 `tests/fixtures/compat/v0.1.0/`）**：
- 现状：`tests/corpus_compat.rs` 注释明确「真实历史版本 golden fixture 待首个正式发布后补」—— v0.1.x 已发布，fixture 可生成。
- 体积可控：fixture 只需小段数据（<100KB，几文档 + header/vectors/stored/idmap/scalars/inverted），不提交百万文档 fixture。
- CI 友好：无需双 tag checkout，`cargo test --test corpus_compat` 直接读 fixture。

> **编排者补充（未决问题 #1 自查已解决）**：`git tag` 确认仓库存在 `v0.1.0` / `v0.1.1` / `v0.1.2` / `v0.2.0` 四个 tag。v0.1.0 tag 真实存在，方案 A 的 fixture 可用 v0.1.0 tag 离线生成「真实 v1 格式」fixture，无需伪构造。

#### fixture 结构

```
crates/vane-core/tests/fixtures/compat/
├── v0.1.0/
│   ├── manifest.json          # v0.1.0 格式 manifest
│   ├── segments/
│   │   └── seg_<ulid>/
│   │       ├── header.bin     # HEADER_FORMAT_V1
│   │       ├── vectors.bin    # VECTORS_FORMAT_V1（v1 头 8 字节无 dim）
│   │       ├── stored.bin     # STORED_FORMAT_V1（裸 JSON）
│   │       ├── idmap.bin      # IDMAP_FORMAT_V1
│   │       ├── scalars.col    # SCALARS_FORMAT_V1
│   │       └── inverted.bin  # FORMAT_VERSION=1
│   └── wal.log                # 可选：含 WAL 记录
└── README.md                  # fixture 来源、生成方式、格式版本
```

**fixture 生成方式**：用 v0.1.0 tag 的 vane-core 写一个 `tests/fixtures/gen_compat_fixture.rs`（或独立 script），在 commit 时离线运行生成 fixture 提交。CI 不重新生成，只读取。

#### M2-08 per-file format_version v1/v2 双模验证覆盖

现状：`types.rs` 已有 `HEADER_FORMAT_V1` / `VECTORS_FORMAT_V1` / `VECTORS_FORMAT_V2` / `STORED_FORMAT_V1` / `STORED_FORMAT_V2` / `IDMAP_FORMAT_V1` / `SCALARS_FORMAT_V1` / `HNSW_FORMAT_V1`。`header.rs::decode_header` 校验 `version != HEADER_FORMAT_V1` 返 `VaneError::Version`。

**双模覆盖是否足够**：
- v1 fixture（v0.1.0 产物）：验证当前版本读 v1 不变。
- v2 fixture（当前版本 zstd-encode 开启时产物）：验证 v2 读取正确（已有 `tests/corpus_compat.rs` 覆盖当前版本自写自读，但**未覆盖 v0.1.0 v1 fixture 被当前版本读**）。
- **缺口**：无 v0.1.0 tag 的真实 v1 fixture（现状 corpus_compat 是 fresh repo 自写自读，非跨版本）。

**测试用例骨架**：
```rust
// crates/vane-core/tests/cross_version_compat.rs
#[test]
fn reads_v0_1_0_fixture() {
    let vfs = StdFsVfs::with_root("tests/fixtures/compat/v0.1.0");
    let db = Db::open(Arc::new(vfs), "testdb", OpenOptions::default()).unwrap();
    let col = db.collection("docs", build_schema(), CollectionOptions::default()).unwrap();
    // 验证文档可见、external_id 回填、search 结果与基线一致
    let hits = col.search(&query).unwrap();
    assert_eq!(hits.len(), expected_len);
    for h in &hits { assert!(known_ids.contains(&h.id)); }
}

#[test]
fn migrates_v0_1_0_via_reindex() {
    // 若未来格式升级：旧格式 → 新格式迁移器
    // 当前 v1 不需迁移（双模读取），此测试占位
}

#[test]
fn v1_and_v2_segments_coexist() {
    // 同一 DB 内混合 v1（旧段）+ v2（新 flush 段），search 一致
}
```

#### 文件位置

- `crates/vane-core/tests/fixtures/compat/v0.1.0/`（提交二进制 fixture）。
- `crates/vane-core/tests/cross_version_compat.rs`（新集成测试）。
- `crates/vane-core/tests/fixtures/compat/README.md`（fixture 来源说明）。
- fixture 生成 script：`scripts/gen_compat_fixture.rs`（离线运行，不进 CI）。

#### CI

- 新增 `cross-version-compat` job（needs test，`cargo test --test cross_version_compat`）。或合并到现有 `corpus-compat` job。

#### 取舍 / 风险

- **fixture 维护成本**：格式变更需重新生成 + 提交。**推荐**：fixture 只覆盖 v0.1.0（首个发布），后续版本变更用迁移器 + 双模读取，fixture 不必每个版本都建。
- **二进制 fixture 的平台无关性**：fixture 是段文件（LE 字节序，跨平台）。**无风险**。
- ~~**v0.1.0 tag 是否真实存在**：需确认 git tag。若无 v0.1.0 tag，用当前 HEAD 生成"伪 v0.1.0" fixture（标注生成 commit hash）。**未决问题**。~~ → **编排者已自查：v0.1.0 tag 存在，此风险消除。**

---

### 3.5 tracing feature 骨架

#### 方案

**`tracing` crate 是否触黑名单的判断**：

黑名单：regex / tokio / prost / tonic / openssl / lindera / ndarray / wee_alloc / dashmap / parking_lot。

`tracing` crate 0.1.x 的传递依赖：
- `tracing-core`（tracing 核心，无外部依赖）
- `thread_local`（无黑名单依赖）
- `cfg-if`（无黑名单依赖）
- 可选 `tracing-attributes`（仅 `#[instrument]` 宏，dev/feature 门控）
- **不依赖 regex / tokio / prost / tonic / openssl / lindera / ndarray / wee_alloc / dashmap / parking_lot**。

**结论**：`tracing` 不触黑名单。**可引入**。

**但 wasm 体积约束**：tracing 0.1.x + tracing-core 编译到 wasm 约 +30-50KB gzip（估算）。启用时不影响（feature 门控）；不启用时编译期消除，体积不变。**关键**：确保所有 tracing 调用经 `cfg(feature="tracing")` 门控，不启用时无任何 tracing 符号进 wasm。

#### Cargo.toml 定义

```toml
# crates/vane-core/Cargo.toml
[dependencies]
# tracing：可选，feature="tracing" 启用。dev/optional，默认不启用。
# 不触黑名单（传递依赖无 regex/tokio/prost/tonic/openssl/lindera/ndarray/wee_alloc/dashmap/parking_lot）。
# wasm32 不启用（体积红线；feature 默认 off，编译期消除）。
tracing = { version = "0.1", optional = true }

[features]
# tracing：埋点能力开关（I-5）。启用时检索延迟/段数/merge 频率/缓存命中率指标可观测；
# 不启用时编译期消除，wasm/native 体积不变（800KB gzip 红线）。
tracing = ["dep:tracing"]
```

#### 埋点位置

| 指标 | 埋点位置 | span/事件 |
|---|---|---|
| 检索延迟 p50/p99 | `api/collection.rs::search` 入口/出口 | `tracing::span!(Level::INFO, "search", top_k, mode)` span + elapsed |
| 段数 | `flush` / `merge_segments` 后 | `tracing::info!(segment_count = n, "flush done")` |
| 索引大小 | `flush` 后（段文件大小） | `tracing::info!(segment_ulid, bytes = n, "segment persisted")` |
| merge 频率 | `merge_segments` 入口 | `tracing::info!(sources = ?, target = ?, "merge start")` |
| 缓存命中率 | `PageCache::read` 命中/未命中 | `tracing::debug!(hit = bool, "page_cache")` |
| WAL append 次数 | `Wal::append` | `tracing::debug!(record = ?, "wal append")` |
| 词典状态 | `set_user_dict` / `reindex` | `tracing::info!(state = ?, "dict state transition")` |

**埋点机制**：用 `tracing::span!` / `tracing::info!` / `tracing::debug!` 宏，全部经 `#[cfg(feature="tracing")]` 门控。**推荐**：定义内部宏避免散落 cfg：

```rust
// crates/vane-core/src/telemetry.rs（新增）
#[cfg(feature="tracing")]
macro_rules! trace_span {
    ($($args:tt)*) => { tracing::span!(Level::INFO, $($args)*) };
}
#[cfg(not(feature="tracing"))]
macro_rules! trace_span { ($($args:tt)*) => { }; }  // 编译期消除
// 类似 trace_info! / trace_debug!
```

或更简：直接在各模块用 `#[cfg(feature="tracing")] tracing::info!(...)`。**推荐后者**，少一层抽象。

#### 不启用时 wasm 体积不变的验证方法

- `wasm32-size` job 已有 `check-wasm-size.sh`，测量 vane-wasm default + vane-core --export-all。
- **新增**：`check-wasm-size.sh` 在 tracing feature off（默认）时测体积；**额外加** tracing feature on 时测体积（验证启用后增量 <50KB gzip）。或单独 job `wasm32-size-tracing-on` 对比。
- **编译期消除验证**：`cargo build --target wasm32-unknown-unknown -p vane-core` 默认（tracing off），用 `wasm-strip` + `wasm-opt` 后 grep tracing 符号（应无）。**推荐**：`scripts/check-wasm-size.sh` 加一行 `wasm-objdump -x vane_core.wasm | grep -c tracing` 断言 = 0（tracing off 时无符号）。

#### 取舍

- **tracing vs 自研轻量 span**：tracing 不触黑名单，生态成熟，订阅者可接 opentelemetry / fmt。自研轻量 span 工作量大且不必要。**推荐 tracing**。
- **默认不启用**：避免 wasm 体积 + native 零开销。feature on 时用户主动 opt-in。
- **tracing-subscriber 不进 core**：subscriber 是消费侧（应用层），core 只 emit。vane-ffi/vane-node 可按需加 tracing-subscriber dev-dep。

#### 风险

- **tracing 传递依赖版本漂移**：未来 tracing 0.2 可能引黑名单依赖。cargo-deny 守护。
- **wasm 体积增量超预期**：启用时 +50KB 可能超 800KB 红线（vane-wasm default 当前接近红线）。**缓解**：tracing feature 默认 off，vane-wasm 不启用 tracing；启用 tracing 仅在 vane-ffi/vane-node native。
- **埋点散落代码污染**：`#[cfg(feature="tracing")]` 散落各模块，可读性下降。**推荐**：集中在 `search` / `flush` / `merge` 关键路径，不过度埋点。

---

### 3.6 inspect API

#### 方案

**纯新增 pub API，不改 M0-M3 冻结签名**。`Db::stats()` / `Db::segment_info()` 返回新结构体。

#### 关键 API / struct 签名

```rust
// crates/vane-core/src/api/inspect.rs（新增模块）

use std::sync::Arc;
use crate::api::db::Db;

/// DB 级统计信息（SPEC §9 新增 inspect API）。
#[derive(Debug, Clone)]
pub struct DbStats {
    pub db_path: String,
    pub collections: Vec<CollectionStats>,
    /// 词典状态（jieba 是否加载）。
    pub dict_available: bool,  // jieba feature on 时有意义；off 时恒 false
    pub executor_kind: ExecutorKind,  // Serial / Rayon
}

#[derive(Debug, Clone)]
pub struct CollectionStats {
    pub name: String,
    pub segment_count: usize,
    pub total_docs: u64,           // 各段 doc_count 之和（含 tombstoned）
    pub live_docs: u64,           // total_docs - tombstoned
    pub tombstoned_docs: u64,
    pub index_bytes: u64,         // 各段文件大小之和（header+vectors+stored+idmap+scalars+inverted+hnsw）
    pub dict_state: crate::api::types::DictState,  // Stable / PendingReindex / Rebuilding
    pub tokenizer_id: crate::types::TokenizerId,
    pub health: Health,
}

#[derive(Debug, Clone)]
pub struct SegmentInfo {
    pub ulid: String,
    pub doc_count: u32,
    pub docid_base: u64,
    pub tombstoned_count: u64,
    pub format_versions: FormatVersions,
    pub file_sizes: SegmentFileSizes,
    pub health: Health,
}

#[derive(Debug, Clone)]
pub struct FormatVersions {
    pub header: u32,     // HEADER_FORMAT_V1
    pub vectors: u32,    // VECTORS_FORMAT_V1 or V2
    pub stored: u32,     // STORED_FORMAT_V1 or V2
    pub idmap: u32,      // IDMAP_FORMAT_V1
    pub scalars: u32,    // SCALARS_FORMAT_V1
    pub inverted: u32,  // FORMAT_VERSION
    pub hnsw: u32,      // HNSW_FORMAT_V1（若有 hnsw.bin）
}

#[derive(Debug, Clone)]
pub struct SegmentFileSizes {
    pub header: u64,
    pub vectors: u64,
    pub stored: u64,
    pub idmap: u64,
    pub scalars: u64,
    pub inverted: u64,
    pub hnsw: Option<u64>,  // None = 无 hnsw.bin（fallback brute）
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Healthy,
    Degraded,  // 词典降级 / hnsw 缺失 fallback brute / 段文件部分缺失但可读
    Corrupt,   // 段文件损坏（magic/version/CRC 校验失败）
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutorKind { Serial, Rayon }

impl Db {
    /// SPEC §9 新增 inspect API：DB 级统计。
    /// 纯新增，不改 M0-M3 冻结 pub API。&self 返回 DbStats。
    pub fn stats(&self) -> DbStats { /* 遍历 collections 构造 */ }

    /// SPEC §9 新增 inspect API：各段详细信息。
    /// 返回所有 collection 的所有段信息。
    pub fn segment_info(&self) -> Vec<SegmentInfo> { /* 遍历 snapshot readers */ }

    /// 单个 collection 的段信息（便捷重载）。
    pub fn collection_segment_info(&self, name: &str) -> Option<Vec<SegmentInfo>> { /* ... */ }
}
```

#### 健康检查实现依据（读哪些内部状态）

| 健康标志 | 判定来源 | 实现位置 |
|---|---|---|
| **词典是否降级** | `DbInner.jieba_dict` 是否 None（jieba feature on 时）；collection 的 tokenizer 是 Jieba 但 dict None → Degraded | `Db::stats` 读 `inner.jieba_dict.read()` |
| **段是否损坏标记** | `SegmentReader::open` 是否成功（header magic/version 校验、vectors/stored/idmap 解码） | `segment_info` 尝试 `SegmentReader::open`，Err → Corrupt |
| **hnsw 缺失 fallback brute** | `CollectionInner.hnsw_readers` 中 None | `stats` 读 `inner.hnsw_readers.read()` |
| **dict_state** | `CollectionInner.dict_state` | `stats` 读 `inner.dict_state.read()` |
| **index_bytes** | 遍历段目录 `Vfs::list` + 文件大小（Vfs 无 stat，需 read_at 探测长度 或 段文件格式已知 offset 推算） | `segment_info` 读段目录下各文件，用 `Vfs::read_at` 探测 EOF（read_at 返 0 即 EOF，二分或 read 大 buffer 直至 0 推算 size）—— 或在 header.bin 存 size 字段（需格式变更，不推荐）；**推荐**：Vfs 加 `fn size(&self, path: &str) -> Result<u64>`（trait 扩展，但 M0 冻结 trait 签名 —— 不改 trait，用 read_at 探测或段文件 format 内已知字段推算） |

**Vfs trait 不改问题**：`Vfs` trait M0 冻结，无 `size` / `stat` 方法。`index_bytes` / `file_sizes` 需另辟：
- **方案 A**：`read_at` 探测 EOF —— 从 offset=0 读大 buffer（8KB），n=0 即 EOF；否则 offset += n 继续。性能差但可接受（inspect 非热路径）。
- **方案 B**：段文件格式已知字段推算 —— header.bin 的 `doc_count` / `docid_base` / `tombstone_bytes` 可推算 header/vectors/idmap 大小；stored/inverted 需读内容。复杂。
- **方案 C**：`Vfs` trait 加 `fn size(&self, path: &str) -> Result<u64>`（默认实现读空），各 impl 覆写。**但 trait 方法加默认实现不算破坏 trait 对象**（已有 impl 自动获得默认实现，不需改 MemoryVfs/StdFsVfs）。**但 M0 冻结 trait 签名 —— 加方法即使有默认实现也是 trait 签名变更**。**不推荐 C**。
- **推荐 A**：`read_at` 探测 EOF，inspect 非热路径，性能可接受。或 `segment_info` 不返回精确 bytes，返回 `index_bytes: Option<u64>`（MemoryVfs 可精确，StdFsVfs 用 read_at 探测）。

#### 文件位置

- `crates/vane-core/src/api/inspect.rs`（新模块）。
- `crates/vane-core/src/api/mod.rs` 加 `pub mod inspect; pub use inspect::*;`。
- `crates/vane-core/src/api/db.rs` impl Db 新增 `stats()` / `segment_info()` 方法（纯新增，不改现有方法）。

#### 取舍

- **返回结构体 vs JSON**：返回强类型结构体（FFI 层序列化为 JSON）。**推荐结构体**，FFI 层薄壳序列化。
- **segment_info 遍历开销**：非热路径，可接受 O(segments × files)。
- **Health 标记的实时性**：`stats()` 调用时即时读内部状态，非缓存。若段在读后损坏，下次 stats 才反映。**推荐**：Health 仅反映打开时的状态（SegmentReader::open 成功 = Healthy），不主动重新 open 校验（性能）。

#### 风险

- **Vfs 无 size 方法**：用 read_at 探测 EOF，性能差但可接受。或返回 Option<u64>。
- **Health Corrupt 判定边界**：SegmentReader::open 失败可能因真损坏或 IO 临时错误。inspect 非诊断工具，仅标记。**推荐**：open 失败即 Corrupt，用户用 VaneError 详情排查。
- **不改冻结 pub API 的边界**：新增方法不算改冻结签名，但若新增方法与未来 M5 冲突需谨慎命名。**推荐**：`stats` / `segment_info` 命名通用，不易冲突。

---

## 4. 阶段依赖与实施顺序建议

```
阶段零（本设计）—— 只读设计，无实现
    ↓
阶段一（fuzz）—— 依赖：cargo-fuzz 集成（3.2）、proptest（3.3）可并行
    ├── fuzz targets（3.2）
    └── proptest invariants（3.3）
    [可并行；fuzz 需 vane-fuzz crate 先建]
    ↓
阶段二（崩溃恢复）—— 依赖：FaultVfs（3.1）必须先完成
    └── crash_recovery.rs（FaultVfs 注入 5 场景）
    [FaultVfs 是前置；阶段二不能与阶段一并行（阶段一 fuzz 可能发现 bug 需阶段二验证）]
    ↓
阶段三（跨版本兼容）—— 独立，可与阶段一/二并行
    └── cross_version_compat.rs + fixtures（3.4）
    ↓
阶段四（并发压测）—— 依赖：真实并发（非 FaultVfs），可独立
    └── stress tests（多线程 search+insert+flush+merge）
    [不依赖 FaultVfs；依赖真实线程模型，rayon Executor]
    ↓
阶段五（可观测性）—— 独立于测试基础设施
    ├── tracing feature（3.5）
    └── inspect API（3.6）
    [可并行；tracing 与 inspect 无依赖关系]
    ↓
阶段六（CI 集成 + SPEC 修订 + 总结）
```

**可并行**：阶段一(fuzz) ∥ 阶段三(兼容) ∥ 阶段五(tracing/inspect) ∥ 阶段四(压测)。
**串行**：阶段二(崩溃恢复) 依赖 FaultVfs（3.1）实现完成；FaultVfs 实现可视为阶段零交付后的独立 task。
**推荐实施顺序**：FaultVfs（3.1）→ 阶段二（崩溃恢复）→ 阶段一（fuzz/proptest）→ 阶段三（兼容）→ 阶段五（tracing/inspect）→ 阶段四（压测）→ 阶段六（CI+SPEC）。

---

## 5. SPEC 影响清单（v1.4 → v1.5，用户批准）

### §9 API（inspect 加什么）

在 §9.2 函数面**补列** inspect API 的 FFI 落地（core 层是 `Db::stats()` / `Db::segment_info()`，FFI 层加）：
```
vane_db_stats(db_h, out_arena*) -> i32              // DbStats JSON
vane_db_segment_info(db_h, out_arena*) -> i32       // Vec<SegmentInfo> JSON
```
返回结构体字段见 §3.6。core 层 inspect API 不改 M0-M3 冻结签名，纯新增。

### §10 错误码（诊断加什么）

VaneError 诊断上下文增强（不改错误码表，改 error payload）：
- 现状：`VaneError::Io(String)` / `Corrupt(String)` 等，String 是诊断信息。
- M4 增强：String 内附上下文（哪段 ULID / 哪文档 docid / 哪操作 / 建议操作）。
- **不改错误码**（-1..-11 不变），仅丰富 String。SPEC §10 表后补注释：「错误 String 含上下文：段 ULID / docid / 操作 / 建议」。
- 可选：`VaneError::context()` 方法返回结构化上下文（非 String）。**推荐**：先丰富 String，结构化上下文列为 Could。

### §13.2 门禁（加哪些 fuzz / 崩溃恢复 / 兼容 / 压测 DoD）

在 §13.2 质量门禁**新增** 6-9 项：
```
6. fuzz-smoke（每 target 60s，push/PR）无 panic/crash。
7. fuzz-long（cron/workflow_dispatch，10min/target）crash 上传 artifact。
8. 崩溃恢复（FaultVfs 注入 5 场景：meta_slot/WAL/merge/ENOSPC/部分写）全通过，数据一致。
9. 跨版本兼容（v0.1.0 fixture 当前版本读取）通过。
10. 并发压测（N 线程 search+insert+flush+merge，timeout 内无 panic/死锁/数据不一致）通过。
11. proptest 不变量（检索稳定/round-trip/merge 不丢）256 cases 全通过。
```

### §14 I-5 tracing feature（怎么写）

在 §14 I-5 注释**扩展**：
```
- `cfg(feature="tracing")` 是可观测性能力开关（类似 zstd-encode），允许出现在
  api/segment/persistence/wal 模块的埋点位置（span/info/debug 宏调用）。
  不启用时编译期消除，wasm/native 体积不变（800KB gzip 红线守护）。
  tracing crate 不触依赖黑名单（传递依赖无 regex/tokio/prost/tonic/openssl/
  lindera/ndarray/wee_alloc/dashmap/parking_lot），cargo-deny 守护。
```
**注意**：I-5 不变量核心（核心零平台分支）不变 —— tracing 是 feature 能力开关，非平台分支。

---

## 6. 风险与未决问题

### 风险

1. **FaultVfs path pattern 自研 matcher**：不引 regex（黑名单），需轻量 glob/前缀匹配。实现简单（`*` 通配 + 前缀），但需测试覆盖。
2. **cargo-fuzz nightly 依赖**：CI 需 nightly toolchain，可能 break。pin 特定版本。
3. **proptest 失败 seed 持久化**：`proptest-regressions/` 需提交，否则 CI 不可复现。但二进制文件提交可能争议。
4. **v0.1.0 fixture 真实性**：~~若 v0.1.0 tag 不存在或格式与当前差异大，fixture 需标注生成 commit。~~ → **编排者自查：v0.1.0 tag 存在，fixture 用真实 tag 生成，此风险消除。**
5. **tracing wasm 体积增量**：启用时 +30-50KB gzip 可能超红线（若 vane-wasm 当前接近 800KB）。**缓解**：tracing feature 默认 off，vane-wasm 不启用。
6. **inspect API 的 Vfs 无 size 方法**：read_at 探测 EOF 性能差，或返回 Option<u64>。
7. **libfuzzer-sys 传递依赖触黑名单**：预判不触，需 cargo-deny 验证。
8. **default-members vs --workspace**：vane-fuzz 加入 workspace 后，`cargo test --workspace --all-features` 会尝试编译 vane-fuzz（需 nightly + libfuzzer）。CI test job 需 `--exclude vane-fuzz`。

### 未决问题（需用户拍板，编排者据以下 AskUserQuestion）

1. ~~**v0.1.0 git tag 是否存在？**~~ → **编排者已自查：存在（v0.1.0/v0.1.1/v0.1.2/v0.2.0 四 tag 在）。fixture 用 v0.1.0 tag 生成。此问题已解决。**
2. **tracing feature 默认 off 是否接受？** 还是要求 vane-wasm 也支持 tracing（需体积预算重估）？
3. **inspect API 的 `index_bytes` 用 read_at 探测 EOF（性能差）还是返回 Option<u64>（部分后端不可用）？**
4. **FaultVfs 是 `cfg(test)` 纯内联还是 dev-feature `fault-injection`？** 若 vane-ffi 集成测试需 FaultVfs，需 dev-feature；若仅 vane-core 内联测试，`cfg(test)` 够。
5. **proptest-regressions/ 目录是否提交？**（CI 复现 vs 仓库整洁）
6. **vane-fuzz 加入 workspace 但 default-members 排除，CI test job 改为 `--exclude vane-fuzz` 是否接受？**（影响现有 16 jobs 的 test job 命令）
7. **SPEC v1.5 修订的 5 处（§9/§10/§13.2/§14）是否一次性批准，还是分批 AskUserQuestion？**
8. **LostWrite 故障类型是否实现？**（MemoryVfs 难以真模拟丢写；StdFsVfs 已 fsync 难以模拟；列为 Could 非 Must 是否接受？）
9. **并发压测是否用 loom？**（loom 与 rayon 兼容性、CI 时间）还是纯压力测试？

### SPEC 矛盾（M4 需求与 SPEC v1.4 现状冲突处）

1. **§13.3 依赖黑名单列表 vs M4 需 tracing/fuzz/proptest**：SPEC §13.3 列黑名单为 regex/tokio/prost/tonic/openssl/lindera/ndarray/wee_alloc（v1.4 未列 dashmap/parking_lot，但 deny.toml 已加）。tracing/proptest/libfuzzer-sys 不在黑名单，但 **SPEC §13.3 文本需明确补注**「dev/optional 依赖（tracing/proptest/cargo-fuzz）不触运行时黑名单，cargo-deny 守护」。**非矛盾，是 SPEC 文本需补全**。
2. **§9.2 FFI 函数面未列 inspect API**：v1.4 §9.2 函数面是 M0-M3 冻结，新增 `vane_db_stats` / `vane_db_segment_info` 需 SPEC v1.5 补列。**非矛盾，是 SPEC 演进**。
3. **§14 I-5 释义 vs tracing feature**：v1.4 I-5 释义已含 `cfg(feature)` 能力开关，tracing feature 符合释义。**无矛盾**，仅需 SPEC v1.5 补 tracing feature 的明确列举。
4. **§13.2 门禁列表 vs M4 新增门禁**：v1.4 §13.2 有 5 项，M4 需新增 6 项（fuzz/崩溃/兼容/压测/proptest）。**非矛盾，是 SPEC 演进**。

---

## 给编排者的执行摘要

**状态**：DONE（完整设计已产出，覆盖六项 + 阶段依赖 + SPEC 影响 + 风险未决）。

**关键决策（5 条）**：

1. **FaultVfs 用 path + op + 调用计数器三层匹配**（层 1+2），不实现层 3 hook（避免污染生产代码）。MemoryVfs 为主力，StdFsVfs + tempdir 用于 conformance 对齐。`cfg(test)` 为主 + 可选 dev-feature `fault-injection` 供 vane-ffi 集成测试。

2. **cargo-fuzz 用独立 crate `crates/vane-fuzz`**（不污染 vane-core），workspace default-members 排除，CI test job 加 `--exclude vane-fuzz`。fuzz-smoke（60s/target，push/PR）+ fuzz-long（cron/workflow_dispatch，10min/target）。nightly toolchain pin 版本。

3. **proptest 作为 vane-core dev-dep**，3 个不变量（检索稳定 / round-trip / merge 不丢），`proptest-regressions/` 提交确保 CI 复现。不触黑名单（待 cargo-deny 验证）。

4. **跨版本 fixture 提交仓库 `tests/fixtures/compat/v0.1.0/`**（非 CI 现场生成），小段数据 <100KB，离线 script 用 v0.1.0 tag 生成。覆盖 v1/v2 双模读取。

5. **tracing feature 默认 off**，`tracing` crate 不触黑名单（传递依赖验证）。埋点经 `#[cfg(feature="tracing")]` 门控，编译期消除。inspect API 纯新增 `Db::stats()` / `Db::segment_info()`，不改冻结签名。

**未决问题清单（需用户拍板）**：见 §6 未决问题 2-9（#1 已由编排者自查 git tag 解决）。

**SPEC 矛盾**：无硬矛盾。4 处为 SPEC 演进需求（§9.2 补 inspect FFI、§10 补诊断上下文注释、§13.2 补 6 项门禁、§14 补 tracing feature 列举），均属 v1.4→v1.5 修订范围，需用户批准。

### Critical Files for Implementation
- /Users/ximing/project/mygithub/vane/crates/vane-core/src/vfs/mod.rs
- /Users/ximing/project/mygithub/vane/crates/vane-core/src/persistence/mod.rs
- /Users/ximing/project/mygithub/vane/crates/vane-core/src/wal/mod.rs
- /Users/ximing/project/mygithub/vane/crates/vane-core/src/api/collection.rs
- /Users/ximing/project/mygithub/vane/crates/vane-core/src/api/db.rs
