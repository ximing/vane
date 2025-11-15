# M2-04 Dedicated Worker 壳——评审报告

**评审对象**：M2-04 VaneWorker + dict_loader + worker.js + init 探针
**评审模式**：只读，未跑 cargo（基于 diff + 实读源码 + core API 契约交叉核实）
**BASE..HEAD**：1a609d0..9ed63bf（vane-wasm）
**评审日期**：2026-08-10

## 状态：PASS_WITH_FINDINGS

- **B（Blocker）**：0
- **I（Important）**：1
- **M（Minor）**：8

---

## 1. 评审逐项结论

### 1.1 I-8 薄壳 — PASS
`worker.rs` 全部 `*_sync` 方法委托 `vane_core::api`（`Db::open`/`db.collection`/`col.add/flush/search/delete/compact/reindex`/`db.export/close`），无检索逻辑。JSON 解析（`parse_open_opts`/`parse_schema`/`parse_docs`/`parse_search_query`，worker.rs:152-339）与 `lib.rs` 同构（薄壳解析，不含业务）。Promise 包装正确：同步方法用 `Promise::resolve/reject`（worker.rs:363-369），`create` 用 `future_to_promise`（worker.rs:835-840）。错误映射 `err_to_js`（worker.rs:109-111）调 `VaneError::name()`（types.rs:70 确认存在）。

### 1.2 dict_loader 降级不抛错（§12.3 红线）— PASS（核心红线守住）
- `load_dict` 返 `Option<Vec<u8>>`，**永不返 Err**（dict_loader.rs:105-154）。fetch 失败 → `fetch_cdn` 返 None（:67-90 wasm32 / :92-96 非 wasm32）；sha256 不匹配 → None + warn（:115-117/:142-145）；无 dictData 无 CDN → None（:123 `cdn_url?`）。
- `E_DICT_UNAVAILABLE` 禁止到达：`collection_sync`（worker.rs:718-732）在 jieba 请求且词典不可用时预防式降级 CjkBigram + warn，`#[cfg(not(feature="jieba"))]` 分支亦降级（:727-731）。
- sha256 校验：`verify_sha256_prefix` 前 8 字节比对（dict_loader.rs:33-39），逻辑正确。
- dictData 内联：`extract_dict_data`（worker.rs:616-628）支持 Uint8Array/ArrayBuffer，`load_dict` 渠道 1 优先（:112-120）。
- 测试覆盖：9 单测（sha256 ok/mismatch、内联跳过 CDN、内联 sha256 失败、无数据无 CDN、fetch 失败、cache round-trip、cache hit 跳过 fetch、cache sha256 失败回退）。

### 1.3 init 探针 + Safari 降级 — PASS（含 M-7/M-4）
- `opfs_available()`（worker.rs:380-396）真实探测 `navigator.storage.getDirectory` 是否为函数，经 `js_sys::global()` 反射兼容 Window + WorkerGlobalScope。node 返 false（:398-402）。
- 降级链：OPFS 不可用 → IDB（worker.rs:563-565）→ IDB 失败 → MemoryVfs + warn（:594-602）。OPFS init 失败 → IDB（:571-576）。
- 异步严格限于 init：`select_vfs`/`init_opfs_vfs`/`init_idb_vfs`/`load_dict` 均 async（worker.rs:462-664）；`Db::open` 及之后 core 调用全同步（:689）。

### 1.4 constructor→factory 偏离 — 合理（含 M-1）
`VaneWorker.create(opts)` 静态工厂（worker.rs:834 `#[wasm_bindgen(js_name = create)]`）替代 `#[wasm_bindgen(constructor)]`。理由成立：wasm-bindgen constructor 不能返 Promise，而 init 需异步（future_to_promise）。report §1 已记录。JS 用法 `await VaneWorker.create(opts)`（worker.js:44）正确。SPEC/REQUIREMENTS 未硬绑 constructor 形式（仅 §4.1 要求「postMessage 边界包 Promise」）。

### 1.5 800KB 含 jieba — PASS
- 实测：raw 1696KB → wasm-opt -Oz 1174KB → **gzip 399KB** ≤ 800KB（report §3，口径正确：wasm-opt -Oz + gzip）。
- `dict-zh` 红线永不启：Cargo.toml:54 `jieba = ["vane-core/jieba"]`（仅算法代码透传），无 `dict-zh`。worker feature 含 `jieba`（:65）。
- README 全局约束表已放宽（docs/plans/m2/README.md:385「A-I5 放宽 M1 约束...jieba feature 可在 vane-wasm 非 default 启用，须过 800KB 门禁实测」）。wasm-bindgen-futures 体积评估已登记（:403）。

