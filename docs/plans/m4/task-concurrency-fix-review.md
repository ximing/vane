# M4 阶段四 并发 Bug Fix — Review

> Reviewer: task-reviewer SubAgent（opus，只读，禁编辑源码）。
> 审查对象: commits 354f66e..cedbb17（6 files +682 -131）。
> 审查范围: 并发正确性（死锁 / lock 序 / race 真消除 / 残留 race 定性）。

## 0. 审查方法

1. Read implementer report（`task-concurrency-fix-report.md`）+ stress 原始 report（`task-stress-report.md`）+ review package diff（`task-concurrency-fix-review-package.md`）。
2. Read 实际源码核对：`persistence/mod.rs`（save_atomic/add_segment/update/save_atomic_locked）、`api/collection.rs`（flush/auto_merge_two_smallest/merge_segments/compact/reindex/CompactingGuard/restore_from_manifest）、`api/db.rs`（DbInner Arc<ManifestStore>/Db::open/Db::collection）、`api/reindex.rs`（update_manifest_after_reindex）、`tests/stress_concurrency.rs`（10 测试断言实质）、`tests/crash_recovery.rs`（断言匹配）、`wal/mod.rs`（WAL 线程安全性）、`vfs/memory.rs`+`vfs/std_fs.rs`（Vfs::append 原子性）。
3. 数学推导残留 race（concern #1）的 docid 重叠可能性。
4. 不重跑门禁（结果在 report）。

## 1. Spec 合规

**✅ Spec 合规——2 bug 真修了。**

Bug 1（save_atomic 并发 manifest 损坏）和 Bug 2（auto-merge 竞争 double-count/missing）的根因被真实消除（非 workaround）。frozen pub API 不变。crash_recovery 5 场景断言仍匹配。core 禁 std::fs 不破。

## 2. Bug 1 fix 定性：save_lock Arc 共享真生效 ✅

**save_lock 经 Arc<ManifestStore> 真共享——fix 生效。**

### 2.1 save_lock 入口获取 + Drop 释放

- `persistence/mod.rs:116-119` `save_atomic`：`let _save_guard = self.save_lock.lock().unwrap();` 入口获取，函数返回（含 `?` 提前返回）时 Drop 释放。✓
- `persistence/mod.rs:148-160` `add_segment`：`let _save_guard = self.save_lock.lock().unwrap();` 入口获取，load→modify→`save_atomic_locked`（不重入 save_lock）。✓
- `persistence/mod.rs:169-174` `update`：`let _save_guard = self.save_lock.lock().unwrap();` 入口获取，load→f→`save_atomic_locked`。✓

### 2.2 Arc<ManifestStore> 真共享

- `api/db.rs:44` `Db::open`：`Arc::new(ManifestStore::new(...))`——单一 Arc 实例。✓
- `api/db.rs:56` DbInner 字段 `manifest_store: Arc<ManifestStore>`（`pub(crate)`）。✓
- `api/collection.rs:50` CollectionInner 字段 `manifest_store: Arc<ManifestStore>`。✓
- `api/collection.rs:192` `create_new`：`manifest_store: db.manifest_store.clone()`——克隆 Arc（同一 ManifestStore 实例 → 同一 save_lock）。✓
- `api/collection.rs:219` `restore_from_manifest` 调 `create_new`——同样克隆 Arc。✓
- `api/collection.rs` 中 grep `ManifestStore::new` 仅出现在 `db.rs:44`（`Arc::new(ManifestStore::new(...))`），collection.rs 内无残留 `ManifestStore::new` 调用（flush/merge/reindex 三处均改用 `self.inner.manifest_store`）。✓

**若 save_lock 是 per-instance（各 flush/merge 构造新 ManifestStore）→ fix 无效。已核实：三处均用共享 Arc，fix 真生效。**

### 2.3 save_atomic 拆分避免重入死锁

- `save_atomic`（pub，入口取锁）→ 调 `save_atomic_locked`（私有，不取锁）。
- `add_segment` / `update`（入口取锁）→ 调 `save_atomic_locked`（不取锁）——不重入 save_lock（`std::sync::Mutex` 不可重入，重入死锁）。✓

### 2.4 I16 残留 tmp 清理保留

- `persistence/mod.rs:135` `save_atomic_locked` 开头 `let _ = self.vfs.delete(&tmp);`——保留 I16 裁决语义。✓

### 2.5 无死锁

