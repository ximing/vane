# M4 诊断架构重构报告

## 概要

将 `VaneError` 诊断架构从 ADDITIVE String 拼接模式重构为**精简、结构化**的 `ErrorContext` 架构。所有 11 变体统一携带 `ErrorContext` struct，消费者可程序化访问 `seg`/`docid`/`op`/`hint` 字段，无需 parse Display 字符串。

错误码 -1..-11 + 名称 E_IO 等不变（SPEC §10 硬约束）。`code()`/`name()`/`Display` 行为保持。

## 架构设计

### ErrorContext struct

```rust
pub struct ErrorContext {
    pub message: String,           // 核心错误描述（必填）
    pub seg: Option<String>,       // 段 ULID
    pub docid: Option<u64>,        // 内部数值 docid
    pub op: Option<&'static str>,  // 操作名（flush/merge/search/open...）
    pub hint: Option<String>,      // 建议操作
}
```

- `ErrorContext::new(msg)` 构造 + `.seg()/.docid()/.op()/.hint()` builder 链式
- `From<String>`/`From<&str>` 让 `VaneError::Io("msg".into())` / `VaneError::Io(format!(...).into())` 低摩擦迁移

### VaneError enum — 11 变体统一

```rust
pub enum VaneError {
    Io(ErrorContext),
    Schema(ErrorContext),
    NotFound(ErrorContext),
    Corrupt(ErrorContext),
    Version(ErrorContext),
    TokenizerMismatch(ErrorContext),
    DictTooLarge(ErrorContext),   // 原无 payload → 现带 ErrorContext
    DictUnavailable(ErrorContext), // 原无 payload → 现带 ErrorContext
    Busy(ErrorContext),           // 原无 payload → 现能附 op 上下文
    Unsupported(ErrorContext),    // 原无 payload → 现带 ErrorContext
    InvalidArg(ErrorContext),
}
```

原 4 无 payload 变体 + 8 有 payload 变体的不一致消除。`Busy` 现在能附 `op` 上下文说"哪个操作冲突"。

### Rationale（为什么精简正确）

1. **结构化 > 字符串拼接**：消费者（绑定层/测试/日志）可直接访问 `ctx.seg`/`ctx.op` 字段，不需 parse Display 字符串。Node error.rs 的 `message()` 从 11-arm match 简化为 `e.context().message.clone()`。
2. **统一消除不一致**：所有变体同构，无"有 payload/无 payload"的二元分裂。`Busy` 能附上下文。
3. **builder 风格 > append_context**：`e.with_seg(ulid).with_op("open")` 比 `append_context(e, &format!(" (seg={}, op={}; ...)", ulid, op))` 精简且类型安全。
4. **Display 仍可读**：FFI `vane_last_error_message` 返回 `E_IO: msg [seg=... op=... hint=...]` 格式，结构化字段在方括号中。

### Display 新格式

```
E_IO: disk full [seg=01HABC op=flush hint=检查磁盘空间]
E_INVALID_ARG: bad arg
E_CORRUPT: header too short [seg=01HXYZ op=open header.bin hint=检查段文件完整性或从备份恢复]
```

- 无结构化字段时仅输出 `E_CODE: message`（无方括号）
- 有字段时追加 ` [seg=... op=... docid=... hint=...]`（None 字段省略）
- 零分配：直接流式写入 Formatter，sep 变量追踪前导空格

## 迁移的构造点

### 废弃 append_context

`pub(crate) fn append_context(e: VaneError, ctx: &str) -> VaneError` 完全移除。替代：
- `VaneError::with_seg()/with_op()/with_hint()/with_docid()` — pub(crate) 链式方法
- `ErrorContext::new(msg).seg(...).op(...).hint(...)` — builder 构造

### 废弃 seg_ctx

`pub(crate) fn seg_ctx(segment_dir: &str, op: &str) -> String` 替换为：
```rust
pub(crate) fn seg_err(message: impl Into<String>, segment_dir: &str, op: &'static str) -> ErrorContext
```
返回含 seg ULID + op + 默认 hint 的 ErrorContext。`segment_ulid_from_dir` 保留。

