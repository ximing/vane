# 文档站指令（`website/`）

## 职责

- 此目录是 Vane 文档站：React 19 + Vite + TypeScript 单页应用，部署到 `ximing.github.io/vane`。
- `docs/REQUIREMENTS.md` 与 `docs/SPEC.md` 是需求和技术规范的唯一合同；本目录是面向使用者的呈现层，不要把规范正文复制进来形成第二真相源——引用或精炼即可。

## 放置约束

- 页面放 `src/pages/`，组件放 `src/components/`，路由集中在 `routes.ts`/`nav.ts`；不要在页面目录堆放与路由无关的工具代码。
- `src/data/demo-results.json` 是首页 demo 数据，由真实 `vane-node` 本地构建经 `scripts/gen-demo-data.mjs` 生成（`provenance: 'vane-node'`）。不要手改该 JSON——改文档语料、预置 query 或 `DemoData` 契约（`src/components/contracts.ts`）时重跑生成器。
- vane-core 检索/排序行为升级导致排序漂移时，必须重跑 `gen-demo-data.mjs` 更新 demo 数据，不要保留过期排序结果。

## 约束

- 构建产物 `dist/` 不入库；`npm run build` 会把 `dist/index.html` 复制为 `dist/404.html` 并生成 sitemap，改动构建流程时保持这两步。
- 新增依赖需符合文档站体积预期；`shiki`、`react-router-dom` 等已有依赖的版本升级需确认不破坏代码高亮与路由。

## 验证

- 本地预览：`npm run dev`；构建：`npm run build`（含 404 与 sitemap）。
- 改 demo 数据后运行 `node scripts/gen-demo-data.mjs --check` 校验 shape。
