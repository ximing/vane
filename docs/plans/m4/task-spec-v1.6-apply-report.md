# task-spec-v1.6-apply 报告

**日期**：2026-08-13
**BASE**：`e38fbb9`
**任务**：6b-apply——把用户已批准的 SPEC v1.6 修订应用到 `docs/SPEC.md`，同步更新草案 + 修两个已知小瑕疵。

## 1. SPEC.md 改动清单

### 标题 bump
- 第 1 行：`# Vane 技术规范（SPEC v1.5）` → `# Vane 技术规范（SPEC v1.6）`

### §9.2 函数面——v1.6 补列块（第 325 行起）
在 v1.1 补列块之后、参数/返回 JSON 注释之前，加 **v1.6 补列**块：
- `vane_db_stats(db_h, out_arena*) -> i32` — DbStats JSON
- `vane_db_segment_info(db_h, out_arena*) -> i32` — Vec<SegmentInfo> JSON
- 实现状态注：core 层 [M4, `684a112`] + FFI/Node/Wasm 三绑定层 [M4, `5143885`] 全实现；7 structs 概要；健康检查语义（Corrupt/Degraded/Healthy）；`index_bytes`/`file_sizes` 用 `read_at` 探测 EOF 推算。

### §10 错误码——v1.6 注（第 359 行起）
在"三侧绑定透传 code，不得吞并/重编"之后加 v1.6 注：
- `VaneError` 11 变体统一携带 `ErrorContext` struct（message + seg + docid + op + hint）
- `ErrorContext` builder 链式 `.seg()`/`.docid()`/`.op()`/`.hint()` + `From<String>`/`From<&str>`
- `VaneError` `with_seg()`/`with_docid()`/`with_op()`/`with_hint()` pub(crate) 替代旧 `append_context`
- `context()` pub 返回 `&ErrorContext`
- **错误码 -1..-11 + 名称不变**（本表硬约束）
- Display 格式 `E_CODE: message [seg=... op=... docid=... hint=...]`（None 省略）
- 实现：`c34e473` + `d9dcc5f`

### §13.2 质量门禁——新增第 6-11 项（第 434-439 行）
- 6. fuzz-smoke [M4]（`b4aa743`）
- 7. fuzz-long [M4]（`b4aa743`）
- 8. 崩溃恢复 [M4]（`b4aa743`）
- 9. 跨版本兼容 [M4]（`b4aa743`）
- 10. 并发压测 [M4]（`b4aa743`）
- 11. proptest 不变量 [M4]（`f849c7b` + `34a9b11`）

### §13.3 工程纪律门禁——v1.6 注（第 446 行起）
在 cargo-deny 行之后加 v1.6 注：dev/optional 依赖（tracing/proptest/cargo-fuzz/libfuzzer-sys）不触运行时黑名单；libfuzzer-sys NCSA license 已 allow（`9e262db`）。

### §14 I-5 不变量——v1.6 tracing 扩展（第 461 行起）
在 I-5 现有注释之后加 v1.6 注：`cfg(feature="tracing")` 可观测性能力开关，编译期消除，体积不变，tracing crate 传递依赖不触黑名单（`dae29c6`）。

### Changelog——v1.6 条目（第 486 行）
在 v1.5 条目之后追加 v1.6 条目，覆盖 S1-S4 四节修订 + 不触碰范围。

## 2. spec-v1.6-draft.md §9/§10 重写摘要

### §9 节重写
- 概览表 §9 行："core 层已实现 [M4]，FFI/Node/Wasm 绑定层顺延" → "core + FFI/Node/Wasm 全实现 [M4]"
- v1.6 补列块标题："core 层已实现，FFI/Node/Wasm 绑定顺延" → "core + FFI/Node/Wasm 全实现"
- 实现状态 blockquote：去掉"顺延"标注，改为 FFI/Node/Wasm 全实现 [M4, `5143885`]
- rationale：去掉"但实际 Phase 5b 只实现 core 层——FFI/Node/Wasm 绑定层 inspect 落地顺延"，改为"实际实现与设计一致，FFI/Node/Wasm 全部落地"
- 7 structs 字段概要 + 健康检查语义保留不变
- 偏差记录表 §9 行："仅 core 层实现，FFI/Node/Wasm 绑定未实现 | 诚实标注顺延" → "core + FFI/Node/Wasm 全实现（684a112+5143885）| 与设计一致，全实现"