### 迁移的关键路径（~25 构造点 + ~60 leaf）

| 路径 | 旧模式 | 新模式 |
|---|---|---|
| SegmentReader::open | `format!("bad magic{}", seg_ctx(dir, "open"))` | `seg_err("bad magic", dir, "open")` |
| SegmentReader::load_vectors | 同上 | 同上 |
| InvertedIndexReader::open (bm25.rs) | 同上 | `crate::segment::seg_err(...)` |
| decode_header | `VaneError::Corrupt("header too short".into())` | 不变（From<&str> 适配） |
| Wal::append/read_all | `format!("wal parse: {} (op=wal recover; 建议: ...)", e)` | `ErrorContext::new(format!("wal parse: {}", e)).op("wal recover").hint(...)` |
| ManifestStore::load | `format!("manifest parse: {} (op=load manifest; 建议: ...)", e)` | `ErrorContext::new(format!("manifest parse: {}", e)).op("load manifest").hint(...)` |
| JiebaDict::load | `append_context(e, " (op=dict load; 建议: ...)")` | `e.with_op("dict load").with_hint(...)` |
| Collection::add (dim mismatch) | `format!("dim mismatch: ... (op=add, doc_id={}; 建议: ...)", ...)` | `ErrorContext::new(format!("dim mismatch: ... (doc_id={})", ...)).op("add").hint(...)` |
| Collection::search (topK/dim) | `format!("topK ... (op=search; 建议: ...)", ...)` | `ErrorContext::new(format!("topK ...")).op("search").hint(...)` |
| Db::collection (schema/tok mismatch) | `format!("... (op=open collection; 建议: ...)", ...)` | `ErrorContext::new(format!("...")).op("open collection").hint(...)` |
| compact/merge/reindex | `format!("... (op=merge; 建议: ...)", ...)` | `ErrorContext::new(format!("...")).op("merge").hint(...)` |
| bm25 "docid overflow" | `VaneError::Corrupt("docid overflow".into())` | `VaneError::Corrupt("docid overflow".into()).with_docid(prev_docid)` |
| Busy (7 处) | `VaneError::Busy` | `VaneError::Busy("reindex/compact in progress".into())` |
| DictTooLarge (4 处) | `VaneError::DictTooLarge` | `VaneError::DictTooLarge("user dict exceeds 100000 entries".into())` |
| DictUnavailable (2 处) | `VaneError::DictUnavailable` | `VaneError::DictUnavailable("jieba dict not loaded".into())` |
| Unsupported (2 处) | `VaneError::Unsupported` | `VaneError::Unsupported("platform capability missing".into())` |

### 绑定层迁移

- **vane-ffi**：~40 构造点加 `.into()`；`fail(VaneError::DictUnavailable)` → `fail(VaneError::DictUnavailable("msg".into()))`
- **vane-node**：`error.rs` 的 `message()` 从 11-arm match 简化为 `e.context().message.clone()`；`convert.rs` 的 `err_invalid_arg` 签名改 `impl Into<ErrorContext>`；测试断言更新
- **vane-wasm**：~15 构造点加 `.into()`；`matches!(err, VaneError::Unsupported)` → `matches!(err, VaneError::Unsupported(_))`

## 测试更新

### 诊断测试（Phase 5c → 结构化断言）

- **segment/tests.rs `m4_5c_open_error_contains_segment_context`**：从 String contains（`m.contains("seg=ULID")`）改为结构化断言（`ctx.seg == Some(ulid)` / `ctx.op == Some("open vectors.bin")` / `ctx.hint.is_some()`）
- **wal/tests.rs `m4_5c_wal_parse_error_contains_context`**：同上，改为 `ctx.op == Some("wal recover")` / `ctx.hint.is_some()`
- **persistence/tests.rs manifest context**：同上，改为 `ctx.op == Some("load manifest")` / `ctx.hint.is_some()`
- **types.rs `error_context_structured_fields_and_display`**（新，替代旧 `append_context_enriches_string_preserves_code`）：断言 builder 链式 + `with_*` 链式 + Display 格式 + `From<String>`/`From<&str>`

