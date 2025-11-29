# Vane 文档站实施任务清单

> 上游规划：`docs/plans/docs-site-plan.md`（评审修订后口径）。本文件是唯一执行依据；执行 agent 不需要重新规划内容与接口。
> 工作分支：`feat/docs-site`。站点根目录：`website/`（独立 npm 工程，不进 Cargo workspace）。
> 全局约束：Vite `base: '/vane/'`；React Router v7 `basename="/vane"`；TS strict 零错误、零 console warning；依赖白名单见 plan §5（不得新增 UI/动画/状态库）。
> **样式所有权约定**：每个组件/页面使用同名 colocated 样式文件（如 `CodeBlock.css`、`Home.css`，由对应 TSX import）；`global.css` 仅允许 T1（reset/基础元素）与 T2（token import）修改，后续任务禁止追加；禁止 CSS-in-JS、禁止新增全局选择器文件。

## 任务总览与依赖图

```
T1 脚手架 ──► T2 tokens ──► T3 theme/FOUC ─┐
      │            │                       ├──► T8 Home 静态 ──► T9 SearchDemo ─┐
      │            └──► T4 契约+基础组件 ──► T5 CodeBlock ──► T6 DocsLayout ──┤
      │                                                              │    ▲   │
T7 demo 数据（独立，可与 T1-T6 并行） ─────────────────────────────► T9    └───┘
                                                               T10 QuickStart │
                                                               T11-T14 Guides ├──► T18 收尾 ──► T19 部署 workflow ──► T20 README 链接
                                                               T15-T16 API    │
                                                               T17 Examples ──┘
```

- 可并行：T2/T3 与 T7；T4/T5/T6 在 T2 完成后彼此并行（T5 对 T4 是弱依赖：按 T4 任务内贴的契约编码，`contracts.ts` 由 T4 独占创建，T5 不得自建；T6 依赖 T3 的 theme API 与 T4 契约）；T10–T17 在 T4/T5/T6 完成后彼此并行（多人/多 agent 时按页分配）；T19 的 workflow 文件可与 T18 并行编写，但合并/启用放在最后。
- 关键路径：T1 → T2 → T5 → T6 → T11 → T18 → T19 → T20。

---

## T1 网站脚手架与路由骨架

- 复杂度：中 | 预估：2-3h
- 创建：`website/package.json`、`tsconfig.json`（strict）、`vite.config.ts`、`index.html`、`src/main.tsx`、`src/App.tsx`、`src/styles/global.css`（reset + 基础元素样式，此后冻结——见头部样式所有权约定）、`src/pages/` 下 14 个占位页组件（Home/QuickStart/4 Guides/7 API/Examples，各返回页名即可）。
- 要点：
  - `vite.config.ts`：`base: '/vane/'`、`build.outDir: 'dist'`、`@vitejs/plugin-react`。
  - `App.tsx`：`BrowserRouter basename="/vane"`，路由表集中写在一个 `routes` 数组（后续 sitemap 复用此数组——把路由数组抽到 `src/routes.ts` 导出）。
  - SPA 回退：`package.json` 的 `build` 脚本末尾追加 `cp dist/index.html dist/404.html`（用 node 一行脚本或 `node -e`，跨平台）。
  - 依赖仅：react@^19、react-dom@^19、react-router-dom@^7；dev：vite、@vitejs/plugin-react、typescript、@types/react(-dom)。锁 lockfile 并 commit。
- 验收：`cd website && npm ci && npm run build` 通过；`npx vite preview` 下 `/vane/`、`/vane/quickstart`、`/vane/api/errors` 均可访问；`dist/404.html` 存在；`tsc --noEmit` 零错误。

## T2 设计 tokens（tokens.css 完整变量清单，定死）

