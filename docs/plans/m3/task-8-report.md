# Task 8 Report：examples/webpack/ 最小 webpack 5 示例

## 状态

DONE

## Commits

- `55073cb` `feat(examples): webpack 5 最小示例验证 @vane-rs/web + @vane-rs/dict-zh 可打包`

## 测试摘要

`npm install`（file: symlink，371 packages）+ `npm run build` 成功：wasm asset×3（simd/scalar/bg）+ dict asset×2（dict.bin/sha256_prefix.bin）+ worker ESM chunk（13.1 KB）+ 主线程 ESM chunk（4.3 KB）产出，1 warning（html-webpack-plugin 内部 `with` 语句与 ESM 严格模式兼容性，不影响功能）。

## Concerns

1. **`experiments.outputModule` 足够，不需要 `asyncWebAssembly`**：设计 §9.3 说法验证成立。@vane-rs/web 的 worker.js 用 `init(wasmUrl)` 显式 fetch 加载 wasm，不依赖 webpack 的 wasm 模块导入机制。webpack 只需把 `.wasm` 文件作为 asset 产出 URL，`init()` 内部 `fetch(url)` 加载。`outputModule: true` + `asset/resource` 规则即可。

2. **`HtmlWebpackPlugin.scriptLoading: 'module'` 必需**：`experiments.outputModule` 产出 ESM（使用 `import.meta.url`），但 html-webpack-plugin 默认 `scriptLoading: 'defer'` 注入 `<script defer>`。`import.meta` 在 classic script 中是 SyntaxError，必须显式设 `scriptLoading: 'module'` 注入 `<script type="module">`。这不是 @vane-rs/web 的问题，是 webpack ESM 输出 + html-webpack-plugin 的通用配置要求。

3. **html-webpack-plugin `with` 语句 warning**：`experiments.outputModule`（ESM 严格模式）与 html-webpack-plugin 内部 loader（用 `with` 语句编译模板）不兼容，报 1 warning。这是 html-webpack-plugin 的已知兼容性问题（非用户代码），不影响 HTML 产出——`index.html` 正确注入 `<script type="module">`。build 退出码 0。

4. **tsconfig 不能用 `noEmit: true` + `allowImportingTsExtensions`**：ts-loader 尊重 `noEmit: true` 导致 "TypeScript emitted no output" 报错；`allowImportingTsExtensions` 要求 `noEmit` 或 `emitDeclarationOnly`。解法：移除两者。ts-loader 在内存中转译交给 webpack，不需要 `noEmit`。vite 示例用 `noEmit` 是因为 vite 用 esbuild 转译（不经 tsc），webpack + ts-loader 不同。

5. **worker chunk 文件名含 chunk ID**：webpack 产出 `675.index.js`（chunk ID + `.index.js`），非 `worker.js`。这是 webpack 的 chunk 命名机制（无显式 `chunkFilename` 时用 `[id].index.js`）。功能正确——主线程 chunk 内 `new Worker(new URL(..., 675.index.js), {type:'module'})` 正确引用。

6. **`vane_wasm_bg.wasm` 也被产出**：vane_wasm.js 末尾默认 `new URL('vane_wasm_bg.wasm', import.meta.url)` 被 webpack 静态分析识别为 asset，产出 `assets/vane_wasm_bg.wasm`（796 KB = scalar 别名）。worker.js 显式传 simd/scalar URL 覆盖默认，bg.wasm 运行时不使用，但 webpack 仍产出（静态分析无法判定运行时不走此路径）。+796 KB 产出体积，不影响功能。

## webpack build 产出

| 文件 | 大小 | 说明 |
|------|------|------|
| `dist/index.html` | 298 B | HTML（html-webpack-plugin 注入 `<script type="module">`） |
| `dist/index.js` | 4.26 KB | 主线程 ESM chunk（createVane + dict fetch + 渲染） |
| `dist/675.index.js` | 13.1 KB | Worker ESM chunk（wasm-bindgen glue + SIMD 探针 + postMessage 路由） |
| `dist/assets/vane_wasm_simd.wasm` | 785 KB | SIMD128 加速变体 |
| `dist/assets/vane_wasm_scalar.wasm` | 796 KB | scalar 兜底变体 |
| `dist/assets/vane_wasm_bg.wasm` | 796 KB | bg 别名（= scalar，vane_wasm.js 默认 URL，运行时不使用） |
| `dist/assets/dict.bin` | 1.41 MB | @vane-rs/dict-zh 词典 asset |
| `dist/assets/sha256_prefix.bin` | 8 B | sha256 前缀 asset |

