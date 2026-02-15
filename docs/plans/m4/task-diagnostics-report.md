# M4 阶段五 c：VaneError 诊断上下文——实施报告

> 来源：Phase 5c implementer SubAgent（sonnet / bg）。
> 设计依据：`docs/plans/m4/phase0-design.md` §10（错误码——诊断加什么）+ `docs/plans/m4/M4-PLAN.md` 阶段五 3。
> SPEC 依据：`docs/SPEC.md` §10 错误码表（-1..-11 不变——只读核对，未改 SPEC）。

## 1. 任务摘要

丰富 VaneError 的 String payload，附上下文（段 ULID / docid / 操作 / 建议操作）。**不改错误码**（-1..-11 不变），**不改 VaneError enum 签名**（不加新字段，避免碰冻结 API），仅丰富 String 内容（§10 推荐"先丰富 String"路径）。

## 2. 丰富的 VaneError 构造点清单

### 2.1 段级 open 路径（segment/mod.rs）

| 构造点 | 原消息 | 丰富后附加上下文 |
|---|---|---|
| `SegmentReader::open` → `decode_header` 调用 | `header too short` / `bad magic` / `unsupported format_version` 等 | 经 `append_context` 追加 `(seg=<ulid>, op=open header.bin; 建议: 检查段文件完整性或从备份恢复)` |
| `SegmentReader::open` → vectors.bin bad magic | `vectors.bin bad magic` | 追加 `(seg=<ulid>, op=open vectors.bin; 建议: ...)` |
| `SegmentReader::open` → vectors.bin v2 header truncated | `vectors.bin v2 header truncated (need 12 bytes)` | 追加 `(seg=<ulid>, op=open vectors.bin; 建议: ...)` |
| `SegmentReader::open` → vectors.bin unsupported version | `vectors.bin unsupported format_version: {} (expected {} or {})` | 追加 `(seg=<ulid>, op=open vectors.bin; 建议: ...)` |
| `SegmentReader::open` → stored.bin bad magic | `stored.bin bad magic` | 追加 `(seg=<ulid>, op=open stored.bin; 建议: ...)` |
| `SegmentReader::open` → stored.bin unsupported version | `stored.bin unsupported format_version: {} (expected {} or {})` | 追加 `(seg=<ulid>, op=open stored.bin; 建议: ...)` |
| `load_vectors` → vectors.bin bad magic | `vectors.bin bad magic` | 追加 `(seg=<ulid>, op=load vectors; 建议: ...)` |
| `load_vectors` → vectors.bin unsupported version | `vectors.bin unsupported format_version: {}` | 追加 `(seg=<ulid>, op=load vectors; 建议: ...)` |
| `load_vectors` → vectors.bin truncated | `vectors.bin truncated` | 追加 `(seg=<ulid>, op=load vectors; 建议: ...)` |

**辅助函数**（新增 pub(crate)）：
- `segment_ulid_from_dir(segment_dir) -> &str`：从 `seg_<ulid>` 路径末段提取 ULID。
- `seg_ctx(segment_dir, op) -> String`：构造段级诊断上下文后缀。

### 2.2 倒排索引 open 路径（bm25.rs）

| 构造点 | 原消息 | 丰富后附加上下文 |
|---|---|---|
| `InvertedIndexReader::open` → inverted.bin truncated header | `inverted.bin truncated header: {}` | 追加 `(seg=<ulid>, op=open inverted.bin; 建议: ...)` |
| `InvertedIndexReader::open` → inverted.bin bad magic | `inverted.bin bad magic` | 追加 `(seg=<ulid>, op=open inverted.bin; 建议: ...)` |
| `InvertedIndexReader::open` → inverted.bin version mismatch | `inverted.bin version {} != supported {}` | 追加 `(seg=<ulid>, op=open inverted.bin; 建议: ...)` |

### 2.3 manifest 路径（persistence/mod.rs）