- save_lock 是 save_atomic/add_segment/update 内唯一持有的 Rust 锁。Vfs 调用（create/write_at/sync/rename/delete/append）不经 Rust 锁（MemoryVfs 用 RwLock 保护内部 HashMap，但那是 Vfs 实现细节，不与 save_lock 形成 lock-order）。
- flush 在调 add_segment 前 `drop(state)`（释放 write_state）。merge 在调 update 前 write_state 已 drop（块作用域结束）。无「持 write_state 等 save_lock」死锁。✓

## 3. Bug 2 fix 定性：4 层逐层 ✅ ADDRESSED

### 3.1 compacting guard ✅ ADDRESSED

`api/collection.rs:518-544` `auto_merge_two_smallest`：

- `try_lock` compacting：`Ok(guard)` 且 `*guard==false` → 设 true，drop MutexGuard（块结束），建 `CompactingGuard`。✓
- `Err(WouldBlock)` → `return Ok(())`（skip，best-effort 降级）。✓
- `Err(Poisoned(e))` → `e.into_inner()`（恢复），设 true 继续。✓
- `*guard==true`（Poisoned 残留）→ `return Ok(())`（保守 skip）。✓

**panic-safe**：`CompactingGuard`（line 103-112）Drop 时 `self.flag.lock()` 设 false——panic 时 Drop 复位。guard 持 `&Mutex<bool>`（非 MutexGuard），Drop 时重新取锁。与 compact（line 1226-1235）/reindex（line 1317-1328）模式一致。✓

**无死锁**：auto_merge 持 compacting → merge_segments（不重入 compacting）→ write_state/snapshot/offsets 短持。compact 用阻塞 `lock()` 等 auto_merge 释放。锁序一致（compacting → write_state → snapshot → offsets → ...），无 lock-order deadlock。✓

**skip 语义正确**：auto-merge 是 best-effort 优化（段数超阈值时触发，skip 安全降级，下次 flush 再触发）。✓

### 3.2 docid 原子预留 ✅ ADDRESSED

`api/collection.rs:611-623` `merge_segments`：

```rust
let target_docid_base = {
    let mut state = self.inner.write_state.lock().unwrap();
    let tdb = if is_full_merge { 0 } else { max_non_source_end.max(state.next_docid) };
    let reserved_end = tdb + estimated_new_count;
    if reserved_end > state.next_docid { state.next_docid = reserved_end; }
    tdb
};
```

- **原子 read+bump**：一次 write_state lock 内读 next_docid + 算 target_base + bump next_docid=reserved_end。✓
- **消除 TOCTOU**：并发 add 在本块之前/之后拿锁，看到的 next_docid 都已 bump 到 reserved_end，其分配的 docid 推到 merge 新段之后。无「merge 读 next_docid → 并发 add 分配旧 next_docid → merge 写新段覆盖」窗口。✓
- **partial merge**：`tdb = max(保留段 base+count, next_docid)`——并入 next_docid 消除与 about-to-flush 缓冲段 docid 重叠。✓
- **full merge（compact）**：`tdb=0`，`reserved_end = estimated_new_count`。因 estimated_new_count = sum(source doc_counts) <= next_docid（next_docid 单调递增且 >= sum），`reserved_end > next_docid` 为 false → next_docid 不 bump（与旧代码行为一致）。✓
- **estimate 上界**：estimated_new_count = source 段 doc_count 之和（含 tombstone），实际 new_count <= estimate（tombstone 清除后），多预留区间留空无危害。✓

### 3.3 flush base_docid 连续性 ✅ ADDRESSED

`api/collection.rs:323-342` `flush`：

```rust
let first_docid = docs.first().map(|d| d.docid).unwrap_or(0);
let contiguous = docs.iter().enumerate().all(|(i, d)| d.docid == first_docid + i as u64);
let base_docid = if contiguous {
    first_docid  // 保持 inspect base=0 语义
} else {
    let base = state.next_docid;  // rebase 到当前 next_docid
    state.next_docid = base + count;
    base
};
```

- **连续性检测正确**：`contiguous` 检查每文档 docid == first + index。若并发 merge 在 add 之间 bump next_docid → 缓冲文档 docid 有 gap → `contiguous=false`。✓
- **连续路径**：用 first_docid 作 base（保持 inspect `base=0` 语义；merge 的 target_base 已并入 next_docid，新段在本段之上）。✓
- **非连续路径**：rebase 到 `state.next_docid`（merge 已 bump 到新段末尾之上）+ bump next_docid 预留本 flush 区间。不丢/不重 docid。✓
- **兼容 inspect**：连续 → base=0 语义保持（inspect 测试不碰）。✓

