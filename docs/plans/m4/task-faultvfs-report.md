# M4 阶段二 a：FaultVfs 故障注入 VFS 实现报告

> 任务：实现 FaultVfs 故障注入 VFS（设计 §3.1）+ 机器本身单测。
> 来源 brief：`docs/plans/m4/phase0-design.md` §3.1。
> 状态：**DONE**。

## 1. 实现摘要

实现 `FaultVfs` —— 包装任意 `Arc<dyn Vfs>` 的透明故障注入 VFS，按
`(path_pattern, op, 调用计数)` 三层匹配注入可控故障，供崩溃恢复测试
精确模拟 IO 错误 / 部分写 / ENOSPC / 延迟。纯新增模块，不改 M0-M3
冻结 pub API。

**核心机制**：
- **层 1（path + op）**：`glob_match` 轻量 glob 匹配（`*` 通配任意序列含 `/`，
  自研 DP 实现，不引 regex 黑名单）。pattern 匹配整段 path。
- **层 2（调用计数）**：`call_counts: Mutex<HashMap<(VfsOp, String), u32>>`
  按规则 (op, path_pattern) 维护计数。`trigger_on_nth=0` 每次匹配触发；
  `trigger_on_nth=N` 仅第 N 次匹配触发。
- **one_shot 消费**：触发后从规则表移除；`one_shot=false` 持久（每次触发保留）。
- **first-fire-wins**：首条触发的规则返回后停止；之前的非触发规则仍递增计数器。
- **check_fault 在调 inner 前执行**：返错则不调 inner，保证 inner 状态不变。

**用户已批准的决策执行情况**：
1. ✅ FaultVfs 启用范围 = `cfg(any(test, feature="fault-injection"))`：
   `fault.rs` 整模块门控；`Cargo.toml` 加 `fault-injection = []`（默认不启用）。
2. ✅ LostWrite 列为 Could/暂不实现：Fault enum **省略** LostWrite 变体；
   fault.rs 留 `// TODO(M4-Could): LostWrite 真实丢写模拟，暂用 sync 失败注入近似`
   注释。`FaultAction`/`FaultVfs` 均无 `lost_writes`/`MarkLostWrite` 死代码。

## 2. 文件清单

| 文件 | 操作 | 说明 |
|---|---|---|
| `crates/vane-core/src/vfs/fault.rs` | 新增 | Fault enum（IoError/PartialWrite/Enospc/Delay，无 LostWrite）/ VfsOp / FaultAction / FaultVfs struct / Vfs impl 全 8 方法 / `check_fault` / `glob_match` / `sleep_ms` / 8 单测 |
| `crates/vane-core/src/vfs/mod.rs` | 改 | 加 `#[cfg(any(test, feature="fault-injection"))] pub mod fault;`（仅 test/feature 下暴露） |
| `crates/vane-core/Cargo.toml` | 改 | `[features]` 加 `fault-injection = []`（dev/optional，默认不启用） |

**未触碰**：docs/SPEC.md、现有 pub API、CI yml、vane-wasm/Cargo.toml、
target/ 生成产物。

## 3. 单测清单与结果

`fault.rs` 内 `#[cfg(test)] mod tests`，共 8 个测试：

| # | 测试名 | 验证 | 结果 |
|---|---|---|---|
| 1 | `io_error_one_shot_consumed` | IoError one_shot=true：第 1 次 sync Err，第 2 次 Ok（规则已消费），规则表空 | ✅ |
| 2 | `trigger_on_nth_fires_on_nth` | trigger_on_nth=3：前 2 次 Ok，第 3 次 Err，第 4 次 Ok（已消费） | ✅ |
| 3 | `partial_write_writes_n_bytes_then_err` | PartialWrite bytes=8：write_at 32 字节 → Err；read_at → inner 恰好 8 字节 | ✅ |
| 4 | `enospc_returns_err_inner_unchanged` | Enospc：write_at 16 字节 → Err；read_at → inner 不变（4 字节基线） | ✅ |
| 5 | `path_matcher_star_and_prefix` | glob_match：`*` 通配（含 `/`）、前缀精确、多段 `*`、非匹配 | ✅ |
| 6 | `non_matching_path_passes_through_inner` | pattern 不匹配 path → inner 正常写读；规则仍在（未消费） | ✅ |
| 7 | `persistent_io_error_fires_every_call` | one_shot=false + trigger_on_nth=0：5 次调用全 Err；规则不移除 | ✅ |
| 8 | `rename_fault_blocks_and_inner_unchanged` | Rename IoError：rename → Err；inner 不变（target=旧，tmp 仍在） | ✅ |

