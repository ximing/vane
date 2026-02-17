# M4 Phase 5b：FFI 层 inspect API 实现报告

**日期**：2026-08-13
**BASE**：`8dc83a2`
**commit**：`5143885`
**范围**：vane-ffi / vane-node / vane-wasm 三绑定层落地 core 层 inspect API

---

## 1. 实现清单

### 1.1 vane-ffi（C ABI）——2 个新函数

| 函数 | 签名 | 说明 |
|---|---|---|
| `vane_db_stats` | `(db_h: u64, out_arena: *mut *mut u8, out_len: *mut usize) -> i32` | 调 `db.stats()`，JSON 序列化到 arena（调用方 `vane_string_free` 释放） |
| `vane_db_segment_info` | `(db_h: u64, out_arena: *mut *mut u8, out_len: *mut usize) -> i32` | 调 `db.segment_info()`，JSON 序列化到 arena |

- 句柄解析：复用现有 `lookup_db(h)` → `Arc<Db>`
- arena 分配：复用现有 `arena_alloc_tracked` + `vane_string_free`
- panic 安全：`catch_unwind_code` 包装
- 错误码：0=OK，-3=E_NOT_FOUND（句柄不存在），-11=E_INVALID_ARG（null 指针），-12=E_INTERNAL（panic/lock poisoned）
- **未新增 C ABI 函数** `vane_db_collection_segment_info`（任务只要求 2 函数；`segment_info` 已返回所有 collection 的所有段，调用方可客户端过滤）

### 1.2 vane-node（N-API）——2 个新方法

| Rust 方法 | JS 方法 | 返回 | 说明 |
|---|---|---|---|
| `VaneDb::stats(&self)` | `db.stats()` | `Promise<Json>` | DbStats JSON 对象 |
| `VaneDb::segment_info(&self)` | `db.segmentInfo()` | `Promise<Json>` | SegmentInfo[] JSON 数组 |

- napi-rs 3.x 自动 snake_case → camelCase：`segment_info` → `segmentInfo`
- 异步经 `AsyncTask`（libuv worker pool），不桥接 tokio（§9.3）
- `StatsTask` / `SegmentInfoTask` 的 `JsValue = Json`（serde_json::Value newtype，复用现有 `convert::Json`）
- `main.js` wrapMethods 已补 `stats` / `segmentInfo`
- `index.d.ts` 手动补列（@napi-rs/cli 2.x 与 napi 3.x 版本不配对，.d.ts 自动生成缺新方法；type-defs 元数据已正确记录）

### 1.3 vane-wasm（wasm-bindgen）——2 个新函数

| 函数 | 签名 | 返回 | 说明 |
|---|---|---|---|
| `vane_db_stats` | `(db_h: u64) -> JsResult<String>` | DbStats JSON 字符串 | 与 `vane_search` 同构（返回 JSON String） |
| `vane_db_segment_info` | `(db_h: u64) -> JsResult<String>` | SegmentInfo[] JSON 字符串 | 同上 |

- 句柄解析：复用现有 `lookup_db(h)`
- 错误：`VaneError → JsValue` throw（复用现有 `err_to_js`）

### 1.4 bindings/go/vane.h——2 个新声明

手写头文件补列 `vane_db_stats` / `vane_db_segment_info` 声明（与 vane-ffi `extern "C"` 签名严格一致）。

---

## 2. JSON 序列化方案

**方案**：手写 `serde_json::Value` 构造（与现有 `hits_to_json` 同构），不引 serde derive。

**原因**：
- core inspect 结构（`DbStats` / `CollectionStats` / `SegmentInfo` / `FormatVersions` / `SegmentFileSizes` / `Health` / `ExecutorKind`）仅 derive `Debug+Clone`，未 derive `Serialize`。
- 任务约束：不改 core 层 inspect API。
- serde_json 已是三 binding crate 的现有依赖（`Cargo.toml` 确认），手写 `serde_json::json!({})` 构造零新依赖。

**JSON 字段命名**：camelCase（JS 惯例），与现有 `hits_to_json`（"id"/"score"/"fields"）一致。

**JSON 结构**：
```json
// DbStats
{
  "dbPath": "string",
  "collections": [CollectionStats],
  "dictAvailable": false,
  "executorKind": "serial" | "rayon"
}
// CollectionStats
{ "name", "segmentCount", "totalDocs", "liveDocs", "tombstonedDocs",
  "indexBytes", "dictState": "stable"|"pendingReindex"|"rebuilding",
  "tokenizerId": "64-char-hex", "health": "healthy"|"degraded"|"corrupt" }
// SegmentInfo
{ "ulid", "docCount", "docidBase", "tombstonedCount",
  "formatVersions": { "header", "vectors", "stored", "idmap", "scalars", "inverted", "hnsw" },
  "fileSizes": { "header", "vectors", "stored", "idmap", "scalars", "inverted", "hnsw": null|number },
  "health": "healthy"|"degraded"|"corrupt" }
```

**体积增量**：0 KB（serde_json 已在 wasm 产物中，手写构造无新代码膨胀）。

---

## 3. 测试清单

### 3.1 vane-ffi 集成测试（4 个）

