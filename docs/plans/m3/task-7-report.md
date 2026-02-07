# Task 7 Report：examples/vite/ 最小 vite 示例

## 状态

DONE

## Commits

- `feat(examples): vite 最小示例验证 @vane-rs/web + @vane-rs/dict-zh 可打包`

## 测试摘要

`npm install`（file: symlink）+ `npm run build` 成功：7 模块转换，worker chunk（12.76 kB）+ 双变体 wasm asset + dict.bin asset 正确产出，无报错。

## Concerns

1. **`assetsInclude: ['**/*.bin']`**：vite 默认 assetsInclude 含 `*.wasm` 但不含 `*.bin`。@vane-rs/dict-zh 的 `dict.bin` / `sha256_prefix.bin` 导入需在 vite.config.ts 显式声明 `assetsInclude: ['**/*.bin']`。这不是 wasm/worker 插件（设计 §9 零配置目标仅指无需 wasm/worker plugin），但用户需加此一行配置。建议 Task 11 发版前在 @vane-rs/web README 的 vite 集成节补充说明此配置项。

2. **sha256_prefix.bin 内联为 data URI**：8 字节 < vite 默认 assetsInlineLimit（4KB），vite 自动内联为 `data:application/octet-stream;base64,...`。这是 vite 标准行为，不影响功能（fetch 可解析 data URI）。

3. **vane_wasm_scalar.wasm 与 vane_wasm_bg.wasm 去重**：vite 内容哈希去重——bg.wasm 是 scalar.wasm 的 cp 别名（同内容），vite 只产出一个文件（`vane_wasm_bg-*.wasm`，814.62 kB）。worker 内 `new URL('./vane_wasm_scalar.wasm', import.meta.url)` 被重写指向同一 asset。功能正确，无数据丢失。

4. **worker 格式为 IIFE（非 ESM）**：vite 默认 `worker.format: 'iife'`，worker chunk 输出为 IIFE。即使源码用 `{ type: 'module' }`，vite 自动调整为 IIFE + 去掉 type 标记。功能正确（vite 内部处理，所有 import 被内联），但如果未来 @vane-rs/web worker 依赖动态 import 等 ESM 特性，可能需 `worker: { format: 'es' }`。当前验证通过。

## vite build 产出

| 文件 | 大小 | 说明 |
|------|------|------|
| `dist/index.html` | 0.37 kB | HTML 入口 |
| `dist/assets/index-*.js` | 4.79 kB (gzip 2.36 kB) | 主线程 chunk（createVane + dict fetch + 渲染） |
| `dist/assets/worker-*.js` | 12.76 kB | Worker chunk（wasm-bindgen glue + SIMD 探针 + postMessage 路由） |
| `dist/assets/vane_wasm_simd-*.wasm` | 803.91 kB (gzip 320.34 kB) | SIMD128 加速变体 |
| `dist/assets/vane_wasm_bg-*.wasm` | 814.62 kB (gzip 322.43 kB) | scalar/bg 兜底变体（vite 去重 scalar→bg） |
| `dist/assets/dict-*.bin` | 1,479.45 kB | @vane-rs/dict-zh 词典 asset |
| `sha256_prefix.bin` | — | 8 字节内联为 data URI（< 4KB assetsInlineLimit） |

- **wasm asset 路径**：`dist/assets/vane_wasm_simd-*.wasm` + `dist/assets/vane_wasm_bg-*.wasm`
- **worker chunk 路径**：`dist/assets/worker-*.js`

## 验证结果

1. ✅ `bindings/web/dist/` 存在（index.js / worker.js / vane_wasm_simd.wasm / vane_wasm_scalar.wasm / vane_wasm_bg.wasm）
2. ✅ `cd examples/vite && npm install` 成功（file: symlink @vane-rs/web → bindings/web/，@vane-rs/dict-zh → crates/vane-dict-zh/，16 packages）
3. ✅ `cd examples/vite && npm run build` 成功（7 模块转换，worker chunk + wasm asset + dict asset 产出，零报错，138ms）
4. ✅ git diff 确认未改 bindings/web/ / crates/vane-wasm/ / crates/vane-dict-zh/

## 文件清单

```
examples/vite/
├── .gitignore                    # node_modules/ + dist/
├── README.md                     # 前置(build-web.sh) + 运行 + 产出 + vite.config 说明
├── index.html                    # 挂载点 + <script type="module" src="/src/main.ts">
├── package.json                  # file: 引用 @vane-rs/web + @vane-rs/dict-zh + vite/typescript
├── package-lock.json             # npm install 产出（lockfileVersion 3）
├── tsconfig.json                 # bundler moduleResolution + ES2022 + DOM lib
├── vite.config.ts                # assetsInclude ['**/*.bin']（无 wasm/worker 插件）
└── src/
    ├── main.ts                   # createVane → open → collection(jieba) → add → flush → search 全链路
    └── vite-env.d.ts             # *.bin 模块声明 + vite/client 引用
```
