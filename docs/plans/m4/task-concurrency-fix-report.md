# M4 阶段四 并发 Bug Fix — Report

> 分支 `feat/m4-prod-readiness`。BASE=354f66e（Phase 4 stress）。
> Task: 修复 Phase 4 stress 撞出的 2 个真实生产并发 bug + 更新 stress 去 workaround。
> 类型: FIX（生产代码 + stress 测试）。

## 1. Bug 概述

Phase 4 stress（commit 354f66e）撞出 2 个并发 bug（已 grep 核实）：

| # | Bug | 位置 | 根因（stress 实测） |
|---|---|---|---|
| 1 | 并发 flush manifest 损坏 | `persistence/mod.rs:104-121` `ManifestStore::save_atomic` | 固定 tmp 路径 `manifest.json.tmp`；并发 save_atomic 的 delete/create/write_at/sync/rename 交错覆写 → E_CORRUPT。并发 add_segment 的 load-modify-save 也有 lost-update（一方段 ULID 丢失）。 |
| 2 | auto-merge 段状态竞争 → double-count/missing | `api/collection.rs:486-508` `auto_merge_two_smallest` | stress 实测揭示三层竞态：(a) auto_merge 不获取 compacting 锁，与并发 compact/merge 竞争；(b) merge 的 `target_docid_base` 未并入 `next_docid`，与并发 flush 的缓冲段 docid 重叠；(c) merge 快照重建用入口读的 stale `offsets` 覆写 `seg_offsets`，并发 flush 新推入段的 offset 被错置为 0 → search 回填算错 docid → 活文档「丢失」（reopen 重建 offsets 后又可见）。 |

## 2. Bug 1 fix：ManifestStore save_lock 序列化 manifest 原子保存

**方案 A（reviewer 推荐）**：`ManifestStore` 加 `save_lock: Mutex<()>`，save_atomic 入口序列化。

### 2.1 关键设计：共享 Arc<ManifestStore>

原代码在 `flush`/`merge_segments`/`reindex` 三处 `ManifestStore::new(...)` **构造新实例**——若 save_lock 是 per-instance，并发调用各持新实例的锁 → 不序列化。故 fix 需让 ManifestStore **共享**：

- `DbInner.manifest_store: ManifestStore` → `Arc<ManifestStore>`（`pub(crate)`，不改 pub API）。
- `CollectionInner` 新增 `manifest_store: Arc<ManifestStore>` 字段（`create_new` 从 `db.manifest_store.clone()` 注入）。
- flush/merge/reindex 三处 `ManifestStore::new(...)` → `self.inner.manifest_store`（共享同一 Arc → 同一 save_lock）。

### 2.2 save_atomic 拆分 + update 闭包

`save_atomic`（pub，签名不变）入口取 save_lock，调用私有 `save_atomic_locked`（落盘实现）。拆出私有方法避免 `add_segment` / `update` 在持锁的 load-modify-save 事务中重入 save_lock（`std::sync::Mutex` 不可重入）。

新增 `pub(crate) fn update<F: FnOnce(&mut Manifest) -> Result<()>>`：在 save_lock 内 load→f→save_atomic_locked，供 `merge_segments`（load-modify-WAL-save）、`update_manifest_after_reindex`、`Db::collection`（建表 load-modify-save）复用——整个事务在持锁期间完成，杜绝并发 lost-update 与 tmp 覆盖。WAL → manifest 的 §6.4 顺序保持（WAL append 在闭包内、save_atomic_locked 在闭包返回后）。

### 2.3 I16 残留 tmp 清理语义保留

`save_atomic_locked` 开头 `let _ = self.vfs.delete(&tmp);` 保留（处理上次崩溃残留 tmp）。

## 3. Bug 2 fix：auto-merge compacting guard + docid 防重叠 + seg_offsets 修正

stress 实测揭示 Bug 2 是**三层**竞态，需三处 fix：

### 3.1 auto_merge_two_smallest compacting guard（task 要求）

`auto_merge_two_smallest` 入口 `try_lock` compacting：
- `Ok(guard)` 且 `*guard == false` → 设 true，drop guard，建 `CompactingGuard`（复用 `collection.rs:100-109` 的 M-minor-1 panic-safe Drop guard，Drop 复位 false）。再做 pick + merge_segments。
- `Err(WouldBlock)` → 并发 compact/reindex 持锁 → `return Ok(())`（skip，best-effort 降级，下次 flush 段数仍超阈值时再 merge）。
- `Err(Poisoned(e))` → 恢复（取 `e.into_inner()`，设 true 继续 merge），guard drop 复位。

