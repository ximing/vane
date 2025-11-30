# Vane 文档站规划

> 规划产出：docs-site（Vite + React + TS，GitHub Pages 项目页）。由规划 SubAgent 完成，主会话落盘。

## 0. 前提与约束

- 站点目录：仓库根 `website/`，独立 npm 工程（不进入 Cargo workspace）。
- 技术栈：Vite（latest stable，锁 lockfile）+ React 19 + TypeScript（strict）。
- 部署：GitHub Pages 项目页，`https://ximing.github.io/vane/`。
  - **Vite `base: '/vane/'`（硬要求）**，路由同理需要 basename。
  - 构建产物：`website/dist/`。GitHub Actions 中 `actions/upload-pages-artifact` 指向该目录。
  - SPA 需要 `404.html` 回退（构建后把 `index.html` 复制为 `404.html`），否则深链接刷新 404。
- 站名 "Vane"，定位语沿用 README："One Rust core, four runtimes" / "sqlite-vec 的嵌入式形态 + Tantivy 级 BM25 + 一体化 RRF 混合排序"。
- 版本状态锚点：v0.1.x，`filter` 未透出到绑定层——文档站必须如实标注，不夸大。

## 1. 站点定位与受众

**定位**：开源库的"产品页 + 文档"合一站点。让一个应用开发者（非 infra 工程师）在 5 分钟内判断"Vane 适不适合我"，并在 15 分钟内跑通第一个 hybrid search。

| 受众 | 入口路径 | 看完能做 |
|---|---|---|
| 评估者（选型嵌入式检索/RAG 检索层） | Home → Why Vane（对比表） | 判断 Vane 与 sqlite-vec+FTS5 / Orama / Tantivy 的取舍，知道 Won't-have（无内置 embedding、无服务端） |
| 上手开发者（Node/Go/Browser 之一） | Quick Start → 对应语言页 | 复制 install + 30 行 quickstart 跑通 add/flush/search，知道 tokenizer 怎么选 |
| 深度使用者 | Guides / API Reference | 正确理解 flush 可见性、setUserDict/reindex 状态机、RRF vs linear 的适用边界、错误码语义 |

受众以英文为主（README 主体为英文）。**v1 站点只做英文，中文作为后续 i18n 候选。**

## 2. 信息架构

### 2.1 页面清单与内容大纲

**Home `/`**
- Hero：一句话定位 + 三平台 badge（Node/Go/Browser）+ "Get Started" / "GitHub" 双 CTA
- Live demo 区块（交互亮点 #1，见 §6）
- 卖点四宫格：One core four runtimes / Hybrid by default / First-class Chinese / Embedded & durable
- 三语言并排代码示例（Node/Go/Browser tab 切换，各 ~20 行 quickstart 节选）
- 对比表（README "What is Vane" 的三行取舍表，视觉化重排）
- 性能数字条（P99 <50ms、≥5k docs/s、recall@10 ≥0.95 硬门禁、wasm ≤800KB gzip）
- Won't-have 声明（"Vane does not generate embeddings…" 引言块）
- Footer：License Apache-2.0、GitHub、SPEC/REQUIREMENTS 链接

**Quick Start `/quickstart`**
- 三语言 tab（Node.js / Go / Browser），统一 4 步骨架：Install → Open → Index → Search
  - Node：npm install → 预编译平台包说明表 → 30 行完整示例（open→schema→add→flush→hybrid search）
  - Go：构建/下载静态库 → go get → LoadDict 注意事项 → 完整示例 → `vane_nodict` tag 提示；**wazero 一律按"stub/deferred"口径写**（README Status 节原文如此，bindings/go/wazero 目前只是 stub），不得照 README Install 节写成可用替代方案
  - Browser：build-wasm-variants.sh → 双变体说明 → demo 运行指引 → 词典 CDN/sha256/OPFS 缓存链路
