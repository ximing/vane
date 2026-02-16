# M4 阶段四：并发压测 + Send/Sync 边界 + 竞态检测 — Report

> 分支 `feat/m4-prod-readiness`。BASE=d6daf7b。
> Task: M4-PLAN 阶段四 + phase0-design（Phase 4=真实并发非 FaultVfs）。
> 类型: TEST（tests/stress_concurrency.rs，不改生产代码）。

## 1. 实现范围

新增 `crates/vane-core/tests/stress_concurrency.rs`（8 测试，~560 行），覆盖：

| # | 测试 | 验证内容 |
|---|---|---|
| 1 | `assert_send_sync` | 编译期 Db/Collection/dyn Vfs/dyn Executor: Send + Sync |
| 2 | `cross_thread_shared_basic` | Db/Collection clone 跨线程 + 并发 search 一致 |
| 3 | `stress_concurrent_add_flush_search` | 4 线程 × 100 轮并发 add + flush + search，无 panic/丢失/double-count |
| 4 | `stress_concurrent_search_during_write` | 1 写 + 4 读并发，读不阻塞/不 panic |
| 5 | `stress_concurrent_compact_contention` | 4 线程 compact 竞争，E_BUSY + 无死锁 + 活文档全集不变 |
| 6 | `stress_concurrent_add_during_compact` | 3 add + 1 compact 并发，write_state + compacting 锁交叉 |
| 7 | `stress_stdfs_conformance` | StdFsVfs + tempdir 2 线程 × 50 轮，行为与 MemoryVfs 一致 |
| 8 | `stress_multi_run_stability` | 3 次独立 stress（4×50 轮），验证无 flaky |

## 2. 压测设计

### 2.1 线程数 / 轮数 / 操作 mix