### 1.6 不变量 — PASS
- I-5（vane-wasm cfg 允许）：`worker`/`opfs`/`idb`/`jieba` feature 仅在 vane-wasm crate，core 零 cfg 改动。
- I-8（薄壳）：见 1.1。
- core 零改动：diff 仅触 vane-wasm（Cargo.toml/dict_loader.rs/lib.rs/worker.js/worker.rs）+ Cargo.lock + report。

### 1.7 postMessage Promise 边界 — PASS（含 M-3）
worker.js 路由 11 个 op（create/open/collection/add/flush/search/delete/compact/reindex/export/close），错误 catch 后 `{id, error: String(err)}` 回传（worker.js:97-100）。错误透传正确。

### 1.8 TDD 覆盖 — PASS
16 新增（dict_loader 9 + worker 7），覆盖薄壳路由/降级/init 分支/close 拒绝/schema 不一致/export unsupported。浏览器异步路径（OPFS/IDB/fetch/worker.js round-trip）标注待手动验证（report §6）——node 无浏览器 API，合理。

### 1.9 panic 安全 — 见 M-5

---

## 2. 发现

### I-1：`write_cache` 无法刷新已存在缓存（dict OPFS 缓存刷新路径失效）

**证据**：
- `dict_loader.rs:56-61` `write_cache`：
  ```rust
  fn write_cache(cache_vfs: &dyn Vfs, bytes: &[u8]) -> Result<()> {
      cache_vfs.create(DICT_CACHE_PATH)?;   // ← 已存在则 Err
      cache_vfs.write_at(DICT_CACHE_PATH, bytes, 0)?;
      Ok(())
  }
  ```
- `MemoryVfs::create`（crates/vane-core/src/vfs/memory.rs:32-33）：`if files.contains_key(path) { return Err(VaneError::Io("file already exists")) }`
- `MemOverlay::create`（crates/vane-wasm/src/vfs/overlay.rs:387-389）：同样「file already exists」Err，且有测试 `create_already_exists_errors`（overlay.rs:1291-1295）守护此语义。
- `OpfsVfs::create`（crates/vane-wasm/src/vfs/opfs.rs:101-103）委托 `self.overlay.create(path)` → 同语义。
- `IdbVfs::create`（crates/vane-wasm/src/vfs/idb.rs:179）同模式。
- 错误被 `dict_loader.rs:150` 吞掉：`let _ = write_cache(vfs, &bytes);`

**影响**：首次缓存写入成功后，词典更新（新 `dictSha256`）时 `write_cache` 恒在 `create` 步失败 → 缓存永久停留首版 → 每次启动 `read_cache` 读旧字节 → sha256 不匹配 → 回退 CDN fetch → `write_cache` 再失败。correctness 由 sha256 守护（无误数据到达用户），但「二次启动零网络」（§12.3 缓存设计意图，计划测试 7）在词典变更后退化为「恒走网络」，违背缓存目的。浏览器库长期运行后缓存形同虚设。

**修复**：Vfs trait 有 `delete`（vfs/mod.rs:11）。`write_cache` 改为：
```rust
let _ = cache_vfs.delete(DICT_CACHE_PATH);  // best-effort 清旧
cache_vfs.create(DICT_CACHE_PATH)?;
cache_vfs.write_at(DICT_CACHE_PATH, bytes, 0)?;
```
或在 Vfs trait 增 `create_or_truncate` 语义。

**验证建议**：补单测——`write_cache` 两次调用（第二次覆盖第一次），`read_cache` 返回第二次字节。

---

### M-1：计划 §3 契约文本未同步 `create` 静态工厂

**证据**：
- 计划 `docs/plans/m2/modules/M2-04-worker-shell.md:42-43` 仍写：
  ```
  #[wasm_bindgen(constructor)]
  pub fn new(opts: serde_json::Value) -> js_sys::Promise;
  ```
- 实装 `worker.rs:834`：`#[wasm_bindgen(js_name = create)] pub fn create(opts: JsValue) -> js_sys::Promise`

**影响**：偏离本身合理（wasm-bindgen constructor 不能返 Promise，report §1 已记录决策），但计划契约文本与实装不一致，后续 M2-14 Demo / 维护者读计划时困惑。

**修复**：更新计划 §3 接口契约为 `#[wasm_bindgen(js_name = create)] pub fn create(opts: JsValue) -> js_sys::Promise`，加注「非 constructor：wasm-bindgen 构造器不能返 Promise」。

