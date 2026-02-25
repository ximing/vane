# Vane Vite 示例

验证 `@vane-rs/web` + `@vane-rs/dict-zh` 在 vite 中可 import + 检索，零 clone/build/CDN。

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

# 开发模式（浏览器打开 http://localhost:5173）
npm run dev

# 生产构建（打包到 dist/，验证 vite 可正确打包 wasm/worker/dict asset）
npm run build

# 预览生产构建
npm run preview
```

## 预期输出

`npm run dev` 打开浏览器后，控制台输出：

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
- `index.html` + `assets/index-*.js`（主线程 chunk）
- `assets/worker-*.js`（worker chunk，@vane-rs/web 的 worker.js）
- `assets/vane_wasm_bg-*.wasm` + `vane_wasm_simd-*.wasm`（wasm 双变体：bg 为非 SIMD 回退，simd 为 SIMD 加速）
- `assets/dict-*.bin`（@vane-rs/dict-zh 词典 asset；`sha256_prefix.bin` 仅 8 字节，vite 按 `assetsInlineLimit` 默认 4096 内联为 data URI，不单独产出文件）

## vite.config.ts 说明

@vane-rs/web 用 `new URL(..., import.meta.url)` 原生支持 wasm/worker asset，**无需 vite-plugin-wasm 或 worker 插件**。

| 配置项 | 用途 | 是否必需 |
|--------|------|----------|
| `assetsInclude: ['**/*.bin']` | 将 @vane-rs/dict-zh 的 .bin 词典文件识别为静态 asset | 是（vite 默认不含 .bin） |
| wasm 插件 | — | 否（new URL 原生） |
| worker 插件 | — | 否（vite 6+ 原生识别 new Worker + new URL） |

`assetsInclude` 是唯一的非零配置项，与 wasm/worker 无关——仅告诉 vite 把 `.bin` 当静态 asset 处理（vite 默认 assetsInclude 含 `*.wasm` 但不含 `*.bin`）。

## 文件结构

```
examples/vite/
├── package.json          # file: 本地引用 @vane-rs/web + @vane-rs/dict-zh
├── vite.config.ts        # assetsInclude .bin（无 wasm/worker 插件）
├── tsconfig.json         # TS 配置（bundler moduleResolution）
├── index.html            # 挂载点 + <script type="module">
├── src/
│   ├── main.ts           # createVane → open → collection(jieba) → add → search 全链路
│   └── vite-env.d.ts     # *.bin 模块声明 + vite/client 引用
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