- 复杂度：低 | 预估：1-2h
- 创建：`src/styles/tokens.css`。
- 双套机制：`:root` 定义亮色全套，`[data-theme="dark"]` 覆盖为暗色（暗色为主调，参考 `demo/index.html` 已有写法）。**禁止**在此之外的发明变量；后续任务只消费。
- 必须包含的变量（命名即契约，值可微调但不得缺项）：
  - 颜色：`--bg`、`--bg-elevated`（卡片/代码窗口）、`--bg-inset`（终端/嵌套块）、`--fg`、`--fg-muted`、`--fg-faint`（角标/次要标签）、`--border`、`--accent`（暗色 `#3b9eff` 量级）、`--accent-hover`、`--accent-fg`（accent 上的文字色）、`--code-bg`、`--note-bg/--note-border`、`--warning-bg/--warning-border`、`--gap-bg/--gap-border`（known-gap 标注）、`--highlight`（搜索命中词高亮）。
  - 间距：`--space-1..--space-8`（4/8/12/16/24/32/48/64px）。
  - 字体：`--font-sans`（系统栈）、`--font-mono`（`"JetBrains Mono", "IBM Plex Mono", ui-monospace, monospace`）、`--text-sm/base/lg/xl/2xl/3xl`、行高 `--leading-tight/normal`。
  - 其他：`--radius-sm/md/lg`、`--content-max`（文档栏内容最大宽度）、`--sidebar-w`、`--toc-w`、`--topbar-h`。
  - 两套主题下 `--fg` vs `--bg`、`--fg-muted` vs `--bg` 对比度均须满足 WCAG AA（4.5:1）；用对比度公式实际核算并在此任务交付说明中贴出四个比值数值（最终由 T18 Lighthouse a11y ≥95 兜底）。
- 验收：tokens.css 被 global.css import；构建通过；用一个临时页面肉眼核对暗/亮两套渲染（截屏或描述即可，合入前删除临时页）。

## T3 明暗主题切换 + 防 FOUC + 自托管字体

- 复杂度：中 | 预估：2h | 依赖：T1、T2
- 创建/修改：`src/theme.ts`、`index.html`、`public/fonts/`（**钉死 JetBrains Mono**，只取 regular/bold 两个字重并子集化到 latin 集）、`tokens.css` 增补 `@font-face`。
- 要点：
  - `theme.ts`：导出 `initTheme()`（读 localStorage `vane-theme` → 无则 `prefers-color-scheme` → 设置 `<html data-theme>`）与 `toggleTheme()`（切换并持久化）；监听系统主题变化（仅在无手动偏好时跟随）。
  - `index.html <head>` 内联一段 `<script>`（不可外链、不可 defer），在首帧绘制前同步设置 `data-theme`，防 FOUC；同时 `<link rel="preload">` 两个 woff2。
  - 顶栏 toggle 按钮的接线留给 T8/T6 的布局任务，本任务只交付 `theme.ts` API 与一个最小可点按钮验证。
- 验收：暗色系统 + 无 localStorage 首屏直接暗色、无白闪；手动切换后刷新保持；`npm run build` 产物内字体为本地路径，无外部字体请求。

## T4 共享组件契约 + Callout + LangTabs

- 复杂度：中 | 预估：2-3h | 依赖：T1、T2
- 创建：`src/components/Callout.tsx` + `Callout.css`、`src/components/LangTabs.tsx` + `LangTabs.css`、`src/components/contracts.ts`（集中导出以下接口，后续所有页面任务只 import 不 redefine；**本文件由 T4 独占创建，其他任务不得自建**）。
- **组件契约（全体任务必须遵守，写进 `contracts.ts`）**：