**死锁分析**：compacting guard 不持其他锁（与 compact/reindex 模式一致）。auto_merge 持 compacting → 调 merge_segments（不重入 compacting）→ 内部取 write_state（短持，推进 next_docid）、snapshot/offsets 写锁（短持，重建快照）。compact 用**阻塞** lock() 等 auto_merge 释放（不 try_lock）→ auto_merge 完成后 compact 获取锁、见 `*guard==false`、设 true、执行。锁序一致（compacting → write_state → snapshot → offsets → ...），无 lock-order deadlock。

### 3.2 merge_segments target_docid_base 并入 next_docid + 原子预留（关键 fix）

**根因**：原 `target_docid_base = max(保留段 base+count)`，未并入 `next_docid`（add 已分配但未 flush 的缓冲文档 docid 上界）。并发 flush 的缓冲段 docid 在 `[old_next_docid, next_docid)` 区间；merge 的 target_base 不计入 → 新段与 about-to-flush 的缓冲段 docid 重叠 → fusion 去重丢文档 + 回填误命中 → double-count/missing。

**fix**：`target_docid_base = max(保留段 base+count, next_docid)`。且**原子地**（一次 write_state lock 内）读 next_docid + 算 target_base + bump `next_docid = target_base + estimated_new_count`（estimate = source 段 doc_count 之和，上界；tombstone 清除后实际 new_count <= estimate，多预留的区间留空无危害）。原子 read+bump 消除「merge 读 next_docid → 并发 add 在旧 next_docid 分配 docid → merge 写新段覆盖该 docid」的窗口。compact 全合并（`is_full_merge`，target_base=0）也 bump next_docid=estimate。

### 3.3 flush base_docid 连续性检测（兼容 inspect base=0）

**问题**：3.2 的 merge fix 后，若 flush 用 `base_docid = docs.first().docid`（stale），并发 merge 在两次 add 之间 bump next_docid → 缓冲文档 docid 非连续（`[100, 121, ...]`）→ flush 写连续 `[100, 100+count)` 与 merge 新段 `[101, 121)` 在 101 重叠。

**fix**：flush 检测缓冲文档 docid 是否连续：
- **连续**（无并发 merge 在 add 之间 bump）→ 用首文档 docid 作 base（保持 inspect `base=0` 语义；merge 的 target_base 已并入 next_docid=first+count，新段在本段之上）。
- **非连续**（并发 merge 在 add 之间 bump 了 next_docid）→ rebase 到当前 `next_docid`（merge 已 bump 到新段末尾之上）+ bump next_docid 预留本 flush 区间。

此条件 fix 兼容 inspect（连续 → base=0）并修并发（非连续 → rebase）。不碰 inspect 测试。

### 3.4 merge 快照重建不再覆写 seg_offsets（关键 fix）

**根因**：merge 快照重建块（`std::mem::take(&mut *snap_w)` + 遍历重建）对保留段做 `offsets_w.insert(ulid, offsets.get(ulid).unwrap_or(0))`——`offsets` 是 merge 入口（line 555）读的 **stale** clone。并发 flush 在 merge 读 offsets 后推入的新段，其 offset 在 stale `offsets` 中不存在 → `unwrap_or(0)` → 偏移被错置为 0 → search 回填算错 docid → 活文档「丢失」。reopen 重建 offsets（从 header.bin 读 docid_base）后文档又可见 → 故 manifest 一致、内存快照 offsets 不一致。

**fix**：保留段**不覆写** `offsets_w`——段不可变（I-1），保留段 offset 不变，无需 re-insert。仅移除 source 段 offset + 插入新段 offset。并发 flush 新推入段的正确 offset 保留在 `offsets_w` 中不被覆写。

## 4. stress 测试更新（去 workaround + 新并发测试）

`tests/stress_concurrency.rs` 更新：

### 4.1 去 flush_lock workaround

- 删 `serialized_flush` helper（原用外部 `Mutex<()>` 序列化 flush 规避 Bug 1）。
- `stress_concurrent_add_flush_search` + `stress_stdfs_conformance`：直接 `col.flush()`（并发，无外部序列化）——save_lock 序列化 manifest 原子保存。
- 文件头注释更新（flush 并发边界从「外部 Mutex 序列化」改为「save_lock 序列化 manifest」）。