- **wasm asset 路径**：`dist/assets/vane_wasm_simd.wasm` + `dist/assets/vane_wasm_scalar.wasm` + `dist/assets/vane_wasm_bg.wasm`
- **worker chunk 路径**：`dist/675.index.js`

## webpack.config 关键配置

```js
experiments: {
  // ESM 输出：@vane-rs/web 是 ESM 包，worker 需 {type:'module'}
  // 不需要 asyncWebAssembly——init(wasmUrl) 显式 fetch 绕过
  outputModule: true,
},
module: {
  rules: [
    { test: /\.ts$/, use: 'ts-loader', exclude: /node_modules/ },
    // .wasm + .bin 作 asset/resource
    // new URL('./x.wasm', import.meta.url) 由 webpack 5 原生处理
    // 此规则额外覆盖 import dictBinUrl from '.../*.bin' 的直接导入
    { test: /\.(wasm|bin)$/, type: 'asset/resource' },
  ],
},
plugins: [
  new HtmlWebpackPlugin({
    template: './index.html',
    // ESM 产出用 import.meta.url，defer 会 SyntaxError
    scriptLoading: 'module',
  }),
],
```

| 配置项 | 用途 | 是否必需 |
|--------|------|----------|
| `experiments.outputModule: true` | ESM 输出 | 是 |
| `experiments.asyncWebAssembly` | — | 否（init(wasmUrl) 绕过） |
| `{ test: /\.(wasm\|bin)$/, type: 'asset/resource' }` | .wasm + .bin asset module | 是 |
| `HtmlWebpackPlugin.scriptLoading: 'module'` | 注入 `<script type="module">` | 是 |
| wasm/worker 插件 | — | 否 |

## 验证结果

1. ✅ `bindings/web/dist/` 存在（build-web.sh 前置已产出）
2. ✅ `cd examples/webpack && npm install` 成功（file: symlink，371 packages）
3. ✅ `cd examples/webpack && npm run build` 成功（wasm asset×3 + dict asset×2 + worker chunk + 主线程 chunk，1 warning 不影响功能）
4. ✅ git diff 确认未改 bindings/web/ / crates/ / examples/vite/（仅 docs/plans/m3/task-1-design.md 有会话前已存在的修改）

## 文件清单

```
examples/webpack/
├── .gitignore                    # node_modules/ + dist/
├── README.md                     # 前置(build-web.sh) + 运行 + 产出 + webpack.config 说明 + 已知 warning
├── index.html                    # 挂载点（html-webpack-plugin 注入 <script type="module">）
├── package.json                  # file: 引用 @vane-rs/web + @vane-rs/dict-zh + webpack5/ts-loader/html-webpack-plugin
├── package-lock.json             # npm install 产出（lockfileVersion 3）
├── tsconfig.json                 # bundler moduleResolution + ES2022 + DOM lib（无 noEmit，适配 ts-loader）
├── webpack.config.js             # experiments.outputModule + asset/resource + scriptLoading:'module'
└── src/
    ├── main.ts                   # createVane → open → collection(jieba) → add → flush → search 全链路
    └── env.d.ts                  # *.bin 模块声明
```

## 设计 §9.3 验证结论

设计 §9.3 称"webpack 5 需 `experiments: { outputModule: true }`，用 `init(wasmUrl)` 显式 fetch 可绕过 asyncWebAssembly 需求"——**验证成立**。

- `outputModule: true` 是必需的（@vane-rs/web 是 ESM 包，worker 需 `{type:'module'}`）。
- `asyncWebAssembly` 不需要（worker 内 `init(wasmUrl)` 显式 fetch，不依赖 webpack wasm 模块导入）。
- `new URL('./x.wasm', import.meta.url)` 由 webpack 5 原生识别为 asset module，自动产出 wasm 文件 + 重写 URL。
- 额外发现：`HtmlWebpackPlugin.scriptLoading: 'module'` 也是必需的（ESM 输出的通用要求，非 @vane-rs/web 特有）。