```ts
export type Lang = 'node' | 'go' | 'browser';

export interface CodeBlockProps {
  code: string;
  lang: 'rust' | 'js' | 'ts' | 'go' | 'bash' | 'json';
  title?: string;             // 窗口框标题（文件名）
}

export interface LangTabsProps {
  node: React.ReactNode;
  go: React.ReactNode;
  browser: React.ReactNode;   // 三个 pane 内容；当前语言由全局偏好决定
}

export interface DocsLayoutProps {
  children: React.ReactNode;  // 页面内容；h2 必须手写 id，TOC 自动扫描
}

export interface CalloutProps {
  type: 'note' | 'warning' | 'gap'; // gap = known-gap 如实标注（filter 未透出等）
  title?: string;
  children: React.ReactNode;
}

// SearchDemo 数据契约（T7 产物必须严格符合）
export interface DemoHit { id: string; title: string; snippet: string; score: number; }
export interface DemoQuery { q: string; hybrid: DemoHit[]; vector: DemoHit[]; text: DemoHit[]; }
export interface DemoData {
  docs: Array<{ id: string; title: string; body: string }>; // ~30 条中英混合
  queries: DemoQuery[];                                       // ≥6 个预置 query
  provenance: 'vane-node' | 'manual'; // 数据来源：真实库生成 / 手写降级；T9 据此渲染标注文案
}
```

- LangTabs 行为契约：渲染三个 tab 按钮 + 当前 pane；选中语言写 `sessionStorage['vane-lang']` 并用自定义事件/`storage` 机制让**同页及跨页所有 LangTabs 实例同步**；默认 `node`。
- Callout：三种 type 对应 T2 的 `--note-*` / `--warning-*` / `--gap-*` 变量。
- 验收：写一个临时 story 页（合入前删）验证三 tab 切换、两个 LangTabs 实例联动、三种 Callout 渲染；TS 编译通过。

## T5 CodeBlock（shiki 懒加载独立 chunk）

- 复杂度：高 | 预估：3h | 依赖：T1、T2（T4 契约——弱依赖：按 T4 任务内贴的契约编码，`contracts.ts` 由 T4 独占创建，本任务 import 即可，不得自建）
- 创建：`src/components/CodeBlock.tsx` + `CodeBlock.css`、`src/lib/highlighter.ts`。
- 要点：
  - **先 `npm i shiki`（在规划 §5 白名单内），commit 更新后的 lockfile。**
  - `highlighter.ts`：`shiki/core` + `@shikijs/engine-javascript`（JS 正则引擎，禁 oniguruma WASM）+ 按需注册 6 lang（rust/js/ts/go/bash/json）+ 双主题单 pass（CSS variables 输出，theme 切换不重跑高亮）；整模块只在 `CodeBlock` 内 `import()` 动态加载，确保 Vite 拆成独立 lazy chunk。
  - CodeBlock UI：macOS 三圆点窗口框 + `title` + 语言角标 + 复制按钮（`navigator.clipboard`，复制成功短暂反馈）；shiki 未加载完成时渲染等宽纯文本兜底（同尺寸容器，无布局跳动/闪烁）。
  - 实现 `contracts.ts` 的 `CodeBlockProps`。
- 验收：`npm run build` 后 `dist/assets/` 中 shiki 相关代码在独立 chunk 且不被 `index-*.js` 静态引用；首屏入口 chunk gzip ≤ 150KB（用 `npm run build` 的 gzip 输出或 `source-map-explorer`/手工 `gzip -c` 核对）；`vite preview` 下代码块高亮正常、复制可用、暗亮切换高亮配色跟随。

## T6 DocsLayout（侧 nav + 自动 TOC + 顶栏）

- 复杂度：中 | 预估：2-3h | 依赖：T2、T3、T4
- 创建：`src/components/DocsLayout.tsx` + `DocsLayout.css`、`src/components/TopBar.tsx` + `TopBar.css`、`src/nav.ts`（侧栏导航静态配置，结构按 plan §2.2）。
- 要点：
  - 布局：顶栏（logo 词标等宽小写 "vane" / Docs(→/quickstart) / Guides(→/guides/hybrid-search) / API(→/api/overview) / Examples / GitHub SVG icon / 明暗 toggle（接 T3 `toggleTheme`））；文档页三栏：左 nav（`nav.ts` 渲染，当前项高亮）、中内容（`max-width: var(--content-max)`）、右 TOC。
  - TOC 机制（plan §5 定死）：内容区渲染后用 `useEffect` + `MutationObserver`（或挂载后单次扫描）收集内容容器内 `h2[id]`（可选 `h3[id]`）生成锚点列表；当前 section 用 IntersectionObserver 高亮。**页面作者只负责给 heading 写 id，TOC 零手维护。**
  - Home 不用 DocsLayout（全宽）；`/quickstart`、`/guides/*`、`/api/*`、`/examples` 全部套 DocsLayout（各页在 T10+ 任务中接入，本任务先用一个演示页验证）。
  - 移动端：≤900px 侧栏折叠为可展开抽屉，TOC 隐藏；不做花哨动画。