---

### M-2：计划「SPEC §12.4」节号误引

**证据**：
- 计划 `M2-04-worker-shell.md:69,72` 写「降级 bigram + console.warn（不抛错，SPEC §12.4）」。
- 实际 `docs/SPEC.md:396-398` §12.4 是「版本与发布」（crates.io/npm/Go 版本号同步），非词典降级。
- 降级语义实际在 `docs/SPEC.md:392`（§12.3 末行「fetch 失败自动降级 bigram + console.warn，不抛错」）+ `docs/SPEC.md:345`（§10 E_DICT_UNAVAILABLE 注释「WASM 侧禁止到达此错误——自动降级 bigram + warn，见 §12.4」——SPEC 自身的 §12.4 引用也是悬空）。

**影响**：文档溯源困难。非代码缺陷。

**修复**：计划改引「§12.3 / §10」；SPEC §10:345 的「见 §12.4」也应改为「见 §12.3」。

---

### M-3：worker.js `pending` Map 死代码

**证据**：`crates/vane-wasm/src/worker.js:31` `const pending = new Map(); // id → { resolve, reject }` 声明后全文无引用（grep `pending` 仅此一行）。

**影响**：Worker 侧仅回传 `{id, result/error}`，主页面需自行管理 pending Map——设计可接受，但变量未用，易误导读者以为 Worker 侧有关联逻辑。

**修复**：删除 `:31` 行；或在注释说明「pending 由主页面维护，Worker 侧仅回传 id」。

---

### M-4：`select_vfs` 不检查 persistence 模式，silent MemoryVfs 降级

**证据**：
- `worker.rs:559-588` `select_vfs` 仅据 `VfsKind` 选 Vfs，未读 `OpenOptions.persistence`。
- `worker.rs:594-602`：OPFS + IDB 均失败时 `Ok(Arc::new(MemoryVfs::new()))` + warn。
- `parse_open_opts`（worker.rs:152-161）区分 `PersistenceMode::Persistent` vs `BestEffort`，但 `select_vfs` 不消费此信息。

**影响**：用户请求 `persistence: "persistent"` 且 OPFS+IDB 均不可用时，Worker 静默用 MemoryVfs（数据不持久化）但仍「正常」运行——违反 persistent 契约，生产环境可能导致数据丢失且无显式错误（仅 console.warn）。SPEC §10 降级不抛错，但 persistent 模式降级到非持久化应更显著告警或拒绝。

**修复**：`select_vfs` 传入 persistence 模式；persistent 模式下 OPFS+IDB 全失败时返 Err（或更高级别告警），best-effort 才降级 MemoryVfs。

---

### M-5：wasm-bindgen 导出未包 `catch_unwind`，与 vane-ffi 规范不对称

**证据**：
- `crates/vane-ffi/src/lib.rs:11-13` 规范：「所有 `extern "C"` 入口经 `catch_unwind_code` 包装：panic 时返 `E_INTERNAL`(-12)」；helper `:207-213`；全部 16 入口包装（`:500,548,600,...`）。
- `crates/vane-wasm/src/worker.rs:828+` 全部 `#[wasm_bindgen]` 方法（create/open/collection/add/flush/search/delete/compact/reindex/export/close）无 `catch_unwind`（`grep catch_unwind crates/vane-wasm/` 零命中）。
- SPEC §13.3 未对 wasm 导出 panic 安全立规矩。

**影响**：wasm-bindgen panic 在 Worker 内 = RuntimeError abort（非 FFI UB，wasm 内存安全），但 Worker 崩溃后无法恢复，postMessage 队列挂起。单线程串行 postMessage 下 `borrow()`/`borrow_mut()` 无重入 panic 风险；但 `js_sys::JSON::stringify`、`Reflect::get`、JS 边界调用理论上可抛 JS 异常 → Rust 侧 panic。

**修复**：可选——为 `#[wasm_bindgen]` 方法包一层 `panic::catch_unwind(AssertUnwindSafe(|| {...}))`，panic 时 `Promise::reject("internal panic")` 而非 abort。优先级低于 I-1。

---

### M-6：`read_cache` 每次 init 分配 16MB 缓冲区

**证据**：`dict_loader.rs:46` `let mut buf = vec![0u8; 16 * 1024 * 1024];`——每次 `load_dict` 调用（即每次 Worker init）均分配 16MB，即使缓存不存在（首次启动 / 无 cache_vfs）。

**影响**：Worker 内存敏感场景（浏览器 tab 配额）浪费 16MB 堆分配 + zero-fill。dict.bin 实际约 5MB（注释 :44）。