- 共用尾节：伪向量说明（"Vane 不内置 embedding，示例用 4 维 dummy 向量"）+ 接入真实 embedding 指引（OpenAI/ollama/transformers.js + 指向 examples/）

**Guides `/guides/*`**（4 篇，全部新写）
- `/guides/hybrid-search`：HNSW 召回路 / Block-Max WAND BM25 召回路 / RRF 融合（k=60）图解；`mode` 三值语义表；`candidateMultiplier`；linear fusion 的 minmax 归一化与"分数跨语料不可比"警告
- `/guides/tokenizers`：`standard` / `cjk_bigram` / `jieba` 对照表；中英混排统一规则图解；词典在三平台的分发差异（Node 自动 / Go LoadDict / Browser CDN+降级）；userDict 注入示例
- `/guides/reindex`：Stable → PendingReindex → Rebuilding → Stable 状态图（SPEC §7.4）；`setUserDict` 暂存语义；`needsReindex` 响应标志；最佳实践（建库时传 userDict）
- `/guides/persistence`：目录布局（manifest/segments/wal.log）；flush 是原子可见性边界；auto-commit 默认（1s/1000 docs）；tombstone + compact；export() 单文件快照；崩溃恢复

**API Reference `/api/*`**
- 组织方式：**按"语言无关 IDL"为主线，每个 API 页内嵌语言 tab（Node/Go/WASM）**——SPEC §4 明确"三侧绑定共用 IDL，binding 是薄壳"。
- `/api/overview`：六动词 + 四管理函数总表（IDL / JS / Go 三列）+ 错误风格说明
- `/api/open`：open(path, OpenOptions)——persistence / autoCommit / pageCacheMb
- `/api/collection`：db.collection + Schema 字段类型完整约束（恰一个 vector 字段、dim≤4096、metric 三值、scalar kind 四值）+ CollectionOptions
- `/api/documents`：add / flush / delete——Document 结构、幂等 upsert by id、AddReport、可见性边界
- `/api/search`：search(query) → Hit[]——SearchQuery 全字段、Hit 结构；**filter 小节标注"core 已实现，绑定层未透出（known gap）"**
- `/api/maintenance`：compact / reindex（ReindexHandle.progress/wait）/ export / close
- `/api/errors`：错误码表（-1 到 -11，含 WASM 侧 E_DICT_UNAVAILABLE 不可达的说明）

**Examples `/examples`**
- 三张卡片：Node 三列排序对比 demo（`examples/demo/`）、Browser 纯前端 Markdown 搜索（`demo/`，截图 + 特性清单）、Go example（`bindings/go/example/`）
- 每张卡片：做了什么 / 怎么跑（**写真实前置条件**：`examples/demo/` 通过 file:link 依赖本地 `napi build` 产物，需先在 crates/vane-node 构建；Go example 需先摆好 `libvane_ffi.a`。照抄各示例 README 的运行步骤，不写成"一键运行"）/ 仓库链接
- 共用伪向量提示组件

**Why Vane**：并入 Home（不单列页面），包含对比表 + 架构图 + Won't-have 清单。

### 2.2 导航结构

```
顶栏：Vane logo | Docs(=Quick Start)  Guides  API  Examples  | GitHub icon  明暗切换
侧栏（DocsLayout 共用）：
  Getting Started
    Quick Start
  Guides
    Hybrid Search
    Tokenizers
    Custom Dict & Reindex
    Persistence & Visibility
  API Reference
    Overview / open / collection / documents / search / maintenance / errors
  Examples
```

Home 全宽无侧栏；`/quickstart`、`/guides/*`、`/api/*`、`/examples` 共用 DocsLayout（左 nav + 右 TOC + 中间内容）。

## 3. 首页设计方向

**基调**：工程感、克制、信息密度高。"基础设施项目"气质（参考 Tantivy / SQLite / turborepo docs），而非 SaaS 营销页。

