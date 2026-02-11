# FaultVfs 故障注入 VFS — Reviewer 审查（只读）

> 审查对象：commits 985cc06..03319ca（`feat(core): FaultVfs 故障注入 VFS + 单测（M4 阶段二 a）`）。
> 来源 brief：`docs/plans/m4/phase0-design.md` §3.1。
> Implementer 报告：`docs/plans/m4/task-faultvfs-report.md`。
> 审查范围：spec 合规 + 代码质量（correctness / simplification / test hygiene）+ 全局约束。
> 审查模式：只读，未编辑任何源码，未重跑门禁（信任 report 数据）。

---

## 1. Spec 合规判定

**✅ 合规** —— §3.1 brief 全部硬性要求满足。

| Brief 要求 | 实现位置 | 判定 |
|---|---|---|
| FaultVfs struct（inner + faults Mutex） | `fault.rs:137-143` | ✅（额外加 `call_counts` 保持 faults 签名不变） |
| Vfs impl 全 8 方法（create/read_at/write_at/append/sync/rename/delete/list）桩填实 | `fault.rs:203-309` | ✅ 8 方法全实现，无 stub |
| Fault enum 含 IoError/PartialWrite/Enospc/Delay | `fault.rs:20-53` | ✅ 4 变体全有 |
| **无 LostWrite**（用户决策 #3）+ TODO 注释 | `fault.rs:18` | ✅ `// TODO(M4-Could): LostWrite ...` 注释到位；无 `lost_writes`/`MarkLostWrite` 死代码 |
| check_fault 层 1（path+op 匹配）+ 层 2（trigger_on_nth 计数 + one_shot 消费） | `fault.rs:173-200` | ✅ glob_match + per-rule 计数 + fire 后 remove |
| path matcher 自研前缀+`*`通配、**未引 regex** | `fault.rs:313-335` | ✅ DP 实现，无 regex crate；见 Minor #3（full-match vs 前缀） |
| inject 链式 API | `fault.rs:160-163` | ✅ `inject(&self, fault) -> &Self` |
| check_fault 在调 inner 前执行（inner 不变保证） | `fault.rs:204-309` 各 Vfs 方法 | ✅ check_fault 返回 action 后释放锁，再调 inner；返错不调 inner |
| 单测覆盖 6 必需机器不变量 | `fault.rs:347-565` | ✅ 8 测试（6 必需 + 2 补充） |

**implementer 自报偏离分析（给所有 Fault 变体加 `one_shot`+`trigger_on_nth`）**：
设计 §3.1 enum 骨架仅在 `IoError` 示意 `one_shot`，但同一 §3.1 的层 2 机制描述
（"Fault::IoError { trigger_on_nth, ... }"）已将 `trigger_on_nth` 作为通用字段示意。
brief 的机制总述（"触发后消费（one-shot）或持久（每次命中）"）是**对所有故障类型**的
通用语义描述，非 IoError 专属。将 `one_shot`+`trigger_on_nth` 一致化到所有变体是
**合理的统一化实现决策**：避免 PartialWrite/Enospc/Delay 的隐式"always fire"语义
歧义，使崩溃恢复测试能精确控制任意故障类型的触发时机（如"第 2 次 append 部分写"）。
**判定：不算偏离**。设计骨架字面仅 IoError 有 one_shot 属示意性骨架，非硬约束。

---

## 2. Findings（按严重度排序）

**无 Critical，无 Important，7 条 Minor。**

### Minor

#### M1. `fault.rs:183` call_counts 计数器 key 共享导致 trigger_on_nth 误计

`call_counts` 的 key 是 `(VfsOp, path_pattern)`（per-brief §3.1 设计），故两条
共享相同 `(op, path_pattern)` 但不同 `trigger_on_nth` 的规则共享同一计数器。

**失败场景**：注入 Fault A（trigger_on_nth=3, one_shot）+ Fault B（trigger_on_nth=5,
one_shot），同 op 同 path。每次 check_fault 遍历到匹配规则即递增共享计数器，A 的
counter 在 B 匹配时也被递增 → A 在第 2 次调用即 fire（非预期的第 3 次），B 在第 4 次
fire（非预期的第 5 次）。

