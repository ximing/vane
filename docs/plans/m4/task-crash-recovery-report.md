# M4 阶段二 b：crash_recovery 测试报告

> 产出：`crates/vane-core/tests/crash_recovery.rs`（5 场景 FaultVfs 注入崩溃恢复测试）
> 前置：FaultVfs（阶段二 a，commit 03319ca）已实现 + 审查通过。
> 日期：2026-08-11

## 1. 5 场景实现摘要

### 场景 1：meta_slot 翻转崩溃

- **FaultVfs 规则**：`IoError{op:Sync, path:"*/manifest.json.tmp", one_shot:true, trigger_on_nth:0}`
- **注入点位**：`ManifestStore::save_atomic` 的 `sync(manifest.json.tmp)` 步骤（persistence/mod.rs:109）
- **机制**：第一批 flush 成功（基线段 A + manifest 切换）；注入故障后第二批 flush → `save_atomic` 的 sync(tmp) 失败 → 不执行 rename → manifest 未切换。WAL 已有 AddSegment(B) 但 manifest 不含 B → recover 清理孤儿段 B。
- **断言不变量**：
  - flush 返回 Err 且错误信息含 `manifest.json.tmp`
  - manifest 未切换：`segment_count == 1`（save_atomic 失败 → snapshot 未更新）
  - 重开后：第一批 d0-d2 可见（基线数据完好）
  - 重开后：第二批 d3-d5 不可见（flush 失败，段为孤儿被清理）

### 场景 2：WAL flush 崩溃

- **FaultVfs 规则**：`IoError{op:Append, path:"*/wal.log", one_shot:true, trigger_on_nth:0}`
- **注入点位**：`Wal::append` 的 `vfs.append(path, line)` 步骤（wal/mod.rs:69）
- **机制**：flush 5 文档后，delete d0（WAL append 成功 = 已确认）；注入故障后 delete d1 → WAL append 失败 → delete 返回 Err → 内存位图未更新。WAL 有 AddTombstone(d0) 无 AddTombstone(d1)。
- **断言不变量**：
  - delete d1 返回 Err（WAL append 失败）
  - d1 仍在内存中可见（位图未更新）
  - 重开后：d0 被删除（已确认事务从 WAL 重放）
  - 重开后：d1 仍可见（未确认事务不在 WAL，不重放）
  - 重开后：恰好 4 个活文档（d0 删除，d1-d4 活）

### 场景 3：merge 中断崩溃

- **FaultVfs 规则**：`IoError{op:WriteAt, path:"*/segments/seg_*/inverted.bin", one_shot:true, trigger_on_nth:0}`
- **注入点位**：`finalize_merge` 的 `write_inverted` 调用（merge/mod.rs:287）
- **机制**：两次 flush 产生 2 段 + delete d0；注入故障后 compact → finalize_merge 的 write_inverted 失败 → compact 返回 Err → save_atomic 未执行 → manifest 未切换。新段（半成品 inverted.bin 缺失）为孤儿。
- **断言不变量**：
  - compact 返回 Err 且错误信息含 `inverted.bin`
  - manifest 未切换：`segment_count == 2`（旧段保留）
  - 重开后：2 段保留，d0 被删除（tombstone 从 WAL 重放），d1-d5 可见（5 活文档）
  - compact 可重试（one_shot 故障已消费）：重试后 `segment_count == 1`，5 活文档不变

### 场景 4：ENOSPC

- **FaultVfs 规则**：`Enospc{op:WriteAt, path:"*", one_shot:true, trigger_on_nth:0}`
- **注入点位**：`SegmentWriter::finalize` 的首个 `write_at` 调用
- **机制**：第一批 flush 成功；注入 ENOSPC 后第二批 flush → finalize 首个 write_at 返 ENOSPC → flush 返回 Err → save_atomic 未执行。
- **断言不变量**：
  - flush 返回 Err 且错误信息含 `ENOSPC`（可操作错误信息）
  - 已有数据不损：search 正常返回 d0-d2（基线段完好）
  - d3-d5 不可见（flush 失败）
  - 重开后：1 段，3 文档（基线数据一致）
  - 段目录无孤儿（ENOSPC 在 finalize 首个 write_at 即失败，recover 清理）

### 场景 5：部分写

- **FaultVfs 规则**：`PartialWrite{op:WriteAt, path:"*/header.bin", bytes_before_fail:8, one_shot:true, trigger_on_nth:0}`
- **注入点位**：`SegmentWriter::finalize` 的 `write_at(header.bin, hbytes, 0)` 调用（segment/mod.rs:300）
- **机制**：第一批 flush 成功；注入 PartialWrite 后第二批 flush → finalize 的 header.bin write_at 写 8 字节（magic "VANE" + version 1）后失败 → finalize 返回 Err → flush 返回 Err → manifest 未切换。损坏段（header.bin 仅 8 字节）为孤儿。
- **断言不变量**：
  - flush 返回 Err 且错误信息含 `partial write`
  - manifest 未切换：`segment_count == 1`
  - 损坏 header.bin 恰好 8 字节：magic `b"VANE"` + `format_version=1`（LE）
  - 重开后：孤儿段被 recover 清理，段目录仅 1 个（基线段）
  - 重开后：3 文档可见（d0-d2），d3-d5 不可见