**测试覆盖说明**：6 个为 brief 必需（#1-6），2 个为机器健壮性补充（#7 持久
故障、#8 manifest 原子切换 rename 路径——崩溃恢复测试 b 的关键前置）。

## 4. 各门禁真实输出

### 4.1 cargo fmt --all -- --check

```
$ cargo fmt --all -- --check
（无输出，退出码 0）
```

### 4.2 cargo clippy --all-targets --all-features -- -D warnings

```
$ cargo clippy --all-targets --all-features -- -D warnings
    Checking vane-core v0.2.0 (/Users/ximing/project/mygithub/vane/crates/vane-core)
    Checking vane-wasm v0.2.0 (/Users/ximing/project/mygithub/vane/crates/vane-wasm)
    Checking vane-ffi v0.2.0 (/Users/ximing/project/mygithub/vane/crates/vane-ffi)
    Checking vane-dict-zh v2026.8.0 (/Users/ximing/project/mygithub/vane/crates/vane-dict-zh)
    Checking vane-node v0.2.0 (/Users/ximing/project/mygithub/vane/crates/vane-node)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 8.72s
```

（修过一次 `clippy::doc_lazy_continuation`：check_fault doc 注释列表后
加空行分隔段落，复跑绿。）

### 4.3 cargo test -p vane-core --all-features

```
$ cargo test -p vane-core --all-features
     Running unittests src/lib.rs (target/debug/deps/vane_core-709c5df420ffac7b)
test result: ok. 321 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 1.20s

     Running tests/cold_start_gate.rs (... )
test result: ok. 0 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out

     Running tests/corpus_compat.rs (... )
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

     Running tests/hnsw_recall.rs (... )
test result: ok. 3 passed; 0 failed; 0 ignored

     Running tests/jieba_compat.rs (... )
test result: ok. 1 passed; 0 failed; 0 ignored

     Running tests/million_scale.rs (... )
test result: ok. 3 passed; 0 failed; 2 ignored

     Running tests/ndcg_wiki.rs (... )
test result: ok. 2 passed; 0 failed; 0 ignored

     Running tests/ndcg_wiki_zh.rs (... )
test result: ok. 3 passed; 0 failed; 0 ignored

     Running tests/pre_filter.rs (... )
test result: ok. 11 passed; 0 failed; 0 ignored

     Running tests/recall.rs (... )
test result: ok. 1 passed; 0 failed; 0 ignored

     Running tests/recall_fixture.rs (... )
test result: ok. 0 passed; 0 failed; 0 ignored

     Running tests/recall_regression.rs (... )
test result: ok. 7 passed; 0 failed; 0 ignored

     Running tests/text_persistence.rs (... )
test result: ok. 3 passed; 0 failed; 0 ignored

     Running tests/tombstone_merge.rs (... )
test result: ok. 9 passed; 0 failed; 0 ignored

     Running tests/userdict_reindex.rs (... )
test result: ok. 5 passed; 0 failed; 0 ignored

     Running tests/wal_crash.rs (... )
test result: ok. 9 passed; 0 failed; 0 ignored

   Doc-tests vane_core
test result: ok. 0 passed; 0 failed; 0 ignored
```

