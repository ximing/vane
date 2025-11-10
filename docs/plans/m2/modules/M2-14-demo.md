# M2-14 Demo（纯前端 markdown 搜索）

## 1. 目标
纯前端页面：拖入 markdown 文件夹，本地混合搜索（含中文），无后端。展示浏览器交付闭环（SPEC §15 M2 Demo，REQUIREMENTS §7 M2 Demo）。

SPEC 节号：§15（M2 Demo：纯前端拖入 markdown 文件夹本地混合搜索含中文）。

## 2. 涉及文件
- **Create** `demo/` 或 `examples/browser-markdown/`：前端 demo 工程。
  - `index.html`：拖入区 + 搜索框 + 结果列表。
  - `main.js`：主页面 JS，加载 Worker（M2-04）+ 拖入处理 + 调 Worker API。
  - `worker.js`：Worker 入口（M2-04 产出）。
  - `vane_wasm_simd.wasm` / `vane_wasm_scalar.wasm`（M2-05 双产物，按探针加载）。
  - 词典 CDN fetch（M2-04 dict_loader）或内联 dictData。
- **Create** `demo/README.md`：使用说明 + 截图。
- **Consumes** M2-04 `VaneWorker` API + M2-05 双产物 + M2-02 OPFS Vfs。

## 3. 接口契约
### Consumes from
- M2-04 `VaneWorker`（`new VaneWorker(opts)` + open/collection/add/flush/search/export/close）。
- M2-05 双产物（simd/scalar，init 探针选）。
- M2-02 OPFS Vfs（持久化）。
- 词典 CDN fetch 或内联（M2-04 dict_loader）。

### Produces for
- 浏览器交付闭环演示（SPEC §15 M2 Demo）。
- 截图/录屏用于发版说明。

## 4. TDD 测试清单（行为测试，reviewer B-M8：明确 e2e 脚本自动化）

> Demo 是前端页面，TDD 用 e2e 脚本（Playwright/headless browser）自动化，非 unit test。10 个测试均编写为可重复执行的 e2e 用例（`demo/e2e/` 目录，`node run-e2e.mjs`）；无法自动化的步骤降级为人工验收清单（标注「人工验收」）。
1. **拖入文件夹**：拖入含 100+ markdown 文件的目录 → 解析为 `{id, text, vector?}` 文档（vector 用占位或简单 hash 向量，demo 不接 embedding 模型——SPEC Won't-have）。
2. **建库**：`VaneWorker.open("demo_db")` → `collection("docs", {dim:.., tokenizer:"jieba"})` → `add(docs)` → `flush`。
3. **中文搜索**：搜索框输入中文查询（如"人工智能"）→ `search({text:"人工智能", topK:10, mode:"hybrid"})` → 结果列表显示匹配 markdown 片段。
4. **混合搜索**：`search({text, vector, mode:"hybrid"})`（vector 用占位）→ 结果按 RRF 融合排序。
5. **持久化**：刷新页面 → `VaneWorker.open("demo_db")` 读 OPFS 既有库 → 数据保留。
6. **SIMD 探针**：浏览器支持 simd128 → 加载 simd 产物；不支持 → scalar 产物（console.log 探针结果）。
7. **词典加载**：首次访问 → CDN fetch 词典 → sha256 校验 → OPFS 缓存；二次访问 → 零网络。
8. **词典降级**：断网 + 无缓存 → 降级 bigram + console.warn → 中文搜索仍可用（bigram 切分，质量下降但不报错）。
9. **export 快照**：点击"导出备份" → `db.export("backup.vane")` → OPFS 写快照文件（M2-12）。
10. **规模**：1000 markdown 文件搜索 <500ms（浏览器端放宽档，SPEC §13.1 WASM 放宽 3~5 倍）。

## 5. 验收标准
- Demo 在 Chrome/Edge/Firefox 主流浏览器跑通（拖入→搜索→结果）。
- 中文搜索命中正确（jieba 分词生效）。
- 持久化 + 刷新数据保留。
- SIMD 双产物按探针加载。
- 词典 CDN + OPFS 缓存 + 降级 bigram 三路径演示。
- export 快照可导出。
- README 含截图 + 使用说明。

## 6. 前置依赖
- M2-04（Worker 壳）。
- M2-05（SIMD 双产物）。

## 7. 不变量覆盖
- **§15 M2 Demo**：本模块直接落实。
- **I-8 binding 薄壳**：Demo 是产品演示，无检索逻辑（逻辑在 core，经 Worker 调）。
- **Won't-have 不触碰**：Demo 不内置 embedding 模型（vector 用占位或文档明示用户自行接入 transformers.js/OpenAI，REQUIREMENTS §2 Won't-have + examples 样板）。
- **体积门禁**：Demo 产物 wasm gzip ≤800KB（M2-01 门禁守护）。
