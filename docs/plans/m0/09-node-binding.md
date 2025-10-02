# Node-Binding 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development。步骤用 checkbox。
> 本计划产出 `crates/vane-node`，是 Vane 三侧绑定中的 Node 侧。它是一层无逻辑薄壳（不变量 I-8），仅做 JSON ↔ Rust 结构转换与 napi 导出；全部检索行为在 `vane-core` 内部。

---

## Goal

为 Node.js 提供 `@vane/node` 原生包，覆盖 SPEC §4.1 的 M0 函数面（`open` / `collection` / `collections` / `add` / `flush` / `search` / `close`，外加 `delete` 占位 reject），三侧签名映射遵循 §4.3：JS 侧全部 `Promise<T>`，reject 携带含 `code` 的 `VaneError`。M0 交付 4 平台 prebuilt 配置（§12.2）。

## Architecture

```
┌─────────────── JS (index.js + VaneError class) ───────────────┐
│  open / collection / collections / add / flush / search / ... │  ← Promise<T>
└──────────────────────────┬────────────────────────────────────┘
                           │ napi-rs（N-API v6+，不经过 C ABI §9.3）
┌──────────────────────────▼────────────────────────────────────┐
│                    crates/vane-node (薄壳)                     │
│  VaneDb / VaneCollection  +  AsyncTask<...>  +  JSON 转换      │
│  无检索逻辑（I-8）；不桥接 tokio（§9.3）                        │
└──────────────────────────┬────────────────────────────────────┘
                           │ 直连（Rust API，非 FFI 句柄）
┌──────────────────────────▼────────────────────────────────────┐
│                  vane_core::api（07-api-core）                 │
│  Db / Collection / OpenOptions / SearchQuery / Hit / ...       │
└────────────────────────────────────────────────────────────────┘
```

关键架构决策：

1. **异步经 `AsyncTask`，不用 `async fn`。** napi-rs 的 `#[napi] async fn` 默认跑在 napi 内置 tokio runtime 上，违反 SPEC §9.3「不桥接 tokio」。因此所有异步方法手写 `impl Task for XxxTask`，`compute` 在 libuv worker pool 上同步执行（core 内部用 rayon Executor 并行），`resolve` 把结果转回 JS。JS 侧仍是 `Promise<T>`（§4.3 满足）。
2. **`VaneDb` / `VaneCollection` 持有 core 类型。** 因 `AsyncTask` 需要 `'static` 数据，core 的 `Db` / `Collection` 必须是 `Clone + Send + Sync`（Arc-based 浅拷贝，下文 Consumes 已声明）。每个 async 方法在入队前 `clone` 一份 inner 进 task。
3. **错误 code 透传：** Rust 侧 `From<VaneError> for napi::Error` 把 `code`/`name` 编进 reason 前缀；JS 侧 `index.js` 包一层 catch，把原生 Error 重包装为 `VaneError` 子类并挂 `.code` / `.name`（纯胶水，无检索逻辑，不违反 I-8）。
4. **Vfs 默认 `StdFsVfs`（native）。** `open` 时 binding 内部构造 `StdFsVfs::new()` 传给 `Db::open`。`MemoryVfs` 仅测试用，通过测试辅助函数注入（不暴露到 JS API）。

## Tech Stack

- **napi-rs**：`napi` + `napi-derive` crate；`@napi-rs/cli` 构建/打包。
- **serde_json**：JS Value ↔ Rust 结构转换（binding 薄壳原则 §9.2）。
- **异步**：`napi::Task` + `AsyncTask<T>`（libuv worker pool），不引入 tokio。
- **平台包**：`@napi-rs/cli` 的 `napi build --platform` 生成 4 个 optionalDependencies 子包；主包 `index.js` 按 `process.platform`/`process.arch` 选择性 require。

## SPEC 引用

- §4.1 函数清单（6 动词 + 4 管理函数；M0 实现子集）
- §4.2 参数结构（OpenOptions / CollectionOptions / SearchQuery / Hit / AddReport / Schema）
- §4.3 三侧签名映射：JS `Promise<T>`，reject 携带 `VaneError`（含 code）
- §9.3 Node 不经过 C ABI，napi-rs 直连 core，N-API v6+，异步经 AsyncTask，不桥接 tokio
- §10 错误码（code 透传，不得吞并/重编）
- §12.1 workspace：`crates/vane-node`
- §12.2 目标矩阵 M0：x86_64-linux-gnu / aarch64-apple-darwin / x86_64-apple-darwin / x86_64-pc-windows-msvc
- §13.2-4 平台四包管理器（npm/yarn/pnpm/bun）安装矩阵
- §14 不变量 I-7（FFI 内存铁律；napi 无裸指针出边界）、I-8（binding 薄壳）

## 前置依赖

- **07-api-core**：`Db`、`Collection`、`OpenOptions`、`CollectionOptions`、`SearchQuery`、`SearchMode`、`FusionSpec`、`Filter`、`Hit`、`AddReport`、`Doc`、`ScalarValue`、`Schema`、`FieldDef`、`VaneError`
- **01-vfs**：`StdFsVfs`（生产）、`MemoryVfs`（测试）
- **00-workspace**：`VaneError::code()` / `name()`

## 验收标准

1. `cargo test -p vane-node` 全绿（Rust 单元测试：JSON 转换 + VaneError 映射）。
2. `yarn test`（或 `pnpm test`）JS 集成测试全绿：用 MemoryVfs 注入跑 `open → collection → add → flush → search → close` 全流程，验证返回结构符合 §4.2。
3. `delete` 调用 reject `VaneError` 且 `err.code === -10`、`err.name === 'E_UNSUPPORTED'`。
4. 错误透传：构造 `E_SCHEMA`（vector dim 不匹配）场景，断言 `err.code === -2`。
5. `cargo check -p vane-node` 无 warning；`cargo clippy -p vane-node -- -D warnings` 通过。
6. `napi build` 在本机产出 `.node` 文件；`node -e "require('@vane/node')"` 不抛错。
7. binding crate 内 `grep -rE 'tokio|std::fs|hnsw|bm25|cosine' src/` 为空（薄壳验证 I-8）。