- 验收：演示页含 3+ 个 `h2[id]` 时右侧 TOC 自动生成、点击跳转、滚动高亮；移动端宽度下布局不横向滚动；toggle 生效。

## T7 首页 demo 预计算数据（主路径 + 降级路径）

- 复杂度：高 | 预估：2-3h（主路径）/ 1h（降级）| 依赖：无（与 T1-T6 并行）
- 创建：`website/scripts/gen-demo-data.mjs`、`src/data/demo-results.json`（commit 入库）。
- **主路径（优先尝试）**：脚本 import 本地已构建的 `crates/vane-node`（需先 `cd crates/vane-node && napi build`），造 ~30 条中英混合文档（内容可从 README/SPEC 摘句 + 自撰），用 4 维伪向量，对 ≥6 个预置 query（中英文都有）各跑 hybrid/vector/text 三种 mode，输出严格符合 `DemoData` 契约的 JSON（`contracts.ts`，见 T4）。snippet 字段截取含命中词的片段。
- **降级路径（主路径受阻时启用，优先级 P0 保底）**：不跑真实库，手写 `demo-results.json`（`provenance: 'manual'`）——30 条文档 + 6 个 query 的三列排序结果，排序差异要"讲得通"（vector 列语义相近、text 列字面命中、hybrid 居优）。**这是对规划的显式偏离：启用前须上报主会话确认，并在 `website/README.md` 记录此次偏离及原因。**
- 无论哪条路径：在 `website/README.md`（新建，几行即可）注明"数据如何重新生成、何时需要重跑"（plan §6.1 维护要求）。
- 验收：`src/data/demo-results.json` 存在且符合契约（写一个 `scripts/` 内或测试内的 JSON shape 校验，含 `provenance` 字段枚举值校验，`node` 跑通）；主路径成功时 `provenance: 'vane-node'`（页面标注文案由 T9 按该字段渲染，本任务不涉及页面）。

## T8 Home 页（静态区块）

- 复杂度：高 | 预估：3h | 依赖：T2、T3、T4、T5、T6
- 修改：`src/pages/Home.tsx` + 创建 `Home.css`（全宽，不套 DocsLayout，但含 TopBar + Footer）。
- **组件归属**：TopBar 由 T6 交付，本任务仅 import；新建 `src/components/Footer.tsx` + `Footer.css`（**仅 Home 使用**，其他页面不得加页脚）。
- 小节与内容来源：
  1. Hero：定位语（README L1 标题区 + §"What is Vane" 首段）+ 三平台 badge（Node/Go/Browser）+ "Get Started"(→/quickstart) / "GitHub" 双 CTA；右侧终端窗口卡片：等宽字体、纯 CSS/JS 打字动画循环播放 `npm install @vane-rs/node` → 代码 → hits 输出（静态假数据，非 SearchDemo）。
  2. 卖点四宫格 + 自绘 inline SVG 线框 icon（禁 emoji、禁图标库）：One core four runtimes / Hybrid by default / First-class Chinese / Embedded & durable（README "Features" 节 L73-95 提炼）。
  3. 三语言并排 quickstart 节选：用 LangTabs + CodeBlock，各 ~20 行，取自 README "Quick start" 三节（L175/214/261）删节。
  4. 对比表：README "What is Vane"（L51-71）的取舍表视觉化重排（对比 sqlite-vec+FTS5 / Orama / Tantivy），表格样式，禁渐变大色块、禁玻璃拟态。
  5. 架构图：README "Architecture"（L380-403）提炼为一张 inline SVG 分层图（core + 4 bindings）。
  6. 性能数字条：大号等宽数字 + 完整口径小字——HNSW P99 < 50ms 与 brute-force P99 < 150ms 并排（100k docs × 384d native，来源 README "Performance" L405-419 / SPEC §13.1）；recall@10 ≥ 0.95 (CI gate)；batch add ≥ 5,000 docs/s（含索引构建，同口径）；core wasm ≤ 800KB gzip（含 tokenizer、不含词典数据）。
  7. Won't-have 声明引言块：README "What is Vane"/"Status" 中 "Vane does not generate embeddings…" 原文。
  8. SearchDemo 挂载位（本任务先放空占位 div，T9 填入）。
  9. Footer：Apache-2.0、GitHub、SPEC/REQUIREMENTS 仓库链接。