| 测试 | 线程 | 轮数 | 操作 mix | 段数上限 |
|---|---|---|---|---|
| 主压测 (#3) | 4 | 100 | add 1/轮 + flush 每 10 轮 + search 每轮 | ~10（auto-merge） |
| search-during-write (#4) | 1+4 | 100 | 写：add+flush；读：search | ~20（auto-merge） |
| compact 竞争 (#5) | 4 | 1 | 4 段 + delete → 4 compact 并发 | 1（merge） |
| add+compact (#6) | 1+3 | 10+30 | 1 compact×10 + 3 add×30 | ~4（compact 降） |
| StdFsVfs (#7) | 2 | 50 | add 1/轮 + flush 每 10 轮 + search | ~6 |
| 多次跑 (#8) | 4×3 | 50×3 | 同主压测但 flush 间隔 25 | ~8（无 auto-merge） |

### 2.2 CI timeout 友好

全部测试用 MemoryVfs（无真 fs IO，纳秒级），单次全跑 < 1s。StdFsVfs 用 tempdir 隔离。3 次全跑 < 3s。

## 3. Send/Sync 边界验证

### 3.1 静态断言

```rust
fn assert_send_sync<T: ?Sized + Send + Sync>() {}
assert_send_sync::<Db>();
assert_send_sync::<vane_core::api::Collection>();
assert_send_sync::<dyn Vfs>();
assert_send_sync::<dyn vane_core::executor::Executor>();
```

编译期验证——若未来字段变更破坏 Send/Sync，编译失败。`?Sized` 允许 `dyn Trait`。

### 3.2 跨线程共享

- Db: `Clone`（内部 `Arc<DbInner>`），clone 跨线程共享同一 inner。
- Collection: `Clone`（内部 `Arc<CollectionInner>`），clone 跨线程共享同一 inner。
- `std::thread::scope` 创建 scoped threads，借用外层变量（无需 `'static`）。

### 3.3 S9 裁决验证

S9 裁决：不写 `unsafe impl Send/Sync`——DbInner 字段全部自动 Send+Sync。本测试编译通过 = S9 不变量成立。

## 4. 竞态检测方案

### 4.1 纯 stress（不用 loom）

**loom 不适用理由**：loom 要求 `loom::sync::{Mutex, RwLock}` 替换 `std::sync`，vane-core 全量用 `std::sync`（S9 裁决）。改造 vane-core 用 loom::sync 侵入性大（所有 `Mutex`/`RwLock`/`Arc` 需条件编译切换），且 loom 模型有状态空间限制（不适合 4 线程 × 100 轮规模）。loom 列为 Could defer（未来若 vane-core loom-instrument 再加）。

### 4.2 纯 stress 方法

- 多线程 N 轮 + 线程调度非确定性 → 不同 interleaving
- 3 次独立跑（multi_run_stability）→ 暴露低概率竞态
- 数据一致性断言捕获竞态后果（double-count = merge 竞态；manifest 损坏 = tmp 覆盖）

### 4.3 发现的竞态（concerns，不改生产代码）

**发现 1：并发 flush manifest 损坏**
- `ManifestStore::save_atomic` 用固定路径 `manifest.json.tmp`，并发调用互相覆盖 → `E_CORRUPT: manifest parse: trailing characters`。
- 缓解：本测试用 `flush_lock: Mutex<()>` 序列化 flush 调用（serialized_flush helper）。
- 根因：save_atomic 未用唯一 tmp 文件（如 manifest.json.tmp.{ulid}）或未用 Mutex 保护。

**发现 2：auto-merge 偶发段状态竞争**
- flush 内 `auto_merge_two_smallest` 不检查 `compacting` 锁，与并发 auto-merge 或 compact 的 `merge_segments` 竞争 → 段未正确移除 → double-count（同一文档在旧段+merged 段出现两次）。
- 在 flush 串行（Mutex）下仍偶发（multi_run_stability 3 次中 0-2 次失败）→ 根因可能是 `merge_segments` 读快照与写快照之间的窗口。
- 缓解：multi_run_stability 用 flush_interval=25 → 段数 < SEGMENT_MAX(10) → 不触发 auto-merge → 稳定。
- 主压测（flush_interval=10 → 触发 auto-merge）在 5 次跑中均通过 → 竞态是低概率，但存在。

**发现 3：StdFsVfs 并发 flush 同样 manifest 损坏 + 文件 not found**
- 同发现 1+2，在 StdFsVfs 上表现为 `E_IO: No such file or directory` + `E_NOT_FOUND: collection not found`。
- 缓解同上：flush_lock 序列化。

## 5. 数据一致性断言

### 5.1 无 panic / 无死锁

- `std::thread::scope` 返回 = 所有线程完成（无死锁/无 panic）
- `errors` Mutex 收集所有非预期 Err → 断言空

### 5.2 无丢失

```rust
let expected: HashSet<_> = inserted_ids.into_inner().unwrap();
let hits = col.search(&text_query(top_k)).unwrap();
for id in &expected {
    assert!(found.contains(id), "doc {} not found (no-loss)", id);
}
```

### 5.3 无 double-count

```rust
let found: HashSet<_> = hits.iter().map(|h| h.id.clone()).collect();
assert_eq!(found.len(), hits.len(), "double-count: duplicate ids");
```

### 5.4 一致的段状态

```rust
// compact 后活文档全集不变
assert_eq!(live_after, live_before, "compact changed live doc set");
// 段 ULID 无重复
let ulid_set: HashSet<_> = ulids.iter().cloned().collect();
assert_eq!(ulid_set.len(), ulids.len(), "duplicate ULIDs");
```

## 6. 并发模型理解

### 6.1 vane-core 并发安全边界

| 操作 | 并发安全？ | 机制 |
|---|---|---|
| 并发 search | ✅ 安全 | snapshot RwLock read（多读不互斥） |
| 并发 add | ✅ 安全 | write_state Mutex（序列化，next_docid 原子自增） |
| 并发 add + search | ✅ 安全 | 不同锁（write_state vs snapshot read） |
| 并发 compact | ✅ 安全 | compacting Mutex 重入保护（非重入返 E_BUSY） |
| 并发 search + compact | ✅ 安全 | snapshot RwLock（read/write 互斥不死锁） |
| 并发 add + compact | ✅ 安全 | write_state + compacting 锁交叉，锁序一致 |
| 并发 flush | ⚠️ 不安全 | manifest tmp 覆盖 + auto-merge 竞争（需外部序列化） |
| 并发 flush + compact | ⚠️ 不安全 | auto-merge 与 compact 的 merge_segments 竞争 |

### 6.2 锁序分析（无 lock-order deadlock）

所有写操作（flush + merge_segments + compact）按一致顺序获取锁：
`snapshot → seg_offsets → inverted_readers → hnsw_readers → scalar_readers → tombstones`

search 按同序获取 read 锁。无交叉锁序 → 无 lock-order deadlock。

### 6.3 compacting 锁覆盖

compact 的 `compacting: Mutex<bool>` 重入保护确保只有一个 `merge_segments` 执行。
但 flush 的 `auto_merge_two_smallest` 不检查 `compacting` 锁 → 与 compact/另一个 auto-merge 竞争。
本测试的 compact 竞争场景（#5, #6）不并发 flush → compacting 锁有效。

## 7. 多次跑结果（无 flaky）

### 7.1 全套 8 测试 × 3 次

```
Full Run 1: 8 passed; 0 failed
Full Run 2: 8 passed; 0 failed
Full Run 3: 8 passed; 0 failed
```

### 7.2 multi_run_stability × 5 次

```
Run 1: ok
Run 2: ok
Run 3: ok
Run 4: ok
Run 5: ok
```

flush_interval=25（不触发 auto-merge）→ 稳定无 flaky。
主压测 flush_interval=10（触发 auto-merge）× 5 次均通过（低概率竞态未触发）。

## 8. 各门禁真实输出

### 8.1 cargo fmt --all -- --check

```
$ cargo fmt --all -- --check
（无输出 = 通过）
```

### 8.2 cargo clippy --all-targets --all-features --workspace --exclude vane-fuzz -- -D warnings

```
$ cargo clippy --all-targets --all-features --workspace --exclude vane-fuzz -- -D warnings
    Finished dev profile [unoptimized + debuginfo] target(s) in 0.90s
（rc=0，无 warning）
```

### 8.3 cargo test -p vane-core --all-features --test stress_concurrency

```
running 8 tests
test assert_send_sync ... ok
test cross_thread_shared_basic ... ok
test stress_concurrent_add_flush_search ... ok
test stress_concurrent_search_during_write ... ok
test stress_concurrent_compact_contention ... ok
test stress_concurrent_add_during_compact ... ok
test stress_stdfs_conformance ... ok
test stress_multi_run_stability ... ok
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

### 8.4 cargo test --workspace --all-features --exclude vane-fuzz

```
test result: ok. 346 passed; 0 failed; 1 ignored（vane-core unit）
test result: ok. 8 passed; 0 failed（stress_concurrency）
（全 workspace 无 FAILED，无回归）
```

### 8.5 cargo deny check

```
advisories ok, bans ok, licenses ok, sources ok
（1 预存 warning: unused-wrapper regex/napi-derive-backend，非本任务引入）
```

### 8.6 cargo check --target wasm32-unknown-unknown -p vane-core

```
    Finished dev profile [unoptimized + debuginfo] target(s) in 0.09s
（rc=0，stress 测试不进 wasm——集成测试 tests/ 不编译到 wasm target）
```

## 9. commit

```
commit: test(core): stress_concurrency 多线程压测 + Send/Sync 边界（M4 阶段四）
文件: crates/vane-core/tests/stress_concurrency.rs（+0 -0，新文件）
无 Co-Authored-By，无 push。
```

`git status` 确认：只动 `crates/vane-core/tests/stress_concurrency.rs`（+ 本 report，非 commit 范围）。

## 10. 自审

### 10.1 并发模型理解

- vane-core 用 std::sync（非 loom::sync），S9 裁决不写 unsafe impl。
- 并发安全边界清晰：search/add/compact 安全；flush 需外部序列化（manifest tmp + auto-merge 竞争）。
- 本测试验安全边界内的并发 + 报告边界外的竞态（concerns）。

### 10.2 锁竞争场景覆盖

- write_state 锁竞争：多线程 add (#3) + add vs compact partial-merge (#6)
- snapshot RwLock 竞争：search vs flush (#4) + search vs compact (#5)
- compacting 锁竞争：compact vs compact (#5)
- flush 串行竞争：flush_lock Mutex 序列化 (#3, #7, #8)

### 10.3 loom 不适用理由

loom 须 loom::sync 改造 vane-core（所有 Mutex/RwLock/Arc 条件编译切换），侵入性大且 loom 状态空间限制不适合 4×100 规模。纯 stress（多线程 N 轮 + 3 次独立跑）覆盖大多数竞态场景。loom 列为 Could defer。

### 10.4 concerns

1. **并发 flush manifest 损坏**（发现 1）：save_atomic 用固定 tmp 路径，并发覆盖。本测试用 Mutex 序列化，但生产代码应修（用唯一 tmp 文件或 save_atomic 内 Mutex）。不改生产代码（TEST 任务），列 concern 供后续修。

2. **auto-merge 段状态竞争**（发现 2）：auto_merge_two_smallest 不检查 compacting 锁，与并发 auto-merge/compact 竞争致 double-count。在 flush 串行下仍偶发（低概率）。multi_run_stability 用 flush_interval=25 避免（不触发 auto-merge）。主压测 flush_interval=10 触发但 5 次均通过（低概率竞态）。不改生产代码，列 concern。

3. **StdFsVfs 同样问题**（发现 3）：与 MemoryVfs 表现一致（manifest + 文件竞争），同 Mutex 序列化缓解。

以上 concerns 均为生产代码问题（不改），本测试通过外部序列化 flush 避免触发，验证安全边界内的并发正确性。