## 2. 文件清单

| 文件 | 操作 | 说明 |
|---|---|---|
| `crates/vane-core/tests/crash_recovery.rs` | 新增 | 5 场景集成测试，`#![cfg(feature="fault-injection")]` 门控 |
| `crates/vane-core/Cargo.toml` | 未改 | 无需加 tempfile dev-dep（全用 MemoryVfs） |
| `crates/vane-core/src/vfs/fault.rs` | 未改 | FaultVfs API 充分，无阻塞 |

## 3. 各门禁真实输出

### 3.1 `cargo fmt --all -- --check`
```
（无输出 = 通过）
```

### 3.2 `cargo clippy --all-targets --all-features -- -D warnings`
```
    Checking vane-core v0.2.0
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.98s
（无 warning = 通过）
```

### 3.3 `cargo test -p vane-core --all-features --test crash_recovery`
```
running 5 tests
test crash_1_meta_slot_switch ... ok
test crash_2_wal_flush ... ok
test crash_4_enospc_graceful_degradation ... ok
test crash_5_partial_write ... ok
test crash_3_merge_interrupted ... ok

test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.47s
```

### 3.4 `cargo test -p vane-core --all-features --tests`（无回归）
```
（全部集成测试 + 单元测试通过，无 FAILED。crash_recovery 5 passed，wal_crash 9 passed，无回归）
```

### 3.5 `cargo test -p vane-core --test crash_recovery`（默认 features，门控验证）
```
running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```
门控正确：无 `fault-injection` feature 时文件编译为空，0 测试不报错。

### 3.6 `cargo check --target wasm32-unknown-unknown -p vane-core`
```
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.08s
```
tests/ 不进 wasm 编译，无影响。

## 4. Commit

- 分支：`feat/m4-prod-readiness`
- Hash：见 `git log -1` 输出
- 信息：`test(core): crash_recovery 5 场景 FaultVfs 注入（M4 阶段二 b）`
- 变更：仅 `crates/vane-core/tests/crash_recovery.rs`（新增）
- 无 `Co-Authored-By` trailer

## 5. 自审

### FaultVfs API 评估

FaultVfs API 充分覆盖 5 场景需求，无阻塞：
- `IoError{op, path_pattern, msg, one_shot, trigger_on_nth}` 精确注入 IO 错误——场景 1/2/3 使用
- `Enospc{op, path_pattern, one_shot, trigger_on_nth}` ENOSPC 模拟——场景 4 使用
- `PartialWrite{op, path_pattern, bytes_before_fail, one_shot, trigger_on_nth}` 部分写——场景 5 使用
- glob `*` 通配 path matcher 自研轻量实现，不引 regex 黑名单
- `one_shot` + `trigger_on_nth:0` 组合精确控制"首次命中即触发并消费"
- `check_fault` 在调 inner 前执行 → 返错时 inner 状态不变

### 待改进点

1. **`decode_header` 8 字节 panic**：`header.rs::decode_header` 在 `buf.len() == 8` 时越过 `< 8` 长度门但访问 `buf[8]`（ulid_len）导致 index-out-of-bounds panic，而非返回 `VaneError::Corrupt`。场景 5 的 PartialWrite 写恰好 8 字节，测试中未直接调 `SegmentReader::open` 验证拒绝（避免 panic），改用 Vfs 直读 header.bin + 验证 8 字节 magic+version + recover 孤儿清理间接验证。建议后续将 `< 8` 改为 `< 9` 修复此 latent bug（属 segment 模块，不在本任务范围）。

2. **`hit_ids` 未使用**：初版有 `hit_ids` 辅助函数收集 hit ID 列表，实际测试改用 `contains_id` 判断，`hit_ids` 被 clippy `-D dead_code` 捕获后移除。

3. **gen_qrels 示例预存问题**：`cargo test -p vane-core`（不带 `--tests`）在 `examples/gen_qrels.rs` 报 `main function not found`——此为预存问题（git stash 验证），与本任务无关，不影响 `--tests` 或 `--all-features --tests` 路径。

---

## 6. Fix 循环 round 1（reviewer findings I-1 + I-2 + M-2）

> 来源：opus reviewer 回报（2 Important + 2 Minor），复派 implementer 修复。
> 日期：2026-08-11

### 6.1 修了什么