---

## Global Constraints

| 约束 | 值 | 来源 |
|---|---|---|
| 不桥接 tokio | 异步一律 `impl Task` + `AsyncTask`，禁用 `#[napi] async fn`、禁引 `napi::tokio` | §9.3 |
| binding 无检索逻辑（I-8） | crate 内不得出现分词/BM25/向量/融合算法；仅 JSON 转换 + napi 导出 | §14 I-8 |
| 错误码透传不重编（§10） | `VaneError::code()` 原值透传到 JS `.code`；不得把 `E_SCHEMA` 映成 `GenericFailure` 等模糊码 | §10 |
| 内存铁律（I-7） | napi 无裸指针出边界；`VaneDb`/`VaneCollection` 为 `#[napi]` 托管对象，JS 侧 GC 触发 `Drop` | §14 I-7 |
| 无 `std::fs` | binding crate 内禁 `std::fs`（IO 经 core 的 `StdFsVfs`） | §6.1 |
| 无 `cfg(target)` | 平台差异由 napi-rs 与 `napi build --platform` 处理，binding 源码零 cfg | §11/I-5 |
| N-API 版本 | ≥ v6（`AsyncTask` 依赖 `napi_create_async_work`） | §9.3 |
| 4 平台 prebuilt | x86_64-linux-gnu / aarch64-apple-darwin / x86_64-apple-darwin / x86_64-pc-windows-msvc | §12.2 |

---

## File Structure

```
crates/vane-node/
├── Cargo.toml                  # cdylib + lib；napi-derive；serde_json；vane-core dep
├── napi.config.json            # @napi-rs/cli 配置（packageName/版本/三元组）
├── package.json                # @vane/node 主包；optionalDependencies 4 平台子包
├── index.js                    # JS 入口：平台 require 切换 + VaneError 包装
├── src/
│   ├── lib.rs                  # #[napi] module 入口；register；导出 VaneError JS 构造
│   ├── error.rs                # VaneError → napi::Error 映射；reason 编码
│   ├── convert.rs              # serde_json ↔ OpenOptions/CollectionOptions/SearchQuery/Doc/Hit/AddReport/Schema
│   ├── db.rs                   # VaneDb struct + open/close/collection/collections + *Task
│   └── collection.rs           # VaneCollection struct + add/flush/search/delete + *Task
├── __tests__/                  # JS 集成测试（.test.js，用 ava 或 node:test）
│   ├── open-close.test.js
│   ├── add-flush-search.test.js
│   └── error-passthrough.test.js
├── tests/                      # Rust 集成测试（MemoryVfs 注入）
│   └── integration.rs
└── bindings/                   # @napi-rs/cli 生成的 .d.ts（构建产物，gitignored except index.d.ts）
```

---

## 任务清单（bite-sized TDD）

### Task 1：crate 脚手架 + 最小导出验证构建

**Files:** `crates/vane-node/Cargo.toml`, `crates/vane-node/napi.config.json`, `crates/vane-node/package.json`, `crates/vane-node/index.js`, `crates/vane-node/src/lib.rs`, `crates/vane-node/.gitignore`

**Interfaces:**
- Produces: `@vane/node` 包骨架；一个 `hello()` 导出用于验证 napi 构建链路通。
- Consumes: 无（仅验证构建）。

**步骤：**

- [ ] **1.1** 写 `Cargo.toml`：
  ```toml
  [package]
  name = "vane-node"
  version = "0.1.0"
  edition = "2021"
  publish = false

  [lib]
  crate-type = ["cdylib"]

  [dependencies]
  napi = { version = "2", features = ["napi8"] }   # napi8 = N-API v8，满足 v6+
  napi-derive = "2"
  serde_json = "1"
  vane-core = { path = "../vane-core" }

  [build-dependencies]
  napi-build = "2"

  [profile.release]
  lto = true
  opt-level = 3
  ```
  并加 `build.rs`：`fn main() { napi_build::setup(); }`

- [ ] **1.2** 写 `src/lib.rs` 最小导出：
  ```rust
  #![deny(warnings)]

  #[napi]
  pub fn hello() -> String { "vane-node".to_string() }
  ```
  S13 裁决：Task 1 不声明 4 个 mod（error/convert/db/collection），仅留 hello()。Task 2 起逐个加 mod，避免空 mod 掩盖遗漏。

- [ ] **1.3** 写 `napi.config.json`：
  ```json
  {
    "binaryName": "vane",
    "packageName": "@vane/node",
    "packageVersion": "0.1.0",
    "napi": {
      "name": "vane",
      "triples": {
        "defaults": false,
        "additional": [
          "x86_64-unknown-linux-gnu",
          "aarch64-apple-darwin",
          "x86_64-apple-darwin",
          "x86_64-pc-windows-msvc"
        ]
      }
    }
  }
  ```

- [ ] **1.4** 写 `package.json`：
  ```json
  {
    "name": "@vane/node",
    "version": "0.1.0",
    "main": "index.js",
    "types": "index.d.ts",
    "files": ["index.js", "index.d.ts"],
    "optionalDependencies": {
      "@vane/node-linux-x64-gnu": "0.1.0",
      "@vane/node-darwin-arm64": "0.1.0",
      "@vane/node-darwin-x64": "0.1.0",
      "@vane/node-win32-x64-msvc": "0.1.0"
    },
    "devDependencies": {
      "@napi-rs/cli": "^2.18.0",
      "ava": "^6.0.0"
    },
    "scripts": {
      "build": "napi build --platform --release",
      "build:debug": "napi build --platform",
      "test": "ava",
      "prepublishOnly": "napi prepublish -t npm"
    },
    "napi": { "binaryName": "vane", "packageName": "@vane/node" }
  }
  ```

