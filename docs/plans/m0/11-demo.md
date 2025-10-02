# Demo 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development`。步骤用 checkbox。
> 日期：2026-08-09
> 权威来源：`docs/SPEC.md` v1.0 §15 / §4 / §8；`docs/plans/m0/README.md` Global Interface Contracts。

---

## Goal

交付一个**真实可运行**的 Node.js ESM demo，用 1 万条英文维基摘要灌入 Vane，对同一查询分别跑 `hybrid` / `vector` / `text` 三种模式，输出三列 top-10 id 对比表，并在 README 中给出与"手写 sqlite-vec + FTS5 方案"的代码量对比，证明 Vane 的集成度优势。此 demo 是 M0 验收锚点（SPEC §15）。

## Architecture

```
examples/demo/
├── package.json              # 依赖 @vane/node（本地 link），type: "module"
├── lib/
│   ├── vector.js             # hashToVector(text, dim) —— 确定性伪向量
│   └── data.js               # generateWikiAbstracts(count, seed) —— 1 万合成英文摘要
├── load-wiki.js              # 灌库：open → collection → add 10k → flush
├── compare.js                # 三列对比：同 query 跑 hybrid/vector/text，输出对比表
└── README.md                 # 运行说明 + sqlite-vec+FTS5 代码量对比
```

数据流：
1. `lib/data.js` 用固定 PRNG 种子生成 1 万条 `{id, title, text}`（英文，主题词+模板句式，保证可复现）。
2. `lib/vector.js` 对每条 text 用字符/词 n-gram hash 映射到 384 维并 L2 归一化，得到确定性伪向量（同文本同向量，相近文本向量有余弦相关性）。
3. `load-wiki.js` 调 `@vane/node` 的 `VaneDb.open` → `collection("wiki", schema, {tokenizer:"standard"})` → 批量 `add` → `flush`，落库到 `./vane-data/`。
4. `compare.js` 打开已建库，对一组预设 query（同时提供 `text` 与 `vector=hashToVector(query)`）分别以 `mode: hybrid|vector|text` 搜索，打印三列 top-10 id 对比表。

## Tech Stack

- Node.js ≥ 18，ESM（`"type":"module"`）
- `@vane/node`（本地 file:link `../../crates/vane-node`，由 09-node-binding 产出）
- 无外部依赖：PRNG、hash 全部用 Node 内置 `node:crypto`（sha256），不引入第三方包
- 向量：**确定性伪向量**（hash-based），非真实 embedding

## SPEC 引用

- §15 M0 验收："Demo：1 万条维基摘要（英文语料），hybrid / vector-only / text-only 三列排序对比 + 对比 sqlite-vec+FTS5 手写方案的代码量"
- §4.1/§4.2 API：`open` / `collection` / `add` / `flush` / `search`；`SearchQuery`（text/vector/topK/mode/fusion）；`Hit`（id/score/fields）
- §8.1 三种 mode：`hybrid`（两路召回+融合）/ `vector`（向量距离）/ `text`（BM25）
- §8.2 fusion 默认 `rrf`（k=60 冻结）
- §3.1 Schema：恰好一个 vector 字段；text 字段 ≥1
- §10 错误码：binding 透传 `VaneError`

## 前置依赖

- **09-node-binding** 已完成：`@vane/node` 可 `npm install`（本地 link），导出 `VaneDb` / `VaneCollection`，方法签名见 README `Global Interface Contracts §09-node-binding`。
- 09-node-binding 的 JSON 契约（本 demo 消费侧假设，若 09 实现有差异以 09 为准并回更本计划）：
  - `VaneDb.open(path: string, opts?: object) -> Promise<VaneDb>`
  - `db.collection(name: string, schema: object, opts?: {tokenizer?: "standard"|"cjk_bigram"|"jieba"}) -> Promise<VaneCollection>`
  - `collection.add(docs: object[]) -> Promise<{accepted: number, visibleAfterFlush: boolean}>`
  - `collection.flush() -> Promise<void>`
  - `collection.search(query: object) -> Promise<object[]>`（返回 `[{id, score, fields?}]`）
  - `db.close() -> Promise<void>`