- **配色**：深色优先。无 localStorage 偏好时跟系统（prefers-color-scheme），设计以 dark 为主调；手动切换后持久化 localStorage。单一强调色青蓝系（`#3b9eff` / `#22d3ee`），近黑背景（`#0d1117` 量级）+ 中性灰阶；亮色同强调色 + 白底。**禁止渐变大色块、禁止玻璃拟态**。
- **字体**：正文系统栈；**代码与品牌字用等宽**（JetBrains Mono / IBM Plex Mono，自托管 woff2）。Logo 词标 "vane" 用等宽小写。
- **Hero**：左文右"终端"——右侧等宽字体终端窗口卡片，打字动画播放 `npm install @vane-rs/node` → 代码 → hits 输出（纯 CSS/JS 展示）。
- **卖点区块**：四宫格 + 自绘 inline SVG 线框 icon，不用 emoji。
- **代码示例**：统一 `CodeBlock` 组件，macOS 三圆点窗口框 + 语言角标 + 复制按钮；三语言 tab 全局联动（sessionStorage）。
- **性能数字条**：大号等宽数字 + 小标签，带完整口径：HNSW 路径 P99 < 50ms 与 brute-force P99 < 150ms 并排（100k docs × 384d native）、"recall@10 ≥ 0.95 (CI gate)"、"core wasm ≤ 800KB gzip（含 tokenizer、不含词典数据）"。强调硬门禁属性。

## 4. 内容策略

**直接提炼（改排不改写）**：
- README：install/quickstart/API 对照表/schema/tokenizer 表/search modes 表/架构/性能/状态/Won't-have。
- SPEC §4（IDL 签名）、§10（错误码）、§7.4（reindex 状态机）、§5（分词器管线）——剥离里程碑标注与 changelog，只写当前行为。
- demo/README：Browser quickstart 的 Worker 协议、词典三渠道链路、SIMD 探针。
- examples/demo/README：三列排序对比输出、代码量对比表。

**需要新写**：
1. 三语言 install/quickstart 步骤化对照（统一 4 步骨架）。
2. Tokenizer 选择决策树（英文 → standard；中文为主 → jieba；无词典兜底 → cjk_bigram）+ 中英混排切分图解。
3. Hybrid 检索原理流水线 SVG（query 分两路 → 各取 topK×3 → RRF 合并）。
4. flush 可见性时序图（add 返回 ≠ 可搜；flush 后原子可见；auto-commit 兜底）。
5. API Reference 每页三语言 tab 内容（同一 IDL 的 JS/Go/WASM 签名 + 最小调用片段 + 错误处理差异）。

**口径纪律**：性能数字一律标注口径；filter 一律标注 known gap；伪向量说明组件化复用。

## 5. 技术方案

| 项 | 选型 | 理由 |
|---|---|---|
| 路由 | react-router **v7** BrowserRouter + 404.html 回退，basename `/vane` | GitHub Pages 无服务端重定向；v6 已进维护期且有 future-flag console warning，与"零 console warning"验收冲突，故直接 v7 |
| 样式 | 手写 CSS（custom properties + 少量 CSS Modules），`:root` / `[data-theme="dark"]` 双套 token | 页面型站点、组件少；不引入 Tailwind |
| 代码高亮 | Shiki，**懒加载独立 chunk**：`shiki/core` + 按需注册 6 个 lang（rust/js/ts/go/bash/json）+ JS 正则引擎（`@shikijs/engine-javascript`，避免 oniguruma WASM）+ 单 pass 双主题 CSS variables 输出；`CodeBlock` 内 dynamic import，未加载完成时渲染纯文本兜底（无闪烁错位） | 内容全为 TSX → 代码字符串运行期才存在，shiki 必然在浏览器跑；通过拆 chunk 保证它不进首屏 bundle。注意 ts/js 语法包 gzip 后可能 100KB+，必须隔离在首屏外 |
| 内容形式 | **v1 全部内容写为 TSX 页面组件**，不引入 markdown 管线、不用 MDX | 类型安全，代码块直接复用 CodeBlock |
| 明暗模式 | `prefers-color-scheme` 初始 + 手动 toggle，`data-theme` 挂 `<html>`，localStorage 持久化；`index.html` `<head>` inline 防 FOUC 脚本 | 无库 |
| 图标 | inline SVG 自绘 + GitHub logo | 无图标库 |
| Vite | `base: '/vane/'`，`build.outDir: 'dist'`，字体自托管 `public/fonts/` | Actions 上传 `website/dist` |
| 依赖上限 | react, react-dom, react-router-dom(v7), shiki（含 engine-javascript）；dev: vite, @vitejs/plugin-react, typescript | 无 UI 库、动画库、状态库 |

