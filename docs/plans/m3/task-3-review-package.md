# Task 3 Review Package（ef8cc04..HEAD）

## Commits
b480be8 feat(web): @vane-rs/web JS/TS 源码 + tsc 编译——M3 Task 3

## Diff stat
 bindings/web/README.md            | 197 +++++++++++++++++++++++++++---
 bindings/web/package-lock.json    |  36 ++++++
 bindings/web/package.json         |   5 +
 bindings/web/scripts/build-web.sh |  30 ++++-
 bindings/web/src/index.ts         | 250 ++++++++++++++++++++++++++++++++++++++
 bindings/web/src/probe.ts         |  61 ++++++++++
 bindings/web/src/types.ts         | 222 +++++++++++++++++++++++++++++++++
 bindings/web/src/vane_wasm.d.ts   |  10 ++
 bindings/web/src/worker.ts        | 113 +++++++++++++++++
 bindings/web/tsconfig.json        |  18 +++
 docs/plans/m3/task-3-report.md    | 206 +++++++++++++++++++++++++++++++
 11 files changed, 1124 insertions(+), 24 deletions(-)

## 完整 diff（bindings/web/ 源码，排除 docs/plans 编排者产出 + package-lock.json 噪音）
diff --git a/bindings/web/README.md b/bindings/web/README.md
index a79950d..e7f519d 100644
--- a/bindings/web/README.md
+++ b/bindings/web/README.md
@@ -2,101 +2,262 @@
 
 Vane 混合检索库的 Web 端 npm 包：向量检索 + BM25 + RRF 融合，跑在浏览器 Worker 内，
 通过 wasm-bindgen `--target web` 产出 ESM 双变体（SIMD128 / scalar），运行时探针自动选择。
 
 - **双变体**：SIMD128 加速 + scalar 兜底，一份 JS 胶水共享。
 - **Worker 模式**：主线程零阻塞，wasm 在 Dedicated Worker 内运行。
 - **VFS**：OPFS / IndexedDB / memory 三后端，持久化到浏览器存储。
 - **词典**：`@vane-rs/dict-zh` 作 optionalDep，`dictData` 内联 transferable 零拷贝。
-
-> 状态：M3 阶段一 Task 2 产出 wasm 产物构建脚本 + 包骨架。JS/TS 源（`src/*.ts` → `dist/index.js` / `worker.js` / `probe.js`）由 Task 3 补全，下方 API 节为占位。
+- **类型安全**：手写 TS 类型，与 worker.rs 字段名严格对齐。
 
 ## 安装
 
 ```bash
 npm install @vane-rs/web
 # optionalDep @vane-rs/dict-zh 自动安装；CDN fallback 或自带词典时：
 npm install @vane-rs/web --no-optional
 ```
 
-## vite 集成（零配置）
+## 快速开始
 
 ```ts
 import { createVane } from '@vane-rs/web';
 import dictBinUrl from '@vane-rs/dict-zh/dict.bin';
 import dictSha256Url from '@vane-rs/dict-zh/sha256_prefix.bin';
 
+// 1. 加载词典字节（@vane-rs/dict-zh optionalDep，零 CDN）
 const dictData = new Uint8Array(await (await fetch(dictBinUrl)).arrayBuffer());
 const sha256Hex = Array.from(new Uint8Array(await (await fetch(dictSha256Url)).arrayBuffer()))
   .map(b => b.toString(16).padStart(2, '0')).join('');
 
-const vane = await createVane({ vfs: 'opfs', dbPath: 'vane.db', dictData, dictSha256: sha256Hex });
+// 2. 创建 Vane 实例（Worker 自动启动 + SIMD 探针 + 选 wasm 变体）
+const vane = await createVane({
+  vfs: 'opfs',
+  dbPath: 'vane.db',
+  dictData,         // transferable 零拷贝，transfer 后主线程不可再访问
+  dictSha256: sha256Hex,
+});
+
+// 3. 打开数据库 + 创建 collection
 await vane.open();
-const col = await vane.collection('docs', { fields: [{ name: 'text', type: 'text' }] }, { tokenizer: 'jieba' });
+const col = await vane.collection('docs', {
+  fields: [
+    { name: 'text', type: 'text' },
+    { name: 'vec', type: 'vector', dim: 64, metric: 'cosine' },
+  ],
+}, { tokenizer: 'jieba' });
+
+// 4. 灌入文档 + 搜索
+await vane.add(col, [
+  { id: 'd1', text: '向量检索入门', vector: [/* 64 维 */] },
+  { id: 'd2', text: 'BM25 文本检索', vector: [/* 64 维 */] },
+]);
+await vane.flush(col);
+
+const hits = await vane.search(col, {
+  text: '检索',
+  vector: [/* 64 维 query */],
+  topK: 10,
+  mode: 'hybrid',
+});
+console.log(hits); // [{ id: 'd1', score: 0.92, fields: {...} }, ...]
+
+// 5. 关闭
+await vane.close();
 ```
 
-vite 6+ 原生识别 `new Worker(new URL('@vane-rs/web/worker', import.meta.url), {type:'module'})` 与 `new URL('./x.wasm', import.meta.url)`，无需 wasm/worker plugin。
+## CDN 模式（不装 @vane-rs/dict-zh）
+
+未提供 `dictData` 且未指定 `dictUrl` 时，`createVane` 自动填入 jsdelivr CDN 默认 URL：
+
+```ts
+const vane = await createVane({ vfs: 'opfs', dbPath: 'vane.db' });
+// → worker 内 dict_loader 从 CDN fetch dict.bin → OPFS 缓存 → 降级 bigram（CDN 失败时）
+```
+
+CDN fetch 失败时自动降级 bigram 分词（不抛错，SPEC §12.4 铁律）。
+
+## vite 集成（零配置）
+
+vite 6+ 原生识别 `new Worker(new URL('./worker.js', import.meta.url), {type:'module'})` 与 `new URL('./x.wasm', import.meta.url)`，无需 wasm/worker plugin。
+
+```ts
+import { createVane } from '@vane-rs/web';
+const vane = await createVane();
+```
+
+`vite.config.ts` 无需任何 wasm/worker 相关配置。
 
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
 
-> Task 3 补全。下方为设计草案（见 `docs/plans/m3/task-1-design.md` §4/§6），最终以 Task 3 产出 `dist/index.d.ts` 为准。
-
 ### `createVane(opts?): Promise<Vane>`
 