- Schema JSON 形状（假设 09 反序列化接受）：
  ```json
  { "fields": [
    {"name": "text", "type": "text"},
    {"name": "embedding", "type": "vector", "dim": 384, "metric": "cosine"}
  ]}
  ```
- Doc JSON 形状（对齐 07-api-core `Doc` 结构）：`{id, text, vector, meta?}`
- SearchQuery JSON 形状：`{text?, vector?, topK?, mode?, fusion?}`

## 验收标准

| # | 判据 |
|---|------|
| AC1 | `node load-wiki.js` 在 09-node-binding 可用的环境下 exit 0，产出 `./vane-data/` 目录（含 manifest.json + segments/） |
| AC2 | 灌库文档数 = 10000；`collection.search` 在 flush 后能召回结果（非空） |
| AC3 | `node compare.js` exit 0，stdout 打印 ≥ 3 组 query 的三列对比表，每列 10 个 id，格式 `query | hybrid top10 ids | vector top10 ids | text top10 ids` |
| AC4 | 对比表中可见三列排序**存在差异**（至少有一组 query 的 hybrid 列与 vector 列、text 列均不完全相同），证明融合在起作用 |
| AC5 | `README.md` 含：① 运行步骤 ② 伪向量说明（标注"demo 用伪向量，生产用真实 embedding API"） ③ sqlite-vec+FTS5 等价方案代码量对比表 |
| AC6 | demo 不引入任何第三方 npm 依赖（仅 `@vane/node` + Node 内置模块） |
| AC7 | 重复运行 `load-wiki.js`（删库重建）产出的库可被 `compare.js` 打开并搜索成功 |

---

## Global Constraints

| 约束 | 说明 |
|------|------|
| 不内置 embedding | SPEC Won't-have；demo 用 `hashToVector` 确定性伪向量，**必须在 README 与代码注释明确标注**"demo 用伪向量，生产用真实 embedding API" |
| 英文语料 | SPEC §15 明确英文；分词器用 `standard`（SPEC §5.1） |
| 伪向量可复现 | 同 text 永远产出同 vector（确定性 hash）；L2 归一化以支持 cosine |
| 伪向量有语义相关性 | 用词 unigram + bigram 的 TF 分量填入 384 维 bucket，使共享词项的文本 cosine 相似度更高（近似 BM25 的词项重叠信号，但维度连续可被向量路召回） |
| 无第三方依赖 | 仅 `@vane/node` + `node:crypto` + `node:fs` + `node:path` |
| 数据可复现 | PRNG 固定种子（mulberry32，seed=42），1 万条文本跨机器一致 |
| 真实可运行代码 | 禁止占位符；所有脚本含完整可执行 JS |
| 本地 link | `package.json` 中 `"@vane/node": "file:../../crates/vane-node"` |

---

## File Structure

```
examples/demo/
├── package.json
├── lib/
│   ├── vector.js     # export hashToVector(text, dim=384)
│   └── data.js       # export generateWikiAbstracts(count=10000, seed=42)
├── load-wiki.js      # main(): build db, add 10k, flush
├── compare.js        # main(): 3 queries × 3 modes 对比表
└── README.md
```

运行产物（gitignore）：`./vane-data/`（Vane 库目录）。

---

## 任务清单（bite-sized TDD）

> 纪律：每个 Task 先写验证脚本/断言（测试先行），再写实现，最后跑验证勾选。demo 的"测试"以可执行的 smoke 检查脚本 + 断言为主，不强制测试框架（保持零依赖）。

### Task 1 · 伪向量模块 `lib/vector.js`

**Files:** `examples/demo/lib/vector.js`

**Interfaces:**
- Produces: `hashToVector(text: string, dim: number = 384): number[]`（L2 归一化）
- Consumes: 无（纯函数，仅 `node:crypto`）