- 验收：桌面/移动两档宽度渲染正常；无 emoji icon、无渐变 hero；性能数字均带口径；`npm run build` 通过。

## T9 SearchDemo 组件并接入 Home

- 复杂度：中 | 预估：2-3h | 依赖：T4、T7、T8
- 创建：`src/components/SearchDemo.tsx`；修改：`src/pages/Home.tsx`。
- 要点：搜索框 + 预置 query 建议 chips；输入时在 `DemoData.queries` 中精确/模糊匹配预置 query（无匹配时回退到对 `docs` 的简单 contains 重排并提示 "fallback ranking"），展示三列排序对比（hybrid / vector / text 并排卡片，列头标注 mode），命中词用 `--highlight` 高亮；区块角落按 `DemoData.provenance` 渲染标注——`'vane-node'` 时 "pre-computed on real Vane output"，`'manual'` 时仅 "pre-computed"。
- 验收：至少 6 个预置 query 均出三列结果；输入未收录 query 有兜底行为而非空白；移动端三列折叠为单列 tab 或纵向堆叠。

## T10 Quick Start 页

- 复杂度：中 | 预估：3h | 依赖：T4、T5、T6
- 修改：`src/pages/QuickStart.tsx`（套 DocsLayout）。
- 小节大纲（全页英文，下同所有内容页）：
  1. `h2#choose-your-runtime`：LangTabs 总控，整页步骤代码随 tab 切换（联动契约见 T4）。
  2. `h2#install` / `#open` / `#index` / `#search` 四步骨架（三语言统一）：
     - Node pane：README L99-116（npm install + 预编译平台包说明表精简）→ L175-213 完整 30 行示例（open→schema→add→flush→hybrid search）。
     - Go pane：README L117-140（构建/下载静态库 → go get → `LoadDict` 注意）→ L214-260 完整示例 → `vane_nodict` tag 提示；**wazero 一律按 README "Status" 节（L421-435）"stub/deferred" 口径写，不得照 Install 节写成可用替代**（用 Callout type=warning）。
     - Browser pane：README L141-154（build-wasm-variants.sh → 双变体）+ demo/README "前置条件/构建/启动"（L16-45）→ 词典 CDN/sha256/OPFS 缓存链路（demo/README "技术细节" L88+）。
  3. `h2#about-the-demo-vectors`：伪向量说明 Callout（"Vane 不内置 embedding，示例用 4 维 dummy 向量"）+ 接入真实 embedding 指引（OpenAI/ollama/transformers.js，指向仓库 `examples/`）。
- 验收：三 tab 内容完整可逐行对应源文件；与其他页 tab 联动一致；wazero 口径为 stub/deferred（grep 页面源码确认无 "pure-Go alternative available" 类表述）。

## T11 Guide：Hybrid Search（含 HybridPipeline SVG 动画）