- [ ] **1.5** 写 `index.js`（平台 require 切换 + VaneError 包装骨架）：
  ```js
  // napi-rs 约定：子包名 = `${packageName}-${platformArch}`，platformArch 按 napi-rs naming
  const PlatformPackages = {
    'linux': { 'x64': '@vane/node-linux-x64-gnu' },
    'darwin': { 'arm64': '@vane/node-darwin-arm64', 'x64': '@vane/node-darwin-x64' },
    'win32': { 'x64': '@vane/node-win32-x64-msvc' },
  };
  function loadNative() {
    const plat = process.platform, arch = process.arch;
    const pkg = PlatformPackages[plat]?.[arch];
    if (!pkg) throw new Error(`@vane/node: unsupported platform ${plat}-${arch}`);
    try { return require(pkg); }
    catch (e) {
      // 开发期：未安装子包时回退本地构建产物（.node 文件名由 napi build 生成）
      try { return require('./vane.linux-x64-gnu.node'); } catch (_) { throw e; }
    }
  }
  const native = loadNative();
  // VaneError 包装（详见 Task 2）
  class VaneError extends Error {
    constructor(message, code, name) { super(message); this.code = code; this.name = name; }
  }
  function wrapErr(p) {
    return p.catch(e => {
      const m = /^(-?\d+):(\w+):([\s\S]*)$/.exec(e.message);
      if (m) throw new VaneError(m[3], Number(m[1]), m[2]);
      throw e;
    });
  }
  module.exports = { /* Task 2 起填充 */ __native: native, VaneError };
  ```

- [ ] **1.6** 验证：`cd crates/vane-node && napi build` 成功产出 `.node`；`node -e "console.log(require('./index.js').__native.hello())"` 输出 `vane-node`。

---

### Task 2：VaneDb::open / close / collection / collections + VaneError 映射

**Files:** `src/error.rs`, `src/convert.rs`, `src/db.rs`, `src/lib.rs`, `index.js`, `__tests__/open-close.test.js`

**Interfaces:**
- Consumes from 07-api-core: `Db`, `OpenOptions`, `CollectionOptions`, `PersistenceMode`, `Schema`, `FieldDef`, `Metric`, `BuiltinTokenizer`, `UserDictEntry`
- Consumes from 01-vfs: `StdFsVfs`
- Consumes from 00-workspace: `VaneError`（含 `code()` / `name()`）
- Produces: `VaneDb`（napi 导出）、`open` / `close` / `collection` / `collections`

**关键设计：**
- `VaneDb` 持有 `Db`；要求 `Db: Clone + Send + Sync`（07-api-core 须保证 Arc-based 浅克隆）。async 方法入队前 `self.inner.clone()`。
- `open` 的 `AsyncTask` 内部构造 `StdFsVfs::new()` + 解析 `OpenOptions` + 调 `Db::open`。
- VaneError 映射：`reason` 编码为 `"{code}:{name}:{message}"`，JS 侧解析回 `VaneError`。

**步骤：**

- [ ] **2.1** 写 `src/error.rs`（先写测试）：
  ```rust
  use napi::{bindgen_prelude::*, Status};
  use vane_core::types::VaneError as CoreErr;

  /// 把 core VaneError 转成 napi::Error，reason 编码为 "{code}:{name}:{message}"
  /// JS 侧 index.js 解析回 VaneError(.code/.name)。
  impl From<CoreErr> for Error {
      fn from(e: CoreErr) -> Self {
          let code = e.code();
          let name = e.name();
          let msg = e.to_string();
          let status = match code {
              -11 | -2 => Status::InvalidArg,        // E_INVALID_ARG / E_SCHEMA
              -3 => Status::NotFound,                 // E_NOT_FOUND
              -9 => Status::WouldBlock,               // E_BUSY（语义近）
              -10 => Status::GenericFailure,         // E_UNSUPPORTED
              _ => Status::GenericFailure,
          };
          Error::new(status, format!("{code}:{name}:{msg}"))
      }
  }
  pub type NapiResult<T> = std::result::Result<T, Error>;
  ```
  S15/S20 裁决：M0 的 napi::Status 映射略粗糙（E_UNSUPPORTED → GenericFailure、E_SCHEMA → InvalidArg 等），JS 侧用 `.code` 判定（不依赖 napi::Status）。M1 可细化映射。
  测试（`#[cfg(test)]`）：
  ```rust
  #[test]
  fn reason_round_trip() {
      let e: Error = CoreErr::Schema("dim mismatch".into()).into();
      assert_eq!(e.reason, "-2:E_SCHEMA:dim mismatch");
  }
  ```
  S14 裁决：锁定 napi 版本（`napi = "=2.x"`），用 getter 而非直接访问字段。napi::Error 的 reason 可通过 `e.reason` 访问（napi 2.x 公开该字段）。若 napi 版本升级导致字段不可访问，改用 `format!` 重建。