### 3.4 merge 快照 offsets 不覆写 ✅ ADDRESSED

`api/collection.rs:719-744` merge 快照重建：

- 旧代码：`offsets_w.insert(r.meta().ulid.clone(), offsets.get(&r.meta().ulid).copied().unwrap_or(0))`——用 merge 入口读的 stale `offsets` 覆写 `offsets_w`。并发 flush 新推入段的 offset 被错置为 0。
- 新代码：保留段**不覆写** `offsets_w`（注释 line 722-727 解释）。段不可变（I-1），保留段 offset 不变，无需 re-insert。仅移除 source 段 offset（`offsets_w.remove`）+ 插入新段 offset（`offsets_w.insert(new_meta.ulid, new_meta.docid_base)`）。✓
- 并发 flush 新推入段的正确 offset 保留在 `offsets_w` 中（`offsets_w` 是 live map，非 take'n）。✓

## 4. 残留 race 定性（concern #1）：adequately mitigated（Minor defer）

**关键判断：残留 race 非真实可触发——数学推导排除 docid 重叠。**

### 4.1 场景

concern #1：compact（is_full_merge，target_base=0）+ 并发 flush 的 docid 重叠。implementer 称 compact bump next_docid=estimate，但 compact 前已 buffer 的 stale docid 仍可能与 compact 新段 [0, total) 重叠。

### 4.2 数学推导排除重叠

**不变量**：`next_docid` 单调递增，且 `next_docid >= sum(所有段 doc_count)`（每 add 自增 next_docid 并赋 docid < next_docid；flush 写段 [base, base+count) 其中 base+count <= next_docid_at_flush）。

**compact 新段 docid 范围**：`[0, actual_new_count)` 其中 `actual_new_count <= estimated_new_count = sum(source doc_counts) <= next_docid`。

**buffered docs docid 范围**：`[old_next_docid, new_next_docid)` 其中 `old_next_docid >= sum(所有段 doc_counts) >= estimated_new_count >= actual_new_count`。

**结论**：buffered docs 的 docid >= old_next_docid >= actual_new_count，不在 compact 新段 [0, actual_new_count) 内。**无重叠。**

### 4.3 compact 不 bump next_docid 的验证

`api/collection.rs:613-621`：is_full_merge 时 `tdb=0`，`reserved_end = estimated_new_count`。因 `estimated_new_count <= next_docid`（4.2 不变量），`reserved_end > next_docid` 为 false → next_docid 不 bump。compact 后 next_docid 保持 >= actual_new_count。后续 add 分配的 docid >= next_docid >= actual_new_count，不与 compact 新段重叠。

### 4.4 并发 flush + compact 的 snapshot 一致性

若并发 flush 在 compact merge 期间推入新段：
- flush 的 `add_segment`（save_lock）与 compact 的 `update`（save_lock）互斥 → manifest 无 lost-update。✓
- compact merge 快照重建时 `std::mem::take(&mut *snap_w)` 取当前 snapshot（含 flush 新段）。flush 新段是保留段（不在 source_ulids）→ push 到 snap_w，其 offset 不被覆写（fix 3.4）。最终 snapshot = compact 新段 + flush 新段。✓
- flush 新段 docid >= old_next_docid >= actual_new_count，compact 新段 [0, actual_new_count) → 无重叠。✓

### 4.5 测试覆盖

- `stress_concurrent_add_during_compact`（line 593）：1 compact + 3 add 线程。最终 flush + 搜索验证无丢失。通过（report §5）。但**不检查 double-count**（仅 `found.contains(id)`），且**无 compact 期间并发 flush**（add 仅 buffer，flush 在 compact 后）。
- `stress_concurrent_auto_merge_no_double_count`（line 806）：检查 `found.len() == hits.len()`（double-count 核心断言）。通过。但**无 compact**（仅 auto-merge）。

### 4.6 定性

**adequately mitigated（Minor defer）**。数学推导排除 compact + 并发 flush 的 docid 重叠（4.2-4.3）。现有 stress 测试覆盖 compact+add 和 auto-merge+flush，均通过。**缺失 compact + 并发 flush 的直测**——建议未来加一个 stress 测试（compact 期间并发 flush），但数学保证正确性，非必须。

## 5. auto-merge skip 定性：acceptable ✅

