# Vane Webpack 5 示例

验证 `@vane-rs/web` + `@vane-rs/dict-zh` 在 webpack 5 中可 import + 检索，零 clone/build/CDN。

## 前置条件

`@vane-rs/web` 的 `dist/` 是构建产物（.gitignore 忽略），file: 本地引用需要 dist/ 存在。首次运行前先产出：

```bash
# 在仓库根目录
bash bindings/web/scripts/build-web.sh
```

产出 `bindings/web/dist/`（index.js / worker.js / vane_wasm_simd.wasm / vane_wasm_scalar.wasm 等）。

## 运行

```bash
# 安装依赖（file: 本地链接 @vane-rs/web + @vane-rs/dict-zh）
npm install

# 开发模式（浏览器打开 http://localhost:8080）
npm run serve

# 生产构建（打包到 dist/，验证 webpack 可正确打包 wasm/worker/dict asset）
npm run build
```

## 预期输出

`npm run serve` 打开浏览器后，控制台输出：

```
[vane] 加载词典...
[vane] 词典加载完成（1479454 字节），sha256 前缀: xxxxxxxx
[vane] 创建 Vane 实例（memory VFS）...
[vane] collection 创建成功, handle: 1
[vane] 灌入 3 篇文档并 flush
[vane] 搜索 "检索" 结果（3 条）:
  d1  score=0.xxxx  fields={...}
  d2  score=0.xxxx  fields={...}
  d3  score=0.xxxx  fields={...}
[vane] 已关闭
```

页面显示搜索结果列表。

`npm run build` 产出 `dist/`，含：
- `index.html`（html-webpack-plugin 注入 `<script type="module">`）
- `index.js`（主线程 ESM chunk，~4.3 KB）
- `<chunkId>.index.js`（worker ESM chunk，@vane-rs/web 的 worker.js，~13 KB）
- `assets/vane_wasm_simd.wasm` + `vane_wasm_scalar.wasm` + `vane_wasm_bg.wasm`（wasm asset）
- `assets/dict.bin` + `sha256_prefix.bin`（@vane-rs/dict-zh 词典 asset）

## webpack.config.js 说明

@vane-rs/web 用 `new URL(..., import.meta.url)` 原生支持 wasm/worker asset，**无需 asyncWebAssembly 实验**。

| 配置项 | 用途 | 是否必需 |
|--------|------|----------|
| `experiments.outputModule: true` | ESM 输出（@vane-rs/web 是 ESM 包，worker 需 `{type:'module'}`） | 是 |
| `experiments.asyncWebAssembly` | — | 否（worker 内 `init(wasmUrl)` 显式 fetch，绕过 webpack wasm 模块导入） |
| `{ test: /\.(wasm\|bin)$/, type: 'asset/resource' }` | .wasm + .bin 作 asset module | 是（.bin 直接导入需此规则；.wasm 的 `new URL` 由 webpack 5 原生处理） |
| `HtmlWebpackPlugin.scriptLoading: 'module'` | 注入 `<script type="module">`（ESM 产出用 `import.meta.url`，`defer` 会 SyntaxError） | 是 |
| wasm 插件 | — | 否（`new URL` + `init(url)` 模式绕过） |
| worker 插件 | — | 否（webpack 5 原生识别 `new Worker(new URL(...))` 模式） |

### 已知 Warning

`npm run build` 会报 1 个 warning（不影响功能）：

```
WARNING in ./index.html (./node_modules/html-webpack-plugin/lib/loader.js!./index.html)
`with` statements are not allowed. The output is an ES module, which runs in strict mode.
```

这是 html-webpack-plugin 内部 loader 与 `experiments.outputModule`（ESM 严格模式）的已知兼容性问题。`with` 语句在 html-webpack-plugin 的模板编译器内部（非用户代码），不影响 HTML 产出——`index.html` 正确注入 `<script type="module">`。

### 关键设计验证

1. **`experiments.outputModule` 足够**：不需要 `asyncWebAssembly`。@vane-rs/web 的 worker.js 用 `init(wasmUrl)` 显式 fetch 加载 wasm，不依赖 webpack 的 wasm 模块导入机制。webpack 只需把 `.wasm` 文件作为 asset 产出 URL，`init()` 内部 `fetch(url)` 加载。

2. **`new URL('./x.wasm', import.meta.url)` 原生支持**：webpack 5 识别此模式为 asset module，自动产出 wasm 文件 + 重写 URL。worker chunk 内的 `import.meta.url` 被正确重写为 worker chunk 的 URL。

3. **worker chunk 为 ESM**：`outputModule: true` 使 worker chunk 输出为 ESM，配合 `{type:'module'}` 创建 Worker。

## 文件结构

```
examples/webpack/
├── package.json          # file: 本地引用 @vane-rs/web + @vane-rs/dict-zh
├── webpack.config.js     # experiments.outputModule + asset/resource 规则
├── tsconfig.json         # TS 配置（bundler moduleResolution）
├── index.html            # 挂载点（html-webpack-plugin 注入 script）
├── src/
│   ├── main.ts           # createVane → open → collection(jieba) → add → search 全链路
│   └── env.d.ts          # *.bin 模块声明
└── README.md
```

## 关于 file: 本地引用

`@vane-rs/web` + `@vane-rs/dict-zh` 尚未发 npm registry（M3 Task 11 release.yml 才发版）。本示例用 `file:` 本地路径引用：

```json
{
  "@vane-rs/web": "file:../../bindings/web",
  "@vane-rs/dict-zh": "file:../../crates/vane-dict-zh"
}
```

`npm install` 时 npm 创建 symlink 指向本地目录。发版后可改为常规版本号 `"@vane-rs/web": "^0.3.0"`。