+创建 Vane 实例。内部启动 Worker + SIMD 探针 + 选 wasm 变体 + 初始化 VaneWorker。
+
+#### `VaneWorkerOpts`
+
 ```ts
 interface VaneWorkerOpts {
-  vfs?: 'opfs' | 'idb' | 'memory';
-  dbPath?: string;
-  dictData?: Uint8Array | ArrayBuffer;  // 优先于 dictUrl；transferable 零拷贝
-  dictUrl?: string;                     // CDN fallback
-  dictSha256?: string;                  // 16 字符 hex
+  vfs?: 'opfs' | 'idb' | 'memory';          // 默认 'opfs'（不可用降级 idb → memory）
+  dbPath?: string;                           // 默认 'vane.db'
+  dictData?: Uint8Array | ArrayBuffer;       // 词典字节（优先于 dictUrl，transferable 零拷贝）
+  dictUrl?: string;                          // CDN fallback URL（未提供时自动填默认 CDN）
+  dictSha256?: string;                       // 16 字符 hex（sha256 前 8 字节）
 }
 ```
 
+> **⚠️ dictData transferable**：传入后 buffer 被 transfer 到 Worker，主线程不可再访问（detached）。每次 `fetch` 新建 buffer 或用 `.slice()` 拷贝。若 `Uint8Array` 是大 buffer 的部分视图，先 `.slice()` 取出完整字节。
+
 ### `Vane` 接口
 
-`open(path?, opts?)` / `collection(name, schema, opts?)` / `add(col, docs)` / `flush(col)` / `search(col, query)` / `delete(col, ids)` / `compact(col)` / `reindex(col)` / `export(dest)` / `readFile(path)` / `close()`
+```ts
+interface Vane {
+  open(path?: string, opts?: OpenOptions): Promise<void>;
+  collection(name: string, schema: Schema, opts?: CollectionOptions): Promise<number>;
+  add(col: number, docs: Doc[]): Promise<number>;
+  flush(col: number): Promise<void>;
+  search(col: number, query: SearchQuery): Promise<Hit[]>;
+  delete(col: number, ids: string[]): Promise<number>;
+  compact(col: number): Promise<void>;
+  reindex(col: number): Promise<number>;
+  export(dest: string): Promise<void>;
+  readFile(path: string): Promise<Uint8Array>;
+  close(): Promise<void>;
+}
+```
+
+### Schema
+
+```ts
+interface Schema {
+  fields: FieldSchema[];
+}
+
+// 判别联合：type 决定可选字段
+type FieldSchema =
+  | { name: string; type: 'text' }
+  | { name: string; type: 'vector'; dim: number; metric?: 'cosine' | 'l2' | 'dot' }
+  | { name: string; type: 'scalar'; kind?: 'int' | 'float' | 'bool' | 'keyword' };
+```
+
+### Doc
+
+```ts
+interface Doc {
+  id: string;
+  text?: string;
+  vector?: number[];
+  meta?: Record<string, number | boolean | string>;
+}
+```
+
+### SearchQuery
+
+```ts
+interface SearchQuery {
+  text?: string;                    // BM25 查询文本
+  vector?: number[];                // 向量查询
+  topK?: number;                    // 默认 10
+  mode?: 'hybrid' | 'vector' | 'text' | 'auto';  // 默认 'auto'
+  fusion?: 'rrf' | { linear: { alpha?: number } };  // 默认 'rrf'
+  candidateMultiplier?: number;     // 默认 3
+}
+```
+
+> `text` 和 `vector` 至少提供一个。`filter` 在 wasm 端不支持（勿传）。
+
+### Hit
+
+```ts
+interface Hit {
+  id: string;
+  score: number;
+  fields: Record<string, string> | null;
+}
+```
+
+### OpenOptions
+
+```ts
+interface OpenOptions {
+  persistence?: 'persistent' | 'best-effort';  // 默认 'persistent'
+  autoCommit?: 'off' | { intervalMs?: number; maxDocs?: number };  // 默认 On{1000, 1000}
+  pageCacheMb?: number;                         // 默认 32
+}
+```
+
+### CollectionOptions
+
+```ts
+interface CollectionOptions {
+  tokenizer?: 'jieba' | 'cjk_bigram' | 'standard';  // 默认 'standard'
+  userDict?: Array<string | { term: string; freq: number }>;
+  autoCommit?: 'off' | { intervalMs?: number; maxDocs?: number };
+}
+```
+
+> `jieba` 无词典时自动降级 `cjk_bigram`（不抛错）。
 
 ### SIMD 探针
 
 ```ts
 import { simd128Supported, SIMD128_TEST_MODULE } from '@vane-rs/web/probe';
+
 simd128Supported();  // boolean，WebAssembly.validate 测试模块
 ```
 
+`SIMD128_TEST_MODULE` 与 `crates/vane-wasm/src/simd_probe.rs` 的常量逐字节一致（维护红线）。
+
+### 导出快照 + 下载
+
+```ts
+await vane.export('backup.vane');
+const bytes = await vane.readFile('backup.vane');
+const blob = new Blob([bytes], { type: 'application/octet-stream' });
+const url = URL.createObjectURL(blob);
+const a = document.createElement('a');
+a.href = url;
+a.download = 'backup.vane';
+a.click();
+URL.revokeObjectURL(url);
+```
+
 ## 构建
 
 `dist/` 是构建产物，由 `scripts/build-web.sh` 产出：
 
 ```bash
-bash scripts/build-web.sh
+cd bindings/web && npm install && npm run build
 ```
 
-流程：cargo build 双变体（simd128 / scalar）→ wasm-bindgen `--target web` → wasm-opt `-Oz` → 拷贝到 `dist/` → `cp vane_wasm_scalar.wasm vane_wasm_bg.wasm` 别名（默认 URL 兼容）→ gzip 体积门禁 ≤800KB。
+流程：cargo build 双变体（simd128 / scalar）→ wasm-bindgen `--target web` → wasm-opt `-Oz` → 拷贝到 `dist/` → `cp vane_wasm_scalar.wasm vane_wasm_bg.wasm` 别名 → **tsc 编译 `src/*.ts` → `dist/*.js` + `.d.ts`** → gzip 体积门禁 ≤800KB。
 
 产物：
 
 | 文件 | 来源 | 说明 |
 |------|------|------|
 | `dist/vane_wasm.js` | wasm-bindgen 生成 | ESM 胶水，含 `__wbg_init` |