- try_lock+skip 在高并发 flush 下 skip 部分 auto-merge → 段数临时累积（report 称 10-22 范围）。✓
- 下次 flush 段数仍超阈值时再 merge → 最终收敛。✓
- skip 是 best-effort 降级（task 要求），非正确性问题。✓
- 无无限累积（stress 8x 验证）。✓

## 6. stress non-vacuous + 8x 无 flaky 定性 ✅

### 6.1 断言实质（non-vacuous）

- `stress_concurrent_flush_no_corruption`（line 804）：4 线程 × 15 轮 = 60 并发 flush。断言：`errs.is_empty()`（无 E_CORRUPT）+ `flush_count == 60` + `found.contains(id)` 全文档（无丢失）+ `found.len() == hits.len()`（无 double-count）+ **reopen Db 不 E_CORRUPT + hits2.len()==hits.len()**（manifest 一致性核心断言）+ `ulid_set.len()==ulids.len()`（无重复 ULID）。**实质断言。** ✓
- `stress_concurrent_auto_merge_no_double_count`（line 806）：预填 12 段（>10）+ 4 线程 × 15 轮。断言：`errs.is_empty()` + `found.len()==hits.len()`（Bug 2 核心断言）+ `found==expected`（全文档）+ `ulid_set.len()==ulids.len()`。**实质断言。** ✓
- `stress_multi_run_stability`（line 763）：5 次 × 4×50 轮 × flush_interval=10（触发 auto-merge）。间接复用 `run_stress_add_flush_search` 的断言 + reopen 一致性。✓

### 6.2 8x 无 flaky

- report §5 称 8 次全跑均 `10 passed; 0 failed`。**无法从 diff 验证**（不重跑门禁）。但测试结构（MemoryVfs 纳秒级 + 4 线程 × 15 轮 + 5 次 multi-run）设计合理，断言实质，non-vacuous。✓

### 6.3 去 workaround 验证

- 删 `serialized_flush` helper（原用外部 `Mutex<()>` 序列化 flush 规避 Bug 1）。✓
- `stress_concurrent_add_flush_search` + `stress_stdfs_conformance`：直接 `col.flush()`（并发，无外部序列化）。✓
- 文件头注释更新（flush 并发边界从「外部 Mutex 序列化」改为「save_lock 序列化 manifest」）。✓

## 7. 不改冻结 pub API 定性 ✅

- `ManifestStore`：`pub struct`（字段私有）。加 `save_lock: Mutex<()>` private 字段不改 pub API。`pub fn new`/`load`/`save_atomic`/`add_segment` 签名不变。新增 `pub(crate) fn update`（crate 内可见）+ 私有 `save_atomic_locked`。✓
- `DbInner` / `CollectionInner`：`pub(crate)` 结构。`manifest_store: ManifestStore` → `Arc<ManifestStore>`（`pub(crate)` 字段，非 pub）。新增 `manifest_store` 字段（`pub(crate)`）。不改 pub API。✓
- `flush` / `auto_merge_two_smallest`（私有）/ `merge_segments`（私有）/ `compact` / `reindex` / `update_manifest_after_reindex`：签名不变。✓
- 不碰 SPEC.md / CI yml / fault.rs / crash_recovery / vane-fuzz / proptest / cross_version / tracing / inspect / VaneError 诊断。✓

## 8. crash_recovery 仍绿定性 ✅

### 8.1 crash_1（meta_slot 翻转）

- 注入 `Fault::IoError{op:Sync, path:*/manifest.json.tmp}`。
- `save_atomic_locked` 调 `self.vfs.sync(&tmp)?`——失败时返回 Err，不执行 rename。manifest 未切换。✓
- `add_segment`（flush 调）调 `save_atomic_locked` 失败 → `add_segment` 返回 Err → flush `?` 传播 → snapshot 不更新 → `segment_count` 仍 1。✓
- 断言 `err_msg.contains("manifest.json.tmp")`：FaultVfs 注入的 msg 含 "manifest.json.tmp"，错误传播路径不变（`sync(&tmp)?` 直接返回 Vfs 错误）。✓
- 重开 recover：WAL 有 AddSegment(孤儿) → 清理。旧 manifest 完好。✓

### 8.2 crash_3（merge 中断）

- 注入 `Fault::IoError{op:WriteAt, path:*/segments/seg_*/inverted.bin}`。
- `finalize_merge` → `write_inverted` 失败 → `merge_segments` 返回 Err（在 `update` 闭包之前）。✓
- manifest 未切换（`update` 未调）。segment_count 仍 2。✓
- 断言 `err_msg.contains("inverted.bin")`：FaultVfs msg 含 "inverted.bin"。✓
- 重开 recover：旧段保留，d0 tombstone 重放。✓