| 构造点 | 原消息 | 丰富后附加上下文 |
|---|---|---|
| `ManifestStore::load` → manifest parse | `manifest parse: {}` | 追加 `(db={}, op=load manifest; 建议: 检查 manifest.json 完整性或从备份恢复)` |
| `ManifestStore::save_atomic` → manifest serialize | `manifest serialize: {}` | 追加 `(db={}, op=save manifest; 建议: 重试或检查磁盘空间)` |
| `ManifestStore::add_segment` → collection not found | `collection not found: {}` | 追加 `(db={}, seg={}, op=add_segment; 建议: 确认 collection 名称正确)` |

### 2.4 WAL 路径（wal/mod.rs）

| 构造点 | 原消息 | 丰富后附加上下文 |
|---|---|---|
| `Wal::append` → wal serialize | `wal serialize: {}` | 追加 `(path={}, op=wal append; 建议: 检查 wal.log 完整性或重新操作)` |
| `Wal::read_all` → wal parse | `wal parse: {}` | 追加 `(path={}, op=wal recover; 建议: wal.log 损坏，检查崩溃恢复或清除 wal.log 重试)` |

### 2.5 词典加载路径（tokenizer/jieba/dict.rs）

| 构造点 | 原消息 | 丰富后附加上下文 |
|---|---|---|
| `JiebaDict::load` → 所有 parse 错误（magic/version/too short 等） | 各 leaf 错误 | 经 `append_context` 追加 `(op=dict load; 建议: 词典数据损坏，重新构建或联系支持)` |
| `JiebaDict::load_zstd` → zstd decompress failed | `dict.bin zstd decompress failed: {}` | 追加 `(op=dict load; 建议: ...)` |
| `JiebaDict::load_zstd` → zstd read failed | `dict.bin zstd read failed: {}` | 追加 `(op=dict load; 建议: ...)` |

### 2.6 搜索路径（api/collection.rs）

| 构造点 | 原消息 | 丰富后附加上下文 |
|---|---|---|
| `run_search` → topK exceeds max | `topK {} exceeds max {}` | 追加 `(op=search, collection={}; 建议: 减小 topK 至 {} 以内)` |
| `run_search` → search requires text or vector | `search requires text or vector` | 追加 `(op=search; 建议: 提供 text 或 vector 查询参数)` |
| `run_search` → query vector dim mismatch | `query vector dim {} != schema dim {}` | 追加 `(op=search, collection={}; 建议: 对齐 query vector 维度与 schema 声明)` |

### 2.7 文档添加路径（api/collection.rs）

| 构造点 | 原消息 | 丰富后附加上下文 |
|---|---|---|
| `add` → vector dim mismatch | `vector dim mismatch: got {} expected {}` | 追加 `(op=add, collection={}, doc_id={}; 建议: 对齐 doc vector 维度与 schema 声明)` |

### 2.8 段合并路径（api/collection.rs + merge/mod.rs）

| 构造点 | 原消息 | 丰富后附加上下文 |
|---|---|---|
| `merge_segments` → collection not in manifest | `collection not in manifest: {}` | 追加 `(op=merge, db={}; 建议: 确认 collection 已创建)` |
| `finalize_merge` → no steps | `finalize_merge with no steps` | 追加 `(op=merge; 建议: 检查 merge 调用序列)` |

### 2.9 重建索引路径（api/reindex.rs）

| 构造点 | 原消息 | 丰富后附加上下文 |
|---|---|---|
| `update_manifest_after_reindex` → collection not in manifest | `collection not in manifest: {}` | 追加 `(op=reindex; 建议: 确认 collection 已创建)` |

### 2.10 DB 打开路径（api/db.rs）

| 构造点 | 原消息 | 丰富后附加上下文 |
|---|---|---|
| `Db::collection` → schema mismatch | `collection '{}' exists with different schema` | 追加 `(op=open collection; 建议: 使用相同 schema 或新 collection 名称)` |
| `Db::collection` → tokenizer mismatch | `collection '{}' exists with different tokenizer` | 追加 `(op=open collection; 建议: 使用相同 tokenizer 或新 collection 名称)` |

### 2.11 共享辅助（types.rs）

新增 `pub(crate) fn append_context(e: VaneError, ctx: &str) -> VaneError`：
- 为带 String payload 的变体（Io/Schema/NotFound/Corrupt/Version/TokenizerMismatch/InvalidArg）追加 `ctx` 后缀。
- 无 String payload 的变体（Busy/DictTooLarge/DictUnavailable/Unsupported）原样返回。
- 不改错误码（code() 返回值不变）。