**实际影响**：§3.1 注入点映射表的 10 个场景全部使用不同的 (op, path_pattern) 组合
（`*.json.tmp` / `*/wal.log` / `*/segments/seg_*/inverted.bin` 等），不会触发此问题。
阶段 2b 崩溃恢复测试若注入多条同 path 规则（如"第 2 次 sync 失败"+"第 5 次 sync
失败"同 path），会命中此 bug。implementer 自审 #3 已部分提及"counter side effect"。

**严重度**：Minor（brief 指定 key 结构，implementer 忠实实现；mapping table 场景不触发；
latent for multi-fault-per-path）。

#### M2. `fault.rs:229,254` PartialWrite 的 `?` 传播 inner 错误而非 PartialWrite 错误

```rust
self.inner.write_at(path, &buf[..n], offset)?;
return Err(VaneError::Io(format!("partial write ..."));
```

若 `self.inner.write_at/append` 本身返 Err（如 StdFsVfs 真实磁盘满），`?` 提前返回
inner 的错误，而非 PartialWrite 故障的语义错误。

**失败场景**：用 StdFsVfs 作为 inner，磁盘在写前 N 字节中途满 → 返回 inner 的
`VaneError::Io("...")`（无 "partial write" 标识），测试断言错误消息含 "partial write"
会失败。MemoryVfs（主力）的 write_at/append 实质不返错，无实际影响。

**严重度**：Minor（MemoryVfs 主力场景不受影响；StdFsVfs conformance 对齐场景的边界）。

#### M3. `fault.rs:313-335` glob_match 用全匹配而非 brief 的"前缀匹配"

brief §3.1 §取舍/风险 推荐"前缀匹配 + 通配 `*`"，实现为**全匹配**（pattern 必须匹配
整段 path，`dp[m][n]`）。测试 `path_matcher_star_and_prefix` 明确断言
`!glob_match("db/wal", "db/wal.log")`。

**失败场景**：测试期望 `pattern="db/wal"` 匹配 `path="db/wal.log"`（前缀语义）→ 实现返
false（全匹配语义），故障不触发。但 §3.1 注入点映射表的 pattern 全部以具体后缀
（`*.json.tmp` / `*/wal.log`）或 `*`（`"*"`）结尾，全匹配与前缀匹配在这些 pattern
下行为一致。故映射表场景不受影响。

**严重度**：Minor（实现更精确，是更好的选择；但与 brief 措辞不一致，design 文档应
reconcile 为"全匹配 + `*` 通配"）。

#### M4. `fault.rs:186` trigger_on_nth=N + one_shot=false 语义交互令人意外

`one_shot=false, trigger_on_nth=3`：第 3 次匹配时 fire，规则保留；第 4 次起
`*count == 3` 恒 false，规则永不再 fire 但仍留在表内。

**失败场景**：用户读 `one_shot=false` 期望"持久/重复触发"，但配 `trigger_on_nth=3`
实际只 fire 一次（第 3 次）后静默永不再触发。若崩溃恢复测试期望"每 3 次 sync 失败
一次"，会得到"仅第 3 次失败一次"的错误行为。

**严重度**：Minor（brief 措辞"仅在第 N 次匹配时触发"是单数次语义，实现忠实；但与
`one_shot=false` 的"持久"直觉冲突。建议文档明确此交互，或实现 `count % N == 0`
的重复语义——但后者改 brief 契约，需用户决策）。

#### M5. `fault.rs:204-211,255-258,285-287` catch-all `_ => {}` 静默消费不适用故障

`create`/`read_at`/`sync`/`rename`/`delete`/`list` 的 match 用 `_ => {}` 兜底
PartialWrite/Enospc 动作。若注入 `Enospc{op:Create, ...}`，check_fault 匹配成功、
one_shot 规则被消费移除，但 Enospc 动作落入 `_ => {}` 被忽略，inner 正常调用。