| Finding | 级别 | 修复 |
|---|---|---|
| **I-2** | Important | `segment/header.rs:39`：`decode_header` off-by-one——`if buf.len() < 8` 改为 `< 9`。原门限允许 `buf.len()==8`，但 line 52-53 `let pos=8; let ulid_len=buf[pos]`（即 `buf[8]`）越界 **panic**（非 `VaneError::Corrupt`）。改为 `< 9` 后 8 字节 header 返 `Corrupt("header too short")`，不再 panic。 |
| **I-1** | Important | `crash_recovery.rs` 场景 5：加直接 `decode_header` Corrupt 断言。修 I-2 后，场景 5 读到的 8 字节损坏 header 可安全调 `decode_header`（不再 panic），断言 `matches!(decode_result, Err(VaneError::Corrupt(ref msg)) if msg.contains("too short"))`。覆盖 decode_header 拒绝路径。 |
| **M-2** | Minor | `crash_recovery.rs` 场景 5 注释校准：原"8 字节恰好过长度门但缺 ulid_len → 无效段"暗示 panic，改为明确"decode_header 长度门 `< 9` 拒绝 8 字节，返 Corrupt（非 panic）"。 |
| M-1 | Minor | defer（场景 1 I16 tmp 清理断言，不强求）。 |

### 6.2 覆盖测试清单

| 测试 | 文件 | 验证内容 |
|---|---|---|
| `crash_5_partial_write` | `tests/crash_recovery.rs` | 场景 5 新增 `decode_header(&buf[..8])` Corrupt 断言（I-1） |
| `decode_header_8_bytes_returns_corrupt_not_panic` | `segment/header.rs #[cfg(test)]` | 防回归单测：8 字节 header（magic+version）返 `Corrupt("header too short")` 而非 panic（I-2 回归守护） |

### 6.3 各门禁真实输出

#### `cargo fmt --all -- --check`
```
（无输出 = 通过）
```

#### `cargo clippy --all-targets --all-features -- -D warnings`
```
    Checking vane-core v0.2.0
    Checking vane-wasm v0.2.0
    Checking vane-dict-zh v2026.8.0
    Checking vane-node v0.2.0
    Checking vane-ffi v0.2.0
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 9.18s
（无 warning = 通过）
```

#### `cargo test -p vane-core --all-features --test crash_recovery`
```
running 5 tests
test crash_1_meta_slot_switch ... ok
test crash_2_wal_flush ... ok
test crash_3_merge_interrupted ... ok
test crash_4_enospc_graceful_degradation ... ok
test crash_5_partial_write ... ok

test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.49s
```

#### `cargo test -p vane-core --all-features --lib segment::header::tests`（防回归单测）
```
running 1 test
test segment::header::tests::decode_header_8_bytes_returns_corrupt_not_panic ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 322 filtered out; finished in 0.00s
```

#### `cargo test -p vane-core --all-features --tests`（无回归）
```
（322 单元测试 + 全部集成测试通过，无 FAILED。crash_recovery 5 passed，wal_crash 9 passed，header 回归 1 passed）
```

#### `cargo check --target wasm32-unknown-unknown -p vane-core`
```
    Checking vane-core v0.2.0
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.84s
```
header.rs 改动（`< 8` → `< 9`）是普通 fn 内逻辑变更，无 std::fs，不影响 wasm 编译。

### 6.4 Commit

- 分支：`feat/m4-prod-readiness`
- Hash：`acbd23d`
- 信息：`fix(segment): decode_header off-by-one (< 8 → < 9) + crash_recovery 场景 5 Corrupt 断言（M4 阶段二 b fix r1）`
- 变更：`segment/header.rs`（+29 -4）+ `tests/crash_recovery.rs`（+5 -1）= 2 files, 34 insertions, 5 deletions
- 无 `Co-Authored-By` trailer

### 6.5 自审

- **I-2 修复彻底性**：`< 9` 门限覆盖 `buf.len() ∈ {0..8}` 全部返 Corrupt("header too short")，`buf.len()==8` 不再 panic。防回归单测 `decode_header_8_bytes_returns_corrupt_not_panic` 守护。
- **I-1 覆盖路径**：场景 5 在 PartialWrite 写 8 字节后直接调 `decode_header(&buf[..8])`，断言 `Corrupt("header too short")`。此前因 I-2 panic 避开直接调用，现已安全覆盖。
- **SegmentMeta 无 Debug derive**：初版 `{:?}` 格式化 `decode_result: Result<SegmentMeta, VaneError>` 触发 clippy `E0277`（SegmentMeta 无 Debug）。改为断言消息不含 `{:?}` + decode_result，消除依赖。不改 M0 冻结 pub struct（不加 derive）。
- **M-1 defer**：场景 1 I16 tmp 清理仅注释无断言——不影响正确性，defer 到后续。