- [ ] **2.2** 写 `src/convert.rs` 的 `OpenOptions` / `CollectionOptions` / `Schema` 解析：
  ```rust
  use serde_json::Value;
  use vane_core::api::{OpenOptions, CollectionOptions, PersistenceMode};
  use vane_core::persistence::AutoCommitConfig;
  use vane_core::tokenizer::{BuiltinTokenizer, UserDictEntry};
  use vane_core::types::{Schema, FieldDef, Metric, ScalarKind, VaneError};
  use crate::error::NapiResult;

  pub fn parse_open_opts(v: &Value) -> NapiResult<OpenOptions> {
      let persistence = match v.get("persistence").and_then(Value::as_str) {
          Some("best-effort") => PersistenceMode::BestEffort,
          _ => PersistenceMode::Persistent,
      };
      let auto_commit = match v.get("autoCommit") {
          Some(Value::String(s)) if s == "off" => AutoCommitConfig::Off,
          Some(o) => AutoCommitConfig::On {
              interval_ms: o.get("intervalMs").and_then(Value::as_u64).unwrap_or(1000) as u32,
              max_docs: o.get("maxDocs").and_then(Value::as_u64).unwrap_or(1000) as u32,
          },
          None => AutoCommitConfig::default(),
      };
      let page_cache_mb = v.get("pageCacheMb").and_then(Value::as_u64).unwrap_or(32) as u32;
      Ok(OpenOptions { persistence, auto_commit, page_cache_mb })
  }

  pub fn parse_collection_opts(v: &Value) -> NapiResult<CollectionOptions> {
      let tokenizer = match v.get("tokenizer").and_then(Value::as_str) {
          Some("cjk_bigram") => BuiltinTokenizer::CjkBigram,
          Some("jieba") => BuiltinTokenizer::Jieba,
          _ => BuiltinTokenizer::Standard,
      };
      let user_dict = v.get("userDict")
          .and_then(Value::as_array)
          .map(|a| a.iter().map(parse_dict_entry).collect::<Result<_,_>>())
          .transpose()?.unwrap_or_default();
      Ok(CollectionOptions { tokenizer, user_dict })
  }

  fn parse_dict_entry(v: &Value) -> NapiResult<UserDictEntry> {
      match v {
          Value::String(s) => Ok(UserDictEntry::Word(s.clone())),
          o => Ok(UserDictEntry::WordWithFreq {
              term: o.get("term").and_then(Value::as_str).ok_or(VaneError::InvalidArg("userDict.term missing".into()))?.to_string(),
              freq: o.get("freq").and_then(Value::as_u64).unwrap_or(0) as u32,
          }),
      }
  }

  pub fn parse_schema(v: &Value) -> NapiResult<Schema> {
      // B6 裁决：统一为数组形式（与 core Schema{fields: Vec<(String, FieldDef)>} 同构）
      let fields_arr = v.get("fields").and_then(Value::as_array)
          .ok_or(VaneError::InvalidArg("schema.fields must be an array".into()))?;
      let mut fields = Vec::new();
      for entry in fields_arr {
          let name = entry.get("name").and_then(Value::as_str)
              .ok_or(VaneError::InvalidArg("field.name missing".into()))?.to_string();
          let fd = parse_field(entry)?;
          fields.push((name, fd));
      }
      Schema::new(fields).map_err(Into::into)
  }

  fn parse_field(v: &Value) -> NapiResult<FieldDef> {
      let t = v.get("type").and_then(Value::as_str).ok_or(VaneError::InvalidArg("field.type missing".into()))?;
      Ok(match t {
          "text" => FieldDef::Text,
          "vector" => FieldDef::Vector {
              dim: v.get("dim").and_then(Value::as_u64).ok_or(VaneError::InvalidArg("vector.dim missing".into()))? as u32,
              metric: match v.get("metric").and_then(Value::as_str) { Some("l2")=>Metric::L2, Some("dot")=>Metric::Dot, _=>Metric::Cosine },
          },
          "scalar" => FieldDef::Scalar {
              kind: match v.get("kind").and_then(Value::as_str) {
                  Some("int")=>ScalarKind::Int, Some("float")=>ScalarKind::Float,
                  Some("bool")=>ScalarKind::Bool, _=>ScalarKind::Keyword,
              },
          },
          other => return Err(VaneError::InvalidArg(format!("unknown field type {other}")).into()),
      })
  }
  ```
  单测覆盖：`parse_open_opts` 默认值、`parse_schema` 恰好一个 vector 字段（多/零 vector 报 `E_SCHEMA` code=-2）、`parse_collection_opts` jieba 透传。

- [ ] **2.3** 写 `src/db.rs`：
  ```rust
  use napi::bindgen_prelude::*;
  use napi_derive::napi;
  use napi::{AsyncTask, Env};
  use std::sync::Arc;
  use vane_core::api::{Db, OpenOptions};
  use vane_core::vfs::StdFsVfs;
  use crate::convert::{parse_open_opts, parse_schema, parse_collection_opts};
  use crate::collection::VaneCollection;
  use crate::error::NapiResult;

  #[napi]
  pub struct VaneDb { pub(crate) inner: Db }

  pub struct OpenTask { path: String, opts: serde_json::Value }
  #[napi]
  impl Task for OpenTask {
      type Output = Db;
      type JsValue = VaneDb;
      fn compute(&mut self) -> NapiResult<Self::Output> {
          let opts: OpenOptions = parse_open_opts(&self.opts)?;
          let vfs = Arc::new(StdFsVfs::new());
          Db::open(vfs, &self.path, opts).map_err(Into::into)
      }
      fn resolve(&mut self, _env: Env, output: Self::Output) -> NapiResult<Self::JsValue> {
          Ok(VaneDb { inner: output })
      }
  }

  pub struct CollectionTask {
      db: Db, name: String, schema: serde_json::Value, opts: serde_json::Value,
  }
  #[napi]
  impl Task for CollectionTask {
      type Output = vane_core::api::Collection;
      type JsValue = VaneCollection;
      fn compute(&mut self) -> NapiResult<Self::Output> {
          let schema = parse_schema(&self.schema)?;
          let opts = parse_collection_opts(&self.opts)?;
          self.db.collection(&self.name, schema, opts).map_err(Into::into)
      }
      fn resolve(&mut self, _env: Env, output: Self::Output) -> NapiResult<Self::JsValue> {
          Ok(VaneCollection { inner: output })
      }
  }

  pub struct CloseTask { db: Db }
  #[napi]
  impl Task for CloseTask {
      type Output = ();
      type JsValue = ();
      fn compute(&mut self) -> NapiResult<Self::Output> { self.db.close().map_err(Into::into) }
      fn resolve(&mut self, _env: Env, _: ()) -> NapiResult<Self::JsValue> { Ok(()) }
  }

  #[napi]
  impl VaneDb {
      #[napi]
      pub fn open(path: String, opts: serde_json::Value) -> AsyncTask<OpenTask> {
          AsyncTask::new(OpenTask { path, opts })
      }
      #[napi]
      pub fn collection(&self, name: String, schema: serde_json::Value, opts: serde_json::Value) -> AsyncTask<CollectionTask> {
          AsyncTask::new(CollectionTask { db: self.inner.clone(), name, schema, opts })
      }
      #[napi]
      pub fn collections(&self) -> NapiResult<Vec<String>> {
          Ok(self.inner.collections())
      }
      #[napi]
      pub fn close(&self) -> AsyncTask<CloseTask> {
          AsyncTask::new(CloseTask { db: self.inner.clone() })
      }
  }
  ```