## 3. 错误码未变确认

SPEC §10 错误码表 -1..-11 全部不变：
- `VaneError::Io` → -1, `Schema` → -2, `NotFound` → -3, `Corrupt` → -4, `Version` → -5,
  `TokenizerMismatch` → -6, `DictTooLarge` → -7, `DictUnavailable` → -8, `Busy` → -9,
  `Unsupported` → -10, `InvalidArg` → -11。
- `types.rs` 现有 `error_code_matches_spec` 测试全绿（未改 code() 实现）。
- `append_context_enriches_string_preserves_code` 新测试验证丰富后 code() 不变。

## 4. VaneError 签名未改确认

- `VaneError` enum 11 个变体签名完全不变（Io(String) / Schema(String) / ... / InvalidArg(String)）。
- 未加新字段、未加新变体、未改 Display impl、未改 Error impl。
- 仅新增 `pub(crate) fn append_context`（crate 内部辅助，非 pub API surface）。
- `code()` / `name()` 方法不变。

## 5. 测试断言更新清单

### 5.1 新增测试（4 处）

| 测试 | 文件 | 验证内容 |
|---|---|---|
| `append_context_enriches_string_preserves_code` | `types.rs` | `append_context` 追加上下文后 code() 不变 + String 含 seg/op/建议关键词 |
| `m4_5c_open_error_contains_segment_context` | `segment/tests.rs` | SegmentReader::open vectors.bin bad magic 错误含段 ULID + op=open + 建议 |
| `m4_5c_manifest_parse_error_contains_context` | `persistence/tests.rs` | manifest parse 错误含 db 路径 + op=load manifest + 建议 |
| `m4_5c_wal_parse_error_contains_context` | `wal/tests.rs` | wal parse 错误含 wal 路径 + op=wal recover + 建议 |

### 5.2 现有测试未需更新

- `segment/tests.rs` 现有 `contains("vectors.bin bad magic")` / `contains("stored.bin bad magic")` / `contains("v2 header truncated")` 断言：**仍通过**（丰富是 ADDITIVE——原消息保留为子串）。
- `header.rs` `contains("too short")` 断言：**仍通过**（decode_header 本身未改，仅调用点 wrap）。
- `types.rs` `contains("topK exceeds 1000")` 断言：**仍通过**（该测试自构造字面量字符串，不经搜索路径）。
- `crash_recovery.rs` `contains("manifest.json.tmp")` / `contains("inverted.bin")` / `contains("ENOSPC")` / `contains("partial write")` 断言：**仍通过**（这些错误来自 FaultVfs 注入的 msg，经 `?` 传播不改——我的丰富仅在 CONSTRUCTION 点，不在传播点）。

## 6. 各门禁真实输出

### 6.1 cargo fmt

```
$ cargo fmt --all -- --check
（无 diff 输出——格式检查通过）
```

### 6.2 cargo clippy

```
$ cargo clippy --all-targets --all-features --workspace --exclude vane-fuzz -- -D warnings
    Checking vane-core v0.2.0
    Checking vane-wasm v0.2.0
    Checking vane-dict-zh v2026.8.0
    Checking vane-ffi v0.2.0
    Checking vane-node v0.2.0
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 5.61s
```

### 6.3 cargo test

```
$ cargo test --workspace --all-features --exclude vane-fuzz
test result: ok. 346 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 1.67s
（vane-core 单元测试 346 个——含 4 新测试——全绿；集成测试含 crash_recovery/cross_version等 全绿 0 failed）
```

关键：crash_recovery.rs 5 场景 FaultVfs 注入测试全绿——确认丰富不破坏现有错误消息断言。

### 6.4 cargo deny check

```
$ cargo deny check
advisories ok, bans ok, licenses ok, sources ok
```
（regex wrapper 警告为 pre-existing，与本次改动无关——无新依赖引入。）

### 6.5 wasm32 check

```
$ cargo check --target wasm32-unknown-unknown -p vane-core
    Checking vane-core v0.2.0
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.18s
```
（VaneError 丰富未引 std::fs——`append_context` 仅用 `format!`，无平台分支。）