- 复杂度：高 | 预估：3h | 依赖：T4、T5、T6
- 创建：`src/pages/guides/HybridSearch.tsx`、`src/components/HybridPipeline.tsx`。
- 小节大纲：
  1. `h2#two-recall-paths`：HNSW 召回路 / Block-Max WAND BM25 召回路（SPEC §8.1 + README "Architecture"）。
  2. `h2#rrf-fusion`：RRF 融合（k=60）图解（SPEC §8.2）；`h2#search-modes`：`mode` 三值语义表（README "Search modes & fusion" L336-348）；`h2#candidate-multiplier`：candidateMultiplier 语义；`h2#linear-fusion`：minmax 归一化 + "分数跨语料不可比" Callout(warning)。
  3. HybridPipeline：流水线 SVG——query 分两路（HNSW 节点点亮 / WAND posting 扫描）并行推进、汇入 RRF 输出 ranked list；CSS animation + IntersectionObserver 进入视口触发，无动画库；`prefers-reduced-motion` 时静态呈现。
- 验收：SVG 动画在视口内播放、离开视口不重播（或重播但不影响布局）；reduced-motion 下无动画；表格与 README/SPEC 数值一致（k=60）。

## T12 Guide：Tokenizers

- 复杂度：中 | 预估：2-3h | 依赖：T4、T5、T6
- 创建：`src/pages/guides/Tokenizers.tsx`。
- 小节大纲：
  1. `h2#built-in-tokenizers`：`standard`/`cjk_bigram`/`jieba` 对照表（README "Tokenizers" L315-335 + SPEC §5.1）。
  2. `h2#choosing`：选择决策树（新写，口径：英文→standard；中文为主→jieba；无词典兜底→cjk_bigram），用简单列表/流程式 HTML，不必 SVG。
  3. `h2#mixed-text`：中英混排统一规则图解（SPEC §5.1 管线描述，新写示例 token 序列）。
  4. `h2#dict-distribution`：词典在三平台的分发差异表——Node 自动 / Go `LoadDict` / Browser CDN+sha256+降级（SPEC §12.3 + demo/README 技术细节）。
  5. `h2#custom-dict`：userDict 注入示例（README "Custom dictionary & reindex" L349-368 节选，CodeBlock 双语言即可，详细状态机指向 /guides/reindex）。
- 验收：对照表与 SPEC §5.1 一致；分发差异表三平台口径与 SPEC §12.3 一致。

## T13 Guide：Custom Dict & Reindex

- 复杂度：中 | 预估：2h | 依赖：T4、T5、T6
- 创建：`src/pages/guides/Reindex.tsx`。
- 小节大纲：
  1. `h2#state-machine`：Stable → PendingReindex → Rebuilding → Stable 状态图（SPEC §7.4，inline SVG 或表格式状态转移图）。
  2. `h2#set-user-dict`：`setUserDict` 暂存语义（不立即生效）；`h2#needs-reindex`：响应标志语义（SPEC §7.4 + README L349-368）。
  3. `h2#best-practices`：最佳实践——建库时传 userDict，避免 reindex（README 同节）。
- 验收：状态图四个状态及转移条件与 SPEC §7.4 逐条对应。

## T14 Guide：Persistence & Visibility

- 复杂度：中 | 预估：2-3h | 依赖：T4、T5、T6
- 创建：`src/pages/guides/Persistence.tsx`。
- 小节大纲：
  1. `h2#directory-layout`：manifest/segments/wal.log 目录布局（SPEC §6.2）。
  2. `h2#flush-visibility`：flush 是原子可见性边界——"add 返回 ≠ 可搜；flush 后原子可见"时序图（新写，纯 HTML/CSS 时序条即可）（SPEC §7.1）。
  3. `h2#auto-commit`：auto-commit 默认口径（1s/1000 docs，README/SPEC §4.2 OpenOptions）。
  4. `h2#tombstones-compact`：tombstone + compact（SPEC §7.2/§7.3）；`h2#export`：export() 单文件快照（README API 节）；`h2#crash-recovery`：崩溃恢复（SPEC §6.4）。
- 验收：时序图正确表达三阶段；数值口径（1s/1000 docs）与 SPEC 一致。

