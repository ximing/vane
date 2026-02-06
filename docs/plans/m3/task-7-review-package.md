# Task 7 Review Package（d750558..HEAD）

## Commits
8dadd7b feat(examples): vite 最小示例验证 @vane-rs/web + @vane-rs/dict-zh 可打包

## Diff stat
 docs/plans/m3/task-7-report.md  |   61 ++
 examples/vite/.gitignore        |    2 +
 examples/vite/README.md         |   94 ++++
 examples/vite/index.html        |   12 +
 examples/vite/package-lock.json | 1168 +++++++++++++++++++++++++++++++++++++++
 examples/vite/package.json      |   19 +
 examples/vite/src/main.ts       |  122 ++++
 examples/vite/src/vite-env.d.ts |    7 +
 examples/vite/tsconfig.json     |   15 +
 examples/vite/vite.config.ts    |   11 +
 10 files changed, 1511 insertions(+)

## 完整 diff（examples/vite/ 源码，排除 package-lock.json 噪音）
diff --git a/examples/vite/.gitignore b/examples/vite/.gitignore
new file mode 100644
index 0000000..b947077
--- /dev/null
+++ b/examples/vite/.gitignore
@@ -0,0 +1,2 @@
+node_modules/
+dist/
diff --git a/examples/vite/README.md b/examples/vite/README.md
new file mode 100644
index 0000000..0240947
--- /dev/null
+++ b/examples/vite/README.md
@@ -0,0 +1,94 @@
+# Vane Vite 示例
+
+验证 `@vane-rs/web` + `@vane-rs/dict-zh` 在 vite 中可 import + 检索，零 clone/build/CDN。
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
+# 开发模式（浏览器打开 http://localhost:5173）
+npm run dev
+
+# 生产构建（打包到 dist/，验证 vite 可正确打包 wasm/worker/dict asset）
+npm run build
+
+# 预览生产构建
+npm run preview
+```
+
+## 预期输出
+
+`npm run dev` 打开浏览器后，控制台输出：
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
+- `index.html` + `assets/index-*.js`（主线程 chunk）
+- `assets/worker-*.js`（worker chunk，@vane-rs/web 的 worker.js）
+- `assets/vane_wasm_simd-*.wasm` + `vane_wasm_scalar-*.wasm`（wasm 双变体）
+- `assets/dict-*.bin` + `sha256_prefix-*.bin`（@vane-rs/dict-zh 词典 asset）
+
+## vite.config.ts 说明
+
+@vane-rs/web 用 `new URL(..., import.meta.url)` 原生支持 wasm/worker asset，**无需 vite-plugin-wasm 或 worker 插件**。
+
+| 配置项 | 用途 | 是否必需 |
+|--------|------|----------|
+| `assetsInclude: ['**/*.bin']` | 将 @vane-rs/dict-zh 的 .bin 词典文件识别为静态 asset | 是（vite 默认不含 .bin） |
+| wasm 插件 | — | 否（new URL 原生） |
+| worker 插件 | — | 否（vite 6+ 原生识别 new Worker + new URL） |
+
+`assetsInclude` 是唯一的非零配置项，与 wasm/worker 无关——仅告诉 vite 把 `.bin` 当静态 asset 处理（vite 默认 assetsInclude 含 `*.wasm` 但不含 `*.bin`）。
+
+## 文件结构
+
+```
+examples/vite/
+├── package.json          # file: 本地引用 @vane-rs/web + @vane-rs/dict-zh
+├── vite.config.ts        # assetsInclude .bin（无 wasm/worker 插件）
+├── tsconfig.json         # TS 配置（bundler moduleResolution）
+├── index.html            # 挂载点 + <script type="module">
+├── src/
+│   ├── main.ts           # createVane → open → collection(jieba) → add → search 全链路
+│   └── vite-env.d.ts     # *.bin 模块声明 + vite/client 引用
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
diff --git a/examples/vite/index.html b/examples/vite/index.html
new file mode 100644
index 0000000..9157975
--- /dev/null
+++ b/examples/vite/index.html
@@ -0,0 +1,12 @@
+<!DOCTYPE html>
+<html lang="zh-CN">
+  <head>
+    <meta charset="UTF-8" />
+    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
+    <title>Vane Vite 示例</title>
+  </head>
+  <body>
+    <div id="app">加载中... 详见控制台（F12）。</div>
+    <script type="module" src="/src/main.ts"></script>
+  </body>
+</html>
diff --git a/examples/vite/package.json b/examples/vite/package.json
new file mode 100644
index 0000000..54f06fc
--- /dev/null
+++ b/examples/vite/package.json
@@ -0,0 +1,19 @@
+{
+  "name": "vane-vite-example",
+  "private": true,
+  "version": "0.0.0",
+  "type": "module",
+  "scripts": {
+    "dev": "vite",
+    "build": "vite build",
+    "preview": "vite preview"
+  },
+  "dependencies": {
+    "@vane-rs/web": "file:../../bindings/web",
+    "@vane-rs/dict-zh": "file:../../crates/vane-dict-zh"
+  },
+  "devDependencies": {
+    "typescript": "^5.8.0",
+    "vite": "^6.3.0"
+  }
+}
diff --git a/examples/vite/src/main.ts b/examples/vite/src/main.ts
new file mode 100644
index 0000000..773c5e0
--- /dev/null
+++ b/examples/vite/src/main.ts
@@ -0,0 +1,122 @@
+/**
+ * Vane Vite 示例：验证 @vane-rs/web + @vane-rs/dict-zh 在 vite 中零配置可 import + 检索。
+ *
+ * 链路（设计 §4.3 用法示例）：
+ *   1. import createVane from @vane-rs/web
+ *   2. import dict.bin / sha256_prefix.bin from @vane-rs/dict-zh（vite asset url）
+ *   3. fetch 词典字节 → createVane({ dictData, dictSha256 }) → worker 内零 CDN
+ *   4. open → collection(jieba) → add → flush → search → console.log
+ *
+ * 运行：npm run dev（浏览器打开 http://localhost:5173）
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
+      <h1>Vane Vite 示例</h1>
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
diff --git a/examples/vite/src/vite-env.d.ts b/examples/vite/src/vite-env.d.ts
new file mode 100644
index 0000000..d411966
--- /dev/null
+++ b/examples/vite/src/vite-env.d.ts
@@ -0,0 +1,7 @@
+/// <reference types="vite/client" />
+
+// @vane-rs/dict-zh 的 .bin 词典文件作 vite asset URL 导入。
+declare module '*.bin' {
+  const src: string;
+  export default src;
+}
diff --git a/examples/vite/tsconfig.json b/examples/vite/tsconfig.json
new file mode 100644
index 0000000..af2b38a
--- /dev/null
+++ b/examples/vite/tsconfig.json
@@ -0,0 +1,15 @@
+{
+  "compilerOptions": {
+    "target": "ES2022",
+    "module": "ESNext",
+    "moduleResolution": "bundler",
+    "lib": ["ES2022", "DOM", "DOM.Iterable"],
+    "strict": true,
+    "skipLibCheck": true,
+    "noEmit": true,
+    "esModuleInterop": true,
+    "isolatedModules": true,
+    "allowImportingTsExtensions": true
+  },
+  "include": ["src"]
+}
diff --git a/examples/vite/vite.config.ts b/examples/vite/vite.config.ts
new file mode 100644
index 0000000..b586f2d
--- /dev/null
+++ b/examples/vite/vite.config.ts
@@ -0,0 +1,11 @@
+import { defineConfig } from 'vite';
+
+// @vane-rs/web 设计用 new URL(..., import.meta.url) 原生支持 wasm/worker asset，
+// 无需 vite-plugin-wasm 或 worker 插件（vite 6+ 原生识别 new URL + Worker 模式）。
+//
+// assetsInclude：将 @vane-rs/dict-zh 的 .bin 词典文件识别为静态 asset。
+// vite 默认 assetsInclude 含 *.wasm 但不含 *.bin，需显式声明。
+// 这是唯一的非零配置项，与 wasm/worker 无关。
+export default defineConfig({
+  assetsInclude: ['**/*.bin'],
+});