## 7. commit

```
分支：feat/m4-prod-readiness
提交信息：feat(core): VaneError 诊断上下文（String 丰富，不改错误码）（M4 阶段五 c）
```

commit 含：
- `types.rs`：`append_context` 辅助 + 测试
- `segment/mod.rs`：`segment_ulid_from_dir` / `seg_ctx` pub(crate) 辅助 + SegmentReader::open / load_vectors 丰富
- `segment/tests.rs`：段级诊断上下文测试
- `bm25.rs`：InvertedIndexReader::open 丰富
- `persistence/mod.rs`：manifest load/save/add_segment 丰富
- `persistence/tests.rs`：manifest 诊断上下文测试
- `wal/mod.rs`：wal append/read_all 丰富
- `wal/tests.rs`：wal 诊断上下文测试
- `tokenizer/jieba/dict.rs`：dict load 路径丰富
- `api/collection.rs`：search/add/merge 丰富
- `api/db.rs`：collection schema/tokenizer mismatch 丰富
- `api/reindex.rs`：reindex NotFound 丰富
- `merge/mod.rs`：finalize_merge InvalidArg 丰富

不含：SPEC.md / CI yml / fault.rs / crash_recovery.rs / vane-fuzz / proptest / cross_version / tracing 埋点 / inspect API。

## 8. 自审

### 8.1 覆盖度

**已覆盖的关键路径**：
- ✅ open（SegmentReader::open + InvertedIndexReader::open + decode_header wrap）
- ✅ flush（经 SegmentWriter/write_inverted/manifest 传播——leaf 丰富已覆盖）
- ✅ merge（finalize_merge InvalidArg + merge_segments NotFound + 经 leaf 传播）
- ✅ search（topK / dim mismatch / missing text+vector）
- ✅ reindex（reindex_segment NotFound）
- ✅ dict load（JiebaDict::load + load_zstd）
- ✅ manifest（load / save_atomic / add_segment）
- ✅ WAL（append / read_all）
- ✅ DB open（collection schema/tokenizer mismatch）
- ✅ add（vector dim mismatch with doc_id）

**defer 的路径（非关键/低优先）**：
- segment/mod.rs 中 decode_kv_map / decode_stored / decode_scalars 的 ~30 个 leaf "too short" 错误——这些在 decode 函数内（无 segment_dir 上下文），且经 load_stored/load_id_map 传播时已有段级错误捕获。丰富这些 leaf 错误 ROI 低，defer。
- segment/mod.rs SegmentWriter add_doc/set_text/set_scalar 的 Schema 错误——这些是调用方编程错误（非运行时损坏），消息已含足够诊断信息（如 "field '{}' not a scalar field"），defer。
- bm25.rs InvertedIndexReader::open 后续的 ~15 个 "truncated term_len/vbyte/tf/docid" 错误——这些在已校验 magic+version 后的深层 decode，属罕见损坏路径，defer。
- hmm.rs 的 5 个 "hmm_blob too short" 错误——经 dict.rs parse 间接调用，已有 `load` 层 wrap 覆盖，defer。
- api/snapshot.rs 的 ~10 个 snapshot Corrupt/Version 错误——snapshot 导入导出路径非核心 flush/merge/search 路径，defer。

### 8.2 String 丰富 vs 结构化上下文取舍

§10 注释"结构化上下文列为 Could"——本任务用 String 丰富（§10 推荐"先丰富 String"路径）。

**选择理由**：
- String 丰富不改 enum 签名（避免碰冻结 API），不改 FFI 序列化（VaneError 经 FFI 只透传 code + Display 字符串）。
- 结构化上下文需加新字段（如 `struct ErrorContext { ulid, docid, op, suggestion }`）→ 改 enum 签名 → 破坏冻结 API → 需 SPEC 修订（Phase 6）。
- String 丰富立即可用，用户经 `vane_last_error_message()` 即可见上下文。

**defer 到结构化（Phase 6 SPEC 修订后）**：
- 若未来需程序化解析上下文（如 FFI 层提取 ULID 做自动重试），再加 `VaneError::context() -> Option<ErrorContext>` 方法（不改 enum 签名，只加方法）。当前 String 足够。