**失败场景**：误配 `PartialWrite{op:Create}` → fault 被消费但 create 正常返回 Ok，
测试期望 create 失败会断言失败。代码注释已说明"PartialWrite/Enospc 对 create 无写入
语义，忽略并转发 inner"，但消费仍发生。

**严重度**：Minor（属用户误配 fault op；但 silent consumption 而非 warn/error 可能
让测试作者困惑。建议：不适用动作不消费规则，或 debug_assert! 提示）。

#### M6. `fault.rs:174-175,161,210` 等 `Mutex::lock().unwrap()` 中毒级联 panic

所有锁获取用 `.unwrap()`，若某线程持锁时 panic（中毒），后续 `check_fault`/`inject`
均级联 panic。

**失败场景**：`fault-injection` feature 用于 vane-ffi 多线程集成测试，某线程 panic
持锁 → 其他线程访问 FaultVfs 级联 panic，掩盖真实失败原因。

**严重度**：Minor（cfg(test) 场景标准做法；feature 启用下的多线程集成测试可考虑
`lock().unwrap_or_else(|e| e.into_inner())` 容忍中毒。非 bug）。

#### M7. `fault.rs` 测试覆盖缺口

8 个测试覆盖 brief 必需的 6 不变量 + 2 补充（持久故障 / rename 阻塞）。缺口：

- `append` 的 PartialWrite / Enospc 路径未测（仅 `write_at` 测了这两类动作）。
- `Delay` 变体完全未测（`sleep_ms` 无测试覆盖）。
- `create`/`read_at`/`delete`/`list` 的 IoError 注入路径未测（仅 sync/write_at/rename
  有 IoError 测试）。
- 无测试验证"两条规则同 path 的 counter 共享行为"（M1 场景无回归守护）。
- 无测试验证 M5 的"不适用动作消费"边界。

**严重度**：Minor（必需不变量已覆盖；缺口属健壮性补充，阶段 2b 崩溃恢复测试会
间接覆盖部分 append 路径）。

---

## 3. 全局约束审查（binding 项）

| 约束 | 审查结果 |
|---|---|
| **cfg 门控不泄漏**：fault.rs + mod 声明须 `#[cfg(any(test, feature="fault-injection"))]` | ✅ `mod.rs:22` `#[cfg(any(test, feature = "fault-injection"))] pub mod fault;`；fault.rs 整模块经 mod 声明门控，无独立 `#![cfg]`（mod 门控足够）。wasm32 check 不设 test/feature → fault.rs 不编译。report §4.4 验证 wasm32 check 绿。 |
| Cargo.toml 改动仅为 `[features] fault-injection = []`，无新 `[dependencies]` | ✅ diff 仅 5 行 feature 声明（含注释），`[dependencies]`（line 12）和 `[dev-dependencies]`（line 67）无新增条目。无 `default` feature 包含 fault-injection。 |
| **core 禁 std::fs/std::net/mmap** | ✅ fault.rs 用 `std::collections::HashMap` / `std::sync::{Arc, Mutex}` / `std::thread::sleep`（后者 `#[cfg(not(target_arch="wasm32"))]` 门控）。无 `std::fs::` / `std::net::` / `mmap`。`check-no-std-fs.sh` 扫描 fault.rs（非 tests.rs 文件名）通过（report §4.5）。 |
| **不改冻结 pub API** | ✅ `fault` mod 仅 cfg-gated `pub`；无现有 pub fn/struct/trait 签名被改。Vfs trait（mod.rs:5-14）未动。新增 pub 项（Fault/VfsOp/FaultVfs）仅在 test/feature 下可见。 |
| **core 禁平台分支泄漏**（I-5 不变量） | ✅ fault.rs 内仅 `#[cfg(not(target_arch="wasm32"))]` 用于 sleep_ms（非平台分支核心，是守护性 no-op）；整个 fault.rs 已被 mod 级 cfg 门控，不进生产/wasm。 |

---

## 4. 无法从 diff 验证项（需跨任务/未改代码核验）

以下项超出本 diff 范围，编排者自行核验：

