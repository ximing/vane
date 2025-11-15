# M2-04 Dedicated Worker 壳

## 1. 目标
实现 Dedicated Worker 壳：主页面 async ↔ Worker 同步 core 的 postMessage Promise 边界，init 探针选择 Vfs（OPFS/IDB）+ 加载词典（CDN/内联/降级 bigram），封装 `VaneWorker` wasm-bindgen 导出（SPEC §4.1/REQUIREMENTS §4.1，OPFS 强制 Worker 架构）。

SPEC 节号：§4.1（WASM Worker 架构）、§11（core 同步 IO，异步只在 postMessage 边界）、§12.3（WASM 词典 CDN fetch）。

## 2. 涉及文件
- **Create** `crates/vane-wasm/src/worker.rs`：`VaneWorker` struct + `#[wasm_bindgen]` impl。
- **Create** `crates/vane-wasm/src/dict_loader.rs`：词典 CDN fetch + sha256 校验 + OPFS 缓存 + 降级 bigram（SPEC §12.3）。
- **Create** `crates/vane-wasm/src/worker.js`（或 `src/worker.ts`）：Worker 入口 JS 胶水（加载 wasm + 路由 postMessage）。
- **Modify** `crates/vane-wasm/Cargo.toml`：`[features] worker = ["dep:web-sys", "dep:js-sys", "dep:wasm-bindgen-futures"]`（worker 引 wasm-bindgen-futures 处理 Promise；评估体积，登记）。
- **Modify** `crates/vane-wasm/src/lib.rs`：`#[cfg(feature="worker")] pub mod worker; pub mod dict_loader;`。

## 3. 接口契约
### Consumes from
- M0/M1 `vane_core::api::{Db, Collection, ReindexHandle}`（全部 pub API）。
- M2-02 `OpfsVfs`、M2-03 `IdbVfs`（init 探针选择）。**Worker init 异步序列**（与 M2-02 §4.7 一致）：
  ```
  Worker init（JS 异步上下文，wasm-bindgen-futures）:
    1. root = await navigator.storage.getDirectory()
    2. fh = await root.getFileHandle("vane.db", {create:true})
    3. sah = await fh.createSyncAccessHandle()        // 唯一同步句柄
    4. OpfsVfs::from_handle(sah)                      // 重建文件表（读 superblock + meta_slot）
    5. Db::open(Arc<OpfsVfs>, db_path)                // core 同步打开
  OPFS 不可用（Safari 历史 OPFS 写入 bug / API 缺失）→ 能力探测降级：
    1. idb = await open_idb("vane_db")
    2. blob = await idb.get("container") ?? Vec::new()
    3. IdbVfs::from_blob(blob)                        // 内存 Vec + overlay
    4. Db::open(Arc<IdbVfs>, db_path)
  ```
  异步性严格限于 init + postMessage 边界；步骤 4-5 进入 core 同步世界（REQUIREMENTS §4.1）。
- M2-05 `simd_probe::simd128_supported()`（init 选 SIMD/Scalar 产物）。
- M1 `vane_core::tokenizer::jieba::{JiebaDict, JiebaTokenizer}`（`tokenizer/jieba/dict.rs:46` `JiebaDict::load`、`tokenizer/jieba/mod.rs:41` `JiebaTokenizer::new`）——词典加载后注入 collection。**jieba feature（仅算法代码 DAT/HMM/seg，无词典数据）在 vane-wasm 非 default 启用**（README 全局约束表已放宽：`dict-zh` 红线永不启，`jieba` 可启用须过 800KB 门禁实测）；词典数据运行时 fetch/内联注入。

### Produces for
```rust
#[wasm_bindgen]
pub struct VaneWorker { /* db: Option<Db>, collections: HashMap<u32, Collection>, vfs: Box<dyn Vfs> */ }
#[wasm_bindgen]
impl VaneWorker {
    // 异步工厂（非 constructor——wasm-bindgen 构造器不能返 Promise）。
    // JS 用 `const worker = await VaneWorker.create(opts)`。
    #[wasm_bindgen(js_name = create)]
    pub fn create(opts: JsValue) -> js_sys::Promise;  // init: 选 Vfs + 词典加载 + SIMD 探针
    pub fn open(&self, path: String, opts: JsValue) -> js_sys::Promise;
    pub fn collection(&self, name: String, schema: JsValue, opts: JsValue) -> js_sys::Promise;
    pub fn add(&self, col: u32, docs: JsValue) -> js_sys::Promise;
    pub fn flush(&self, col: u32) -> js_sys::Promise;
    pub fn search(&self, col: u32, query: JsValue) -> js_sys::Promise;
    pub fn delete(&self, col: u32, ids: JsValue) -> js_sys::Promise;
    pub fn compact(&self, col: u32) -> js_sys::Promise;
    pub fn reindex(&self, col: u32) -> js_sys::Promise;
    pub fn export(&self, dest: String) -> js_sys::Promise;  // M2-12 接入
    pub fn close(&self) -> js_sys::Promise;
}

// dict_loader.rs
pub async fn load_dict_cdn(url: &str, expected_sha256_prefix: [u8;8], cache_vfs: &dyn Vfs) -> Result<Vec<u8>>;
// CDN fetch → sha256 校验 → OPFS 缓存（二次启动零网络）；失败降级 bigram + console.warn（不抛错）
pub fn dict_unavailable_fallback() -> ();  // 降级 CjkBigram + warn
```
下游：M2-14 Demo。

