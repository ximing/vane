# @vane-rs/web

Vane 混合检索库的 Web 端 npm 包：向量检索 + BM25 + RRF 融合，跑在浏览器 Worker 内，
通过 wasm-bindgen `--target web` 产出 ESM 双变体（SIMD128 / scalar），运行时探针自动选择。

- **双变体**：SIMD128 加速 + scalar 兜底，一份 JS 胶水共享。
- **Worker 模式**：主线程零阻塞，wasm 在 Dedicated Worker 内运行。
- **VFS**：OPFS / IndexedDB / memory 三后端，持久化到浏览器存储。
- **词典**：`@vane-rs/dict-zh` 作 optionalDep，`dictData` 内联 transferable 零拷贝。
- **类型安全**：手写 TS 类型，与 worker.rs 字段名严格对齐。

## 安装

```bash
npm install @vane-rs/web
# optionalDep @vane-rs/dict-zh 自动安装；CDN fallback 或自带词典时：
npm install @vane-rs/web --no-optional
```

## 快速开始

```ts
import { createVane } from '@vane-rs/web';
import dictBinUrl from '@vane-rs/dict-zh/dict.bin';
import dictSha256Url from '@vane-rs/dict-zh/sha256_prefix.bin';

// 1. 加载词典字节（@vane-rs/dict-zh optionalDep，零 CDN）
const dictData = new Uint8Array(await (await fetch(dictBinUrl)).arrayBuffer());
const sha256Hex = Array.from(new Uint8Array(await (await fetch(dictSha256Url)).arrayBuffer()))
  .map(b => b.toString(16).padStart(2, '0')).join('');

// 2. 创建 Vane 实例（Worker 自动启动 + SIMD 探针 + 选 wasm 变体）
const vane = await createVane({
  vfs: 'opfs',
  dbPath: 'vane.db',
  dictData,         // transferable 零拷贝，transfer 后主线程不可再访问
  dictSha256: sha256Hex,
});

// 3. 打开数据库 + 创建 collection
await vane.open();
const col = await vane.collection('docs', {
  fields: [
    { name: 'text', type: 'text' },
    { name: 'vec', type: 'vector', dim: 64, metric: 'cosine' },
  ],
}, { tokenizer: 'jieba' });

// 4. 灌入文档 + 搜索
await vane.add(col, [
  { id: 'd1', text: '向量检索入门', vector: [/* 64 维 */] },
  { id: 'd2', text: 'BM25 文本检索', vector: [/* 64 维 */] },
]);
await vane.flush(col);

const hits = await vane.search(col, {
  text: '检索',
  vector: [/* 64 维 query */],
  topK: 10,
  mode: 'hybrid',
});
console.log(hits); // [{ id: 'd1', score: 0.92, fields: {...} }, ...]

// 5. 关闭
await vane.close();
```

## CDN 模式（不装 @vane-rs/dict-zh）

未提供 `dictData` 且未指定 `dictUrl` 时，`createVane` 自动填入 jsdelivr CDN 默认 URL：

```ts
const vane = await createVane({ vfs: 'opfs', dbPath: 'vane.db' });
// → worker 内 dict_loader 从 CDN fetch dict.bin → OPFS 缓存 → 降级 bigram（CDN 失败时）
```

CDN fetch 失败时自动降级 bigram 分词（不抛错，SPEC §12.4 铁律）。

## vite 集成（零配置）

vite 6+ 原生识别 `new Worker(new URL('./worker.js', import.meta.url), {type:'module'})` 与 `new URL('./x.wasm', import.meta.url)`，无需 wasm/worker plugin。

```ts
import { createVane } from '@vane-rs/web';
const vane = await createVane();
```

`vite.config.ts` 无需任何 wasm/worker 相关配置。

## webpack 5 集成

需开启 ESM 输出：

```js
// webpack.config.js
export default {
  experiments: { outputModule: true },
};
```

webpack 5 原生支持 `new URL(..., import.meta.url)` asset 与 ESM Worker。`init(wasmUrl)` 显式 fetch 加载 wasm，不依赖 `experiments.asyncWebAssembly`。

## API

### `createVane(opts?): Promise<Vane>`

创建 Vane 实例。内部启动 Worker + SIMD 探针 + 选 wasm 变体 + 初始化 VaneWorker。

#### `VaneWorkerOpts`

```ts
interface VaneWorkerOpts {
  vfs?: 'opfs' | 'idb' | 'memory';          // 默认 'opfs'（不可用降级 idb → memory）
  dbPath?: string;                           // 默认 'vane.db'
  dictData?: Uint8Array | ArrayBuffer;       // 词典字节（优先于 dictUrl，transferable 零拷贝）
  dictUrl?: string;                          // CDN fallback URL（未提供时自动填默认 CDN）
  dictSha256?: string;                       // 16 字符 hex（sha256 前 8 字节）
}
```

> **⚠️ dictData transferable**：传入后 buffer 被 transfer 到 Worker，主线程不可再访问（detached）。每次 `fetch` 新建 buffer 或用 `.slice()` 拷贝。若 `Uint8Array` 是大 buffer 的部分视图，先 `.slice()` 取出完整字节。