### 8.3 定性

code path 保留（save_atomic_locked 用同一 tmp 路径 + 同一 Vfs 调用序列 + 同一错误传播）。断言仍匹配。report 称 5 passed。✓ **无法从 diff 验证**（不重跑）。

## 9. reindex 改动定性 ✅

`api/reindex.rs:221-246` `update_manifest_after_reindex`：

- 旧代码：`manifest_store.load()?; modify; manifest_store.save_atomic(&manifest)?;`（load-modify-save 分三步，不原子）。
- 新代码：`manifest_store.update(|manifest| { modify; Ok(()) })?;`（load-modify-save 在 save_lock 内原子完成）。✓
- 逻辑保留：ULID 替换（retain old + push new）、tokenizer_id/user_dict 更新。✓
- 错误消息保留：`VaneError::NotFound("collection not in manifest: ... (op=reindex; ...)")`。✓
- import 清理：`Manifest` 不再导入（闭包接收 `&mut Manifest`，无需直接引用类型）。`CollectionMeta` + `ManifestStore` 仍导入（函数签名用）。✓
- `api/collection.rs:1400` `reindex` 调用处：`let manifest_store = &self.inner.manifest_store;`（共享 Arc 的引用）→ `update_manifest_after_reindex(manifest_store, ...)`。✓

## 10. Findings

### Critical: 无

### Important: 无

### Minor

1. **`crates/vane-core/tests/stress_concurrency.rs:593` | 缺 compact + 并发 flush 直测 | concern #1 残留 race 数学排除但无直测** — `stress_concurrent_add_during_compact` 仅测 compact + add（buffer），无 compact 期间并发 flush。数学推导（§4.2）排除 docid 重叠，但缺直测降信心。建议未来加 compact + 并发 flush stress 测试。

2. **`crates/vane-core/src/vfs/std_fs.rs:90-102` | StdFsVfs::append 非 atomic（pre-existing，非本 fix 引入） | 并发 flush 在 StdFsVfs 上 WAL append 可能交错** — `seek(SeekFrom::End(0))` + `write_all` 间有 gap，两线程并发 append 可能互相覆写。MemoryVfs 安全（RwLock write lock 序列化）。merge 路径 WAL append 在 save_lock 内（部分缓解），flush 路径 WAL append 仍在 save_lock 外（pre-existing）。**非本 fix 引入，不阻断合并。**

3. **`crates/vane-core/src/api/collection.rs:667-695` | merge 的 WAL append 在 save_lock 内、flush 的 WAL append 在 save_lock 外 | 不一致但非正确性问题** — merge 的 `update` 闭包内 WAL append + save_atomic_locked 都在 save_lock 内（事务原子）。flush 的 WAL append 在 save_lock 外，add_segment 在 save_lock 内。intentional（flush WAL 仅记录、merge WAL 是事务一部分），但不一致值得记录。

## 11. ⚠️ 无法从 diff 验证项

- **8x stress 全跑均 10 passed**：report §5 称 8 次均 OK。不重跑（task 要求）。测试结构 + 断言实质合理，但 8x 无 flaky 信任 report。
- **crash_recovery 5 passed**：report §6 称 5 passed。不重跑。code path 保留 + 断言匹配（§8 分析），信任 report。
- **cargo fmt/clippy/deny/wasm check 绿**：report §6 称全绿。不重跑。fix 用 `std::sync::{Arc, Mutex}`（已在用），不引 std::fs，WASM 应绿。
- **concern #1 的 compact + 并发 flush 实际不重叠**：数学推导排除（§4），但无 stress 直测。

## 12. 总体

**不进 fix 循环。**

- 2 bug 真修了（Bug 1 save_lock Arc 共享真生效；Bug 2 4 层 fix 全 ADDRESSED）。
- 残留 race（concern #1）数学排除（next_docid 单调递增 + compact 新段 [0, actual_new_count) <= next_docid），adequately mitigated，Minor defer。
- auto-merge skip acceptable（降级非正确性）。
- stress 测试 non-vacuous + 断言实质。
- frozen pub API 不变。
- crash_recovery code path 保留 + 断言匹配。
- reindex 改动正确。
- 无 Critical / Important findings。

Minor findings（3 条）均为 defer / pre-existing / non-blocking，不构成进 fix 循环理由。