### crash_recovery.rs 断言

- 场景 5（line 561）：`Err(VaneError::Corrupt(ref msg)) if msg.contains("too short")` → `Err(VaneError::Corrupt(ref ctx)) if ctx.message.contains("too short")`
- 场景 1-4 的 Display contains 断言（`"manifest.json.tmp"`/`"inverted.bin"`/`"ENOSPC"`/`"partial write"`）**不需改动**——Display 仍包含 message，这些关键消息在 message 字段中保留

### error_code_matches_spec / error_name_matches_spec

- 单元变体从 `VaneError::DictTooLarge.code()` 改为 `VaneError::DictTooLarge("x".into()).code()`——code/name 值不变

### Node error.rs 测试

- `reason_round_trip_unsupported`：`CoreErr::Unsupported` → `CoreErr::Unsupported("platform capability missing".into())`；expected reason 从 `"-10:E_UNSUPPORTED:"` 改为 `"-10:E_UNSUPPORTED:platform capability missing"`
- `code_passthrough_not_remapped`：同上

### 模式匹配修复

所有 `matches!(x, VaneError::X)` (无 payload 变体) → `matches!(x, VaneError::X(_))`：
- tokenizer/mod.rs, tokenizer/jieba/mod.rs, reindex_tests.rs, stress_concurrency.rs, dict_tests.rs, vfs/idb.rs

## 全量门禁结果

| 门禁 | 结果 |
|---|---|
| `cargo fmt --all -- --check` | ✅ pass (exit 0) |
| `cargo clippy --workspace --all-targets --all-features --exclude vane-fuzz -- -D warnings` | ✅ pass (0 warnings) |
| `cargo test --workspace --all-features --exclude vane-fuzz` | ✅ all pass |
| `cargo check --target wasm32-unknown-unknown -p vane-core` | ✅ pass |
| `cargo check --target wasm32-unknown-unknown -p vane-wasm` | ✅ pass |
| `bash scripts/check-wasm-size.sh` | ✅ vane-wasm 364KB ≤ 800KB / core --export-all 653KB ≤ 800KB |
| `cargo deny check` | ✅ advisories ok, bans ok, licenses ok, sources ok |
| `cargo test -p vane-ffi` | ✅ 14/14 pass |
| `cd crates/vane-node && npm test` | ✅ 21/21 pass |

### crash_recovery 5/5

- crash_1_meta_slot_switch ✅
- crash_2_wal_flush ✅
- crash_3_merge_interrupted ✅
- crash_4_enospc_graceful_degradation ✅
- crash_5_partial_write ✅

## wasm 体积

- vane-wasm default: 364KB gzip（≤ 800KB）
- vane-core --export-all: 653KB gzip（≤ 800KB）

增量评估：去 String 拼接（`seg_ctx` format! 后缀）+ 加 struct 字段（Option），净增量可忽略。Display 的流式写入避免 Vec 分配。

## Concerns

1. **`with_docid` 字段类型 `u64`**：外部 doc ID 是 `String`，`docid` 字段仅用于内部数值 docid（如 bm25 "docid overflow" 路径）。外部 doc ID 保留在 message 中（如 `format!("... (doc_id={})", doc.id)`）。
2. **`op` 字段类型 `&'static str`**：操作名均为编译期常量（"flush"/"merge"/"search"/"open header.bin" 等），不支持运行时动态字符串。如未来需动态 op，可改为 `Cow<'static, str>`。
3. **FFI `vane_last_error_message`**：Display 格式变更（结构化字段在 `[...]` 中），向后不兼容但用户授权不考虑兼容性。FFI 测试 14/14 绿，Node 测试 21/21 绿——reason 编码 `{code}:{name}:{msg}` 不含 `[...]` 部分（`message()` 取 `ctx.message`）。
4. **vane-fuzz 排除**：vane-fuzz 的 libfuzzer import 错误是预存环境问题，与本次重构无关。CI 用 `--exclude vane-fuzz`。

## Commit

- 主重构：`c34e473`
- fix（persistence 2 处真结构化 + report hash + 注释）：`7c5dd6d`