- [ ] **2.4** 在 `src/lib.rs` 移除 `hello()`，注册 `mod db; mod collection; mod error; mod convert;`。在 `index.js` 暴露 `open`：
  ```js
  const { VaneDb, VaneCollection } = native;
  module.exports = {
    VaneError,
    open: (path, opts = {}) => wrapErr(VaneDb.open(path, opts)),
    VaneDb,
    VaneCollection,
  };
  ```
  （`wrapErr` 在 Task 1 已定义；`VaneDb.prototype.collection` 等方法需在 JS 侧包一层以应用 `wrapErr`，见下。）

- [ ] **2.5** JS 侧方法包装：在 `index.js` 给 `VaneDb`/`VaneCollection` 原型方法套 `wrapErr`：
  ```js
  for (const [cls, methods] of [
    [VaneDb, ['collection', 'close']],
    [VaneCollection, ['add', 'flush', 'search', 'delete']],
  ]) {
    for (const m of methods) {
      const orig = cls.prototype[m];
      cls.prototype[m] = function (...args) { return wrapErr(orig.apply(this, args)); };
    }
  }
  ```

- [ ] **2.6** 写 `__tests__/open-close.test.js`（ava）：
  ```js
  const test = require('ava');
  const { open, VaneError } = require('..');
  const tmp = require('node:os').tmpdir() + '/vane-test-' + Date.now();

  test('open + close', async t => {
    const db = await open(tmp, {});
    t.truthy(db);
    t.deepEqual(db.collections(), []);
    await db.close();
  });

  test('open with bad opts rejects with VaneError', async t => {
    // pageCacheMb 非法类型 → core 内部不校验 u32 范围外的字符串，但 schema 非法可触发
    const db = await open(tmp + '-b', {});
    await t.throwsAsync(db.collection('c', { fields: [] }, {}), {
      is: VaneError, code: -2,   // E_SCHEMA（零 vector 字段）
    });
    await db.close();
  });
  ```

- [ ] **2.7** 跑 `cargo test -p vane-node --lib`（error/convert 单测）+ `napi build && ava`（JS 测试）。全绿后标记 Task 2 完成。

---

### Task 3：VaneCollection::add / flush（AsyncTask 包装）

**Files:** `src/collection.rs`, `src/convert.rs`（补 Doc/AddReport 序列化）, `__tests__/add-flush-search.test.js`（add/flush 部分，search 断言留 Task 4 后补）

**Interfaces:**
- Consumes from 07-api-core: `Collection`, `Doc`, `AddReport`, `ScalarValue`
- Produces: `VaneCollection::add` / `flush`

**步骤：**

- [ ] **3.1** 在 `src/convert.rs` 补 `parse_docs` 与 `add_report_to_json`：
  ```rust
  pub fn parse_docs(v: &Value) -> NapiResult<Vec<Doc>> {
      let arr = v.as_array().ok_or(VaneError::InvalidArg("docs must be array".into()))?;
      arr.iter().map(parse_doc).collect()
  }
  fn parse_doc(v: &Value) -> NapiResult<Doc> {
      let id = v.get("id").and_then(Value::as_str).ok_or(VaneError::InvalidArg("doc.id missing".into()))?.to_string();
      let text = v.get("text").and_then(Value::as_str).map(String::from);
      let vector = v.get("vector").and_then(Value::as_array).map(|a| {
          a.iter().filter_map(|x| x.as_f64().map(|f| f as f32)).collect::<Vec<_>>()
      });
      let meta = v.get("meta").and_then(Value::as_object).map(|o| {
          o.iter().filter_map(|(k, v)| parse_scalar(v).map(|sv| (k.clone(), sv))).collect()
      });
      Ok(Doc { id, text, vector, meta })
  }
  fn parse_scalar(v: &Value) -> Option<ScalarValue> {
      match v {
          Value::Number(n) if n.is_i64() => Some(ScalarValue::Int(n.as_i64().unwrap())),
          Value::Number(n) if n.is_f64() => Some(ScalarValue::Float(n.as_f64().unwrap())),
          Value::Bool(b) => Some(ScalarValue::Bool(*b)),
          Value::String(s) => Some(ScalarValue::Keyword(s.clone())),
          _ => None,
      }
  }
  pub fn add_report_to_json(r: AddReport) -> Value {
      serde_json::json!({ "accepted": r.accepted, "visibleAfterFlush": r.visible_after_flush })
  }
  ```

- [ ] **3.2** 写 `src/collection.rs` 的 `add` / `flush`：
  ```rust
  use napi::bindgen_prelude::*;
  use napi_derive::napi;
  use napi::{AsyncTask, Env};
  use vane_core::api::{Collection, Doc, AddReport};
  use crate::convert::{parse_docs, add_report_to_json};
  use crate::error::NapiResult;

  #[napi]
  pub struct VaneCollection { pub(crate) inner: Collection }

  pub struct AddTask { col: Collection, docs: serde_json::Value }
  #[napi]
  impl Task for AddTask {
      type Output = AddReport;
      type JsValue = serde_json::Value;
      fn compute(&mut self) -> NapiResult<Self::Output> {
          let docs: Vec<Doc> = parse_docs(&self.docs)?;
          self.col.add(&docs).map_err(Into::into)
      }
      fn resolve(&mut self, _env: Env, output: Self::Output) -> NapiResult<Self::JsValue> {
          Ok(add_report_to_json(output))
      }
  }

  pub struct FlushTask { col: Collection }
  #[napi]
  impl Task for FlushTask {
      type Output = (); type JsValue = ();
      fn compute(&mut self) -> NapiResult<Self::Output> { self.col.flush().map_err(Into::into) }
      fn resolve(&mut self, _env: Env, _: ()) -> NapiResult<Self::JsValue> { Ok(()) }
  }

  #[napi]
  impl VaneCollection {
      #[napi]
      pub fn add(&self, docs: serde_json::Value) -> AsyncTask<AddTask> {
          AsyncTask::new(AddTask { col: self.inner.clone(), docs })
      }
      #[napi]
      pub fn flush(&self) -> AsyncTask<FlushTask> {
          AsyncTask::new(FlushTask { col: self.inner.clone() })
      }
  }
  ```
  （`search` / `delete` 在 Task 4/5 补。）