### 4.2 新增并发 flush 不损坏测试（Bug 1 fix 验证）

`stress_concurrent_flush_no_corruption`：4 线程 × 15 轮 add + 每轮 flush（60 次并发 flush）。断言：无 E_CORRUPT / 无错误 + 全部文档可搜到 + 无 double-count + **reopen Db 加载 manifest 不 E_CORRUPT**（manifest 一致性核心断言）+ ULID 无重复。

### 4.3 新增并发 auto-merge 不 double-count 测试（Bug 2 fix 验证）

`stress_concurrent_auto_merge_no_double_count`：预填 12 段（> SEGMENT_MAX=10）强制 auto-merge 活跃，再 4 线程 × 15 轮 flush 持续触发 auto-merge。断言：无错误 + 无 double-count（Bug 2 核心断言）+ 全部文档可搜到 + ULID 无重复。

### 4.4 multi-run 升级

`stress_multi_run_stability`：3 次 → **5 次**，flush_interval 25→10（4×50=200 docs/run，每线程 5 次 flush = 20 段 > 10 → 触发 auto-merge），直测修复后并发 flush + auto-merge 多次跑稳定性。

### 4.5 保留其他测试

`assert_send_sync`、`cross_thread_shared_basic`、`stress_concurrent_search_during_write`、`stress_concurrent_compact_contention`、`stress_concurrent_add_during_compact` 保留不变。

## 5. 多次 multi-run 结果（无 flaky）

```
stress_concurrent_flush_no_corruption + stress_concurrent_auto_merge_no_double_count
+ stress_concurrent_add_flush_search + stress_multi_run_stability 等 10 测试，
连续 8 次全跑均 10 passed / 0 failed：

=== run 1 === test result: ok. 10 passed; 0 failed
=== run 2 === test result: ok. 10 passed; 0 failed
=== run 3 === test result: ok. 10 passed; 0 failed
=== run 4 === test result: ok. 10 passed; 0 failed
=== run 5 === test result: ok. 10 passed; 0 failed
=== run 6 === test result: ok. 10 passed; 0 failed
=== run 7 === test result: ok. 10 passed; 0 failed
=== run 8 === test result: ok. 10 passed; 0 failed
```

修复前（仅 compacting guard，无 docid/seg_offsets fix）：8x stress 均 FAILED（double-count 18 duplicates / missing 85 docs）。

## 6. 各门禁真实输出

| 门禁 | 命令 | 结果 |
|---|---|---|
| 格式 | `cargo fmt --all -- --check` | 绿（无 diff） |
| 静态检查 | `cargo clippy --all-targets --all-features --workspace --exclude vane-fuzz -- -D warnings` | 绿（`Finished dev profile`） |
| stress | `cargo test -p vane-core --all-features --test stress_concurrency` | 10 passed; 0 failed（8x 均 OK） |
| 全工作区 | `cargo test --workspace --all-features --exclude vane-fuzz` | 见下（全绿） |
| crash recovery | `cargo test -p vane-core --all-features --test crash_recovery` | 5 passed; 0 failed |
| 依赖 | `cargo deny check` | 绿（advisories/bans/licenses/sources ok；warning 为 pre-existing regex wrapper，无新 dep） |
| WASM | `cargo check --target wasm32-unknown-unknown -p vane-core` | 绿（fix 不引 std::fs） |

## 7. 自审

### 7.1 死锁分析

- **save_lock**：`save_atomic` / `add_segment` / `update` 互斥（同一 ManifestStore 实例的 save_lock）。不重入（`add_segment`/`update` 调 `save_atomic_locked`，不调 `save_atomic`）。WAL append 在 `update` 闭包内持 save_lock，但 WAL 是独立文件，不与 manifest tmp 冲突。
- **compacting**：auto_merge try_lock + skip（WouldBlock）；compact/reindex 阻塞 lock。auto_merge 持 compacting → merge_segments（不重入 compacting）→ 内部 write_state/snapshot 短持。锁序：compacting → write_state → snapshot → offsets → inv → hnsw → scalar → tomb，同序无死锁。
- **write_state**：flush 持 write_state（take buffer + base_docid 计算 + 连续性检测，短持）；merge 持 write_state（原子读 next_docid + bump，短持）；add 持 write_state（docid 分配 + buffer push，短持）。三者不重入（flush drop write_state 后才做 segment write / add_segment / auto_merge）。