### §10 节重写
- 概览表 §10 行："String payload ADDITIVE，错误码 -1..-11 不变" → "ErrorContext 结构化字段替代 String 拼接，错误码 -1..-11 不变"
- v1.6 注 block：从"ADDITIVE String payload + append_context helper（`5fc4ac4`）"重写为"ErrorContext struct + builder 链式 + with_* pub(crate) + context() pub + Display 新格式（`c34e473`+`d9dcc5f`）"
- rationale：从"先丰富 String，结构化列 Could（非本次）"改为"实际实现超越设计提议，直接结构化落地"
- 不再列"结构化 context() 列 Could（非本次）"——已直接结构化落地
- 错误码 -1..-11 + 名称不变的硬约束保留

### 其他 draft 更新
- 标题："待用户批准" → "用户已批准"
- 版本说明：更新为"用户已批准（2026-08-12），§9/§10 节已根据最终实现更新"
- changelog 草案条目：§9 改"FFI/Node/Wasm 全实现（5143885）"非顺延；§10 改"ErrorContext 结构化（c34e473+d9dcc5f）"非 ADDITIVE
- 检查点：加"用户已批准"resolution note（Q1 立即实现 / Q2 ErrorContext 结构化 / Q3 全批准 / Q4 v1.6 一次性）

## 3. inspect.rs §3.6→§9.2 修正（3 处）

`crates/vane-core/src/api/inspect.rs` 中 3 处引用不存在的"§3.6"（SPEC §3 只有 3.1/3.2/3.3，无 §3.6），改为引用 §9.2 inspect API 节：

1. **第 6 行**（模块 doc）：`健康检查（§3.6 表）：` → `健康检查（§9.2 inspect API）：`
2. **第 15 行**（模块 doc）：`inspect 非热路径，性能可接受（§3.6 取舍）。` → `inspect 非热路径，性能可接受（§9.2 取舍）。`
3. **第 228 行**（函数注释）：`注：§3.6 取舍建议"不主动重新 open 校验"，但表 spec 要求"SegmentReader::open 失败 → Corrupt"。` → `注：§9.2 inspect API 要求"SegmentReader::open 失败 → Corrupt"（健康检查语义）。`

仅改注释，不改代码逻辑。

## 4. proptest commit hash 修正

草案 §13.2 第 11 项 proptest 原标 commit `f793e93`（实为 Phase 4 docs commit，非 proptest 实现）。SPEC.md §13.2-11 + draft changelog 修正为：
- `f849c7b`（proptest 3 不变量主实现）
- `34a9b11`（proptest 不变量 1 加非空 guard fix）

已验证：`git show --stat f849c7b` = `tests/proptest_invariants.rs` 421 行新增；`git show --stat 34a9b11` = proptest fix r1。

## 5. 验证结果

```
=== Remaining §3.6 in inspect.rs ===
(none, grep exit 1)

=== cargo check -p vane-core ===
Checking vane-core v0.2.0
Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.89s

=== cargo fmt --all -- --check ===
(exit 0, no diff)
```

- `cargo check -p vane-core`：通过（inspect.rs 注释改动未影响编译）
- `cargo fmt --all -- --check`：通过（inspect.rs 注释改动未破 fmt）
- SPEC.md / draft 无残留"顺延"（§9）/ "ADDITIVE"（§10）/ `5fc4ac4`（旧 commit）/ `f793e93`（错误 proptest commit）

## 6. 改动文件清单

| 文件 | 改动类型 |
|---|---|
| `docs/SPEC.md` | 标题 bump v1.5→v1.6 + §9.2/§10/§13.2/§13.3/§14 四节修订 + changelog v1.6 条目 |
| `docs/plans/m4/spec-v1.6-draft.md` | §9/§10 节重写 + 概览表 + changelog + 偏差记录 + 检查点 resolution |
| `crates/vane-core/src/api/inspect.rs` | 3 处注释 §3.6→§9.2（仅注释，不改代码） |

## 7. commit

```
docs(spec): SPEC v1.6 apply——§9 inspect + §10 ErrorContext + §13.2 门禁 + §14 tracing（M4 6b-apply）
```

apply 后 SPEC.md = v1.6，M4 四节修订正式写入规范。