-| `dist/vane_wasm.d.ts` | wasm-bindgen 生成 | TS 类型 |
+| `dist/vane_wasm.d.ts` | wasm-bindgen 生成 | VaneWorker TS 类型 |
 | `dist/vane_wasm_simd.wasm` | cargo build + wasm-opt | SIMD128 加速变体 |
 | `dist/vane_wasm_scalar.wasm` | cargo build + wasm-opt | scalar 兜底变体 |
 | `dist/vane_wasm_bg.wasm` | cp scalar 别名 | wasm-bindgen 默认 URL 兼容 |
-| `dist/index.js` / `worker.js` / `probe.js` | Task 3 手写 TS | 主线程 API + Worker 入口 + 探针 |
+| `dist/index.js` / `.d.ts` | tsc 编译 `src/index.ts` | 主线程 API（createVane 工厂 + 类型） |
+| `dist/worker.js` / `.d.ts` | tsc 编译 `src/worker.ts` | Worker 入口（探针 + wasm 加载 + postMessage 路由） |
+| `dist/probe.js` / `.d.ts` | tsc 编译 `src/probe.ts` | SIMD128 探针 |
+| `dist/types.js` / `.d.ts` | tsc 编译 `src/types.ts` | TS 类型定义 |
 
 ## License
 
 Apache-2.0（见 [LICENSE](./LICENSE)）。
diff --git a/bindings/web/package.json b/bindings/web/package.json
index e018c59..602bace 100644
--- a/bindings/web/package.json
+++ b/bindings/web/package.json
@@ -12,11 +12,16 @@
   "exports": {
     ".": { "types": "./dist/index.d.ts", "import": "./dist/index.js", "default": "./dist/index.js" },
     "./worker": { "types": "./dist/worker.d.ts", "import": "./dist/worker.js", "default": "./dist/worker.js" },
     "./probe": { "types": "./dist/probe.d.ts", "import": "./dist/probe.js", "default": "./dist/probe.js" },
     "./vane_wasm.js": "./dist/vane_wasm.js",
     "./package.json": "./package.json"
   },
   "optionalDependencies": { "@vane-rs/dict-zh": "2026.8.0" },
+  "devDependencies": { "typescript": "^5.7.0" },
+  "scripts": {
+    "build": "bash scripts/build-web.sh",
+    "tsc": "tsc"
+  },
   "publishConfig": { "access": "public" },
   "engines": { "node": ">=16" }
 }
diff --git a/bindings/web/scripts/build-web.sh b/bindings/web/scripts/build-web.sh
index 31a4fb3..7bc2051 100755
--- a/bindings/web/scripts/build-web.sh
+++ b/bindings/web/scripts/build-web.sh
@@ -2,20 +2,19 @@
 # @vane-rs/web 构建脚本（M3 阶段一 Task 2）：wasm-bindgen --target web ESM 双变体产物。
 #
 # 流程（对应 docs/plans/m3/task-1-design.md §7.4）：
 #   1. 每变体：cargo build（simd128 / scalar，worker feature）
 #   2. 每变体：wasm-bindgen --target web 后处理（产出 _bg.wasm + glue .js + .d.ts）
 #   3. 每变体：wasm-opt -Oz 优化 _bg.wasm → vane_wasm_{simd,scalar}.wasm
 #   4. 拷贝 JS 胶水 + .d.ts 到 dist/（双变体共享一份，导出一致）
 #   5. cp vane_wasm_scalar.wasm vane_wasm_bg.wasm 别名（§7.3 默认 URL 兼容）
-#   6. W8 校验：vane_wasm.js 含 __wbg_init + new URL(..., import.meta.url)
-#   7. 体积门禁（gzip ≤ 800KB，SPEC §13.2-3）
-#
-# 不含 tsc 编译 src/*.ts（Task 3 扩展；src/ 尚不存在）。
+#   6. tsc 编译 src/*.ts → dist/index.js / worker.js / probe.js + .d.ts（§7.4 Task 3）
+#   7. W8 校验：vane_wasm.js 含 __wbg_init + new URL(..., import.meta.url)
+#   8. 体积门禁（gzip ≤ 800KB，SPEC §13.2-3）
 #
 # 技术说明（与 task brief 第 2 步的差异）：
 #   task brief 称"scalar 不需要再跑 wasm-bindgen"。但 raw .wasm 的 __wbindgen_*
 #   导入需经 wasm-bindgen 重写为 __wbg_* 才匹配 vane_wasm.js glue 的 import object
 #   （键名 __wbg_*），否则 WebAssembly.instantiate 报 TypeError。故双变体都必须
 #   跑 wasm-bindgen 后处理。glue 只拷一份（simd 与 scalar 的 glue 相同，导出一致）。
 #   与 demo/build.sh 同模式（已验证可用）。
 #
@@ -117,32 +116,51 @@ echo "→ $DIST/vane_wasm.d.ts"
 #    wasm-bindgen 生成的 vane_wasm.js 末尾默认 new URL('vane_wasm_bg.wasm', import.meta.url)。
 #    双变体重命名为 _simd/_scalar 后无 _bg.wasm，bundler 静态分析会报错。
 #    cp scalar 别名保守默认 scalar；worker.js 显式传 URL 覆盖默认。
 # ============================================================
 cp "$DIST/vane_wasm_scalar.wasm" "$DIST/vane_wasm_bg.wasm"
 echo "→ $DIST/vane_wasm_bg.wasm (scalar 别名，默认 URL 兼容)"
 
 # ============================================================