**修复**：先查文件 size（Vfs trait 无 `size` 方法，可 `read_at` 小缓冲增量读，或加 `Vfs::size`）；或缓冲区缩至 8MB（覆盖 5MB dict + 余量）。

---

### M-7：`opfs_available` 未做 round-trip 探针，仅检测 API 存在性

**证据**：
- `worker.rs:380-396` `opfs_available()` 仅检测 `navigator.storage.getDirectory` 是否为 function。
- 计划 `M2-04-worker-shell.md:67` test 4 要求「能力探测覆盖：`navigator.storage.getDirectory` 存在性 + `createSyncAccessHandle` 可用性 + 小写 round-trip 探针」。
- Safari 历史 OPFS 写入 bug 由 `init_opfs_vfs` 运行时失败 + 降级 IDB 兜底（worker.rs:571-576），功能上等价但非主动探针。

**影响**：Safari 上 `getDirectory` 存在但写入异常时，需走完整 `init_opfs_vfs`（getDirectory→getFileHandle→createSyncAccessHandle→OpfsVfs::from_handle）才触发降级——多一次异步往返 + warn 噪音。功能正确，效率略损。

**修复**：可选——`opfs_available` 内做小写 round-trip（write 1 byte → read → verify → delete），失败直接返 false。或接受现状（init 失败兜底足够）。

---

### M-8：`close_sync` 不 flush 各 collection

**证据**：`worker.rs:810-821` `close_sync`：`let _ = db.close();` → `inner.collections.clear()` → `inner.closed = true`。未遍历 `inner.collections` 调 `col.flush()`。`Db::close`（crates/vane-core/src/api/db.rs:171-174）是 `Ok(())` 占位（无后台线程 join，flush 由调用方显式调）。

**影响**：用户未显式 flush 就调 `close()` 时，缓冲区内未落盘数据丢失。与 core 设计一致（flush 显式），但 Worker close 文档应提示「close 前先 flush 各 collection」。

**修复**：`close_sync` 内遍历 collections 调 `col.flush()`（best-effort）；或在 `close` 文档/worker.js 注释强调先 flush。

---

## 3. 无法从 diff 确认项

- **浏览器异步路径实测**：`create` 异步 init（OPFS getDirectory→createSyncAccessHandle / IDB open+get）、`worker.js` postMessage round-trip、词典 CDN fetch + sha256 + OPFS 缓存（二次启动零网络）——node 无浏览器 API，report §6 标注待浏览器手动验证。**无法在本次只读评审中确认**，建议 M2-14 Demo 阶段补浏览器 e2e 验证 + 加入 CI（headless browser）。
- **SIMD 探针**：消费 `simd_probe::simd128_supported()` 占位返 false（worker.rs:637），M2-05 落实真实探针后需回归。

---

## 4. 核心红线复核

| 红线 | 结论 | 证据 |
|------|------|------|
| dict_loader 降级不抛错（§12.3） | ✅ PASS | `load_dict` 返 Option，永不 Err；E_DICT_UNAVAILABLE 在 `collection_sync` 预防式降级，禁止到达用户 |
| dict-zh 永不进 wasm | ✅ PASS | Cargo.toml:54 `jieba=["vane-core/jieba"]` 仅算法代码；无 dict-zh |
| 800KB 门禁（含 jieba） | ✅ PASS | gzip 399KB ≤ 800KB |
| I-8 薄壳 | ✅ PASS | 全部委托 vane_core::api，无检索逻辑 |
| core 零改动 | ✅ PASS | diff 仅触 vane-wasm |
| 异步只在 init + postMessage 边界 | ✅ PASS | core 调用全同步 |

---

## 5. 结论

M2-04 核心红线（dict_loader 降级不抛错、E_DICT_UNAVAILABLE 禁止到达、800KB 含 jieba、dict-zh 红线、I-8 薄壳、core 零改动）**全部守住**。constructor→factory 偏离合理（wasm-bindgen 限制）。init 探针 + Safari 降级链功能正确。

**1 个 I 级发现（I-1：write_cache 无法刷新已存在缓存）**应在 M2-14 Demo 前修复——缓存刷新路径失效，词典更新后「二次启动零网络」退化为「恒走网络」，修复极简（delete-then-create）。8 个 M 级发现多为文档同步 / 健壮性增强，不阻塞合并。

建议：**合并后开 issue 跟踪 I-1 + M-1/M-2 文档同步**；浏览器异步路径验证纳入 M2-14 Demo 验收。