## T15 API Reference：Overview / open / collection

- 复杂度：中 | 预估：3h | 依赖：T4、T5、T6
- 创建：`src/pages/api/Overview.tsx`、`Open.tsx`、`Collection.tsx`。
- 通用页面骨架（七页统一，本任务先立模板）：页面顶部一段 IDL 签名块（SPEC §4.1/§4.2）→ LangTabs 三语言签名 + 最小调用片段（SPEC §4.3 映射表）→ 参数表 → 错误处理差异 → 相关错误码链接到 /api/errors。
- Overview 小节：`h2#verb-table` 六动词 + 四管理函数总表（IDL / JS / Go 三列，SPEC §4.1 + §4.3）；`h2#error-style` 错误风格说明（异常 vs 返回码，SPEC §9.1/§9.3 + §10 引言）。
- open 小节：`h2#signature`、`h2#open-options`（persistence / autoCommit / pageCacheMb 全字段表，SPEC §4.2）。
- collection 小节：`h2#schema` Schema 字段类型完整约束（恰一个 vector 字段、dim≤4096、metric 三值、scalar kind 四值，SPEC §3.1）；`h2#collection-options`（README "Schema & documents" L295-314）。
- 验收：三页均含 LangTabs 且三 pane 非空；约束数值与 SPEC §3.1 逐条一致。

## T16 API Reference：documents / search / maintenance / errors

- 复杂度：高 | 预估：3h | 依赖：T4、T5、T6、T15（沿用其页面骨架）
- 创建：`src/pages/api/Documents.tsx`、`Search.tsx`、`Maintenance.tsx`、`Errors.tsx`。
- documents 小节：`h2#document` Document 结构 + 幂等 upsert by id（SPEC §3.2）；`h2#add` AddReport；`h2#flush` 可见性边界（链 /guides/persistence）；`h2#delete`。
- search 小节：`h2#search-query` SearchQuery 全字段表；`h2#hits` Hit 结构；`h2#modes` mode/candidateMultiplier 摘要（链 /guides/hybrid-search）；`h2#filter` **必须用 Callout(type=gap) 标注"core 已实现，绑定层未透出（known gap，v0.1.x）"**（SPEC §8.3 + README "Filtering" L370-379）。
- maintenance 小节：`h2#compact` / `h2#reindex`（ReindexHandle.progress/wait）/ `h2#export` / `h2#close`（SPEC §4.1 + README API 节 L276-294）。
- errors 小节：`h2#error-codes` 错误码表 -1 到 -11 完整搬运（SPEC §10）；`h2#wasm-note` WASM 侧 E_DICT_UNAVAILABLE 不可达的说明；`h2#handling` 三语言错误处理差异片段。
- 验收：错误码表 11 行与 SPEC §10 一一对应；filter known-gap 标注存在（grep 页面源码 "known gap" 命中）。

## T17 Examples 页

- 复杂度：低 | 预估：1-2h | 依赖：T4、T5、T6
- 修改：`src/pages/Examples.tsx`。
- 三张卡片（做了什么 / 怎么跑【真实前置条件】/ 仓库链接）：
  1. Node 三列排序对比 demo（`examples/demo/`）：按 `examples/demo/README.md` L21-68 照抄运行步骤，写明"通过 file:link 依赖本地 napi build 产物，需先在 crates/vane-node 构建"，**不得写成一键运行**；附代码量对比表（L69-117 精简）。
  2. Browser 纯前端 Markdown 搜索（`demo/`）：截图（从 demo 页面截取，存 `public/screenshots/`）+ 特性清单（demo/README "功能" L7-14）+ 运行步骤（L22-45）。
  3. Go example（`bindings/go/example/`）：需先摆好 `libvane_ffi.a` 前置说明。
  4. 页尾：伪向量提示 Callout（复用 T10 口径）。
- 验收：三张卡片前置条件与各自 README 原文一致；截图 < 200KB（压缩）。

## T18 收尾：favicon / OG / robots / sitemap / 404 / Lighthouse