### `Vane` 接口

```ts
interface Vane {
  open(path?: string, opts?: OpenOptions): Promise<void>;
  collection(name: string, schema: Schema, opts?: CollectionOptions): Promise<number>;
  add(col: number, docs: Doc[]): Promise<number>;
  flush(col: number): Promise<void>;
  search(col: number, query: SearchQuery): Promise<Hit[]>;
  delete(col: number, ids: string[]): Promise<number>;
  compact(col: number): Promise<void>;
  reindex(col: number): Promise<number>;
  export(dest: string): Promise<void>;
  readFile(path: string): Promise<Uint8Array>;
  close(): Promise<void>;
}
```

### Schema

```ts
interface Schema {
  fields: FieldSchema[];
}

// 判别联合：type 决定可选字段
type FieldSchema =
  | { name: string; type: 'text' }
  | { name: string; type: 'vector'; dim: number; metric?: 'cosine' | 'l2' | 'dot' }
  | { name: string; type: 'scalar'; kind?: 'int' | 'float' | 'bool' | 'keyword' };
```

### Doc

```ts
interface Doc {
  id: string;
  text?: string;
  vector?: number[];
  meta?: Record<string, number | boolean | string>;
}
```

### SearchQuery

```ts
interface SearchQuery {
  text?: string;                    // BM25 查询文本
  vector?: number[];                // 向量查询
  topK?: number;                    // 默认 10
  mode?: 'hybrid' | 'vector' | 'text' | 'auto';  // 默认 'auto'
  fusion?: 'rrf' | { linear: { alpha?: number } };  // 默认 'rrf'
  candidateMultiplier?: number;     // 默认 3
}
```

> `text` 和 `vector` 至少提供一个。`filter` 在 wasm 端不支持（勿传）。

### Hit

```ts
interface Hit {
  id: string;
  score: number;
  fields: Record<string, string> | null;
}
```

### OpenOptions

```ts
interface OpenOptions {
  persistence?: 'persistent' | 'best-effort';  // 默认 'persistent'
  autoCommit?: 'off' | { intervalMs?: number; maxDocs?: number };  // 默认 On{1000, 1000}
  pageCacheMb?: number;                         // 默认 32
}
```

### CollectionOptions

```ts
interface CollectionOptions {
  tokenizer?: 'jieba' | 'cjk_bigram' | 'standard';  // 默认 'standard'
  userDict?: Array<string | { term: string; freq: number }>;
  autoCommit?: 'off' | { intervalMs?: number; maxDocs?: number };
}
```

> `jieba` 无词典时自动降级 `cjk_bigram`（不抛错）。

### SIMD 探针

```ts
import { simd128Supported, SIMD128_TEST_MODULE } from '@vane-rs/web/probe';

simd128Supported();  // boolean，WebAssembly.validate 测试模块
```

`SIMD128_TEST_MODULE` 与 `crates/vane-wasm/src/simd_probe.rs` 的常量逐字节一致（维护红线）。

### 导出快照 + 下载

```ts
await vane.export('backup.vane');
const bytes = await vane.readFile('backup.vane');
const blob = new Blob([bytes], { type: 'application/octet-stream' });
const url = URL.createObjectURL(blob);
const a = document.createElement('a');
a.href = url;
a.download = 'backup.vane';
a.click();
URL.revokeObjectURL(url);
```

## 构建

`dist/` 是构建产物，由 `scripts/build-web.sh` 产出：

```bash
cd bindings/web && npm install && npm run build
```

流程：cargo build 双变体（simd128 / scalar）→ wasm-bindgen `--target web` → wasm-opt `-Oz` → 拷贝到 `dist/` → `cp vane_wasm_scalar.wasm vane_wasm_bg.wasm` 别名 → **tsc 编译 `src/*.ts` → `dist/*.js` + `.d.ts`** → gzip 体积门禁 ≤800KB。

产物：

| 文件 | 来源 | 说明 |
|------|------|------|
| `dist/vane_wasm.js` | wasm-bindgen 生成 | ESM 胶水，含 `__wbg_init` |
| `dist/vane_wasm.d.ts` | wasm-bindgen 生成 | VaneWorker TS 类型 |
| `dist/vane_wasm_simd.wasm` | cargo build + wasm-opt | SIMD128 加速变体 |
| `dist/vane_wasm_scalar.wasm` | cargo build + wasm-opt | scalar 兜底变体 |
| `dist/vane_wasm_bg.wasm` | cp scalar 别名 | wasm-bindgen 默认 URL 兼容 |
| `dist/index.js` / `.d.ts` | tsc 编译 `src/index.ts` | 主线程 API（createVane 工厂 + 类型） |
| `dist/worker.js` / `.d.ts` | tsc 编译 `src/worker.ts` | Worker 入口（探针 + wasm 加载 + postMessage 路由） |
| `dist/probe.js` / `.d.ts` | tsc 编译 `src/probe.ts` | SIMD128 探针 |
| `dist/types.js` / `.d.ts` | tsc 编译 `src/types.ts` | TS 类型定义 |

## License

Apache-2.0（见 [LICENSE](./LICENSE)）。