**设计：**
- 小写化 → 用 `/[a-z0-9]+/g` 切词
- 每个 unigram token：`sha256(token)` 取前 4 字节为 uint32 → `bucket = h % dim`，`vec[bucket] += 1.0`
- 每个 bigram（相邻两词）：`sha256(word_i + ' ' + word_{i+1})` → `bucket = h % dim`，`vec[bucket] += 0.5`（bigram 权重减半，避免淹没 unigram）
- L2 归一化：`vec[i] /= sqrt(Σ vec[i]^2)`（零向量保持全 0）
- 返回普通 `number[]`（便于 JSON 序列化进 binding）

**验证脚本（inline smoke）：** 文件底部 `if (import.meta.url === ...)` 自检：
- `hashToVector("hello world")` 长度 = 384
- 两次调用同文本结果严格相等
- 两段共享词的文本 cosine > 0；完全不共享词的文本 cosine ≈ 0
- 范数 ≈ 1.0（误差 < 1e-6）

**Steps:**
- [ ] Step 1.1：新建 `examples/demo/lib/vector.js`，写 `hashToVector` 函数（含 sha256 bucket 映射 + unigram/bigram TF + L2 归一化）
- [ ] Step 1.2：在文件底部加自检 main（`if (import.meta.url === pathToFileURL(process.argv[1]).href)`）跑上述 4 条断言，任一失败 `process.exit(1)`
- [ ] Step 1.3：`node lib/vector.js` exit 0，4 条断言全过
- [ ] Step 1.4：在文件顶部注释标注："demo 用伪向量，生产用真实 embedding API（SPEC Won't-have：不内置 embedding）"

### Task 2 · 数据生成模块 `lib/data.js`

**Files:** `examples/demo/lib/data.js`

**Interfaces:**
- Produces: `generateWikiAbstracts(count: number = 10000, seed: number = 42): Array<{id, title, text}>`
- Consumes: 无（纯函数，内置词库）

**设计：**
- 内置 mulberry32 PRNG（seed=42，确定性）
- 词库（写在文件内，约 200 词，覆盖科技/历史/地理/生物/艺术等领域英文词）：`const WORDS = ["algorithm", "quantum", "renaissance", "ecosystem", ...]`
- 标题模板：`capitalizedTopic + " of " + capitalizedTopic`（如 "Quantum Algorithm of Renaissance Europe"）
- 摘要模板（3~5 句）：从句式池随机拼——
  - `"The {topic} is a {adj} {noun} that {verb} {topic2}."`
  - `"Historically, {topic} emerged in {year} as a response to {topic2}."`
  - `"Modern {topic} integrates {topic}, {topic2}, and {topic3}."`
  - `"Researchers note that {topic} influences {topic2} via {adj} mechanisms."`
- 每条 `id = "wiki-" + zeroPad(i, 5)`（如 `wiki-00042`），便于人眼对比
- 每条 text 长度 60~240 词，保证 BM25 有足够词项信号

**验证脚本（inline smoke）：**
- `generateWikiAbstracts(100)` 长度 = 100
- 两次 `generateWikiAbstracts(10000, 42)` 产出完全相同（确定性）
- 第 0 条 id = "wiki-00000"，text 非空且长度 > 50 字符
- 10000 条 id 全唯一

**Steps:**
- [ ] Step 2.1：新建 `examples/demo/lib/data.js`，实现 mulberry32 + 词库 + 标题/摘要模板 + `generateWikiAbstracts`
- [ ] Step 2.2：底部加自检 main 跑上述 4 条断言
- [ ] Step 2.3：`node lib/data.js` exit 0，断言全过
- [ ] Step 2.4：`node -e "import('./lib/data.js').then(m=>{const d=m.generateWikiAbstracts(10000,42);console.log(d[42].id, d[42].title, d[42].text.slice(0,80))})"` 打印一条样本，人工确认是合理英文句子

### Task 3 · `package.json`

**Files:** `examples/demo/package.json`

**Interfaces:**
- Produces: demo 包描述，声明 ESM + 本地依赖