- [ ] **3.3** 单测：`parse_docs` 对缺 id、vector 维度与 schema 不符（在 add 时由 core 校验报 `E_SCHEMA`）的转换路径覆盖。

- [ ] **3.4** JS 测试 `__tests__/add-flush-search.test.js` 的 add/flush 段：
  ```js
  const test = require('ava');
  const { open } = require('..');
  const tmp = require('node:os').tmpdir() + '/vane-af-' + Date.now();

  test('add returns AddReport, flush resolves', async t => {
    const db = await open(tmp, {});
    const col = await db.collection('docs', {
      fields: [
        { name: 'title', type: 'text' },
        { name: 'v', type: 'vector', dim: 3 },
      ],
    }, {});
    const r = await col.add([
      { id: 'a', text: 'hello world', vector: [1,0,0] },
      { id: 'b', text: 'hello rust',  vector: [0,1,0] },
    ]);
    t.is(r.accepted, 2);
    t.true(r.visibleAfterFlush);
    await col.flush();
    await db.close();
  });
  ```

- [ ] **3.5** 跑测试全绿。

---

### Task 4：VaneCollection::search（JSON→SearchQuery + Hit→JSON）

**Files:** `src/convert.rs`（补 SearchQuery 解析、Hit 序列化）, `src/collection.rs`（SearchTask）

**Interfaces:**
- Consumes from 07-api-core: `SearchQuery`, `SearchMode`, `FusionSpec`, `Filter`, `FilterCond`, `ScalarValue`, `Hit`
- Produces: `VaneCollection::search`

**步骤：**

- [ ] **4.1** 在 `src/convert.rs` 补 `parse_search_query`：
  ```rust
  pub fn parse_search_query(v: &Value) -> NapiResult<SearchQuery> {
      let text = v.get("text").and_then(Value::as_str).map(String::from);
      let vector = v.get("vector").and_then(Value::as_array).map(|a|
          a.iter().filter_map(|x| x.as_f64().map(|f| f as f32)).collect::<Vec<_>>());
      if text.is_none() && vector.is_none() {
          return Err(VaneError::InvalidArg("text or vector required".into()).into());
      }
      let top_k = v.get("topK").and_then(Value::as_u64).unwrap_or(10) as u32;
      let mode = match v.get("mode").and_then(Value::as_str) {
          Some("hybrid")=>SearchMode::Hybrid, Some("vector")=>SearchMode::Vector,
          Some("text")=>SearchMode::Text, _=>SearchMode::Auto,
      };
      let fusion = match v.get("fusion") {
          Some(Value::String(s)) if s == "rrf" => FusionSpec::Rrf,
          Some(o) => FusionSpec::Linear {
              alpha: o.get("linear").and_then(|l| l.get("alpha")).and_then(Value::as_f64).unwrap_or(0.5) as f32,
          },
          None => FusionSpec::Rrf,
      };
      // M0: filter 不实现，传非空 filter 由 core 返回 InvalidArg
      let filter = None;
      let candidate_multiplier = v.get("candidateMultiplier").and_then(Value::as_u64).unwrap_or(3) as u32;
      Ok(SearchQuery { text, vector, top_k, mode, fusion, filter, candidate_multiplier })
  }

  pub fn hits_to_json(hits: Vec<Hit>) -> Value {
      Value::Array(hits.into_iter().map(|h| {
          let fields = h.fields.map(|m| {
              let mut o = serde_json::Map::new();
              for (k, v) in m { o.insert(k, Value::String(v)); }
              Value::Object(o)
          }).unwrap_or(Value::Null);
          serde_json::json!({ "id": h.id, "score": h.score, "fields": fields })
      }).collect())
  }
  ```
  注意：M0 `filter` 字段不解析（始终 `None`）；JS 传 `filter` 会被静默忽略——但为遵守「不得吞并/重编」精神，若 `v.get("filter").is_some() && 非空`，应返回 `E_INVALID_ARG`（M0 不支持）。补一行：
  ```rust
  if v.get("filter").map_or(false, |f| !f.is_null()) {
      return Err(VaneError::InvalidArg("filter not supported in M0".into()).into());
  }
  ```

- [ ] **4.2** 在 `src/collection.rs` 加 `SearchTask` 与 `search` 方法：
  ```rust
  pub struct SearchTask { col: Collection, query: serde_json::Value }
  #[napi]
  impl Task for SearchTask {
      type Output = Vec<vane_core::api::Hit>;
      type JsValue = serde_json::Value;
      fn compute(&mut self) -> NapiResult<Self::Output> {
          let q = crate::convert::parse_search_query(&self.query)?;
          self.col.search(&q).map_err(Into::into)
      }
      fn resolve(&mut self, _env: Env, output: Self::Output) -> NapiResult<Self::JsValue> {
          Ok(crate::convert::hits_to_json(output))
      }
  }
  #[napi]
  impl VaneCollection {
      #[napi]
      pub fn search(&self, query: serde_json::Value) -> AsyncTask<SearchTask> {
          AsyncTask::new(SearchTask { col: self.inner.clone(), query })
      }
  }
  ```

- [ ] **4.3** 单测：`parse_search_query` 默认值、`mode`/`fusion` 枚举映射、`filter` 非 null 报 `-11`。

- [ ] **4.4** JS 测试补 search 断言（在 `add-flush-search.test.js` 末尾，flush 后）：
  ```js
  const hits = await col.search({ text: 'hello', vector: [1,0,0], topK: 5, mode: 'hybrid' });
  t.is(hits.length, 2);
  t.is(hits[0].id, 'a');            // 文档 a 同时命中 text+vector
  t.true(typeof hits[0].score === 'number');
  t.true(hits[0].fields === null || typeof hits[0].fields === 'object');
  ```

- [ ] **4.5** 跑测试全绿。

---

### Task 5：delete 占位 reject Unsupported

**Files:** `src/collection.rs`, `__tests__/error-passthrough.test.js`

**Interfaces:**
- Produces: `VaneCollection::delete`（M0 占位，reject `E_UNSUPPORTED`）

**步骤：**