-# 6. W8 校验：vane_wasm.js 含 __wbg_init + new URL(..., import.meta.url)
+# 6. tsc 编译 src/*.ts → dist/index.js / worker.js / probe.js + .d.ts（§7.4 Task 3）
+#    前置依赖：dist/vane_wasm.d.ts（步骤 4 产出），src/vane_wasm.d.ts 桥接类型。
+#    tsc 不发射输入 .d.ts，dist/vane_wasm.d.ts 保留 wasm-bindgen 版本。
+# ============================================================
+echo ""
+echo "=== tsc compile src/*.ts → dist/ ==="
+TSC_BIN="bindings/web/node_modules/.bin/tsc"
+if [ ! -f "$TSC_BIN" ]; then
+  echo "FAIL: tsc not found at $TSC_BIN" >&2
+  echo "      Run 'cd bindings/web && npm install' first." >&2
+  exit 1
+fi
+"$TSC_BIN" -p bindings/web/tsconfig.json
+echo "→ $DIST/index.js + index.d.ts"
+echo "→ $DIST/worker.js + worker.d.ts"
+echo "→ $DIST/probe.js + probe.d.ts"
+echo "→ $DIST/types.js + types.d.ts"
+
+# ============================================================
+# 7. W8 校验：vane_wasm.js 含 __wbg_init + new URL(..., import.meta.url)
 # ============================================================
 echo ""
 echo "=== W8 wasm-bindgen 生成校验 ==="
 if ! grep -q '__wbg_init' "$DIST/vane_wasm.js"; then
   echo "FAIL: vane_wasm.js 缺 __wbg_init（wasm-bindgen 生成结构异常，W8）" >&2
   exit 1
 fi
 if ! grep -qE 'new URL\([^)]*import\.meta\.url\)' "$DIST/vane_wasm.js"; then
   echo "FAIL: vane_wasm.js 缺 new URL(..., import.meta.url)（默认 URL 解析异常，W8）" >&2
   exit 1
 fi
 echo "OK: __wbg_init + new URL(..., import.meta.url) 均存在"
 
 # ============================================================
-# 7. 体积门禁（gzip ≤ 800KB，SPEC §13.2-3）
+# 8. 体积门禁（gzip ≤ 800KB，SPEC §13.2-3）
 # ============================================================
 echo ""
 echo "=== Size gate (gzip ≤ 800KB) ==="
 FAIL=0
 for v in simd scalar; do
   f="$DIST/vane_wasm_${v}.wasm"
   size=$(gzip -c "$f" | wc -c | tr -d ' ')
   raw=$(stat -f%z "$f" 2>/dev/null || stat -c%s "$f")
