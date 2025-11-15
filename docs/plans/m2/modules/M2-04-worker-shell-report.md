# M2-04 Dedicated Worker 壳——完成报告

## 1. 实装概要

### VaneWorker（`crates/vane-wasm/src/worker.rs`）
- `VaneWorker` struct（`Rc<RefCell<WorkerInner>>` 内部可变性，wasm32 单线程安全）。
- `#[wasm_bindgen]` impl：`create`（异步工厂，返 `js_sys::Promise`）+ `open/collection/add/flush/search/delete/compact/reindex/export/close`（同步 core 调用包装为 Promise）。
- **I-8 薄壳**：全部委托 `vane_core::api`，无检索逻辑。Promise 仅包装同步结果（`Promise::resolve`/`reject`），`create` 用 `future_to_promise`。
- **`create` 非 constructor**：`#[wasm_bindgen(constructor)]` 不能返 Promise，改用静态工厂 `VaneWorker.create(opts)` → `Promise<VaneWorker>`（JS 用 `await VaneWorker.create(opts)`）。文档标注。
- 内部同步 API（`open_sync`/`collection_sync`/...）供单测直接调用（MemoryVfs 后端）。

### dict_loader（`crates/vane-wasm/src/dict_loader.rs`）
- `load_dict(dict_data, cdn_url, expected_sha256, cache_vfs) -> Option<Vec<u8>>`：三渠道（内联 → CDN fetch+sha256 → 降级 None）。
- `verify_sha256_prefix`：sha256 前 8 字节校验。
- `dict_unavailable_fallback()`：warn 通知（不抛错）。
- VFS 缓存（`dict.bin.cache` 文件，二次启动零网络）。
- **降级铁律**：fetch 失败/sha256 不匹配 → 返 None（调用方降级 CjkBigram + warn），`E_DICT_UNAVAILABLE` 禁止到达。
- 跨平台 `warn()` 辅助（wasm32→`console.warn`，native→`eprintln`），解决 node 测试 panic。

### worker.js（`crates/vane-wasm/src/worker.js`）
- Worker 入口 JS 胶水：加载 wasm + 路由 postMessage（create/open/collection/add/flush/search/delete/compact/reindex/export/close）。
- postMessage Promise 边界：主页面 `postMessage({op,...})` → Worker 调 VaneWorker 方法 → await → `postMessage({id, result/error})`。
- 浏览器手动验证标注（node 无 Worker/OPFS/IDB）。

### init 探针（`opfs_available()`）
- 真实探针 `navigator.storage.getDirectory` 存在性（js_sys::global 反射，兼容 Window + WorkerGlobalScope）。
- OPFS 不可用（Safari 历史 bug / node 无 navigator）→ 降级 IDB + warn。
- OPFS init 失败 → 降级 IDB → IDB 失败 → 降级 MemoryVfs（测试环境）。
- 异步严格限于 init（getDirectory→getFileHandle→createSyncAccessHandle→OpfsVfs::from_handle）；Db::open 进 core 同步。

### 词典注入
- `open_sync`：Db::open 后若 `dict_bytes` 可用 → `JiebaDict::load` → `db.set_jieba_dict(Arc)`。
- `collection_sync`：tokenizer=Jieba 但 `!db.jieba_dict_available()` → 预防式降级 CjkBigram + warn（`E_DICT_UNAVAILABLE` 禁止到达）。

### SIMD 探针
- 消费 `simd_probe::simd128_supported()`（占位返 false→scalar；M2-05 落实真实探针）。

## 2. Cargo.toml 变更
- `worker` feature：启 `web-sys`/`js-sys`/`wasm-bindgen-futures`/`sha2` + `opfs`/`idb` + `jieba`（vane-core/jieba 透传）。
- 新增 `jieba` feature（透传 `vane-core/jieba`，算法代码 DAT/HMM/seg，无词典数据）。
- **dict-zh 红线永不启**。
- web-sys 子 features：Navigator/Request/RequestInit/Response/console/IdbFactory/IdbRequest/IdbOpenDbRequest/IdbTransactionMode/FileSystemGetFileOptions。

## 3. 800KB 体积实测（关键）

```
cargo build --release --target wasm32-unknown-unknown -p vane-wasm --features worker
→ wasm-opt -Oz → gzip
```

| 口径 | 大小 |
|------|------|
| raw wasm | 1,737,625 bytes (1696 KB) |
| wasm-opt -Oz | 1,202,679 bytes (1174 KB) |
| **gzip** | **409,187 bytes (399 KB)** |
| 门禁 | ≤ 800 KB (819,200 bytes) |
| **结果** | **PASS**（399 KB ≤ 800 KB，余量 401 KB） |

含 jieba 算法代码（DAT/HMM/seg）+ wasm-bindgen-futures + web-sys 浏览器 API + sha2 + opfs + idb。
不含词典数据（dict-zh 红线，运行时 fetch/内联）。

## 4. 自证门禁结果

| # | 门禁 | 结果 |
|---|------|------|
| 1 | `cargo check --target wasm32 -p vane-wasm --features worker` | ✅ PASS |
| 2 | `cargo test --workspace --all-features` | ✅ 453 passed（437 基线 + 16 新增） |
| 3 | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | ✅ clean |
| 4 | `cargo fmt --all -- --check` | ✅ clean |
| 5 | `bash scripts/check-no-std-fs.sh` | ✅ OK |
| 6 | `cargo deny check` | ✅ all ok |
| 7 | 体积门禁（worker+jieba，gzip ≤800KB） | ✅ 399 KB |
| 8 | VaneWorker 薄壳端到端（open→collection→add→flush→search） | ✅ MemoryVfs 单测 |
| 9 | dict_loader 降级测试（fetch 失败→bigram+warn 不抛错） | ✅ 7 单测 |
| 10 | init 探针逻辑分支（opfs_available node 返 false） | ✅ 单测 |
| 11 | worker.js 存在 + 语法可加载 | ✅ 存在（浏览器验证标注） |

## 5. 测试摘要
- dict_loader: 7 单测（sha256 校验/缓存 round-trip/降级不抛错/内联跳过 CDN）。
- worker: 6 单测（端到端检索/jieba 降级 bigram/schema 不一致错误/close 后拒绝/delete+compact+reindex/export 未实装/opfs 探针 node false）。
- 全部 node 可跑（MemoryVfs + 同步 API + 跨平台 warn）。

## 6. 遗留（浏览器异步路径验证）
- `create` 异步 init（OPFS getDirectory→getFileHandle→createSyncAccessHandle / IDB open+get）——wasm32 编译通过，node 无浏览器 API，待浏览器手动验证。
- `worker.js` postMessage Promise 边界 round-trip——待浏览器手动验证。
- 词典 CDN fetch + sha256 校验 + OPFS 缓存（二次启动零网络）——待浏览器手动验证。
- SIMD 探针（M2-05 落实真实探针后回归）。

## 7. 不变量守护
- I-7（句柄注销后使用=明确错误）：close 后调用 reject 单测守护。
- I-8（Worker 薄壳无检索逻辑）：端到端行为与 core 等价单测守护。
- 词典永不进 wasm：dict-zh 红线永不启；jieba 算法代码进 wasm 过 800KB 门禁（399 KB）。
- 降级不抛错：dict_loader 降级 + jieba 降级 bigram 单测守护。
- core 同步 IO：异步只在 init + postMessage 边界。