**TOC 生成机制**：全 TSX 无 markdown 管线，不自动提取 heading。规定：`DocsLayout` 在内容区渲染完成后扫描内容容器内 `h2[id]`（必要时 `h3[id]`）自动生成右侧 TOC——页面作者只需给 heading 写 id，TOC 不手维护、不漂移。

**OG/sitemap**：全站共享 `index.html` 一套 OG meta（含静态 `og:image`，放 `public/`）；`public/robots.txt` + 构建期生成的静态 `sitemap.xml`（路由清单手写在一个数组里，构建脚本输出）。已知取舍：SPA 共享一套 OG meta，不承诺 per-page；404.html 回退下深链接 HTTP 状态码仍为 404，仅渲染正常，SEO 影响 v1 接受。

## 6. 交互亮点

1. **首页 Live Hybrid Search Demo（预计算真实数据，v1 不跑 WASM）**：搜索框 + 预置 ~30 条中英混合文档，输入 query 展示三列排序对比（hybrid / vector / text 并排，灵感来自 `examples/demo` compare 输出），命中词高亮。数据由 `website/scripts/gen-demo-data.mjs` 生成——**钉死运行时机：dev-time 手动运行（需要本地已构建的 vane-node），产物 JSON commit 入库；CI/部署只构建网站、绝不重新生成**；文档或库版本升级时手动重跑并在 plan/README 中注明此维护步骤。页面标注"pre-computed on real Vane output"；真 WASM 运行版为后续增强。
2. **检索流程滚动动画（纯展示）**：Guides/hybrid-search 页流水线 SVG——query 两路（HNSW 节点点亮 / WAND posting 扫描）并行推进，汇入 RRF 输出 ranked list。CSS animation + IntersectionObserver，无动画库。
3. **三语言代码 tab 全局联动**：语言偏好存 sessionStorage，全站 tab 同步；Quick Start 页"步骤条 + 每步代码随 tab 变化"。

真运行 demo 以外链形式指向仓库 `demo/`（后续增强：把 demo 构建产物部署到 `/vane/demo/`）。

## 7. 目录结构

```
website/
├── package.json
├── tsconfig.json
├── vite.config.ts            # base: '/vane/'
├── index.html                # 防 FOUC theme 脚本 + 字体 preload
├── public/
│   ├── fonts/                # 子集化 woff2
│   └── favicon.svg
├── scripts/
│   └── gen-demo-data.mjs     # 构建期跑真实 vane-node 生成首页 demo 预计算 JSON
└── src/
    ├── main.tsx
    ├── App.tsx               # router + basename
    ├── theme.ts              # 明暗模式
    ├── styles/
    │   ├── tokens.css        # 双主题 custom properties
    │   └── global.css
    ├── components/
    │   ├── CodeBlock.tsx     # shiki + 复制 + 窗口框
    │   ├── LangTabs.tsx      # Node/Go/Browser tab（全局联动）
    │   ├── DocsLayout.tsx    # 侧 nav + 右 TOC
    │   ├── SearchDemo.tsx    # 交互亮点 #1
    │   ├── HybridPipeline.tsx# 交互亮点 #2（SVG 动画）
    │   └── Callout.tsx       # note/warning
    ├── data/
    │   └── demo-results.json # 预计算三列排序数据
    └── pages/
        ├── Home.tsx
        ├── QuickStart.tsx
        ├── guides/{HybridSearch,Tokenizers,Reindex,Persistence}.tsx
        ├── api/{Overview,Open,Collection,Documents,Search,Maintenance,Errors}.tsx
        └── Examples.tsx
```