## 4. TDD 测试清单
1. **Worker 构造**：`new VaneWorker({persistence:"persistent"})` 返回 Promise，resolve 后 Worker 就绪（Vfs 选 OPFS 或降级 IDB）。
2. **端到端检索**：`open` → `collection`(schema jieba) → `add`([{id,text,vector}]) → `flush` → `search`({text,topK:10}) 返回 Hit[]，与 vane-core 等价（I-8 薄壳）。
3. **OPFS 优先 + init 异步序列**：`opfs_available()==true` 时按 §3 异步序列 `getDirectory→getFileHandle→createSyncAccessHandle→OpfsVfs::from_handle→Db::open`（断言内部 vfs 类型，或行为间接验证）。
4. **IDB 降级**：`opfs_available()==false`（Safari OPFS 写入 bug / API 缺失）时按 §3 异步序列 `open_idb→get blob→IdbVfs::from_blob→Db::open`，不抛错（SPEC §10 消解）。能力探测覆盖：`navigator.storage.getDirectory` 存在性 + `createSyncAccessHandle` 可用性 + 小写 round-trip 探针。
5. **词典 CDN fetch**：mock fetch 返回 dict.bin → sha256 校验通过 → `JiebaDict::load` 成功 → jieba 分词生效（中文查询命中整词）。
6. **词典 sha256 校验失败**：fetch 返回错误数据 → sha256 不匹配 → 丢弃 + 降级 bigram + console.warn（不抛错，SPEC §12.4）。
7. **词典 OPFS 缓存**：首次 fetch → 缓存写 OPFS；二次 init → 读缓存零网络（mock fetch 不被调用）。
8. **词典内联注入**：`opts.dictData = Uint8Array` → 跳过 CDN fetch 直接 `JiebaDict::load`（离线场景，SPEC §4.2 CollectionOptions.dictData）。
9. **词典降级 bigram**：CDN fetch 失败 + 无 dictData → 降级 CjkBigram + console.warn，`E_DICT_UNAVAILABLE` 禁止到达（SPEC §10/§12.4）。
10. **SIMD 探针**：`simd128_supported()==true` 时加载 simd 产物，`false` 时加载 scalar 产物（M2-05 协同，本测试 mock 探针返回值）。
11. **postMessage Promise 边界**：主页面 `await worker.search(..)` 返回结果；Worker 内同步 core 调用（REQUIREMENTS §4.1）。
12. **错误透传**：core `Err(VaneError::Schema(..))` → Promise reject 携带 code=-2（SPEC §10 错误码透传，I-8）。
13. **close**：`close()` 后再调用任何方法 reject（句柄注销后使用=明确错误，I-7 对齐）。

## 5. 验收标准
- 全部 13 测试绿（wasm-bindgen-test in Worker + JS 行为测试）。
- 体积：启用 `worker` feature 后 vane-wasm gzip ≤800KB（**wasm-bindgen-futures 体积实测登记**：预估 +5~15KB gzip，见 README 全局约束表「wasm 体积预算累计管理」+ 「新依赖体积评估」表）。
- `cargo check --target wasm32-unknown-unknown -p vane-wasm --features worker` 通过。
- 词典永不进 wasm：vane-wasm default features 不启 `dict-zh`（红线，捆绑词典数据）；`jieba` feature（仅算法代码）可在非 default 启用须过 800KB 门禁实测；词典数据运行时 fetch/内联。
- core 零改动。

## 6. 前置依赖
- M2-01（vane-wasm cdylib）。
- M2-02（OpfsVfs）。
- M2-05（simd_probe，init 探针；可先 mock 占位，M2-05 落实后回归）。

## 7. 不变量覆盖
- **I-7 FFI 内存铁律**（对齐）：句柄注销后使用=明确错误。测试 13 守护。
- **I-8 binding 薄壳**：Worker 胶水无检索逻辑，行为测试在 core。测试 2 守护。
- **词典永不进 wasm**：测试 5/8/9 守护（fetch/内联/降级，词典数据不编译进 wasm）。
- **降级不抛错**：测试 4/9 守护（IDB 降级 + 词典降级 bigram）。
- **core 同步 IO**：测试 11 守护（异步只在 postMessage 边界）。