diff --git a/bindings/web/src/index.ts b/bindings/web/src/index.ts
new file mode 100644
index 0000000..1380ec3
--- /dev/null
+++ b/bindings/web/src/index.ts
@@ -0,0 +1,250 @@
+// Vane 主线程 API（src/index.ts → dist/index.js）。
+//
+// §5 createVane 工厂：封装 new Worker + postMessage Promise 边界 + dictData transferable。
+// §6 TS 类型：手写强类型 Vane 接口，不直接 re-export wasm-bindgen 的 VaneWorker（opts 是 any）。
+//
+// 用法：
+//   import { createVane } from '@vane-rs/web';
+//   const vane = await createVane({ vfs: 'opfs', dbPath: 'vane.db' });
+//   await vane.open();
+//   const col = await vane.collection('docs', schema, { tokenizer: 'jieba' });
+
+import type {
+  VaneWorkerOpts,
+  Schema,
+  Doc,
+  Hit,
+  SearchQuery,
+  OpenOptions,
+  CollectionOptions,
+  Vane,
+} from './types.js';
+
+// §4 CDN fallback 默认 URL（@vane-rs/web 层默认值，不改 dict_loader.rs）。
+// 用户未提供 dictData 且未指定 dictUrl 时自动填入；dict_loader fetch 失败降级 bigram。
+const DEFAULT_DICT_URL = 'https://cdn.jsdelivr.net/npm/@vane-rs/dict-zh@2026.8.0/dict.bin';
+
+/**
+ * Vane 实例实现（内部类，不导出）。
+ *
+ * 封装 Worker 通信：
+ * - `pending` Map：id → {resolve, reject}，postMessage Promise 边界。
+ * - `call(op, payload, transfer)`：发 {op, id, ...payload} → 等 {id, result|error}。
+ * - `close()` 后 reject 所有后续调用（I-7 句柄注销）。
+ */
+class VaneImpl implements Vane {
+  private closed = false;
+  private readonly pending = new Map<
+    number,
+    { resolve: (v: unknown) => void; reject: (e: Error) => void }
+  >();
+  private nextId = 1;
+
+  private constructor(private readonly worker: Worker) {
+    // 接收 Worker 响应：{id, result} 或 {id, error}。
+    worker.onmessage = (e: MessageEvent): void => {
+      const { id, result, error } = e.data;
+      if (id == null) return;
+      const p = this.pending.get(id);
+      if (!p) return;
+      this.pending.delete(id);
+      if (error) p.reject(new Error(String(error)));
+      else p.resolve(result);
+    };
+
+    // Worker 级别错误（加载失败、未捕获异常），reject 所有 pending。
+    worker.onerror = (e: ErrorEvent): void => {
+      for (const [, p] of this.pending) p.reject(new Error(e.message));
+      this.pending.clear();
+    };
+  }
+
+  /**
+   * 工厂：创建 Worker + 发 create 消息（含 dictData transferable）。
+   * 由 createVane() 调用，用户不直接使用。
+   */
+  static async create(opts: VaneWorkerOpts): Promise<Vane> {
+    // §5 worker 入口策略：new Worker(new URL('./worker.js', import.meta.url), {type:'module'})
+    // vite 6+ / webpack 5 原生识别此模式，打包 worker 为独立 chunk。
+    const worker = new Worker(new URL('./worker.js', import.meta.url), { type: 'module' });
+    const impl = new VaneImpl(worker);
+    await impl.sendCreate(opts);
+    return impl;
+  }
+
+  /**
+   * 发 create 消息（dictData transferable 零拷贝）。
+   *
+   * §4 dictData 接口：
+   * - Uint8Array → transfer .buffer（整个 backing buffer）。
+   * - ArrayBuffer → transfer 本身。
+   * - 未提供 dictData/dictUrl → 自动填 CDN fallback 默认 URL。
+   *
+   * ⚠️ W3 transferable detached 坑：transfer 后主线程不可再访问该 buffer。
+   * 用户每次 fetch 新建 buffer 或用 slice() 拷贝。
+   */
+  private async sendCreate(opts: VaneWorkerOpts): Promise<void> {
+    const { dictData, dictUrl, ...rest } = opts;
+    const transfer: Transferable[] = [];
+    const createOpts: Record<string, unknown> = { ...rest };
+
+    // CDN fallback 默认 URL
+    if (!dictData && !dictUrl) {
+      createOpts.dictUrl = DEFAULT_DICT_URL;
+    } else if (dictUrl) {
+      createOpts.dictUrl = dictUrl;
+    }
+
+    // dictData transferable 零拷贝
+    if (dictData instanceof Uint8Array) {
+      // ⚠️ transfer 整个 backing buffer；若 Uint8Array 是大 buffer 的部分视图，
+      // 需用户先 .slice() 拷贝。典型用法 new Uint8Array(await (await fetch()).arrayBuffer())
+      // 的 buffer 恰好是完整词典字节，无此问题。
+      const buf = dictData.buffer as ArrayBuffer;
+      createOpts.dictData = buf;
+      transfer.push(buf);
+    } else if (dictData instanceof ArrayBuffer) {
+      createOpts.dictData = dictData;
+      transfer.push(dictData);
+    }
+
+    await this.call('create', { opts: createOpts }, transfer);
+  }
+
+  /**
+   * postMessage Promise 边界：发 {op, id, ...payload} → 等 {id, result|error}。
+   * @param transfer transferable 对象列表（零拷贝移交，detached 后主线程不可访问）。
+   */
+  private call(
+    op: string,
+    payload: Record<string, unknown> = {},
+    transfer: Transferable[] = [],
+  ): Promise<unknown> {
+    if (this.closed) return Promise.reject(new Error('vane worker closed'));
+    const id = this.nextId++;
+    return new Promise((resolve, reject): void => {
+      this.pending.set(id, { resolve, reject });
+      this.worker.postMessage({ op, id, ...payload }, transfer);
+    });
+  }
+
+  // ── Vane 接口实现 ──────────────────────────────────────────────────────────
+
+  async open(path = 'vane.db', opts?: OpenOptions): Promise<void> {
+    await this.call('open', { path, opts: opts ?? {} });
+  }
+
+  async collection(
+    name: string,
+    schema: Schema,
+    opts?: CollectionOptions,
+  ): Promise<number> {
+    const result = await this.call('collection', { name, schema, opts: opts ?? {} });
+    return Number(result);
+  }
+
+  async add(col: number, docs: Doc[]): Promise<number> {
+    const result = await this.call('add', { col, docs });
+    return Number(result);
+  }
+
+  async flush(col: number): Promise<void> {
+    await this.call('flush', { col });
+  }
+
+  async search(col: number, query: SearchQuery): Promise<Hit[]> {
+    const result = await this.call('search', { col, query });
+    // worker.rs search 返回 Hit[] JSON 字符串，主线程反序列化。
+    return typeof result === 'string' ? (JSON.parse(result) as Hit[]) : (result as Hit[]);
+  }
+
+  async delete(col: number, ids: string[]): Promise<number> {
+    const result = await this.call('delete', { col, ids });
+    return Number(result);
+  }
+
+  async compact(col: number): Promise<void> {
+    await this.call('compact', { col });
+  }
+
+  async reindex(col: number): Promise<number> {
+    const result = await this.call('reindex', { col });
+    return Number(result);
+  }
+
+  async export(dest: string): Promise<void> {
+    await this.call('export', { dest });
+  }
+
+  async readFile(path: string): Promise<Uint8Array> {
+    const result = await this.call('readFile', { path });
+    return result as Uint8Array;
+  }
+
+  async close(): Promise<void> {
+    if (this.closed) return;
+    try {
+      await this.call('close');
+    } finally {
+      this.closed = true;
+      this.worker.terminate();
+    }
+  }
+}
+
+/**
+ * 创建 Vane 实例（主线程 API）。
+ *
+ * 内部：new Worker → postMessage create → 返回 Vane 代理。
+ * Worker 内自动：SIMD 探针 → 选 wasm 变体 → init → VaneWorker.create(opts)。
+ *
+ * @param opts VFS / 词典 / dbPath 选项。未提供 dictData/dictUrl 时自动填 CDN fallback URL。
+ * @returns Vane 实例，所有方法返回 Promise。
+ *
+ * @example
+ * ```ts
+ * import { createVane } from '@vane-rs/web';
+ *
+ * const vane = await createVane({ vfs: 'opfs', dbPath: 'vane.db' });
+ * await vane.open();
+ * const col = await vane.collection('docs', {
+ *   fields: [{ name: 'text', type: 'text' }],
+ * }, { tokenizer: 'jieba' });
+ * await vane.add(col, [{ id: 'd1', text: 'hello' }]);
+ * await vane.flush(col);
+ * const hits = await vane.search(col, { text: 'hello', topK: 10 });
+ * await vane.close();
+ * ```
+ */
+export async function createVane(opts?: VaneWorkerOpts): Promise<Vane> {
+  return VaneImpl.create(opts ?? {});
+}
+
+// ── 类型 re-export ──────────────────────────────────────────────────────────
+
+export type {
+  VaneWorkerOpts,
+  VfsKind,
+  VectorMetric,
+  TokenizerKind,
+  SearchMode,
+  FusionSpec,
+  AutoCommit,
+  Schema,
+  FieldSchema,
+  TextFieldSchema,
+  VectorFieldSchema,
+  ScalarFieldSchema,
+  Doc,
+  SearchQuery,
+  Hit,
+  OpenOptions,
+  PersistenceMode,
+  CollectionOptions,
+  UserDictEntry,
+  Vane,
+} from './types.js';
+
+// ── SIMD 探针 re-export（§3，高级用户可选）──────────────────────────────────
+
+export { simd128Supported, SIMD128_TEST_MODULE } from './probe.js';
diff --git a/bindings/web/src/probe.ts b/bindings/web/src/probe.ts
new file mode 100644
index 0000000..3df2a98
--- /dev/null
+++ b/bindings/web/src/probe.ts
@@ -0,0 +1,61 @@
+// SIMD128 探针（src/probe.ts）。
+//
+// §3 双变体探针策略：Worker init 之前用 WebAssembly.validate 探测运行时是否支持
+// SIMD128，据结果选择加载 vane_wasm_simd.wasm 或 vane_wasm_scalar.wasm。
+//
+// ⚠️ §3.5 维护红线：SIMD128_TEST_MODULE 必须与 crates/vane-wasm/src/simd_probe.rs
+// 的 SIMD128_TEST_MODULE 常量逐字节一致。单测校验 magic + FD 0C opcode + 段结构。
+// 若 simd_probe.rs 常量变更，本文件必须同步。
+
+/**
+ * 最小 SIMD128 测试模块（wat2wasm 生成，固定字节）。
+ *
+ * 等价 WAT：
+ * ```wat
+ * (module
+ *   (func (export "t")
+ *     v128.const i32x4 0 0 0 0
+ *     drop
+ *   )
+ * )
+ * ```
+ *
+ * 含 `v128.const` 指令（opcode `FD 0C` + 16 字节立即数），仅 simd128 运行时
+ * 可 validate 通过。模块无 import、无自定义 section、无内存——最小探测开销。
+ *
+ * 逐字节复制自 crates/vane-wasm/src/simd_probe.rs SIMD128_TEST_MODULE（50 bytes）。
+ */
+export const SIMD128_TEST_MODULE = new Uint8Array([
+  // [magic + version] 8 bytes
+  0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00,
+  // [type section (id=1)] 1 type: () -> ()
+  0x01, 0x04, 0x01, 0x60, 0x00, 0x00,
+  // [function section (id=3)] 1 function, type idx 0
+  0x03, 0x02, 0x01, 0x00,
+  // [export section (id=7)] "t" -> function 0
+  0x07, 0x05, 0x01, 0x01, 0x74, 0x00, 0x00,
+  // [code section (id=10)] 1 body, body_size=0x15, 0 locals
+  0x0a, 0x17, 0x01, 0x15, 0x00,
+  // v128.const (opcode FD 0C) + 16-byte immediate (all zeros)
+  0xfd, 0x0c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
+  0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
+  0x00, 0x00,
+  // drop (0x1A) + end (0x0B)
+  0x1a, 0x0b,
+]);
+
+/**
+ * 探测运行时是否支持 WebAssembly SIMD128。
+ *
+ * 实现：`WebAssembly.validate(SIMD128_TEST_MODULE)`。该模块含 `v128.const`
+ * 指令，仅 simd128 运行时 validate 通过返 true；不支持则返 false（或抛错→false）。
+ *
+ * Worker init 调用此函数决定加载 simd 还是 scalar 产物。
+ */
+export function simd128Supported(): boolean {
+  try {
+    return WebAssembly.validate(SIMD128_TEST_MODULE);
+  } catch {
+    return false; // 不支持或 CompileError → 保守走 scalar
+  }
+}
diff --git a/bindings/web/src/types.ts b/bindings/web/src/types.ts
new file mode 100644
index 0000000..5099f44
--- /dev/null
+++ b/bindings/web/src/types.ts
@@ -0,0 +1,222 @@
+// Vane Web 端类型定义（src/types.ts）。
+//
+// ⚠️ 维护红线：所有字段名必须与 crates/vane-wasm/src/worker.rs 的
+// parse_worker_opts / parse_schema / parse_search_query / parse_open_opts /
+// parse_collection_opts / extract_dict_data / hits_to_json 严格对齐（camelCase）。
+// 设计文档 docs/plans/m3/task-1-design.md §6 的草案仅供参考，以 worker.rs 实现为准。
+
+// ── VaneWorkerOpts（worker.rs parse_worker_opts + extract_dict_data）──────────
+
+/** VFS 后端类型。worker.rs parse_worker_opts：默认 "opfs"（OPFS 不可用降级 IDB/memory）。 */
+export type VfsKind = 'opfs' | 'idb' | 'memory';
+
+/** 向量距离度量。worker.rs parse_schema：默认 "cosine"。 */
+export type VectorMetric = 'cosine' | 'l2' | 'dot';
+
+/** 分词器。worker.rs parse_collection_opts：默认 "standard"（空格/标点分词）。 */
+export type TokenizerKind = 'jieba' | 'cjk_bigram' | 'standard';
+
+/** 搜索模式。worker.rs parse_search_query：默认 "auto"（自动选择 vector/text/hybrid）。 */
+export type SearchMode = 'hybrid' | 'vector' | 'text' | 'auto';
+
+/** 融合策略。worker.rs parse_search_query：默认 "rrf"。 */
+export type FusionSpec = 'rrf' | { linear: { alpha?: number } };
+
+/** autoCommit 配置。worker.rs parse_auto_commit：默认 On{intervalMs:1000, maxDocs:1000}。 */
+export type AutoCommit = 'off' | { intervalMs?: number; maxDocs?: number };
+
+/**
+ * createVane 工厂选项。
+ *
+ * - `vfs`：VFS 后端，默认 "opfs"（OPFS 不可用时自动降级 IDB → memory）。
+ * - `dbPath`：数据库逻辑路径，默认 "vane.db"（OPFS 模式下也是文件名）。
+ * - `dictData`：词典字节（zstd 压缩的 dict.bin），优先于 dictUrl。
+ *   传入后以 transferable 零拷贝移交 Worker；**transfer 后主线程不可再访问该 buffer**。
+ * - `dictUrl`：词典 CDN fallback URL。未提供 dictData 且未指定 dictUrl 时，
+ *   @vane-rs/web 层自动填入 jsdelivr CDN 默认 URL。
+ * - `dictSha256`：16 字符 hex（sha256 前 8 字节），用于 worker 内 verify_sha256_prefix 校验。
+ */
+export interface VaneWorkerOpts {
+  vfs?: VfsKind;
+  dbPath?: string;
+  dictData?: Uint8Array | ArrayBuffer;
+  dictUrl?: string;
+  dictSha256?: string;
+}
+
+// ── Schema（worker.rs parse_schema）──────────────────────────────────────────
+
+/** 文本字段。worker.rs parse_schema type="text" → FieldDef::Text。 */
+export interface TextFieldSchema {
+  name: string;
+  type: 'text';
+}
+
+/** 向量字段。worker.rs parse_schema type="vector" → FieldDef::Vector{dim, metric}。 */
+export interface VectorFieldSchema {
+  name: string;
+  type: 'vector';
+  /** 向量维度（必填）。 */
+  dim: number;
+  /** 距离度量，默认 "cosine"。 */
+  metric?: VectorMetric;
+}
+
+/** 标量字段。worker.rs parse_schema type="scalar" → FieldDef::Scalar{kind}。 */
+export interface ScalarFieldSchema {
+  name: string;
+  type: 'scalar';
+  /** 标量类型，默认 "keyword"。 */
+  kind?: 'int' | 'float' | 'bool' | 'keyword';
+}
+
+/** 字段定义（判别联合：type 决定可选字段）。 */
+export type FieldSchema = TextFieldSchema | VectorFieldSchema | ScalarFieldSchema;
+
+/** Schema：字段列表。worker.rs parse_schema fields 数组。 */
+export interface Schema {
+  fields: FieldSchema[];
+}
+
+// ── Doc（worker.rs parse_docs）───────────────────────────────────────────────
+
+/**
+ * 文档。worker.rs parse_docs：id 必填，text/vector/meta 可选。
+ * meta 值支持 number / boolean / string（映射到 ScalarValue）。
+ */
+export interface Doc {
+  id: string;
+  text?: string;
+  vector?: number[];
+  meta?: Record<string, number | boolean | string>;
+}
+
+// ── SearchQuery（worker.rs parse_search_query）───────────────────────────────
+
+/**
+ * 搜索查询。worker.rs parse_search_query：text 和 vector 至少提供一个。
+ *
+ * - `topK`：默认 10。
+ * - `mode`：默认 "auto"。
+ * - `fusion`：默认 "rrf"。Linear 融合 { linear: { alpha } }，alpha 默认 0.5。
+ * - `candidateMultiplier`：默认 3。
+ * - `filter`：⚠️ wasm 端不支持（worker.rs 返 VaneError），勿传。
+ */
+export interface SearchQuery {
+  text?: string;
+  vector?: number[];
+  topK?: number;
+  mode?: SearchMode;
+  fusion?: FusionSpec;
+  candidateMultiplier?: number;
+}
+
+// ── Hit（worker.rs hits_to_json）─────────────────────────────────────────────
+
+/**
+ * 搜索结果。worker.rs hits_to_json：id + score + fields（存储字段 map，可能为 null）。
+ */
+export interface Hit {
+  id: string;
+  score: number;
+  fields: Record<string, string> | null;
+}
+
+// ── OpenOptions（worker.rs parse_open_opts）──────────────────────────────────
+
+/** 持久化模式。worker.rs parse_open_opts：默认 "persistent"。 */
+export type PersistenceMode = 'persistent' | 'best-effort';
+
+/**
+ * open() 选项。worker.rs parse_open_opts。
+ * - `persistence`：默认 "persistent"。
+ * - `autoCommit`：默认 On{intervalMs:1000, maxDocs:1000}。
+ * - `pageCacheMb`：页缓存大小（MB），默认 32。
+ */
+export interface OpenOptions {
+  persistence?: PersistenceMode;
+  autoCommit?: AutoCommit;
+  pageCacheMb?: number;
+}
+
+// ── CollectionOptions（worker.rs parse_collection_opts）──────────────────────
+
+/** 用户词典条目。worker.rs parse_collection_opts userDict：字符串或 {term, freq}。 */
+export type UserDictEntry = string | { term: string; freq: number };
+
+/**
+ * collection() 选项。worker.rs parse_collection_opts。
+ * - `tokenizer`：默认 "standard"。jieba 无词典时自动降级 cjk_bigram（不抛错）。
+ * - `userDict`：用户自定义词典条目列表。
+ * - `autoCommit`：同 OpenOptions.autoCommit。
+ */
+export interface CollectionOptions {
+  tokenizer?: TokenizerKind;
+  userDict?: UserDictEntry[];
+  autoCommit?: AutoCommit;
+}
+
+// ── Vane 接口（主线程 API，封装 Worker postMessage）─────────────────────────
+
+/**
+ * Vane 实例接口。createVane() 返回此接口的实现。
+ *
+ * 所有方法返回 Promise，内部通过 postMessage 路由到 Worker 内的 VaneWorker。
+ * close() 后再调用任何方法 reject（I-7 句柄注销）。
+ */
+export interface Vane {
+  /**
+   * 打开数据库。
+   * @param path 逻辑路径（OPFS 模式下也是文件名），默认 "vane.db"。应与 createVane 的 dbPath 一致。
+   * @param opts 打开选项。
+   */
+  open(path?: string, opts?: OpenOptions): Promise<void>;
+
+  /**
+   * 创建或获取 collection。
+   * @param name collection 名称（同名的 schema 必须一致）。
+   * @param schema 字段定义。
+   * @param opts 分词器等选项。
+   * @returns collection 句柄（u32）。
+   */
+  collection(name: string, schema: Schema, opts?: CollectionOptions): Promise<number>;
+
+  /**
+   * 追加文档。
+   * @returns accepted 数量（可能因 schema 约束少于传入数）。
+   */
+  add(col: number, docs: Doc[]): Promise<number>;
+
+  /** 刷新缓冲区，持久化段。 */
+  flush(col: number): Promise<void>;
+
+  /**
+   * 搜索。
+   * @returns Hit[]（worker 内 JSON 序列化，主线程反序列化）。
+   */
+  search(col: number, query: SearchQuery): Promise<Hit[]>;
+
+  /**
+   * 删除文档。
+   * @returns 已删除数量。
+   */
+  delete(col: number, ids: string[]): Promise<number>;
+
+  /** 触发段合并。 */
+  compact(col: number): Promise<void>;
+
+  /**
+   * 触发 reindex（同步执行）。
+   * @returns progress（0.0–1.0，1.0 表示已完成）。
+   */
+  reindex(col: number): Promise<number>;
+
+  /** 导出数据库快照到 VFS 容器内虚拟路径。配合 readFile() 读回字节下载。 */
+  export(dest: string): Promise<void>;
+
+  /** 读 VFS 容器内指定虚拟路径的文件字节（配合 export 后下载）。 */
+  readFile(path: string): Promise<Uint8Array>;
+
+  /** 关闭 Worker（flush 所有 collection + 注销句柄 + terminate worker 线程）。 */
+  close(): Promise<void>;
+}
diff --git a/bindings/web/src/vane_wasm.d.ts b/bindings/web/src/vane_wasm.d.ts
new file mode 100644
index 0000000..590a7f6
--- /dev/null
+++ b/bindings/web/src/vane_wasm.d.ts
@@ -0,0 +1,10 @@
+// 类型桥接：让 src/*.ts 能 import './vane_wasm.js' 的类型。
+//
+// 实际声明在 dist/vane_wasm.d.ts（wasm-bindgen 生成），此文件仅用于 tsc 编译期
+// 解析。tsc 不将输入 .d.ts 发射到 outDir，故 dist/vane_wasm.d.ts 保留 wasm-bindgen
+// 版本不被覆盖。
+//
+// ⚠️ 编译前置依赖：dist/vane_wasm.d.ts 必须先由 build-web.sh 的 wasm-bindgen 步骤
+// 产出，否则 tsc 报 TS2307 Cannot find module './vane_wasm.js'。
+export * from '../dist/vane_wasm.js';
+export { default } from '../dist/vane_wasm.js';
diff --git a/bindings/web/src/worker.ts b/bindings/web/src/worker.ts
new file mode 100644
index 0000000..ac7e1ee
--- /dev/null
+++ b/bindings/web/src/worker.ts
@@ -0,0 +1,113 @@
+// Vane Worker 入口（src/worker.ts → dist/worker.js）。
+//
+// 职责（§5 worker 入口策略）：
+//   1. SIMD128 探针 → 选择 simd/scalar .wasm 产物。
+//   2. init(wasmUrl) 显式传参加载 wasm（覆盖 vane_wasm.js 默认 bg.wasm URL）。
+//   3. VaneWorker.create(opts) 初始化实例。
+//   4. postMessage 路由：主页面 {op, id, ...} → VaneWorker 方法 → {id, result|error}。
+//
+// 基于 demo/worker.js（M2-14）模式，适配 @vane-rs/web 包结构：
+//   - wasm 路径用 new URL('./vane_wasm_{simd,scalar}.wasm', import.meta.url)
+//     （vite/webpack 识别 new URL 为 wasm asset）。
+//   - 探针从 ./probe.js import（非内联）。
+//
+// postMessage 协议（与 crates/vane-wasm/src/worker.js + demo/worker.js 一致）：
+//   主页面 postMessage({op, id, ...payload}) → Worker 调 VaneWorker 方法 →
+//   postMessage({id, result | error})。
+
+import init, { VaneWorker } from './vane_wasm.js';
+import { simd128Supported } from './probe.js';
+
+// Worker 运行在 DedicatedWorkerGlobalScope，非 Window。
+// lib 同时含 DOM + WebWorker 时 self 类型冲突，显式 cast 到 Worker 上下文。
+const ctx = self as unknown as DedicatedWorkerGlobalScope;
+
+/** Worker 内 VaneWorker 实例（create 后赋值，close 后置 null）。 */
+let worker: VaneWorker | null = null;
+
+/**
+ * 加载 wasm 模块（按 SIMD 探针结果动态选 simd/scalar 产物）。
+ *
+ * init(wasmUrl) 显式传 URL，覆盖 vane_wasm.js 默认的 new URL('vane_wasm_bg.wasm', ...)。
+ * vite/webpack 识别 new URL('./vane_wasm_{simd,scalar}.wasm', import.meta.url) 为 wasm asset。
+ */
+async function loadWasm(): Promise<void> {
+  const simd = simd128Supported();
+  const wasmUrl = simd
+    ? new URL('./vane_wasm_simd.wasm', import.meta.url)
+    : new URL('./vane_wasm_scalar.wasm', import.meta.url);
+  await init(wasmUrl);
+}
+
+ctx.onmessage = async (e: MessageEvent): Promise<void> => {
+  const msg = e.data;
+  // 忽略非请求消息。
+  if (!msg || typeof msg.op !== 'string') return;
+
+  const id: number | undefined = msg.id;
+
+  try {
+    // 首次 create：加载 wasm + init VaneWorker。
+    if (msg.op === 'create') {
+      await loadWasm();
+      worker = await VaneWorker.create(msg.opts ?? {});
+      ctx.postMessage({ id, result: true });
+      return;
+    }
+
+    if (!worker) {
+      ctx.postMessage({ id, error: 'worker not created (send {op:"create"} first)' });
+      return;
+    }
+
+    let result: unknown;
+    switch (msg.op) {
+      case 'open':
+        await worker.open(msg.path ?? 'vane.db', msg.opts ?? {});
+        result = true;
+        break;
+      case 'collection':
+        result = await worker.collection(msg.name, msg.schema ?? {}, msg.opts ?? {});
+        break;
+      case 'add':
+        result = await worker.add(msg.col, msg.docs ?? []);
+        break;
+      case 'flush':
+        await worker.flush(msg.col);
+        result = true;
+        break;
+      case 'search':
+        result = await worker.search(msg.col, msg.query ?? {});
+        break;
+      case 'delete':
+        result = await worker.delete(msg.col, msg.ids ?? []);
+        break;
+      case 'compact':
+        await worker.compact(msg.col);
+        result = true;
+        break;
+      case 'reindex':
+        result = await worker.reindex(msg.col);
+        break;
+      case 'export':
+        await worker.export(msg.dest ?? '');
+        result = true;
+        break;
+      case 'readFile':
+        result = await worker.readFile(msg.path ?? '');
+        break;
+      case 'close':
+        await worker.close();
+        worker = null;
+        result = true;
+        break;
+      default:
+        ctx.postMessage({ id, error: `unknown op: ${msg.op}` });
+        return;
+    }
+    ctx.postMessage({ id, result });
+  } catch (err) {
+    // VaneWorker Promise reject → 透传错误（I-8 错误透传）。
+    ctx.postMessage({ id, error: String(err) });
+  }
+};
diff --git a/bindings/web/tsconfig.json b/bindings/web/tsconfig.json
new file mode 100644
index 0000000..289e95e
--- /dev/null
+++ b/bindings/web/tsconfig.json
@@ -0,0 +1,18 @@
+{
+  "compilerOptions": {
+    "target": "ES2020",
+    "module": "ESNext",
+    "moduleResolution": "bundler",
+    "declaration": true,
+    "declarationMap": false,
+    "sourceMap": false,
+    "outDir": "dist",
+    "rootDir": "src",
+    "strict": true,
+    "esModuleInterop": true,
+    "skipLibCheck": true,
+    "forceConsistentCasingInFileNames": true,
+    "lib": ["ES2020", "DOM", "WebWorker"]
+  },
+  "include": ["src/**/*.ts"]
+}
