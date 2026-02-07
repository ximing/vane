# @vane-rs/web npm 包结构设计（M3 阶段一 Task 1）

> 来源：Task 1 设计 agent（Plan/opus，只读）产出 + 编排者审查。
> 审查结论：✅ 无 SPEC 矛盾，✅ 无冻结契约冲突（不改 crates/vane-wasm/，只新增 bindings/web/）。
> 3 处约束冲突已由编排者决策（见下）。

## 编排者审查结论 + 决策

**审查通过**。设计严格遵守全部约束：词典永不进 wasm（dict.bin 在独立 @vane-rs/dict-zh 包）、core 禁 std::fs（@vane-rs/web 是 JS 包装不触碰 core）、依赖黑名单（无 Rust 依赖）、不改 VaneWorker JS 冻结契约（只新增 bindings/web/）、MoSCoW Won't-have 不触碰。

**3 处约束冲突编排者决策**：
1. **包目录位置** → `bindings/web/`（与 bindings/go/ 平级，语义一致：非 Rust 绑定包；@vane-rs/web 纯 JS 包不应放 crates/ Rust 目录）。
2. **dict-zh 版本钉法** → 钉死 `2026.8.0`（日历版非标准 semver，`^` 行为不确定；dict 升级时同步 bump @vane-rs/web patch）。
3. **dictData 校验时机** → 不在主线程预校验，worker 内 dict_loader.rs verify_sha256_prefix 校验已足够（主线程预校验需 zstd 解压，无收益的复杂度）。

**跨任务依赖**（记入 Task 5 brief）：@vane-rs/dict-zh package.json 需 exports `./dict.bin` + `./sha256_prefix.bin` 作 asset url（vite/webpack `import dictBinUrl from '@vane-rs/dict-zh/dict.bin'`）。

**deferred**（记入 Task 3 brief）：手写 TS 类型需与 worker.rs parse_worker_opts/parse_schema/parse_search_query 字段名严格对齐（camelCase：dbPath/dictUrl/dictSha256/dictData/vfs）。Task 3 实现时逐一核对。

---

## 1. package.json 草案

```json
{
  "name": "@vane-rs/web",
  "version": "0.2.0",
  "description": "Vane 混合检索库 Web 端 npm 包（wasm-bindgen --target web ESM 双变体 + worker + dictData 内联）",
  "license": "Apache-2.0",
  "type": "module",
  "main": "./dist/index.js",
  "module": "./dist/index.js",
  "types": "./dist/index.d.ts",
  "sideEffects": ["./dist/worker.js", "./dist/vane_wasm.js", "**/*.wasm"],
  "files": ["dist/", "README.md", "LICENSE"],
  "exports": {
    ".": { "types": "./dist/index.d.ts", "import": "./dist/index.js", "default": "./dist/index.js" },
    "./worker": { "types": "./dist/worker.d.ts", "import": "./dist/worker.js", "default": "./dist/worker.js" },
    "./probe": { "types": "./dist/probe.d.ts", "import": "./dist/probe.js", "default": "./dist/probe.js" },
    "./vane_wasm.js": "./dist/vane_wasm.js",
    "./package.json": "./package.json"
  },
  "optionalDependencies": { "@vane-rs/dict-zh": "2026.8.0" },
  "publishConfig": { "access": "public" },
  "engines": { "node": ">=16" }
}
```