**内容：**
```json
{
  "name": "@vane/demo",
  "version": "0.0.0",
  "private": true,
  "type": "module",
  "description": "Vane M0 demo: 10k wiki abstracts hybrid/vector/text comparison",
  "scripts": {
    "load": "node load-wiki.js",
    "compare": "node compare.js",
    "smoke:vector": "node lib/vector.js",
    "smoke:data": "node lib/data.js"
  },
  "dependencies": {
    "@vane/node": "file:../../crates/vane-node"
  },
  "engines": { "node": ">=18" }
}
```

**Steps:**
- [ ] Step 3.1：新建 `examples/demo/package.json`（上述内容）
- [ ] Step 3.2：`cd examples/demo && npm install` exit 0（在 09-node-binding 产出后验证；本步可延后到 09 可用时执行，先建文件）
- [ ] Step 3.3：确认 `node_modules/@vane/node` 软链指向 `../../crates/vane-node`

### Task 4 · `load-wiki.js` 灌库脚本

**Files:** `examples/demo/load-wiki.js`

**Interfaces:**
- Consumes from 09-node-binding: `VaneDb.open` / `db.collection` / `collection.add` / `collection.flush` / `db.close`
- Consumes from Task 1/2: `hashToVector` / `generateWikiAbstracts`
- Produces: `./vane-data/` Vane 库；导出 `{ buildDb, DB_PATH, SCHEMA, COLLECTION }` 供 compare.js 复用

**设计：**
- `DB_PATH = "./vane-data"`（相对 demo 目录）
- `SCHEMA = { fields: [{name:"text",type:"text"}, {name:"embedding",type:"vector",dim:384,metric:"cosine"}] }`
  > S17 裁决：schema 的 text 字段名统一为 'text'（与 Doc.text API 字段对齐），避免 'content' 与 'text' 混用造成混淆。
- `COLLECTION = "wiki"`
- `buildDb()`：
  1. 递归删除旧 `DB_PATH`（`fs.rm`）
  2. `const db = await VaneDb.open(DB_PATH, { autoCommit: "off" })`
  3. `const col = await db.collection(COLLECTION, SCHEMA, { tokenizer: "standard" })`
  4. `const docs = generateWikiAbstracts(10000, 42)`
  5. 分批 add（每批 500 条，避免单次 JSON 过大）：`docs.map(d => ({id: d.id, text: d.text, vector: hashToVector(d.text, 384)}))`，`await col.add(batch)`
  6. `await col.flush()`（可见性边界，SPEC §7.1）
  7. `await db.close()`
  8. 打印：文档数、段数（list `vane-data/segments/`）、耗时
- main：`if (import.meta.url === pathToFileURL(process.argv[1]).href) buildDb()`

**验证（AC1/AC2/AC7）：**
- `node load-wiki.js` exit 0
- `./vane-data/manifest.json` 存在且含 `"wiki"` collection
- `./vane-data/segments/` 下至少 1 个 `seg_*` 目录
- stdout 打印 `loaded 10000 docs in <ms>ms`

**Steps:**
- [ ] Step 4.1：新建 `examples/demo/load-wiki.js`，import `VaneDb` from `@vane/node`、`hashToVector` from `./lib/vector.js`、`generateWikiAbstracts` from `./lib/data.js`、`node:fs`、`node:path`、`node:url`
- [ ] Step 4.2：定义 `DB_PATH` / `SCHEMA` / `COLLECTION` 常量并 export
- [ ] Step 4.3：实现 `buildDb()`：删旧库 → open → collection → 分批 add（500/批）→ flush → close → 打印统计
- [ ] Step 4.4：main 守卫调用 `buildDb().catch(e => { console.error(e); process.exit(1); })`
- [ ] Step 4.5：运行 `node load-wiki.js`，确认 exit 0、manifest.json 存在、stdout 打印 10000 docs
- [ ] Step 4.6：二次运行（先删库）确认幂等可复现（AC7）

### Task 5 · `compare.js` 三列对比脚本

**Files:** `examples/demo/compare.js`

**Interfaces:**
- Consumes from 09-node-binding: `VaneDb.open` / `db.collection` / `collection.search` / `db.close`
- Consumes from Task 1/4: `hashToVector` / `DB_PATH` / `SCHEMA` / `COLLECTION`
- Produces: stdout 三列对比表