- 复杂度：中 | 预估：2-3h | 依赖：T8-T17 全部
- 创建/修改：`public/favicon.svg`、`public/og-image.png`（1200×630 静态图，自制简单品牌图：等宽 "vane" 词标 + 定位语，禁截图充数）、`public/robots.txt`、`website/scripts/gen-sitemap.mjs`（从 `src/routes.ts` 的路由数组生成 `dist/sitemap.xml`，接入 build 脚本）、`index.html`（OG meta 全套：og:title/description/type/url/image、twitter:card）。
- 同时验证项：深链接刷新（preview 下直接打开 `/vane/guides/reindex` 正常渲染，dist 404.html 生效）；无 console warning；暗亮对比度 AA。
- Lighthouse（首页 mobile）本地跑分：Performance ≥ 90 / A11y ≥ 95 / BP ≥ 95 / SEO ≥ 90；不达标时只做轻量调优（图片尺寸、字体 display=swap、减少首屏 JS），**不得为提分改设计**。
- 验收：build 产物含 favicon/og-image/robots.txt/sitemap.xml 且 preview 下 URL 直接可访问；sitemap 含全部 14 条路由且 host 为 `https://ximing.github.io/vane`；OG meta 中 `og:url` 钉死 `https://ximing.github.io/vane/`、`og:image` 钉死 `https://ximing.github.io/vane/og-image.png`；Lighthouse 四项达标——结果口头报数即可，不落盘报告；若需留证，截图/JSON 放 `website/.lighthouse/` 并 gitignore。

## T19 GitHub Actions 部署 workflow + ci.yml paths-ignore

- 复杂度：中 | 预估：1-2h | 依赖：T18（仅合入时机；workflow 文件可与 T18 并行编写）
- 创建：`.github/workflows/deploy-docs.yml`；修改：`.github/workflows/ci.yml`。
- deploy-docs.yml 口径（plan §10.8 定死）：触发 `push` 到 `main` 且 `paths: ['website/**']` + `workflow_dispatch`；`permissions: { pages: write, id-token: write }`；`environment: github-pages`；`concurrency: { group: pages }`；步骤：setup-node（带 website/ 的 npm cache）→ `npm ci`（working-directory: website）→ `npm run build` → `actions/upload-pages-artifact`（path `website/dist`）→ `actions/deploy-pages`。
- ci.yml：所有触发器加 `paths-ignore: ['website/**']`（注意不要破坏现有 16 jobs 矩阵）。
- **不在本任务内**：repo Settings → Pages → Source = "GitHub Actions"（主会话用 `gh api` 处理）；首次部署验证由主会话做。
- 验收：`actionlint`（或 `gh workflow view`）语法通过；ci.yml diff 仅 paths-ignore；workflow 文件中无超出上述口径的额外步骤。

## T20 README 双语言加文档站链接

- 复杂度：低 | 预估：0.5-1h | 依赖：T19 合并且站点实际可访问（主会话确认后执行）
- 修改：`README.md`、`README.zh-CN.md` 顶部 badge 区各加一行 Docs 站链接 `https://ximing.github.io/vane/`（英文版 "Docs" / 中文版"文档站"，风格与现有 badge 一致）。
- 验收：两个文件链接可点击、URL 200；不改动其他内容。

---

## 全局验收回勾（全部任务完成后主会话核对）

- [ ] plan §2.1 全部 14 页存在；每个 API 页有三语言 tab；每个代码块可复制。
- [ ] 全站 ≥3 处如实标注：filter known gap（T16）、伪向量（T9/T10/T17）、性能口径（T8）。
- [ ] 首屏 JS gzip ≤ 150KB；shiki 在 lazy chunk（T5）。
- [ ] 明暗无 FOUC（T3）；tab 联动 ≥3 页验证（T10 + 任两页）。
- [ ] 404 回退、robots、sitemap、og:image、README 链接、ci.yml paths-ignore、deploy workflow 全部落地（T18/T19/T20）。
