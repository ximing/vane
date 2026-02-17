# Task Review Package — FFI 层 inspect API（6b-impl-1）

**范围**：commit `5143885`（基于 `8dc83a2`），9 文件 +801 -28
**BASE..HEAD**：`8dc83a2..5143885`
**implementer**：sonnet / bg
**reviewer**：opus / bg
**用户决策**：Q1 选"立即实现 FFI 层"（非顺延）

## diff 范围（9 文件，全绑定层源码+测试）

- `crates/vane-ffi/src/lib.rs`（+302：vane_db_stats/vane_db_segment_info + JSON 序列化 + 4 测试）
- `crates/vane-node/src/db.rs`（+55）+ `convert.rs`（+99）：stats/segmentInfo AsyncTask
- `crates/vane-node/__tests__/inspect.test.js`（+137：4 ava 测试）
- `crates/vane-node/index.d.ts`（+41 -28：手动声明 stats/segmentInfo）
- `crates/vane-node/main.js`（+2：wrapMethods）
- `crates/vane-wasm/src/lib.rs`（+130：2 wasm-bindgen 函数）
- `crates/vane-wasm/tests/web.rs`（+54：2 wasm-bindgen-test）
- `bindings/go/vane.h`（+9：C ABI 声明）

**禁碰文件核验**：未碰 docs/SPEC.md / docs/plans/m4/ / CLAUDE.md / core inspect.rs / db.rs（`git diff -- crates/vane-core/` 空）。

## reviewer 结论

**Spec ✅，0 Critical，0 Important，3 Minor（全既有模式非本次引入），不进 fix 循环。**

### Spec 合规全项通过
- §9.1 句柄：lookup_db 取 Arc<Db>，无效句柄→-3 E_NOT_FOUND 非 panic，null 指针先返 -11。
- §9.1 内存铁律：arena_alloc_tracked 复用 + vane_string_free 释放。
- §9.1 错误：i32 返回（0=OK，负值=§10 码），fail() 写 thread-local。
- §9.2 JSON：手写 serde_json::Value（与 hits_to_json 同构），binding 薄壳。
- 不改 M0-M3 冻结签名：core diff 空，纯新增 2 extern "C" 函数。

### 质量正确性
- 7 struct 字段全覆盖（DbStats 4/CollectionStats 9/SegmentInfo 7/FormatVersions 7/SegmentFileSizes 7/Health 3/ExecutorKind 2/DictState 3），无漏字段无类型错。Option<u64> hnsw → null|number 正确。
- **关键发现**：`Db::stats()`/`segment_info()` 实为**直接返回值非 Result**（inspect 只读，段损坏编码为 Health::Corrupt 字段非 Result::Err）——FFI 无需 VaneError→错误码映射，审查清单原假设有误但实现正确。
- panic 安全：catch_unwind_code 包装 RwLock poisoned panic → -12 E_INTERNAL。
- Node AsyncTask（libuv worker pool，非阻塞 main thread）→ Promise<Json>。
- Wasm JsResult<String>（JSON 字符串），句柄解析同现有。

### 测试 non-vacuous
- FFI 4 测试（实跑全流程 + 11 字段断言 + 错误码 -3/-11）。
- Node 4 ava 测试（字段全断言 + 空 DB + delete 后 tombstoned 反映）。
- Wasm 2 wasm-bindgen-test（JSON 有效 + 字段断言）。

### 3 Minor（全 defer）
- M1：FFI null 指针错误未设 thread-local 错误消息（既有模式，vane_search/vane_dict_version 同样）。
- M2：report §2"体积增量 0 KB"与 §4"362KB"矛盾（实际 +10KB 在限内，代码正确）。
- M3：wasm web.rs 需 wasm-pack test --node 运行（既有 CI 行为，wasm32 check 已验证编译）。

### concerns 核验
- #1 index.d.ts 手动声明：Minor（声明正确匹配实现，@napi-rs/cli 2.18.4 与 napi 3.x 不配对是既有环境问题）。
- #2 未实现 vane_db_collection_segment_info：OK（segment_info 已返回所有段，客户端可过滤）。
- #3 wasm web.rs 需 wasm-pack：OK（既有行为）。

## 编排者全量门禁确认（后台脚本 exit 0）

| 门禁 | 结果 |
|---|---|
| fmt --all --check | rc=0 |
| clippy workspace --all-features --exclude vane-fuzz -D warnings | rc=0 |
| test workspace --all-features --exclude vane-fuzz | rc=0 全过 |
| Node npm test | 21 passed（含 4 新 inspect ✔） |
| wasm32 core check | rc=0 |
| wasm32 vane-wasm check | rc=0 |
| check-wasm-size | vane-wasm 362636B / core 649619B gzip ≤800KB ✓ |
| cargo deny check | advisories/bans/licenses/sources ok |
| no-std-fs | std_fs.rs 命中是 VFS 实现处（SPEC §13.3 允许），非违规；FFI inspect 未碰 core |

## 定案

**6b-impl-1 FFI inspect 完成**（commit 5143885）。reviewer 0 Critical/0 Important + 全量门禁全绿 + core 零改动 + 用户 Q1"立即实现"交付。3 Minor 全既有模式 defer。进 6b-impl-2 §10 诊断精简重构。