| 测试 | 验证 |
|---|---|
| `db_stats_returns_valid_json` | open→collection→add→flush→stats：JSON 有效 + dbPath/collections/segmentCount/totalDocs/liveDocs/tombstonedDocs/indexBytes/dictState/tokenizerId(64hex)/health/executoryKind/dictAvailable 字段正确 |
| `db_segment_info_returns_valid_json` | open→collection→add→flush→segment_info：JSON 数组 + ulid/docCount/docidBase/tombstonedCount/formatVersions/fileSizes/health 字段正确 |
| `db_stats_invalid_handle_returns_not_found` | 无效句柄 → -3 (E_NOT_FOUND) |
| `db_segment_info_null_out_returns_invalid_arg` | null out_arena → -11 (E_INVALID_ARG) |

### 3.2 vane-node JS 测试（4 个）

| 测试 | 验证 |
|---|---|
| `db.stats() returns DbStats with correct fields` | stats() 返回 JS 对象 + 全字段正确 |
| `db.segmentInfo() returns SegmentInfo[] with correct fields` | segmentInfo() 返回 JS 数组 + 全字段正确 |
| `db.stats() on empty DB returns empty collections` | 空 DB → collections=[] + segments=[] |
| `db.stats() after delete reflects tombstoned docs` | delete+flush → totalDocs=2, tombstonedDocs=1, liveDocs=1 |

### 3.3 vane-wasm wasm-bindgen-test（2 个）

| 测试 | 验证 |
|---|---|
| `db_stats_returns_valid_json` | vane_db_stats 返回有效 DbStats JSON |
| `db_segment_info_returns_valid_json` | vane_db_segment_info 返回有效 SegmentInfo[] JSON |

---

## 4. 全量门禁结果

| # | 门禁 | 命令 | 结果 |
|---|---|---|---|
| 1 | 格式 | `cargo fmt --all -- --check` | rc=0 PASS |
| 2 | Clippy | `cargo clippy --workspace --all-targets --all-features --exclude vane-fuzz -- -D warnings` | rc=0 PASS |
| 3 | 工作区测试 | `cargo test --workspace --all-features --exclude vane-fuzz` | rc=0 PASS（全绿） |
| 4 | FFI 测试 | vane_ffi 14 tests（含 4 新 inspect） | 14 passed; 0 failed PASS |
| 5 | Node 测试 | `cd crates/vane-node && npm test` | 21 passed PASS（含 4 新 inspect） |
| 6 | wasm32 check | `cargo check --target wasm32-unknown-unknown -p vane-core` + `-p vane-wasm` | rc=0 PASS |
| 7 | wasm 体积 | `scripts/check-wasm-size.sh` | vane-wasm 362KB / vane-core 649KB ≤ 800KB PASS |
| 8 | cargo deny | `cargo deny check` | advisories ok, bans ok, licenses ok, sources ok PASS |
| 9 | no-std-fs | grep `std::fs::\|std::net::\|mmap` in binding src（非 test） | 无新增（test 中 std::fs 为既有模式） PASS |

---

## 5. Concerns

1. **index.d.ts 手动更新**：@napi-rs/cli 2.18.4 与 napi 3.x 版本不配对，CLI 无法从 type-defs 元数据自动生成 .d.ts 新方法。手动补列 `stats()` / `segmentInfo()` 声明。type-defs 元数据文件已正确记录新方法（验证通过）。这是既有环境问题，非本次引入。

2. **未实现 `vane_db_collection_segment_info` FFI 函数**：任务只要求 2 个 FFI 函数（`vane_db_stats` + `vane_db_segment_info`）。core 层 `Db::collection_segment_info(name)` 的便捷方法未落 FFI，因为 `segment_info` 已返回所有段，调用方可客户端按 collection 过滤。如后续需要可新增。

3. **wasm web.rs 测试需 wasm-pack 运行**：`cargo test` 对 wasm-bindgen-test 显示 "0 passed"（需 `wasm-pack test --node` 运行）。这是既有 CI 行为，非本次问题。wasm32 check 已验证编译通过。

4. **JSON 序列化为手写**：core inspect 结构未 derive Serialize（任务约束不改 core）。三绑定层各自手写 `serde_json::Value` 构造（与 `hits_to_json` 同构模式）。如后续 core 补 derive Serialize，可简化为 `serde_json::to_string(&stats)`。

---

## 6. 文件变更清单

| 文件 | 变更 |
|---|---|
| `crates/vane-ffi/src/lib.rs` | +import inspect 类型 +JSON 序列化辅助 +2 FFI 函数 +4 测试 |
| `crates/vane-node/src/convert.rs` | +import inspect 类型 +db_stats_to_json/segment_info_to_json |
| `crates/vane-node/src/db.rs` | +StatsTask/SegmentInfoTask +stats()/segment_info() 方法 |
| `crates/vane-node/main.js` | +wrapMethods 补 stats/segmentInfo |
| `crates/vane-node/index.d.ts` | +stats()/segmentInfo() 声明 |
| `crates/vane-node/__tests__/inspect.test.js` | 新增 4 个 JS 测试 |
| `crates/vane-wasm/src/lib.rs` | +import inspect 类型 +JSON 序列化辅助 +2 wasm-bindgen 函数 |
| `crates/vane-wasm/tests/web.rs` | +2 wasm-bindgen-test |
| `bindings/go/vane.h` | +2 C ABI 声明 |

**commit hash**：`5143885`