**设计：**
- 预设 3~5 组 query（英文短语，与数据词库有重叠以保证有召回）：
  ```js
  const QUERIES = [
    "quantum algorithm",
    "renaissance europe",
    "ecosystem dynamics",
    "neural network",
    "ancient philosophy"
  ];
  ```
- 对每个 query：
  - `const qvec = hashToVector(query, 384)`
  - 三次 search：
    1. `col.search({vector: qvec, topK: 10, mode: "vector"})`
    2. `col.search({text: query, topK: 10, mode: "text"})`
    3. `col.search({text: query, vector: qvec, topK: 10, mode: "hybrid", fusion: "rrf"})`
  - 取每路前 10 的 `id` 数组
- 输出表（纯文本，对齐，便于人眼对比）：
  ```
  query: "quantum algorithm"
  MODE       top10 ids
  hybrid     wiki-00042  wiki-00017  wiki-00099  ...
  vector     wiki-00042  wiki-00099  wiki-00017  ...
  text       wiki-00017  wiki-00042  wiki-00008  ...
  ---
  ```
  另附"差异行"：标注 hybrid 列与 vector/text 列的 id 顺序差异计数，便于 AC4 判据。
- main：`open(DB_PATH)` → `collection(COLLECTION, SCHEMA, {tokenizer:"standard"})`（幂等返回既有）→ 跑查询 → `close()`

**验证（AC3/AC4）：**
- `node compare.js` exit 0
- 打印 ≥ 3 组 query 对比表
- 至少一组 query 的 hybrid 列与 vector 列顺序不同、与 text 列不同（AC4）

**Steps:**
- [ ] Step 5.1：新建 `examples/demo/compare.js`，import 依赖
- [ ] Step 5.2：定义 `QUERIES` 数组（5 组英文短语）
- [ ] Step 5.3：实现 `runComparison(col)`：对每个 query 跑 3 次 search，收集 id 数组
- [ ] Step 5.4：实现 `printTable(results)`：对齐输出三列 + 差异计数
- [ ] Step 5.5：main：open → collection → runComparison → close，`catch` 退出 1
- [ ] Step 5.6：先 `node load-wiki.js` 建库，再 `node compare.js`，确认输出对比表（AC3）
- [ ] Step 5.7：人工确认至少一组 query 三列排序存在差异（AC4）；若所有 query 三列完全相同，排查 fusion 是否生效（向 07-api-core/03-fusion 反馈）

### Task 6 · `README.md`

**Files:** `examples/demo/README.md`

**内容大纲：**
1. **概述**：1 万英文维基摘要（合成语料）三列排序对比 demo。
2. **前置条件**：09-node-binding 已构建，`@vane/node` 可 install。
3. **运行步骤**：
   ```bash
   cd examples/demo
   npm install
   npm run load      # 灌库 1 万条，产出 ./vane-data/
   npm run compare   # 三列对比
   ```
4. **伪向量说明**（醒目）：标注"demo 用 `hashToVector` 确定性伪向量（sha256 n-gram bucket），**非真实语义 embedding**；生产应替换为真实 embedding API（如 OpenAI/Cohere 本地模型）。SPEC Won't-have：Vane 不内置 embedding。"
5. **输出示例**：贴一段 `npm run compare` 的真实输出片段。
6. **与 sqlite-vec + FTS5 手写方案代码量对比**（核心验收点）：
   - 列出等价手写方案所需步骤与估算代码量（见下方对比表）
   - Vane demo 代码量 = load-wiki.js + compare.js + lib/ 的有效行数（实测量）
   - 结论：Vane 用 N 行完成 hybrid，sqlite-vec+FTS5 方案需 ~M 行（含 RRF 融合手写、双表双插入、结果合并）

**sqlite-vec + FTS5 等价方案（README 内详述，非本 demo 实现）：**

