# M3 Task 9 Report — Web Integration 文档站集成指南页

## 状态

✅ 完成

## Commits

| sha | message |
|-----|---------|
| `4a349fd` | `feat(website): Web Integration 集成指南页（vite/webpack）——routes/nav 注册` |
| `16f0400` | `docs(examples): 修正 vite README 构建产物列表（bg 替代 scalar，sha256 标注内联）` |

## 测试摘要

`cd website && npm run build` 成功（176 modules, 15 routes +1, sitemap 含 `/vane/guides/web-integration`），`npx tsc --noEmit` exit 0；routes.ts/nav.ts 注册已 grep 确认；冻结文件未改（git diff 无 bindings/web / crates/vane-wasm / crates/vane-dict-zh / examples/vite/src+config / examples/webpack）。

## 新增/修改文件清单

### 新增
- `website/src/pages/guides/WebIntegration.tsx` — 集成指南页（DocsLayout 包裹，7 个 h2[id] TOC：install/vite/webpack/usage/dictdata/worker/gotchas）
- `website/src/pages/guides/WebIntegration.css` — BEM `webint-` 前缀，布局对齐 HybridSearch.css/Persistence.css

### 修改
- `website/src/routes.ts` — import WebIntegration + 注册 `{ path: '/guides/web-integration', name: 'Web Integration (vite/webpack)', Component: WebIntegration }`（插在 Persistence 后）
- `website/src/nav.ts` — Guides items 末尾加 `{ label: 'Web Integration', path: '/guides/web-integration' }`
- `examples/vite/README.md` — Task 7 review M1 修正：构建产物列表 `vane_wasm_scalar-*.wasm` → `vane_wasm_bg-*.wasm`；`sha256_prefix-*.bin` 标注 vite 内联（8 字节，低于 assetsInlineLimit 默认 4096，内联为 data URI 不单独产出）

## 页面内容

页面正文英文叙述（与现有 guides 一致），代码注释中文。覆盖 M3-PLAN 要求的全部 7 节：

1. **install** — `npm install @vane-rs/web @vane-rs/dict-zh`（optionalDep 自动装 dict-zh，零 CDN；exports map 三入口说明）
2. **vite** — vite.config.ts 仅 `assetsInclude: ['**/*.bin']`，强调无 wasm/worker 插件 + 配置项表
3. **webpack** — webpack.config.js（`experiments.outputModule` + `asset/resource` 规则 + `HtmlWebpackPlugin.scriptLoading: 'module'`），强调无 `asyncWebAssembly`（init(wasmUrl) 绕过）+ 配置项表
4. **usage** — import createVane + dict.bin → fetch Uint8Array → createVane({dictData}) → open → collection(jieba) → add → flush → search 全链路
5. **dictdata** — dictData 优先（transferable 零拷贝，零 CDN）+ dictSha256 校验 + dictUrl fallback（jsdelivr npm `@vane-rs/dict-zh@2026.8.0/dict.bin`）
6. **worker** — createVane 内部封装 `new Worker(new URL('./worker.js', import.meta.url), {type:'module'})`（引用 index.js 实际实现），用户无需手写
7. **gotchas** — 5 条 Callout：file:// 不行 / 无需 wasm/worker 插件 / webpack scriptLoading:'module' 必需 / dictData transferable 后主线程 buffer detached（用 .slice() 保留）/ SIMD/scalar 双变体运行时探针自动选

## 页面 URL

https://ximing.github.io/vane/guides/web-integration

（部署后；本地 `npm run preview` 可预览）

## Concerns

- **SPA 无预渲染**：website 是纯 SPA（BrowserRouter），`dist/` 只有 `index.html` + `404.html`，无 `dist/guides/web-integration.html` 独立文件。路由由客户端 404.html fallback 处理。sitemap.xml 已含该 path（验证通过），符合任务「build 产物 dist/guides/web-integration.html 存在 **或** sitemap 含该 path」的或条件。
- **chunk 体积 warning**：build 报 highlighter chunk 653KB > 500KB 警告——这是既有的 shiki 懒加载 chunk，与本次新增无关（pre-existing）。
- **task-1-design.md 预存改动**：工作树有 `docs/plans/m3/task-1-design.md` 的 license 备注修正（MIT→Apache-2.0，10:32 改），非本次 Task 9 产出，未纳入提交。
- **dictUrl CDN 版本**：fallback 示例用 `@vane-rs/dict-zh@2026.8.0`（与 bindings/web/dist/index.js `DEFAULT_DICT_URL` 一致）；若后续 dict-zh 发新版需同步更新该常量。