## 8. 明确不做的事（v1 范围控制）

- 不做博客 / changelog 页（指向 GitHub Releases）。
- 不做站内全文搜索（后续可评估 pagefind）。
- 不做 i18n（v1 仅英文）。
- 不做首页真 WASM 运行 demo（后续增强）。
- 不做版本化文档（v0.1.x 单版本）。
- 不引入 UI 组件库、动画库、MDX、Tailwind。
- 不做评论区、无第三方统计脚本。

## 9. 验收标准

**构建与部署**
- `npm run build` 通过，产物在 `website/dist/`；`vite preview` 下以 `/vane/` base 访问所有路由正常。
- 深链接刷新不 404（404.html 回退生效）。
- TypeScript strict 零错误；无 console warning。

**页面完整性**
- §2.1 列出的页面全部存在：Home / Quick Start / 4 篇 Guides / 7 个 API 页 / Examples。
- 每个 API 页有三语言 tab；每个代码块可一键复制。
- 全站至少三处如实标注 known gap / 口径（filter 未透出、伪向量说明、性能口径）。

**质量**
- Lighthouse（首页，mobile）：Performance ≥ 90，Accessibility ≥ 95，Best Practices ≥ 95，SEO ≥ 90。
- 首屏 JS（gzip）≤ 150KB；shiki 及其语法包隔离在首屏外的 lazy chunk。
- 明暗模式无 FOUC、文本对比度符合 WCAG AA。
- 三语言 tab 联动在至少 3 个页面间验证一致。
- `robots.txt`、`sitemap.xml`、`og:image` 均随构建产出，可通过 URL 直接访问。
- README 双语言版本均含文档站链接。

## 10. 实施顺序建议

1. 脚手架：vite + react + ts + router + base/404 回退 + theme + tokens.css + CodeBlock（shiki）。
2. DocsLayout + 侧 nav/TOC + LangTabs。
3. Home（静态卖点区块与终端动画；SearchDemo 先静态后接数据）。
4. Quick Start 三语言内容。
5. Guides 四篇（含 HybridPipeline SVG）。
6. API Reference 七页。
7. Examples + 收尾（favicon、OG meta 与 og:image 静态资源、robots.txt + sitemap.xml、Lighthouse 调优）。
8. GitHub Actions 部署（新 workflow，不与现有 4 个 workflow 冲突）：
   - 触发：`push` 到 `main` 且 `paths: ['website/**']`（+ `workflow_dispatch` 手动兜底）
   - 权限：`permissions: { pages: write, id-token: write }`，`environment: github-pages`，`concurrency: { group: pages }`
   - 步骤：setup-node → `npm ci`（website/）→ `npm run build` → `actions/upload-pages-artifact`（path: `website/dist`）→ `actions/deploy-pages`
   - 一次性 repo 设置：Settings → Pages → Source 选 "GitHub Actions"（用 `gh api` 设置）
   - **同时给 ci.yml 加 `paths-ignore: ['website/**']`**，避免文档站改动触发全套 16 jobs
9. README.md 与 README.zh-CN.md 顶部加文档站链接/badge（站点上线后主入口仍是 README）。

---

### 关键实现参考文件

- README.md — 站点 80% 内容的源
- docs/SPEC.md — API Reference 权威来源（§4 IDL、§7.4 状态机、§10 错误码）
- demo/README.md + demo/main.js — Browser quickstart 与首页 demo 素材
- examples/demo/README.md — 三列排序对比与代码量对比
- demo/index.html — 已有明暗双主题 token 写法可参考