| 步骤 | sqlite-vec + FTS5 手写 | Vane demo |
|------|------------------------|-----------|
| 建表 | `CREATE VIRTUAL TABLE docs_fts USING fts5(content);` + `CREATE VIRTUAL TABLE docs_vec USING vec0(embedding float[384]);` + 关联 rowid 表 | `db.collection("wiki", schema, opts)` 一行 |
| 插入 | 分别向 fts、vec、rowid 表插入，事务包裹 | `col.add(docs)` 一行 |
| flush | 手动 `COMMIT` | `col.flush()` 一行 |
| 文本搜索 | `SELECT rowid, bm25(docs_fts) FROM docs_fts WHERE content MATCH ? ORDER BY bm25(docs_fts) LIMIT 10` | `col.search({text, mode:"text"})` |
| 向量搜索 | `SELECT rowid, distance FROM docs_vec WHERE embedding MATCH ? ORDER BY distance LIMIT 10` | `col.search({vector, mode:"vector"})` |
| hybrid 融合 | **手写**：两路查询 → 取 topK×candidate → 按 rank 算 RRF(1/(60+rank)) → 合并排序 → 取 top10（约 40~60 行 JS） | `col.search({text, vector, mode:"hybrid", fusion:"rrf"})` 一行 |
| 持久化 | 手动管理 db 文件 + WAL | 内置 manifest 原子切换 |
| 估算总有效行数 | ~150~200 行（含融合、双表同步、id 映射） | ~80 行（load + compare + lib） |

**Steps:**
- [ ] Step 6.1：新建 `examples/demo/README.md`，写概述 + 前置 + 运行步骤
- [ ] Step 6.2：写伪向量说明（醒目标注）
- [ ] Step 6.3：运行 `npm run compare`，把真实输出片段贴入"输出示例"
- [ ] Step 6.4：写 sqlite-vec+FTS5 对比表（含等价 SQL + RRF 手写代码示例片段 + 行数估算）
- [ ] Step 6.5：用 `wc -l load-wiki.js compare.js lib/*.js` 统计 Vane demo 实际行数，填入对比表"Vane demo"列
- [ ] Step 6.6：通读 README，确认运行步骤可直接复制执行（AC5）

### Task 7 · 端到端验收

**Files:** 无新增（跑全流程）

**Steps:**
- [ ] Step 7.1：`rm -rf vane-data && npm run load` exit 0
- [ ] Step 7.2：`npm run compare` exit 0，输出对比表
- [ ] Step 7.3：核对 AC1~AC7 全部满足
- [ ] Step 7.4：在 `examples/demo/` 下加 `.gitignore`（内容 `vane-data/`、`node_modules/`）
- [ ] Step 7.5：把 demo 运行截图/输出片段更新进 README（若 Task 6 已做则跳过）

---

## 风险与回退

| 风险 | 影响 | 回退 |
|------|------|------|
| 09-node-binding 的 JSON schema/doc 格式与本计划假设不符 | load-wiki.js 调用失败 | 以 09 实际反序列化为准调整 SCHEMA/Doc JSON 形状；回更本计划"前置依赖"节 |
| 伪向量下 hybrid 与 vector/text 三列排序无差异（AC4 不满足） | 融合未体现价值 | 检查 07-api-core fusion 是否生效；若融合正确但结果恰好相同，改用差异更大的 query（如 query 词在数据中稀疏）；仍不行则向编排者反馈 |
| 1 万条 add 单批 OOM | 灌库失败 | 已分批 500 条；若仍失败降到 100 条/批 |
| `@vane/node` 本地 link 在 npm install 时未就绪 | Task 3/4 无法验证 | Task 3.2/4.5 标注"待 09 可用后执行"；先完成代码编写，验证延后 |
| 合成语料过于模板化导致 BM25 召回集中 | text 列结果同质 | data.js 句式池 ≥ 6 种、词库 ≥ 200 词、每条摘要 3~5 句随机组合；必要时增加 query 数量到 5 组 |

---

## 完成定义（DoD）

- 4 个产出文件（package.json / load-wiki.js / compare.js / README.md）+ 2 个 lib 模块全部存在且含真实可运行代码
- AC1~AC7 全部勾选
- README 含 sqlite-vec+FTS5 代码量对比表与真实行数统计
- 所有代码注释中涉及向量的地方均标注"demo 用伪向量，生产用真实 embedding API"
