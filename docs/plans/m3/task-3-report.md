# M3 Task 3 Report —— @vane-rs/web JS/TS 源码 + tsc 编译

> implementer：Task 3 implementer（Claude）
> 日期：2026-08-11
> 分支：feat/m3-web-npm
> 任务：在 Task 2 骨架上实现 bindings/web/src/*.ts 手写 JS/TS 源码 + 扩展 build-web.sh 加 tsc 编译步骤。

## 状态

**DONE**

核心交付完成：4 个 TS 源文件（types/probe/worker/index）+ vane_wasm.d.ts 类型桥接 + tsconfig.json + build-web.sh tsc 步骤 + package.json devDep/scripts + README 补全。tsc 编译通过，ESM 导出冒烟通过，SIMD128_TEST_MODULE 逐字节对齐，TS 类型与 worker.rs 字段名逐一核对无误，crates/vane-wasm/ 零改动。

## Commits

```
3129850 feat(web): @vane-rs/web JS/TS 源码 + tsc 编译——M3 Task 3
```

（单 commit，含 bindings/web/src/ 5 文件 + tsconfig.json + package-lock.json + 修改 build-web.sh/package.json/README.md）

## 测试摘要

`bash bindings/web/scripts/build-web.sh` 全流程成功（cargo 双变体 + wasm-bindgen + wasm-opt + tsc + W8 + 体积门禁），产出 dist/ 13 文件；tsc 零错误；ESM 冒烟 `createVane` 是 function + `SIMD128_TEST_MODULE` 是 Uint8Array(50)；probe.ts/probe.js/simd_probe.rs 三方字节逐一致（50 bytes）；双变体 gzip simd 318424 / scalar 320589 ≤ 819200；crates/vane-wasm/ 零改动。

## Concerns

### C1：src/vane_wasm.d.ts 类型桥接的编译顺序依赖（低）

- `src/vane_wasm.d.ts` 通过 `export * from '../dist/vane_wasm.js'; export { default } from '../dist/vane_wasm.js';` 桥接 wasm-bindgen 生成的类型。
- tsc 编译时需要 `dist/vane_wasm.d.ts` 已存在（build-web.sh 步骤 4 产出，步骤 6 tsc 消费）。
- 若开发者单独跑 `npm run tsc`（不跑 wasm-bindgen），会报 TS2307 Cannot find module。
- **缓解**：build-web.sh 是唯一构建入口，自动处理顺序；`npm run tsc` 仅供 wasm 已构建后的快速类型检查。README 已说明构建前置。

### C2：dist/types.js 空模块（极低）

- `src/types.ts` 仅含类型声明，tsc 产出 `dist/types.js`（`export {}` 空模块）。
- 无功能影响，不增加运行时开销。npm 包多一个 421 bytes 文件可忽略。

### C3：worker.d.ts 仅 `export {}`（极低）

- `src/worker.ts` 无导出（仅副作用：注册 `self.onmessage`），tsc 产出 `dist/worker.d.ts` 为 `export {}`。
- 用户通过 `@vane-rs/web` 主入口 import `createVane`，不直接 import worker 入口的类型。符合设计（worker 是内部实现细节）。

### C4：TS 5.7 Uint8Array 泛型化适配（低）

- TS 5.7 将 `Uint8Array` 泛型化为 `Uint8Array<TArrayBuffer extends ArrayBufferLike>`，默认 `ArrayBufferLike`（含 `SharedArrayBuffer`）。
- `WebAssembly.validate()` 接受 `BufferSource`（要求 `ArrayBuffer`，非 `SharedArrayBuffer`），显式标注 `: Uint8Array` 会 widen 到 `ArrayBufferLike` 导致类型不兼容。
- **解法**：移除 `SIMD128_TEST_MODULE` 的显式 `: Uint8Array` 标注，让 TS 推断 `Uint8Array<ArrayBuffer>`（`new Uint8Array([...])` 分配新 `ArrayBuffer`）。
- `typescript: ^5.7.0` 钉在 devDeps，版本可控。

### C5：dictData.buffer as ArrayBuffer 类型断言（低）

- `index.ts` 中 `dictData.buffer as ArrayBuffer` 断言假设 `Uint8Array` 的 backing buffer 是 `ArrayBuffer`（非 `SharedArrayBuffer`）。
- 典型用法 `new Uint8Array(await (await fetch()).arrayBuffer())` 的 buffer 恒为 `ArrayBuffer`，无实际风险。
- 文档已说明部分视图需先 `.slice()`。

## 产出文件清单

### bindings/web/src/（入库 5 文件）

```
bindings/web/src/
├── types.ts           # 7.0KB，VaneWorkerOpts/Schema/FieldSchema/Doc/Hit/SearchQuery/OpenOptions/CollectionOptions/Vane 接口
├── probe.ts            # 2.3KB，SIMD128_TEST_MODULE（50 bytes，逐字节复制 simd_probe.rs）+ simd128Supported()
├── worker.ts           # 3.5KB，Worker 入口：探针 + 选 .wasm + init(wasmUrl) + VaneWorker + postMessage 路由
├── index.ts            # 6.5KB，createVane() 工厂 + VaneImpl（postMessage Promise 封装 + dictData transferable）+ 类型 re-export
└── vane_wasm.d.ts      # 0.3KB，类型桥接（export * + export { default } from '../dist/vane_wasm.js'）
```

### bindings/web/dist/（build 产物，不入库，npm 发布内容）

```
bindings/web/dist/
├── index.js            # 7.6KB，tsc 编译 src/index.ts → createVane 工厂 + probe re-export
├── index.d.ts          # 1.3KB，TS 类型（createVane 签名 + 类型 re-export）
├── worker.js           # 4.2KB，tsc 编译 src/worker.ts → Worker 入口
├── worker.d.ts         # 11B，export {}（worker 无导出）
├── probe.js            # 2.3KB，tsc 编译 src/probe.ts → SIMD128_TEST_MODULE + simd128Supported()
├── probe.d.ts          # 0.9KB，TS 类型
├── types.js            # 0.4KB，export {}（类型仅 .d.ts）
├── types.d.ts          # 7.0KB，TS 类型定义
├── vane_wasm.js        # 34KB，wasm-bindgen 生成 ESM 胶水（Task 2，未改动）
├── vane_wasm.d.ts      # 8.3KB，wasm-bindgen 生成 TS 类型（Task 2，tsc 未覆盖）
├── vane_wasm_simd.wasm # 804KB raw / 318KB gzip，SIMD128 加速（Task 2，未改动）
├── vane_wasm_scalar.wasm # 815KB raw / 321KB gzip，scalar 兜底（Task 2，未改动）
└── vane_wasm_bg.wasm   # 815KB，cp scalar 别名（Task 2，未改动）
```

### 其他修改文件

```
bindings/web/
├── tsconfig.json       # 新增，tsc 配置（ES2020/ESNext/bundler/strict/DOM+WebWorker）
├── package.json        # +devDependencies(typescript ^5.7.0) +scripts(build/tsc)
├── package-lock.json   # 新增，npm install 产出
├── README.md           # 补全 API + 用法示例 + 产物表
└── scripts/build-web.sh # +步骤 6 tsc 编译
```

## TS 类型对齐 worker.rs 字段名核对表

### VaneWorkerOpts ↔ parse_worker_opts（worker.rs L424-454）+ extract_dict_data（L616-628）

| worker.rs 字段 | JSON key | TS 类型 | 对齐 |
|----------------|----------|---------|------|
| vfs_kind | `vfs` | `VfsKind = 'opfs' \| 'idb' \| 'memory'` | ✅ 默认 opfs |
| db_path | `dbPath` | `string` | ✅ 默认 "vane.db" |
| dict_url | `dictUrl` | `string` | ✅ |
| dict_sha256 | `dictSha256` | `string`（16 hex chars） | ✅ |
| dict_data | `dictData` | `Uint8Array \| ArrayBuffer` | ✅ extract_dict_data Uint8Array/ArrayBuffer |

### FieldSchema ↔ parse_schema（worker.rs L205-248）

| worker.rs 字段 | JSON key | TS 类型 | 对齐 |
|----------------|----------|---------|------|
| name | `name` | `string` | ✅ |
| FieldDef::Text | `type: "text"` | `TextFieldSchema.type: 'text'` | ✅ |
| FieldDef::Vector{dim,metric} | `type: "vector"`, `dim`, `metric` | `VectorFieldSchema.dim: number, .metric?: VectorMetric` | ✅ metric 默认 cosine |
| FieldDef::Scalar{kind} | `type: "scalar"`, `kind` | `ScalarFieldSchema.kind?: 'int'\|'float'\|'bool'\|'keyword'` | ✅ kind 默认 keyword |

> ⚠️ 设计文档 §6 草案用 `type: 'text' \| 'vector' \| 'keyword' \| 'integer' \| 'float'`，与 worker.rs 实现不一致。Task 3 以 worker.rs 为准：`type: 'text' \| 'vector' \| 'scalar'`（scalar 有子字段 `kind`）。

### Doc ↔ parse_docs（worker.rs L250-292）

| worker.rs 字段 | JSON key | TS 类型 | 对齐 |
|----------------|----------|---------|------|
| id | `id` | `string` | ✅ 必填 |
| text | `text` | `string?` | ✅ |
| vector | `vector` | `number[]?` | ✅ |
| meta | `meta` | `Record<string, number \| boolean \| string>?` | ✅ ScalarValue 映射 |

### SearchQuery ↔ parse_search_query（worker.rs L294-339）

| worker.rs 字段 | JSON key | TS 类型 | 对齐 |
|----------------|----------|---------|------|
| text | `text` | `string?` | ✅ |
| vector | `vector` | `number[]?` | ✅ |
| top_k | `topK` | `number?` | ✅ 默认 10 |
| mode | `mode` | `SearchMode = 'hybrid'\|'vector'\|'text'\|'auto'` | ✅ 默认 auto |
| fusion | `fusion` | `FusionSpec = 'rrf' \| { linear: { alpha? } }` | ✅ 默认 rrf |
| candidate_multiplier | `candidateMultiplier` | `number?` | ✅ 默认 3 |
| filter | `filter` | ❌ 不支持（worker.rs 返 VaneError） | ✅ TS 类型不含 filter |

### OpenOptions ↔ parse_open_opts（worker.rs L142-154）

| worker.rs 字段 | JSON key | TS 类型 | 对齐 |
|----------------|----------|---------|------|
| persistence | `persistence` | `'persistent' \| 'best-effort'` | ✅ 默认 persistent |
| auto_commit | `autoCommit` | `AutoCommit = 'off' \| { intervalMs?, maxDocs? }` | ✅ 默认 On{1000,1000} |
| page_cache_mb | `pageCacheMb` | `number?` | ✅ 默认 32 |

### CollectionOptions ↔ parse_collection_opts（worker.rs L170-203）

| worker.rs 字段 | JSON key | TS 类型 | 对齐 |
|----------------|----------|---------|------|
| tokenizer | `tokenizer` | `TokenizerKind = 'jieba'\|'cjk_bigram'\|'standard'` | ✅ 默认 standard |
| user_dict | `userDict` | `UserDictEntry[] = (string \| { term, freq })[]` | ✅ |
| auto_commit | `autoCommit` | `AutoCommit` | ✅ 同 OpenOptions |

### Hit ↔ hits_to_json（worker.rs L341-360）

| worker.rs 字段 | JSON key | TS 类型 | 对齐 |
|----------------|----------|---------|------|
| id | `id` | `string` | ✅ |
| score | `score` | `number` | ✅ |
| fields | `fields` | `Record<string, string> \| null` | ✅ |

### Vane 接口 ↔ VaneWorker 方法（vane_wasm.d.ts）

| VaneWorker 方法 | Vane 接口方法 | op | 对齐 |
|----------------|---------------|-----|------|
| `create(opts)` | `createVane(opts)` | `create` | ✅ |
| `open(path, opts)` | `open(path?, opts?)` | `open` | ✅ |
| `collection(name, schema, opts)` | `collection(name, schema, opts?)` | `collection` | ✅ 返回 u32→number |
| `add(col, docs)` | `add(col, docs)` | `add` | ✅ 返回 u64→number |
| `flush(col)` | `flush(col)` | `flush` | ✅ |
| `search(col, query)` | `search(col, query)` | `search` | ✅ 返回 JSON string→Hit[] |
| `delete(col, ids)` | `delete(col, ids)` | `delete` | ✅ 返回 u64→number |
| `compact(col)` | `compact(col)` | `compact` | ✅ |
| `reindex(col)` | `reindex(col)` | `reindex` | ✅ 返回 f32→number |
| `export(dest)` | `export(dest)` | `export` | ✅ |
| `readFile(path)` | `readFile(path)` | `readFile` | ✅ 返回 Uint8Array |
| `close()` | `close()` | `close` | ✅ +terminate worker |

## 验证清单

- [x] `bash bindings/web/scripts/build-web.sh` 成功产出 dist/ 全部 13 文件
- [x] tsc 编译零错误（`./node_modules/.bin/tsc` 无输出）
- [x] wasm 体积门禁：双变体 gzip ≤800KB（simd 318424 / scalar 320589）
- [x] W8 校验：vane_wasm.js 含 `__wbg_init` + `new URL(..., import.meta.url)`
- [x] probe.ts SIMD128_TEST_MODULE 与 simd_probe.rs 逐字节一致（50 bytes，三方对比 probe.ts/probe.js/simd_probe.rs）
- [x] TS 类型与 worker.rs 字段名对齐（上述核对表全覆盖）
- [x] git diff 确认未改 crates/vane-wasm/ 任何 .rs（冻结契约遵守）
- [x] ESM 导出冒烟：`node --input-type=module -e "import('./dist/index.js')..."` → createVane 是 function，返回 Promise
- [x] dist/vane_wasm.d.ts 未被 tsc 覆盖（仍为 wasm-bindgen 版本）
- [x] dist/worker.js import 路径正确（`./vane_wasm.js` + `./probe.js`）
- [x] package.json exports/sideEffects/main/module/types 未改动（Task 2 预置，Task 3 只加 devDep + scripts）

## Task 4 衔接

Task 4（如规划中）可在 @vane-rs/web 基础上：
- examples/vite/ + examples/webpack/ 集成示例（Task 7/8）
- 文档站集成页（Task 9）
- @vane-rs/dict-zh 包（Task 5，optionalDep 引用已在 package.json）
- release.yml npm publish（Task 11）
