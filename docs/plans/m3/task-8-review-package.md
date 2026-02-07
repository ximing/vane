# Task 8 Review Package（8dadd7b..HEAD）

## Commits
3f4928f docs(plans): Task 8 report——webpack 5 示例验证完成
55073cb feat(examples): webpack 5 最小示例验证 @vane-rs/web + @vane-rs/dict-zh 可打包

## Diff stat
 docs/plans/m3/task-8-report.md     |  109 +
 examples/webpack/.gitignore        |    2 +
 examples/webpack/README.md         |  112 +
 examples/webpack/index.html        |   12 +
 examples/webpack/package-lock.json | 5000 ++++++++++++++++++++++++++++++++++++
 examples/webpack/package.json      |   21 +
 examples/webpack/src/env.d.ts      |    6 +
 examples/webpack/src/main.ts       |  122 +
 examples/webpack/tsconfig.json     |   13 +
 examples/webpack/webpack.config.js |   73 +
 10 files changed, 5470 insertions(+)

## 完整 diff（examples/webpack/ 源码，排除 package-lock.json 噪音）
diff --git a/examples/webpack/.gitignore b/examples/webpack/.gitignore
new file mode 100644
index 0000000..b947077
--- /dev/null
+++ b/examples/webpack/.gitignore
@@ -0,0 +1,2 @@
+node_modules/
+dist/
diff --git a/examples/webpack/README.md b/examples/webpack/README.md
new file mode 100644
index 0000000..5171c04
--- /dev/null
+++ b/examples/webpack/README.md
@@ -0,0 +1,112 @@
+# Vane Webpack 5 示例
+
+验证 `@vane-rs/web` + `@vane-rs/dict-zh` 在 webpack 5 中可 import + 检索，零 clone/build/CDN。
+
+## 前置条件
+
+`@vane-rs/web` 的 `dist/` 是构建产物（.gitignore 忽略），file: 本地引用需要 dist/ 存在。首次运行前先产出：
+
+```bash
+# 在仓库根目录
+bash bindings/web/scripts/build-web.sh
+```
+
+产出 `bindings/web/dist/`（index.js / worker.js / vane_wasm_simd.wasm / vane_wasm_scalar.wasm 等）。
+
+## 运行
+
+```bash
+# 安装依赖（file: 本地链接 @vane-rs/web + @vane-rs/dict-zh）
+npm install
+
+# 开发模式（浏览器打开 http://localhost:8080）
+npm run serve
+
+# 生产构建（打包到 dist/，验证 webpack 可正确打包 wasm/worker/dict asset）
+npm run build
+```
+
+## 预期输出
+
+`npm run serve` 打开浏览器后，控制台输出：
+
+```
+[vane] 加载词典...
+[vane] 词典加载完成（1479454 字节），sha256 前缀: xxxxxxxx
+[vane] 创建 Vane 实例（memory VFS）...
+[vane] collection 创建成功, handle: 1
+[vane] 灌入 3 篇文档并 flush
+[vane] 搜索 "检索" 结果（3 条）:
+  d1  score=0.xxxx  fields={...}
+  d2  score=0.xxxx  fields={...}
+  d3  score=0.xxxx  fields={...}
+[vane] 已关闭
+```
+
+页面显示搜索结果列表。
+
+`npm run build` 产出 `dist/`，含：
+- `index.html`（html-webpack-plugin 注入 `<script type="module">`）
+- `index.js`（主线程 ESM chunk，~4.3 KB）
+- `<chunkId>.index.js`（worker ESM chunk，@vane-rs/web 的 worker.js，~13 KB）
+- `assets/vane_wasm_simd.wasm` + `vane_wasm_scalar.wasm` + `vane_wasm_bg.wasm`（wasm asset）
+- `assets/dict.bin` + `sha256_prefix.bin`（@vane-rs/dict-zh 词典 asset）
+
+## webpack.config.js 说明
+
+@vane-rs/web 用 `new URL(..., import.meta.url)` 原生支持 wasm/worker asset，**无需 asyncWebAssembly 实验**。
+
+| 配置项 | 用途 | 是否必需 |
+|--------|------|----------|
+| `experiments.outputModule: true` | ESM 输出（@vane-rs/web 是 ESM 包，worker 需 `{type:'module'}`） | 是 |
+| `experiments.asyncWebAssembly` | — | 否（worker 内 `init(wasmUrl)` 显式 fetch，绕过 webpack wasm 模块导入） |
+| `{ test: /\.(wasm\|bin)$/, type: 'asset/resource' }` | .wasm + .bin 作 asset module | 是（.bin 直接导入需此规则；.wasm 的 `new URL` 由 webpack 5 原生处理） |
+| `HtmlWebpackPlugin.scriptLoading: 'module'` | 注入 `<script type="module">`（ESM 产出用 `import.meta.url`，`defer` 会 SyntaxError） | 是 |
+| wasm 插件 | — | 否（`new URL` + `init(url)` 模式绕过） |
+| worker 插件 | — | 否（webpack 5 原生识别 `new Worker(new URL(...))` 模式） |
+
+### 已知 Warning
+
+`npm run build` 会报 1 个 warning（不影响功能）：
+
+```
+WARNING in ./index.html (./node_modules/html-webpack-plugin/lib/loader.js!./index.html)
+`with` statements are not allowed. The output is an ES module, which runs in strict mode.
+```
+
+这是 html-webpack-plugin 内部 loader 与 `experiments.outputModule`（ESM 严格模式）的已知兼容性问题。`with` 语句在 html-webpack-plugin 的模板编译器内部（非用户代码），不影响 HTML 产出——`index.html` 正确注入 `<script type="module">`。
+
+### 关键设计验证
+
+1. **`experiments.outputModule` 足够**：不需要 `asyncWebAssembly`。@vane-rs/web 的 worker.js 用 `init(wasmUrl)` 显式 fetch 加载 wasm，不依赖 webpack 的 wasm 模块导入机制。webpack 只需把 `.wasm` 文件作为 asset 产出 URL，`init()` 内部 `fetch(url)` 加载。
+
+2. **`new URL('./x.wasm', import.meta.url)` 原生支持**：webpack 5 识别此模式为 asset module，自动产出 wasm 文件 + 重写 URL。worker chunk 内的 `import.meta.url` 被正确重写为 worker chunk 的 URL。
+
+3. **worker chunk 为 ESM**：`outputModule: true` 使 worker chunk 输出为 ESM，配合 `{type:'module'}` 创建 Worker。
+
+## 文件结构
+
+```
+examples/webpack/
+├── package.json          # file: 本地引用 @vane-rs/web + @vane-rs/dict-zh
+├── webpack.config.js     # experiments.outputModule + asset/resource 规则
+├── tsconfig.json         # TS 配置（bundler moduleResolution）
+├── index.html            # 挂载点（html-webpack-plugin 注入 script）
+├── src/
+│   ├── main.ts           # createVane → open → collection(jieba) → add → search 全链路
+│   └── env.d.ts          # *.bin 模块声明
+└── README.md
+```
+
+## 关于 file: 本地引用
+
+`@vane-rs/web` + `@vane-rs/dict-zh` 尚未发 npm registry（M3 Task 11 release.yml 才发版）。本示例用 `file:` 本地路径引用：
+
+```json
+{
+  "@vane-rs/web": "file:../../bindings/web",
+  "@vane-rs/dict-zh": "file:../../crates/vane-dict-zh"
+}
+```
+
+`npm install` 时 npm 创建 symlink 指向本地目录。发版后可改为常规版本号 `"@vane-rs/web": "^0.2.0"`。
diff --git a/examples/webpack/index.html b/examples/webpack/index.html
new file mode 100644
index 0000000..445d109
--- /dev/null
+++ b/examples/webpack/index.html
@@ -0,0 +1,12 @@
+<!DOCTYPE html>
+<html lang="zh-CN">
+  <head>
+    <meta charset="UTF-8" />
+    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
+    <title>Vane Webpack 示例</title>
+  </head>
+  <body>
+    <div id="app">加载中... 详见控制台（F12）。</div>
+    <!-- html-webpack-plugin 会自动注入 <script type="module"> -->
+  </body>
+</html>
diff --git a/examples/webpack/package.json b/examples/webpack/package.json
new file mode 100644
index 0000000..7f59386
--- /dev/null
+++ b/examples/webpack/package.json
@@ -0,0 +1,21 @@
+{
+  "name": "vane-webpack-example",
+  "private": true,
+  "version": "0.0.0",
+  "scripts": {
+    "build": "webpack --mode production",
+    "serve": "webpack serve --mode development"
+  },
+  "dependencies": {
+    "@vane-rs/web": "file:../../bindings/web",
+    "@vane-rs/dict-zh": "file:../../crates/vane-dict-zh"
+  },
+  "devDependencies": {
+    "html-webpack-plugin": "^5.6.3",
+    "ts-loader": "^9.5.2",
+    "typescript": "^5.8.0",
+    "webpack": "^5.97.0",
+    "webpack-cli": "^5.1.4",
+    "webpack-dev-server": "^5.2.0"
+  }
+}
diff --git a/examples/webpack/src/env.d.ts b/examples/webpack/src/env.d.ts
new file mode 100644
index 0000000..3ef1644
--- /dev/null
+++ b/examples/webpack/src/env.d.ts
@@ -0,0 +1,6 @@
+// @vane-rs/dict-zh 的 .bin 词典文件作 webpack asset URL 导入。
+// webpack 5 asset module（type: 'asset/resource'）将 .bin 解析为资源 URL 字符串。
+declare module '*.bin' {
+  const src: string;
+  export default src;
+}
diff --git a/examples/webpack/src/main.ts b/examples/webpack/src/main.ts
new file mode 100644
index 0000000..010b197
--- /dev/null
+++ b/examples/webpack/src/main.ts
@@ -0,0 +1,122 @@
+/**
+ * Vane Webpack 5 示例：验证 @vane-rs/web + @vane-rs/dict-zh 在 webpack 5 中可 import + 检索。
+ *
+ * 链路（设计 §4.3 用法示例）：
+ *   1. import createVane from @vane-rs/web
+ *   2. import dict.bin / sha256_prefix.bin from @vane-rs/dict-zh（webpack asset url）
+ *   3. fetch 词典字节 → createVane({ dictData, dictSha256 }) → worker 内零 CDN
+ *   4. open → collection(jieba) → add → flush → search → console.log
+ *
+ * 运行：npm run serve（浏览器打开 http://localhost:8080）
+ */
+
+import { createVane } from '@vane-rs/web';
+import type { Schema, Hit } from '@vane-rs/web';
+import dictBinUrl from '@vane-rs/dict-zh/dict.bin';
+import dictSha256Url from '@vane-rs/dict-zh/sha256_prefix.bin';
+
+// ── 占位 hash 向量（SPEC Won't-have：Vane 不内置 embedding）─────────────────
+// 简单 char unigram bucket → 64 维 L2 归一化。同文本同向量、共享字符的文本
+// cosine 相似度更高，足以演示向量召回。生产应替换为真实 embedding API。
+const DIM = 64;
+
+function hashVector(text: string, dim = DIM): number[] {
+  const vec = new Float32Array(dim);
+  for (const ch of [...text]) {
+    const code = ch.codePointAt(0) || 0;
+    vec[code % dim] += 1;
+  }
+  let norm = 0;
+  for (let i = 0; i < dim; i++) norm += vec[i] * vec[i];
+  norm = Math.sqrt(norm) || 1;
+  for (let i = 0; i < dim; i++) vec[i] /= norm;
+  return Array.from(vec);
+}
+
+// ── 主流程 ────────────────────────────────────────────────────────────────
+
+async function main(): Promise<void> {
+  // 1. 加载词典字节（@vane-rs/dict-zh 本地引用，零 CDN）
+  console.log('[vane] 加载词典...');
+  const dictData = new Uint8Array(await (await fetch(dictBinUrl)).arrayBuffer());
+  const sha256Hex = Array.from(
+    new Uint8Array(await (await fetch(dictSha256Url)).arrayBuffer()),
+  )
+    .map((b) => b.toString(16).padStart(2, '0'))
+    .join('');
+  console.log(`[vane] 词典加载完成（${dictData.byteLength} 字节），sha256 前缀: ${sha256Hex}`);
+
+  // 2. 创建 Vane 实例（memory VFS，避免 OPFS 权限弹窗；生产可用 'opfs' 持久化）
+  console.log('[vane] 创建 Vane 实例（memory VFS）...');
+  const vane = await createVane({
+    vfs: 'memory',
+    dbPath: 'vane.db',
+    dictData, // transferable 零拷贝，transfer 后主线程不可再访问
+    dictSha256: sha256Hex,
+  });
+
+  // 3. 打开数据库 + 创建 collection（jieba 分词）
+  await vane.open();
+  const schema: Schema = {
+    fields: [
+      { name: 'text', type: 'text' },
+      { name: 'vec', type: 'vector', dim: DIM, metric: 'cosine' },
+    ],
+  };
+  const col = await vane.collection('docs', schema, { tokenizer: 'jieba' });
+  console.log(`[vane] collection 创建成功, handle: ${col}`);
+
+  // 4. 灌入中文文档
+  const docs = [
+    { id: 'd1', text: '向量检索入门指南', vector: hashVector('向量检索入门指南') },
+    { id: 'd2', text: 'BM25 文本检索算法原理', vector: hashVector('BM25 文本检索算法原理') },
+    { id: 'd3', text: 'RRF 融合排序策略', vector: hashVector('RRF 融合排序策略') },
+  ];
+  const accepted = await vane.add(col, docs);
+  await vane.flush(col);
+  console.log(`[vane] 灌入 ${accepted} 篇文档并 flush`);
+
+  // 5. 混合检索（文本 + 向量 → RRF 融合）
+  const query = '检索';
+  const hits: Hit[] = await vane.search(col, {
+    text: query,
+    vector: hashVector(query),
+    topK: 10,
+    mode: 'hybrid',
+  });
+  console.log(`[vane] 搜索 "${query}" 结果（${hits.length} 条）:`);
+  for (const hit of hits) {
+    const score = hit.score.toFixed(4);
+    console.log(`  ${hit.id}  score=${score}  fields=${JSON.stringify(hit.fields)}`);
+  }
+
+  // 6. 关闭
+  await vane.close();
+  console.log('[vane] 已关闭');
+
+  // 渲染到页面
+  const app = document.getElementById('app');
+  if (app) {
+    app.innerHTML = `
+      <h1>Vane Webpack 示例</h1>
+      <p>搜索"${query}"返回 ${hits.length} 条结果：</p>
+      <ul>
+        ${hits
+          .map(
+            (h) =>
+              `<li><strong>${h.id}</strong> — score: ${h.score.toFixed(4)}</li>`,
+          )
+          .join('')}
+      </ul>
+      <p>详见控制台输出（F12）。</p>
+    `;
+  }
+}
+
+main().catch((err) => {
+  console.error('[vane] 错误:', err);
+  const app = document.getElementById('app');
+  if (app) {
+    app.innerHTML = `<p style="color:red">错误: ${err.message}</p>`;
+  }
+});
diff --git a/examples/webpack/tsconfig.json b/examples/webpack/tsconfig.json
new file mode 100644
index 0000000..3ef9717
--- /dev/null
+++ b/examples/webpack/tsconfig.json
@@ -0,0 +1,13 @@
+{
+  "compilerOptions": {
+    "target": "ES2022",
+    "module": "ESNext",
+    "moduleResolution": "bundler",
+    "lib": ["ES2022", "DOM", "DOM.Iterable"],
+    "strict": true,
+    "skipLibCheck": true,
+    "esModuleInterop": true,
+    "isolatedModules": true
+  },
+  "include": ["src"]
+}
diff --git a/examples/webpack/webpack.config.js b/examples/webpack/webpack.config.js
new file mode 100644
index 0000000..b7e7ecf
--- /dev/null
+++ b/examples/webpack/webpack.config.js
@@ -0,0 +1,73 @@
+// Vane Webpack 5 示例配置。
+//
+// @vane-rs/web 是 ESM 包（package.json "type":"module"），用 new URL(..., import.meta.url)
+// 原生处理 wasm/worker asset，init(wasmUrl) 显式 fetch 加载 wasm。
+// 设计 §9.3 称 webpack 5 需 experiments.outputModule（ESM 输出），用 init(wasmUrl) 显式
+// fetch 可绕过 experiments.asyncWebAssembly 需求——本配置验证此说法。
+const path = require('path');
+const HtmlWebpackPlugin = require('html-webpack-plugin');
+
+module.exports = {
+  // mode 由 CLI --mode flag 设置（build=production, serve=development）
+  entry: './src/main.ts',
+
+  // §9.3：ESM 输出。@vane-rs/web 是 ESM 包，worker 需 {type:'module'}。
+  // outputModule 使主线程 chunk + worker chunk 均输出为 ESM。
+  // 不需要 experiments.asyncWebAssembly——worker 内 init(wasmUrl) 显式 fetch 加载 wasm，
+  // 不依赖 webpack 的 wasm 模块导入机制。
+  experiments: {
+    outputModule: true,
+  },
+
+  output: {
+    filename: 'index.js',
+    path: path.resolve(__dirname, 'dist'),
+    clean: true,
+    // wasm + bin asset 产出路径（new URL + import 均用此模板）
+    assetModuleFilename: 'assets/[name][ext]',
+  },
+
+  resolve: {
+    extensions: ['.ts', '.js'],
+  },
+
+  module: {
+    rules: [
+      {
+        test: /\.ts$/,
+        use: 'ts-loader',
+        exclude: /node_modules/,
+      },
+      {
+        // §9.4：webpack 5 asset module 处理 .wasm + .bin。
+        // new URL('./x.wasm', import.meta.url) 由 webpack 5 原生识别为 asset，
+        // 此规则额外覆盖 import dictBinUrl from '.../*.bin' 的直接导入。
+        test: /\.(wasm|bin)$/,
+        type: 'asset/resource',
+      },
+    ],
+  },
+
+  plugins: [
+    new HtmlWebpackPlugin({
+      template: './index.html',
+      // experiments.outputModule 产出 ESM（import.meta.url），需 type="module" 加载。
+      // 默认 'defer' 注入 <script defer>，ESM 代码会 SyntaxError。
+      scriptLoading: 'module',
+    }),
+  ],
+
+  // wasm 产物较大，关闭性能提示（最小示例，非生产优化）
+  performance: {
+    hints: false,
+  },
+
+  devServer: {
+    static: {
+      directory: path.join(__dirname, 'dist'),
+    },
+    compress: true,
+    port: 8080,
+    hot: true,
+  },
+};
