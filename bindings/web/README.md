# @vane-rs/web

Vane 混合检索库的 Web 端 npm 包：向量检索 + BM25 + RRF 融合，跑在浏览器 Worker 内，
通过 wasm-bindgen `--target web` 产出 ESM 双变体（SIMD128 / scalar），运行时探针自动选择。

- **双变体**：SIMD128 加速 + scalar 兜底，一份 JS 胶水共享。
- **Worker 模式**：主线程零阻塞，wasm 在 Dedicated Worker 内运行。
- **VFS**：OPFS / IndexedDB / memory 三后端，持久化到浏览器存储。
- **词典**：`@vane-rs/dict-zh` 作 optionalDep，`dictData` 内联 transferable 零拷贝。

> 状态：M3 阶段一 Task 2 产出 wasm 产物构建脚本 + 包骨架。JS/TS 源（`src/*.ts` → `dist/index.js` / `worker.js` / `probe.js`）由 Task 3 补全，下方 API 节为占位。

## 安装

```bash
npm install @vane-rs/web
# optionalDep @vane-rs/dict-zh 自动安装；CDN fallback 或自带词典时：
npm install @vane-rs/web --no-optional
```

## vite 集成（零配置）

```ts
import { createVane } from '@vane-rs/web';
import dictBinUrl from '@vane-rs/dict-zh/dict.bin';
import dictSha256Url from '@vane-rs/dict-zh/sha256_prefix.bin';

const dictData = new Uint8Array(await (await fetch(dictBinUrl)).arrayBuffer());
const sha256Hex = Array.from(new Uint8Array(await (await fetch(dictSha256Url)).arrayBuffer()))
  .map(b => b.toString(16).padStart(2, '0')).join('');

const vane = await createVane({ vfs: 'opfs', dbPath: 'vane.db', dictData, dictSha256: sha256Hex });
await vane.open();
const col = await vane.collection('docs', { fields: [{ name: 'text', type: 'text' }] }, { tokenizer: 'jieba' });
```

vite 6+ 原生识别 `new Worker(new URL('@vane-rs/web/worker', import.meta.url), {type:'module'})` 与 `new URL('./x.wasm', import.meta.url)`，无需 wasm/worker plugin。

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

> Task 3 补全。下方为设计草案（见 `docs/plans/m3/task-1-design.md` §4/§6），最终以 Task 3 产出 `dist/index.d.ts` 为准。

### `createVane(opts?): Promise<Vane>`

```ts
interface VaneWorkerOpts {
  vfs?: 'opfs' | 'idb' | 'memory';
  dbPath?: string;
  dictData?: Uint8Array | ArrayBuffer;  // 优先于 dictUrl；transferable 零拷贝
  dictUrl?: string;                     // CDN fallback
  dictSha256?: string;                  // 16 字符 hex
}
```

### `Vane` 接口

`open(path?, opts?)` / `collection(name, schema, opts?)` / `add(col, docs)` / `flush(col)` / `search(col, query)` / `delete(col, ids)` / `compact(col)` / `reindex(col)` / `export(dest)` / `readFile(path)` / `close()`

### SIMD 探针

```ts
import { simd128Supported, SIMD128_TEST_MODULE } from '@vane-rs/web/probe';
simd128Supported();  // boolean，WebAssembly.validate 测试模块
```

## 构建

`dist/` 是构建产物，由 `scripts/build-web.sh` 产出：

```bash
bash scripts/build-web.sh
```

流程：cargo build 双变体（simd128 / scalar）→ wasm-bindgen `--target web` → wasm-opt `-Oz` → 拷贝到 `dist/` → `cp vane_wasm_scalar.wasm vane_wasm_bg.wasm` 别名（默认 URL 兼容）→ gzip 体积门禁 ≤800KB。

产物：

| 文件 | 来源 | 说明 |
|------|------|------|
| `dist/vane_wasm.js` | wasm-bindgen 生成 | ESM 胶水，含 `__wbg_init` |
| `dist/vane_wasm.d.ts` | wasm-bindgen 生成 | TS 类型 |
| `dist/vane_wasm_simd.wasm` | cargo build + wasm-opt | SIMD128 加速变体 |
| `dist/vane_wasm_scalar.wasm` | cargo build + wasm-opt | scalar 兜底变体 |
| `dist/vane_wasm_bg.wasm` | cp scalar 别名 | wasm-bindgen 默认 URL 兼容 |
| `dist/index.js` / `worker.js` / `probe.js` | Task 3 手写 TS | 主线程 API + Worker 入口 + 探针 |

## License

MIT（见 [LICENSE](./LICENSE)）。