- [ ] **5.1** 在 `src/collection.rs` 加 `DeleteTask`：core 的 `Collection::delete` 已是占位 `Err(VaneError::Unsupported)`，binding 透传：
  ```rust
  pub struct DeleteTask { col: Collection, ids: Vec<String> }
  #[napi]
  impl Task for DeleteTask {
      type Output = u64; type JsValue = u64;
      fn compute(&mut self) -> NapiResult<Self::Output> {
          self.col.delete(&self.ids).map_err(Into::into)
      }
      fn resolve(&mut self, _env: Env, n: u64) -> NapiResult<Self::JsValue> { Ok(n) }
  }
  #[napi]
  impl VaneCollection {
      #[napi]
      pub fn delete(&self, ids: Vec<String>) -> AsyncTask<DeleteTask> {
          AsyncTask::new(DeleteTask { col: self.inner.clone(), ids })
      }
  }
  ```

- [ ] **5.2** 写 `__tests__/error-passthrough.test.js`：
  ```js
  const test = require('ava');
  const { open, VaneError } = require('..');
  const tmp = require('node:os').tmpdir() + '/vane-err-' + Date.now();

  test('delete rejects E_UNSUPPORTED (code -10)', async t => {
    const db = await open(tmp, {});
    const col = await db.collection('c', { fields: [{ name: 'v', type: 'vector', dim: 2 }] }, {});
    await t.throwsAsync(col.delete(['x']), { is: VaneError, code: -10, name: 'E_UNSUPPORTED' });
    await db.close();
  });

  test('dim mismatch rejects E_SCHEMA (code -2)', async t => {
    const db = await open(tmp + '-2', {});
    const col = await db.collection('c', { fields: [{ name: 'v', type: 'vector', dim: 3 }] }, {});
    await t.throwsAsync(col.add([{ id: 'a', vector: [1, 2] }]), { is: VaneError, code: -2 });
    await db.close();
  });
  ```

- [ ] **5.3** 跑测试全绿。验证 `err.code`/`err.name` 透传正确。

---

### Task 5b：export / reindex napi 包装

**Files:** `src/db.rs`, `src/collection.rs`, `index.js`, `__tests__/error-passthrough.test.js`

**Interfaces:**
- Consumes from 07-api-core: `Db::export`、`Collection::reindex`
- Produces: `VaneDb::export`、`VaneCollection::reindex`（均为 `AsyncTask` 包装）

**步骤：**

- [ ] **5b.1** 在 `src/db.rs` 中加 `ExportTask`：
  ```rust
  pub struct ExportTask { db: Db, dest: String }
  #[napi]
  impl Task for ExportTask {
      type Output = (); type JsValue = ();
      fn compute(&mut self) -> NapiResult<Self::Output> {
          self.db.export(&self.dest).map_err(Into::into)
      }
      fn resolve(&mut self, _env: Env, _: ()) -> NapiResult<Self::JsValue> { Ok(()) }
  }
  #[napi]
  impl VaneDb {
      #[napi]
      pub fn export(&self, dest: String) -> AsyncTask<ExportTask> {
          AsyncTask::new(ExportTask { db: self.inner.clone(), dest })
      }
  }
  ```

- [ ] **5b.2** 在 `src/collection.rs` 中加 `ReindexTask`：
  ```rust
  pub struct ReindexTask { col: Collection }
  #[napi]
  impl Task for ReindexTask {
      type Output = (); type JsValue = ();
      fn compute(&mut self) -> NapiResult<Self::Output> {
          self.col.reindex().map_err(Into::into)
      }
      fn resolve(&mut self, _env: Env, _: ()) -> NapiResult<Self::JsValue> { Ok(()) }
  }
  #[napi]
  impl VaneCollection {
      #[napi]
      pub fn reindex(&self) -> AsyncTask<ReindexTask> {
          AsyncTask::new(ReindexTask { col: self.inner.clone() })
      }
  }
  ```

- [ ] **5b.3** 在 `index.js` 的方法包装中追加 `export` 到 `VaneDb`、`reindex` 到 `VaneCollection`：
  ```js
  for (const [cls, methods] of [
    [VaneDb, ['collection', 'close', 'export']],
    [VaneCollection, ['add', 'flush', 'search', 'delete', 'reindex']],
  ]) {
  ```

- [ ] **5b.4** 在 `error-passthrough.test.js` 中追加 reindex reject 测试：
  ```js
  test('reindex rejects E_UNSUPPORTED (code -10)', async t => {
    const db = await open(tmp + '-ri', {});
    const col = await db.collection('c', { fields: [{ name: 'v', type: 'vector', dim: 2 }] }, {});
    await t.throwsAsync(col.reindex(), { is: VaneError, code: -10, name: 'E_UNSUPPORTED' });
    await db.close();
  });
  ```

- [ ] **5b.5** 跑测试全绿。

---

### Task 6：JS 集成测试（MemoryVfs 注入跑全流程）+ 薄壳门禁

**Files:** `tests/integration.rs`, `__tests__/full-cycle.test.js`, `src/lib.rs`（测试辅助导出，`#[cfg(feature = "test-utils")]`）

**Interfaces:**
- Consumes from 01-vfs: `MemoryVfs`
- Produces: 端到端集成测试 + I-8 薄壳自检脚本

**关键设计：**
- Rust 侧 `tests/integration.rs` 用 `MemoryVfs` 直接调 core，验证 core 行为（napi 层不引入 JS，纯 Rust 集成测）。
- JS 侧 `__tests__/full-cycle.test.js` 用真实 `StdFsVfs`（经 `open(path)`）跑 add→flush→search 全链路，验证 napi 桥接无误。
- 不向 JS 暴露 `MemoryVfs`（生产 API 不需要；保持薄壳）。

**步骤：**