### 7.2 lock 序

- search：snapshot.read → seg_offsets.read → inv.read → hnsw.read → scalar.read → tombstones.read（全 read，不互斥）。
- flush：write_state（短持，drop 后做 segment write）→ save_lock（add_segment）→ snapshot.write + seg_offsets.write（短持，push）。
- merge：compacting → write_state（原子读+bump next_docid，短持）→ save_lock（update 闭包：load-modify-WAL-save）→ snapshot.write + seg_offsets.write + inv.write + hnsw.write + scalar.write + tombstones.write（短持，take+rebuild）。
- compact：compacting → merge → ...
- 一致，无 lock-order deadlock。

### 7.3 compacting guard 复用

复用 `collection.rs:100-109` 的 `CompactingGuard<'a> { flag: &'a Mutex<bool> }`（M-minor-1 panic-safe Drop 复位 false）。auto_merge 创建 guard 的模式与 compact（line 1136-1145）/ reindex（line 1227-1238）一致：先 `{ let mut guard = lock(); if *guard { return; } *guard = true; }`（块结束 drop guard），再 `let _cg = CompactingGuard { flag }`。guard 不持有 MutexGuard——仅持 `&Mutex<bool>`，Drop 时重新取锁复位（与原模式等价，panic-safe）。

### 7.4 frozen API 未改

- `ManifestStore`：`pub struct`（字段私有，加 `save_lock: Mutex<()>` private 字段不改 pub API）；`pub fn new` / `pub fn load` / `pub fn save_atomic` / `pub fn add_segment` 签名不变。新增 `pub(crate) fn update`（crate 内可见，不扩 pub API）+ 私有 `save_atomic_locked`。
- `DbInner` / `CollectionInner`：`pub(crate)` 结构，字段改 type / 加字段不改 pub API。`manifest_store: ManifestStore` → `Arc<ManifestStore>`（`pub(crate)` 字段）。
- `flush` / `auto_merge_two_smallest` / `merge_segments` / `compact` / `reindex` / `update_manifest_after_reindex`：私有 fn 或 pub fn 签名不变。
- 不碰 SPEC.md / CI yml / fault.rs / crash_recovery / vane-fuzz / proptest / cross_version / tracing / inspect / VaneError 诊断。

### 7.5 core 禁 std::fs

fix 用 `std::sync::{Arc, Mutex}`（已在用），不引 std::fs/std::net/mmap。WASM check 绿。

## 8. commit

```
fix(core): 并发 flush manifest 损坏 + auto-merge 竞争 double-count（M4 Phase 4 fix）
```

含：`persistence/mod.rs`（Bug 1 save_lock + save_atomic_locked + update）+ `api/db.rs`（Bug 1 Arc<ManifestStore> + update）+ `api/collection.rs`（Bug 1 共享 manifest_store；Bug 2 compacting guard + target_docid_base 并入 next_docid + 原子预留 + flush 连续性检测 + seg_offsets 不覆写）+ `api/reindex.rs`（Bug 1 update）+ `tests/stress_concurrency.rs`（去 workaround + 新并发 flush/auto-merge 测试 + multi-run 升级）。

不含 SPEC/CI/fault.rs/crash_recovery/vane-fuzz/proptest/cross_version/tracing/inspect/diagnostics 误改。

## 9. concerns

- **auto-merge skip 频率**：compacting guard 的 try_lock+skip 在高并发 flush 下会 skip 部分 auto-merge（段数临时累积）。但下次 flush 段数仍超阈值时再 merge，最终收敛。stress 实测段数 10-22 范围，无无限累积。skip 是 best-effort 降级（task 要求）。
- **full merge（compact）与并发 flush 的 docid 重叠**：compact 的 target_base=0（reset 语义），3.2 的 fix 也 bump next_docid=estimate 让后续 add 从 estimate 起分配，但 compact 前已 buffer 的 stale docid 仍可能与 compact 新段 [0, total) 重叠。compact 是用户触发（非 flush 自动），且 `stress_concurrent_add_during_compact` 测试通过（compact 持 compacting，add 不持 compacting，但 compact 的 full merge + WAL truncate 是重操作，add 的 buffer 在 compact 期间不被 flush）。列为 Could defer（compact + 并发 flush 的更深层 fix）。