**关键字段论证**：
- **version=0.2.0**：与 M3 阶段四 bump 0.1.2→0.2.0 三端同步对齐，首发即 0.2.0。@vane-rs/dict-zh 走日历版 2026.8.0（与 DICT_VERSION 解耦）。
- **type=module**：全 ESM。wasm-bindgen --target web 产出 ESM，worker 入口需 `{type:'module'}`。无 CJS 产物（Web 端无需 CJS；Node 端用 @vane-rs/node）。
- **exports map**（vite/webpack 友好条件导出）：`.` 主线程 API（createVane 工厂 + 类型）；`./worker` Worker 入口；`./probe` SIMD 探针（可选）；`./vane_wasm.js` wasm-bindgen glue（内部 + 高级用户）。条件 `types`/`import`/`default`，不产 `require`（无 CJS）。
- **sideEffects**：⚠️ 关键。wasm 模块有副作用（`__wbg_init` 修改模块级 wasm 变量，worker.js 注册 self.onmessage）。标 `sideEffects:false` 会被 tree-shake 掉导致运行时崩溃。必须显式列出 worker.js/vane_wasm.js/*.wasm。其余（index.js/probe.js）纯函数可 shake。
- **optionalDependencies @vane-rs/dict-zh=2026.8.0**（编排者决策：钉死）：`npm i @vane-rs/web` 自动装 dict-zh，零配置体验对齐 `npm i @vane-rs/node`。用户走 CDN fallback 或自带词典时 `--no-optional` 跳过。⚠️ 不违反"词典永不进 wasm"红线（dict.bin 在独立 npm 包，不进 .wasm 文件），但增加 install 体积（dict.bin 1.48MB，可接受——@vane-rs/node 也内嵌 dict.bin 于 .node）。
- **publishConfig.access=public**：@vane-rs 是私有 scope，新包必须 public（与 @vane-rs/node 一致）。
- **files=[dist/]**：只发布 dist/ + README + LICENSE，源码不入包。

---

## 2. 文件布局树（编排者决策：bindings/web/）

```
bindings/web/                          # @vane-rs/web 包源（build 脚本产出 dist/）
├── package.json                       # §1 草案
├── README.md                          # 安装 + vite/webpack 集成 + API
├── LICENSE
├── src/                               # 手写 JS/TS 源（build 前）
│   ├── index.ts                       # 主线程 API：createVane() 工厂 + 类型 re-export
│   ├── worker.ts                      # Worker 入口：探针 + 选 wasm + postMessage 路由
│   ├── probe.ts                       # SIMD128_TEST_MODULE + simd128Supported()
│   └── types.ts                       # VaneWorkerOpts / Schema / Doc / Hit / SearchQuery 类型
├── dist/                              # build 产物（npm 发布内容）
│   ├── index.js                       # 手写 ESM（tsc/esbuild 从 src/index.ts 产出）
│   ├── index.d.ts                     # TS 类型
│   ├── worker.js                      # 手写 ESM Worker 入口
│   ├── worker.d.ts
│   ├── probe.js                       # 手写 ESM 探针
│   ├── probe.d.ts
│   ├── vane_wasm.js                   # ⚠️ wasm-bindgen --target web 生成（非手写）
│   ├── vane_wasm.d.ts                 # ⚠️ wasm-bindgen --target web 生成（非手写）
│   ├── vane_wasm_simd.wasm            # build-web 产物（~803KB raw / 312KB gzip）
│   ├── vane_wasm_scalar.wasm          # build-web 产物（~814KB raw / 315KB gzip）
│   └── vane_wasm_bg.wasm              # cp vane_wasm_scalar.wasm 别名（§7.3 默认 URL 兼容）
└── scripts/
    └── build-web.sh                   # 构建脚本（Task 2 实现）
```

**文件来源**：vane_wasm.js/.d.ts/双变体 .wasm = 自动生成（wasm-bindgen + cargo build + wasm-opt）；index/worker/probe .js/.d.ts = 手写（tsc 从 src/）；package.json/build-web.sh = 手写。

**不入包**：dict.bin/sha256_prefix.bin（在 @vane-rs/dict-zh 独立包，红线 + 解耦）；src/ 源码（files=[dist/]）；node_modules/target（.gitignore + .npmignore）。

---

## 3. 双变体探针策略

**核心矛盾（鸡生蛋）**：Rust 侧 `simd_probe.rs` 的 `simd128_supported()` 是 wasm 模块内函数，必须先 `init(wasm)` 加载实例后才能调用。但"选 simd 还是 scalar .wasm"发生在 init() 之前。worker.rs 第 637 行 `let _simd = simd_probe::simd128_supported();` 确实在 init_worker 内调用（此时 wasm 已 init），且变量名 `_simd`（未使用）仅预留。**Rust 侧探针不参与 .wasm 选择**。

**解法：JS 侧探针**（worker 内，init 之前）。复用 simd_probe.rs 的 SIMD128_TEST_MODULE 字节常量（固定字节，含 v128.const opcode `FD 0C` + 16 字节立即数），JS 侧 `WebAssembly.validate(SIMD128_TEST_MODULE)` 决定加载 simd 还是 scalar。

**探针位置：worker 内**（非主线程）。worker 是 wasm 运行环境，探针结果直接用于 worker 内 init(wasmUrl)；主线程不加载 wasm。

**dist/probe.js 草案**：
```js
// SIMD128 测试模块（与 crates/vane-wasm/src/simd_probe.rs SIMD128_TEST_MODULE 逐字节一致）。
// 含 v128.const 指令（opcode FD 0C + 16 字节立即数），仅 simd128 运行时 validate 通过。
// ⚠️ 维护红线：本常量必须与 simd_probe.rs 同步，单测校验 magic + FD 0C opcode。
export const SIMD128_TEST_MODULE = new Uint8Array([
  0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00,  // magic + version
  0x01, 0x04, 0x01, 0x60, 0x00, 0x00,              // type section: () -> ()
  0x03, 0x02, 0x01, 0x00,                          // function section
  0x07, 0x05, 0x01, 0x01, 0x74, 0x00, 0x00,        // export "t"
  0x0a, 0x17, 0x01, 0x15, 0x00,                    // code section
  0xfd, 0x0c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,  // v128.const + 16-byte immediate
  0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
  0x1a, 0x0b,                                      // drop + end
]);

export function simd128Supported() {
  try { return WebAssembly.validate(SIMD128_TEST_MODULE); }
  catch { return false; }  // 不支持或 CompileError → 保守走 scalar
}
```

**worker.js 内选 .wasm**：
```js
import init, { VaneWorker } from "./vane_wasm.js";
import { simd128Supported } from "./probe.js";

async function loadWasm() {
  const simd = simd128Supported();
  const wasmUrl = simd
    ? new URL('./vane_wasm_simd.wasm', import.meta.url)
    : new URL('./vane_wasm_scalar.wasm', import.meta.url);
  await init(wasmUrl);  // 显式传 URL，覆盖默认
}
```

**探针单测**：probe.js 校验 SIMD128_TEST_MODULE 字节序列与 simd_probe.rs 一致（magic [0..4]==="\x00asm"、含 [0xFD,0x0C]、段结构齐备）。simd_probe.rs tests 模块（第 89-134 行）已有对齐断言。⚠️ 维护红线：若 simd_probe.rs 常量变更，probe.js 必须同步。

**与 Rust 侧探针关系**：JS 探针（init 前，选 .wasm）+ Rust 探针（init 后，日志/预留）。字节常量一致，用途不同。Rust 侧保留不动（冻结契约）。

---

## 4. dictData 接口

vane-wasm worker.rs `extract_dict_data`（第 616 行）已实现从 JsValue opts 提取 dictData（Uint8Array/ArrayBuffer）。dict_loader.rs 三渠道：①dictData 内联（优先）②CDN fetch+VFS缓存 ③bigram 降级。@vane-rs/web 只暴露入口，不新增 Rust 侧逻辑。

**主线程 API**：
```ts
export interface VaneWorkerOpts {
  vfs?: 'opfs' | 'idb' | 'memory';
  dbPath?: string;
  dictData?: Uint8Array | ArrayBuffer;  // @vane-rs/dict-zh 的 dict.bin 字节（优先于 dictUrl）
  dictUrl?: string;                     // CDN fallback URL
  dictSha256?: string;                  // 16 字符 hex（sha256 前 8 字节）
}
export function createVane(opts?: VaneWorkerOpts): Promise<Vane>;
```

**用法（零 CDN）**：
```ts
import { createVane } from '@vane-rs/web';
import dictBinUrl from '@vane-rs/dict-zh/dict.bin';        // vite/webpack asset url
import dictSha256Url from '@vane-rs/dict-zh/sha256_prefix.bin';

const dictData = new Uint8Array(await (await fetch(dictBinUrl)).arrayBuffer());
const sha256Hex = Array.from(new Uint8Array(await (await fetch(dictSha256Url)).arrayBuffer()))
  .map(b => b.toString(16).padStart(2, '0')).join('');

const vane = await createVane({ vfs: 'opfs', dbPath: 'vane.db', dictData, dictSha256: sha256Hex });
await vane.open();
const col = await vane.collection('docs', schema, { tokenizer: 'jieba' });
```

**CDN fallback 默认 URL**（@vane-rs/web 层默认值，不改 dict_loader.rs）：
`https://cdn.jsdelivr.net/npm/@vane-rs/dict-zh@2026.8.0/dict.bin`（npm 包，版本钉死，比 demo 的 gh @main 源语义更稳）。

**dictData transferable 零拷贝**（1.48MB 不拷贝两次）：
```js
const { dictData, ...rest } = opts;
if (dictData instanceof Uint8Array) {
  worker.postMessage({ op: 'create', opts: { ...rest, dictData: dictData.buffer } }, [dictData.buffer]);
} else if (dictData instanceof ArrayBuffer) {
  worker.postMessage({ op: 'create', opts: { ...rest, dictData } }, [dictData]);
}
```
⚠️ 坑：transfer 后主线程 buffer detached，再访问报 TypeError。需文档说明：传 dictData 后主线程不可再访问该 buffer；用户每次 fetch 新建 buffer 或用 slice() 拷贝。

**校验时机**（编排者决策：不在主线程预校验）：worker 内 dict_loader.rs verify_sha256_prefix 校验。若失败降级 bigram+warn，主线程无感知（除非 worker postMessage 通知）。信任 @vane-rs/dict-zh 包完整性 + worker 内校验。

---

## 5. worker 入口策略

**模式**：`new Worker(new URL('@vane-rs/web/worker', import.meta.url), {type:'module'})` —— ESM 标准 worker 模块模式，vite 6+ 和 webpack 5 都原生支持。

**@vane-rs/web 暴露方式**：包内自带 dist/worker.js，exports map `"./worker"` 暴露。createVane 工厂内部封装 `new Worker(...)`，用户无需手写。高级用户可直接 `import` worker 入口。

**跨打包器兼容性**：
| 打包器 | `new URL('@vane-rs/web/worker', import.meta.url)` | worker 内 `new URL('./x.wasm', import.meta.url)` |
|--------|---------------------------------------------------|--------------------------------------------------|
| vite 6+ | ✅ 原生（识别 new URL + Worker，打包 worker 为独立 chunk） | ✅ vite 识别 new URL 为 wasm asset |
| webpack 5 | ✅ 原生（需 output.module:true 或 ESM） | ✅ new URL 原生 asset，init(url) 显式 fetch 不依赖 experiments.asyncWebAssembly |
| Rollup | 需 @rollup/plugin-worker | 需 plugin 处理 wasm asset |

**worker 内 wasm 加载**：`init(wasmUrl)` 显式传 URL（非默认 `new URL('vane_wasm_bg.wasm', import.meta.url)`）。vite/webpack 识别 `new URL('./vane_wasm_simd.wasm', import.meta.url)` 产出 wasm asset + url，init 内部 fetch 加载。

**createVane 工厂草案**（dist/index.js）：
```js
export async function createVane(opts = {}) {
  const worker = new Worker(new URL('./worker.js', import.meta.url), { type: 'module' });
  const pending = new Map();
  let nextId = 1;
  worker.onmessage = (e) => {
    const { id, result, error } = e.data;
    const p = pending.get(id);
    if (!p) return;
    pending.delete(id);
    error ? p.reject(new Error(error)) : p.resolve(result);
  };
  function call(op, payload = {}) {
    const id = nextId++;
    return new Promise((resolve, reject) => {
      pending.set(id, { resolve, reject });
      worker.postMessage({ op, id, ...payload });
    });
  }
  // dictData transferable
  const { dictData, ...rest } = opts;
  const transfer = [];
  let dictDataTransferable = dictData;
  if (dictData instanceof Uint8Array) {
    dictDataTransferable = dictData.buffer;
    transfer.push(dictData.buffer);
  } else if (dictData instanceof ArrayBuffer) {
    transfer.push(dictData);
  }
  worker.postMessage({ op: 'create', opts: { ...rest, dictData: dictDataTransferable } }, transfer);
  await call('create');  // await worker init（Task 3 实现细化 create 消息 id 绑定）
  return new VaneProxy(worker, call);  // 显式方法映射（非 Proxy，见 §6）
}
```

---

## 6. TS 类型（.d.ts）

**类型来源**：dist/vane_wasm.d.ts = wasm-bindgen 生成（VaneWorker 类，opts 为 any）；dist/index.d.ts/worker.d.ts/probe.d.ts = 手写（强类型）。

@vane-rs/web 不直接 re-export wasm-bindgen 的 VaneWorker（opts 是 any，类型不友好）。手写 `Vane` 接口（强类型）+ `createVane()` 工厂（封装 worker postMessage）。用户只 import createVane + Vane 接口。

**src/types.ts 草案**：
```ts
export type VfsKind = 'opfs' | 'idb' | 'memory';
export type TokenizerKind = 'jieba' | 'cjk_bigram' | 'whitespace';
export type VectorMetric = 'cosine' | 'l2' | 'dot';

export interface VaneWorkerOpts {
  vfs?: VfsKind;
  dbPath?: string;
  dictData?: Uint8Array | ArrayBuffer;
  dictUrl?: string;
  dictSha256?: string;  // 16 字符 hex
}

export interface FieldSchema {
  name: string;
  type: 'text' | 'vector' | 'keyword' | 'integer' | 'float';
  dim?: number;
  metric?: VectorMetric;
}
export interface Schema { fields: FieldSchema[]; }
export interface CollectionOptions { tokenizer?: TokenizerKind; }

export interface Doc { id: string; text?: string; vector?: number[]; [key: string]: unknown; }
export interface SearchQuery {
  text?: string; vector?: number[]; topK?: number;
  mode?: 'vector' | 'bm25' | 'hybrid'; filters?: Record<string, unknown>;
}
export interface Hit { id: string; score: number; [key: string]: unknown; }

export interface Vane {
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

export function createVane(opts?: VaneWorkerOpts): Promise<Vane>;
export { simd128Supported, SIMD128_TEST_MODULE } from './probe';
```

⚠️ **deferred（Task 3 实现时对齐）**：手写类型需与 worker.rs parse_worker_opts/parse_schema/parse_search_query 字段名严格对齐（camelCase：dbPath/dictUrl/dictSha256/dictData/vfs）。Task 3 逐一核对。

---

## 7. wasm-bindgen --target 策略

**选择：`--target web` 单一产物**（不产 bundler 双口径）。

理由：
1. **bundler-agnostic**：`new URL(..., import.meta.url)` 是 ESM 标准，vite/webpack 5/Rollup 都原生支持。`--target bundler` 依赖 bundler wasm 模块处理（webpack experiments.asyncWebAssembly / vite plugin），非零配置。
2. **worker 内可用**：ESM import 在 worker 内正常。bundler target 的 `import wasm from './x.wasm'` 在 worker 内需 bundler 预处理，跨打包器兼容性差。
3. **显式 init(url)**：双变体要求 worker.js 显式传 simd/scalar URL，`--target web` 的 `init(module_or_path)` 签名支持。bundler target 的 init 不接受 URL 参数。
4. **demo/build.sh 已验证**：demo 用 --target web 产出可用。

**⚠️ 默认 URL 坑 + 解法**：wasm-bindgen 生成的 vane_wasm.js 末尾默认 `new URL('vane_wasm_bg.wasm', import.meta.url)`，但双变体重命名为 vane_wasm_simd/scalar.wasm，无 bg.wasm 文件。bundler 静态分析会报错。

**解法（编排者采纳：cp 别名）**：build-web.sh 产出后 `cp vane_wasm_scalar.wasm vane_wasm_bg.wasm`（保守默认 scalar，即使 worker.js 不传参也能跑；bundler 静态分析能解析）。+800KB 包体积，npm 包无红线，无 sed 风险。优于 sed 改生成代码（wasm-bindgen 升级会失效）。

**build-web.sh 产出流程**（Task 2 实现）：
1. cargo build 双变体（复用 build-wasm-variants.sh 逻辑，RUSTFLAGS=+simd128 / 无）
2. wasm-bindgen --target web（simd 变体产出 glue + _bg.wasm）
3. wasm-opt -Oz 双变体
4. 拷贝到 bindings/web/dist/：vane_wasm.js + vane_wasm.d.ts + vane_wasm_simd.wasm + vane_wasm_scalar.wasm
5. cp vane_wasm_scalar.wasm vane_wasm_bg.wasm（默认别名）
6. tsc 编译 src/*.ts → dist/*.js + dist/*.d.ts
7. 体积门禁（gzip ≤800KB，复用 check-wasm-size.sh）

---

## 8. 与 vane-wasm 冻结契约的关系

**不改的文件（冻结）**：
| 文件 | 冻结内容 | @vane-rs/web 是否触碰 |
|------|----------|----------------------|
| crates/vane-wasm/Cargo.toml | features/crate-type/wasm-bindgen 依赖 | ❌ 不改 |
| crates/vane-wasm/src/lib.rs | wasm-bindgen 自由函数导出 | ❌ 不改 |
| crates/vane-wasm/src/worker.rs | VaneWorker impl/WorkerOpts/parse_worker_opts/extract_dict_data/init_worker | ❌ 不改 |
| crates/vane-wasm/src/dict_loader.rs | load_dict 三渠道/verify_sha256_prefix/降级铁律 | ❌ 不改 |
| crates/vane-wasm/src/simd_probe.rs | SIMD128_TEST_MODULE 常量/simd128_supported() | ❌ 不改（JS 侧复制常量） |
| crates/vane-wasm/src/worker.js | wasm-bindgen 生成模板 | ❌ 不改（用自己的 dist/worker.js） |

**只新增的文件**（bindings/web/）：package.json / src/index.ts/worker.ts/probe.ts/types.ts / scripts/build-web.sh / dist/*（build 产物）。

@vane-rs/web 的 dist/worker.js **基于** crates/vane-wasm/src/worker.js（wasm-bindgen 生成模板）+ demo/worker.js（M2-14 增强版）的模式，但是一份**独立的手写文件**，不替换原文件。dist/vane_wasm.js 是 wasm-bindgen --target web 对 vane-wasm crate 的**生成产物拷贝**，与 .rs 的 wasm-bindgen 导出严格同源。

✅ 约束遵守：不改 vane-wasm 任何 .rs 文件，符合"JS 契约冻结"约束。

---

## 9. vite/webpack 友好性论证

**exports map**：vite 读 `import` 条件（ESM），webpack 5 支持 exports 字段（需 output.module:true）。TS 4.7+ 读 `types` 条件。

**worker 入口**：`new Worker(new URL('@vane-rs/web/worker', import.meta.url), {type:'module'})` —— vite 6+ 原生支持（自动打包 worker 为独立 chunk），webpack 5 原生支持（output.module:true）。createVane 工厂封装，用户无需手写。

**wasm asset**：worker.js 内 `new URL('./vane_wasm_simd.wasm', import.meta.url)` —— vite 识别为 wasm asset 产出 url，webpack 5 `new URL` 原生 asset。`init(wasmUrl)` 内部 fetch 加载，不依赖 webpack wasm 模块导入（绕过 experiments.asyncWebAssembly 需求）。

**import.meta.url 在打包后**：vite/webpack 5 都正确重写 import.meta.url 为 chunk url，worker 内 import.meta.url 是 worker chunk url。`new URL('./x.wasm', import.meta.url)` 解析为相对 worker chunk 的 wasm asset url。✅

**零配置目标**：vite 项目 `npm i @vane-rs/web @vane-rs/dict-zh` → import createVane + dictData → vite.config.ts 零配置（无需 wasm/worker plugin）。webpack 5 需 `experiments: { outputModule: true }`（仅 ESM 输出声明）。

**常见坑**：
1. wasm asset 必须用 `new URL('./x.wasm', import.meta.url)`（非相对字符串路径，否则 404）。
2. worker 路径必须 `new Worker(new URL('@vane-rs/web/worker', import.meta.url))`（createVane 封装避免此坑）。
3. worker.js 用静态 import（非 dynamic，避免多一次 round-trip）。
4. vane_wasm.js 默认 `new URL('vane_wasm_bg.wasm', import.meta.url)` 需 bg.wasm 存在（§7.3 cp 别名解法）。

---

## 10. 风险/坑

| 编号 | 风险 | 等级 | 缓解 |
|------|------|------|------|
| W1 | wasm-bindgen 生成 vane_wasm.js 默认 URL 与双变体重命名冲突 | 中 | §7.3：cp vane_wasm_scalar.wasm vane_wasm_bg.wasm 别名 |
| W2 | worker 跨打包器 worker chunk 内 import.meta.url 解析 | 中 | vite/webpack 5 都正确处理；examples/vite + examples/webpack 实测验证（阶段三） |
| W3 | dictData 1.48MB postMessage 拷贝开销 | 中 | §4：transferable 零拷贝；文档说明 transfer 后 buffer detached |
| W4 | TS 类型对齐 wasm-bindgen .d.ts | 中 | §6：手写类型对齐 worker.rs 字段名（camelCase），Task 3 逐一核对 |
| W5 | 双变体 .wasm 包体积（simd 803KB + scalar 814KB raw ≈ 1.6MB；gzip ≈ 627KB + bg.wasm 别名 +800KB） | 低 | npm 包无 800KB 红线（仅单变体 wasm 产物有） |
| W6 | SIMD128_TEST_MODULE 字节常量两端同步（Rust vs JS） | 低 | probe.js 单测校验 magic + FD 0C opcode + 段结构 |
| W7 | optionalDep @vane-rs/dict-zh 版本耦合 | 低 | 钉死 2026.8.0（编排者决策）；dict 升级时 bump @vane-rs/web |
| W8 | wasm-bindgen 升级改生成 JS 结构 | 低 | build 脚本校验生成产物含 `__wbg_init` + `new URL(..., import.meta.url)`，否则 fail |
| W9 | webpack 5 experiments.asyncWebAssembly 需求 | 低 | 用 init(url) 显式 fetch，不依赖 webpack wasm 模块导入 |
| W10 | worker.js sideEffects 标记影响 tree-shaking | 中 | sideEffects 显式列出 worker.js/vane_wasm.js/*.wasm |

**⚠️ SPEC 矛盾/约束冲突**：无 SPEC 矛盾。3 处约束冲突已由编排者决策（见文件开头）。

---

## 关键文件路径（Task 2/3 implementer 必读）

- `crates/vane-wasm/src/worker.rs`（VaneWorker 冻结契约，extract_dict_data/parse_worker_opts/init_worker 字段名对齐 TS 类型）
- `crates/vane-wasm/src/simd_probe.rs`（SIMD128_TEST_MODULE 字节常量，probe.js 逐字节复制 + 单测对齐）
- `demo/worker.js`（dist/worker.js 模板原型：探针 + 双变体选择 + postMessage 路由）
- `demo/build.sh`（build-web.sh 模板原型：wasm-bindgen --target web + wasm-opt + 双变体）
- `crates/vane-node/package.json`（@vane-rs/web package.json 参考：scope/publishConfig.access/files 约定）