- [ ] **6.1** 写 `tests/integration.rs`（Rust 集成测试，MemoryVfs + core 直连，校验 napi 转换函数与 core 语义一致）：
  ```rust
  use std::sync::Arc;
  use vane_core::api::{Db, OpenOptions};
  use vane_core::vfs::MemoryVfs;

  #[test]
  fn full_cycle_memory_vfs() {
      let vfs = Arc::new(MemoryVfs::new());
      let db = Db::open(vfs.clone(), "mem", OpenOptions::default()).unwrap();
      let schema = vane_core::types::Schema::new(vec![
          ("t".into(), vane_core::types::FieldDef::Text),
          ("v".into(), vane_core::types::FieldDef::Vector { dim: 2, metric: vane_core::types::Metric::Cosine }),
      ]).unwrap();
      let col = db.collection("c", schema, Default::default()).unwrap();
      let docs = vec![
          vane_core::api::Doc { id: "a".into(), text: Some("foo bar".into()), vector: Some(vec![1.0,0.0]), meta: None },
          vane_core::api::Doc { id: "b".into(), text: Some("foo baz".into()), vector: Some(vec![0.0,1.0]), meta: None },
      ];
      col.add(&docs).unwrap();
      col.flush().unwrap();
      let q = vane_core::api::SearchQuery {
          text: Some("foo".into()), vector: Some(vec![1.0,0.0]),
          top_k: 10, mode: vane_core::api::SearchMode::Hybrid,
          fusion: vane_core::api::FusionSpec::Rrf, filter: None, candidate_multiplier: 3,
      };
      let hits = col.search(&q).unwrap();
      assert_eq!(hits.len(), 2);
      assert!(hits[0].score >= hits[1].score);
      db.close().unwrap();
  }
  ```

- [ ] **6.2** 写 `__tests__/full-cycle.test.js`（JS 端到端，真实 StdFsVfs）：
  ```js
  const test = require('ava');
  const { open } = require('..');
  const tmp = require('node:os').tmpdir() + '/vane-full-' + Date.now();

  test('full cycle: open→collection→add→flush→search→close', async t => {
    const db = await open(tmp, { autoCommit: 'off' });
    const col = await db.collection('wiki', {
      fields: [
        { name: 'title', type: 'text' },
        { name: 'body', type: 'text' },
        { name: 'v', type: 'vector', dim: 4, metric: 'cosine' },
      ],
    }, { tokenizer: 'standard' });

    const r = await col.add([
      { id: '1', text: 'rust programming language', vector: [0.9,0.1,0.0,0.0] },
      { id: '2', text: 'go programming language',  vector: [0.1,0.9,0.0,0.0] },
      { id: '3', text: 'rust memory safety',        vector: [0.8,0.2,0.0,0.0] },
    ]);
    t.is(r.accepted, 3);
    await col.flush();

    // vector-only
    const vhits = await col.search({ vector: [0.9,0.1,0,0], topK: 2, mode: 'vector' });
    t.is(vhits[0].id, '1');

    // text-only
    const thits = await col.search({ text: 'rust', topK: 3, mode: 'text' });
    t.true(thits.map(h => h.id).includes('1'));

    // hybrid
    const hhits = await col.search({ text: 'rust', vector: [0.9,0.1,0,0], topK: 3, mode: 'hybrid' });
    t.is(hhits[0].id, '1');

    t.deepEqual(db.collections(), ['wiki']);
    await db.close();
  });
  ```

- [ ] **6.3** 薄壳自检脚本（加到 `package.json` scripts `"check:thin"`）：
  ```bash
  #!/bin/sh
  # 退出码 0 = 薄壳干净
  if grep -rnE 'tokio|std::fs|hnsw|bm25|cosine|dot_product|rrf|linear_fuse' crates/vane-node/src/; then
    echo "I-8 violation: retrieval logic or forbidden IO found in binding"; exit 1
  fi
  ```
  `package.json` 加 `"check:thin": "sh scripts/check-thin.sh"`（脚本放 `crates/vane-node/scripts/check-thin.sh`）。

- [ ] **6.4** 跑全部测试：`cargo test -p vane-node`、`napi build && yarn test`、`yarn check:thin`。全绿后 Task 6 完成。

---

## 完成定义（DoD）

- 6 个 Task 全部 checkbox 完成。
- `crates/vane-node` 在 4 平台之一（本机）`napi build` 通过，`.node` 可被 `require`。
- `cargo test -p vane-node` + `yarn test` 全绿。
- `yarn check:thin` 通过（I-8 薄壳门禁）。
- `index.d.ts` 类型声明（`napi build` 自动生成）已提交，含 `VaneDb`/`VaneCollection`/`VaneError`/`open`。
- 4 平台 prebuilt 的 `release.yml` 构建配置由 **10-ci-gates** 计划落地；本计划交付 `package.json` + `napi.config.json` + 三方包结构，确保 10-ci-gates 可直接调用 `napi build --platform` 产出 4 子包。

## 风险与备注

- **`Db`/`Collection` 必须 `Clone + Send + Sync`**：本计划依赖 `self.inner.clone()` 把数据移入 `AsyncTask`。若 07-api-core 的 `Db`/`Collection` 不是 Arc-based 浅克隆，需在 07 计划加 `#[derive(Clone)]` 或改 holding 为 `Arc<Db>`（后者偏离 README 契约，优先要求 07 提供 Clone）。
- **napi-rs `async fn` vs `AsyncTask`**：本计划显式选择 `AsyncTask`（libuv worker pool）以满足 §9.3 不桥接 tokio。README 中 `async fn` 签名视为 JS 侧 `Promise<T>` 语义，非 Rust 侧 `async fn` 关键字。
- **错误 `.code` 透传方式**：napi-rs 的 `napi::Error` 无原生 `code` 字段，本计划用 reason 前缀 `{code}:{name}:{msg}` + JS `wrapErr` 解析回 `VaneError` 子类。此为纯胶水（无检索逻辑），不违反 I-8。
- **`index.d.ts`**：`@napi-rs/cli` 自动生成，需在 CI（10-ci-gates）确认随每次 release 更新。
- S17 裁决：schema 的 text 字段名由用户定义（如 'content' / 'text' / 'body'），Doc.text 是固定 API 字段映射到 schema 中第一个 text 类型字段。
- S18 裁决：release.yml 的 napi artifacts/publish 命令需 @napi-rs/cli 支持，Task 5 本地验证命令存在（`napi build --platform --release` + `napi artifacts` + `napi publish`）。