FaultVfs 8 测试全部可见：
```
test vfs::fault::tests::non_matching_path_passes_through_inner ... ok
test vfs::fault::tests::io_error_one_shot_consumed ... ok
test vfs::fault::tests::enospc_returns_err_inner_unchanged ... ok
test vfs::fault::tests::partial_write_writes_n_bytes_then_err ... ok
test vfs::fault::tests::persistent_io_error_fires_every_call ... ok
test vfs::fault::tests::path_matcher_star_and_prefix ... ok
test vfs::fault::tests::rename_fault_blocks_and_inner_unchanged ... ok
test vfs::fault::tests::trigger_on_nth_fires_on_nth ... ok
```

无回归（321 unit + 全集成绿）。

### 4.4 cargo check --target wasm32-unknown-unknown -p vane-core

```
$ cargo check --target wasm32-unknown-unknown -p vane-core
    Checking vane-core v0.2.0 (/Users/ximing/project/mygithub/vane/crates/vane-core)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.88s
```

FaultVfs 不泄漏进 wasm：wasm32 check 不设 cfg(test)、不启 fault-injection feature
（CI wasm32-check job 同命令无 --all-features），fault.rs 整模块不被编译。

### 4.5 bash scripts/check-no-std-fs.sh

```
$ bash scripts/check-no-std-fs.sh
OK
```

fault.rs 用 `std::sync::Mutex` / `std::collections::HashMap` / `std::thread::sleep`
（后者 `#[cfg(not(target_arch="wasm32"))]` 门控），无 `std::fs::` / `std::net::` / `mmap`。

## 5. commit hash

```
（见 git log——提交信息：feat(core): FaultVfs 故障注入 VFS + 单测（M4 阶段二 a））
```

## 6. 自审（待改进点）

1. **Fault enum 扩展了设计骨架字段**：设计 §3.1 骨架仅 IoError 有 `one_shot`，
   其余变体无。实现中给所有变体（IoError/PartialWrite/Enospc/Delay）均加
   `one_shot: bool` + `trigger_on_nth: u32`，以支持 brief 要求的
   "one_shot 消费 / trigger_on_nth 计数"对所有故障类型一致可用。
   理由：层 2 描述（`Fault::IoError { trigger_on_nth, ... }`）已示 trigger_on_nth
   为通用字段；one_shot 对所有类型一致化避免 PartialWrite/Enospc/Delay 的
   隐式消费语义歧义。偏离「按字面采用」约束，但属 brief 显式要求的
   机制实现所必需。

2. **call_counts 计数器不清理**：one_shot 故障移除后，其在 call_counts 的
   key 残留（小内存，测试无影响）。生产不启用此 feature，无泄漏风险。
   可改进：one_shot 移除时同步清理 call_counts entry，但增加复杂度，
   M4 不必需。

3. **first-fire-wins 的计数器副作用**：check_fault 遍历时，触发前的非触发
   匹配规则仍递增计数器。即：若规则 A（trigger_on_nth=3）在规则 B（fire）
   之前匹配，A 的计数器被递增但未触发。这是「Nth 匹配调用」的合理语义
   （每次匹配都算），但若 A 被 B 拦截，A 的计数器仍递增——可能影响
   后续 A 的触发时机判断。对 M4 测试场景（每测试通常只注入 1-2 条规则）
   无实际影响。

4. **Delay 在 wasm32 为 no-op**：`sleep_ms` 用 `#[cfg(not(target_arch="wasm32"))]`
   门控 `std::thread::sleep`。wasm32 无线程模型，Delay 故障在 wasm 下不产生
   实际延迟。FaultVfs 设计不进 wasm 生产，此为守护性 no-op，不影响正确性。

5. **未实现 LostWrite（按 brief 决策 2）**：崩溃恢复测试的「WAL flush 后」
   丢写场景后续用 `IoError{op:Sync,...}` 近似（非本任务范围）。fault.rs 留
   TODO 注释指向 phase0-design.md §3.1 取舍。