1. **wasm32 体积不增**：report §4.4 仅跑 `cargo check --target wasm32-unknown-unknown -p vane-core`（绿），但未跑 `check-wasm-size.sh` 测体积。逻辑推断 fault.rs 不编译则体积不变，但需 wasm32-size CI job 实跑确认（属既有 CI job，本任务未改 CI yml）。
2. **vane-ffi 集成测试能用 fault-injection feature**：Cargo.toml 加了 feature，但 vane-ffi 是否在 `[features]` 或 `[dev-dependencies]` 引用 `vane-core/fault-injection` 未在本 diff 涉及。阶段 2b 若需 vane-ffi 集成测试用 FaultVfs，需 vane-ffi 侧配置。
3. **crash_recovery.rs 集成测试**：brief §3.1 文件位置列出 `tests/crash_recovery.rs`（新集成测试文件），但本 diff 未创建此文件（属阶段 2b 范围，非本 task）。
4. **CI yml 未改**：本 diff 不含 `.github/workflows/ci.yml` 改动。若 test job 跑 `--all-features`，则 `fault-injection` feature 会在 CI test job 编译 fault.rs（report §4.3 跑 `--all-features` 通过）；但 wasm32-check job 不跑 `--all-features`（按既有 CI 配置），fault.rs 不编译进 wasm。此为既有 CI 配置的正确性，非本 task 引入。

---

## 5. 代码质量补充观察（不构成 finding）

- **锁不持有期间调 inner**：`check_fault`（173-200）在函数内完成 faults+counts 双锁获取、遍历、fire、remove，返回 `Option<FaultAction>` 后锁释放。Vfs impl 各方法在 `check_fault` 返回后才调 `self.inner.xxx()`，无持锁调 inner 的死锁/重入风险。✅
- **锁顺序一致**：`check_fault` 始终先 `faults` 后 `counts`；`inject` 仅 `faults`。无逆序，无死锁。✅
- **one_shot 消费并发安全**：并发 check_fault 经 Mutex 串行，首线程 fire+remove，后续线程找不到已移除规则 → 透传 inner。无 double-fire。✅
- **PartialWrite offset 正确**：`write_at` 用原 `offset` 写 `buf[..n]`，`append` 用 append 语义追加。inner 确实有 N 字节（fresh file 场景，测试验证）。✅
- **Enospc 不写 inner**：返 Err 前无 `self.inner.write_at/append` 调用。✅
- **rename check_fault(from) 在 inner 前**：manifest 原子切换前注入返错则 inner 未 rename，状态不变（测试 `rename_fault_blocks_and_inner_unchanged` 验证）。brief 骨架注释"check_fault(Rename, from) 或 (to)"——实现仅查 `from`，与"或"语义一致（任一即可），mapping table pattern 用 `from`（tmp 路径）。✅
- **Fault helper 方法**（op/path_pattern/one_shot/trigger_on_nth/to_action）用 `|` match 臂去重，idiomatic，无可简化的重复。✅
- **glob_match DP O(m\*n) 空间**：对短 path（<100 char）无性能问题；非热路径（测试基建），无需 rolling array 优化。✅
- **无死代码**：FaultAction 无 MarkLostWrite 变体；FaultVfs 无 lost_writes 字段（LostWrite 决策 #3 执行干净）。✅

---

## 6. 总体判定

**可过（仅 Minor，无 Critical/Important）。**

7 条 Minor 均为边界语义、测试覆盖缺口或设计措辞 reconciliation，不影响 brief
§3.1 注入点映射表的 10 个场景正确性，不阻塞阶段 2b 崩溃恢复测试。

**建议（非阻塞）**：
- 阶段 2b 崩溃恢复测试若注入多条同 (op, path_pattern) 规则，先评估 M1（counter
  共享）是否命中；若命中，在 fix 循环修 call_counts key 为 per-fault identity
  （如 fault 注册序号）或文档明确"同 path 仅注入一条 trigger_on_nth 规则"约束。
- M3（full-match vs 前缀）：reconcile design 文档措辞为"全匹配 + `*` 通配"，
  与实现一致。
- M7（测试缺口）：阶段 2b 间接覆盖 append 路径后，补 Delay / create / list 的
  IoError 直测作为回归守护。
