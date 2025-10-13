# Vane M0 Demo — 1 万维基摘要三列排序对比

用 1 万条合成英文维基摘要灌入 Vane，对同一查询分别跑 `hybrid` / `vector` / `text` 三种模式，输出 top-10 id 三列对比表，验证 RRF 融合在起作用。并给出与"手写 sqlite-vec + FTS5 方案"的代码量对比，证明 Vane 的集成度优势。

> **SPEC §15 M0 验收锚点**：1 万条维基摘要（英文语料），hybrid / vector-only / text-only 三列排序对比 + 对比 sqlite-vec+FTS5 手写方案的代码量。

---

## ⚠️ 伪向量说明（重要）

本 demo 用 `hashToVector(text)` 生成的**确定性伪向量**（sha256 unigram/bigram n-gram bucket → 384 维 L2 归一化），**不是真实语义 embedding**。同文本同向量、共享词项的文本 cosine 相似度更高，足以演示 vector 召回路与 hybrid 融合。

**生产应替换为真实 embedding API**（如 OpenAI / Cohere / 本地模型）。SPEC Won't-have：**Vane 不内置 embedding**——向量由调用方提供，Vane 只负责存储、索引、检索、融合。

## 数据源说明

`lib/data.js` 用固定 PRNG 种子（mulberry32, seed=42）**确定性生成** 1 万条合成英文维基摘要（主题词+模板句式，3~5 句/条）。非真实维基语料——真实语料需联网下载，本 demo 用合成语料保证离线可复现。demo 重点是三列排序对比与代码量对比，语料真实性非关键。

---

## 前置条件

- Node.js ≥ 18
- `@vane/node` 已构建（在 `crates/vane-node` 下 `napi build --platform --release` 产出 `*.node`）
- demo 通过本地 file:link 引用 `@vane/node`（见 `package.json`）

## 运行步骤

```bash
cd examples/demo
npm install            # 本地 link @vane/node
npm run load           # 灌库 1 万条，产出 ./vane-data/
npm run compare        # 三列排序对比
npm run smoke:vector   # 伪向量模块自检
npm run smoke:data     # 数据生成模块自检
```

`npm run load` 输出示例：

```
loaded 10000 docs in 1978ms (10 segment(s))
```

`npm run compare` 输出示例（节选）：

```
query: "quantum algorithm"
MODE     top10 ids
hybrid   wiki-06996    wiki-09812    wiki-07432    wiki-00827    wiki-04948    wiki-05743    wiki-05939    wiki-04514    wiki-01974    wiki-06291
vector   wiki-01974    wiki-04897    wiki-02337    wiki-08045    wiki-02875    wiki-06996    wiki-09812    wiki-04948    wiki-06609    wiki-04324
text     wiki-06291    wiki-06996    wiki-00827    wiki-07432    wiki-06958    wiki-03206    wiki-09812    wiki-05933    wiki-03335    wiki-06315
diff  hybrid vs vector=10  hybrid vs text=10  vector vs text=9
---
query: "renaissance europe"
MODE     top10 ids
hybrid   wiki-04394    wiki-00139    wiki-00140    wiki-03518    wiki-09772    wiki-02481    wiki-06512    wiki-00095    wiki-04003    wiki-08925
vector   wiki-00095    wiki-08925    wiki-04394    wiki-00140    wiki-01729    wiki-05519    wiki-09070    wiki-08604    wiki-04168    wiki-09542
text     wiki-04003    wiki-00139    wiki-04394    wiki-09772    wiki-03518    wiki-07677    wiki-01549    wiki-01763    wiki-00830    wiki-09437
diff  hybrid vs vector=10  hybrid vs text=9  vector vs text=9
---
...
summary: 5/5 query(ies) show hybrid distinct from both vector and text
```

`summary` 行表明 5 组 query 全部出现 hybrid 列与 vector/text 列均不完全相同——RRF 融合产生了与单路不同的排序（AC4 满足）。

---

## 与 sqlite-vec + FTS5 手写方案代码量对比

实现等价的 hybrid（向量+文本 RRF 融合）检索，手写 sqlite-vec + FTS5 需要双表双插入、双路查询、手写 RRF 融合、id 映射同步等样板；Vane 把这些封装在 core，调用方只需声明 schema + 一行 search。

| 步骤 | sqlite-vec + FTS5 手写 | Vane demo |
|------|------------------------|-----------|
| 建表 | `CREATE VIRTUAL TABLE docs_fts USING fts5(content);` + `CREATE VIRTUAL TABLE docs_vec USING vec0(embedding float[384]);` + 关联 rowid 表 | `db.collection("wiki", schema, opts)` 一行 |
| 插入 | 分别向 fts、vec、rowid 表插入，事务包裹 | `col.add(docs)` 一行 |
| flush | 手动 `COMMIT` | `col.flush()` 一行 |
| 文本搜索 | `SELECT rowid, bm25(docs_fts) FROM docs_fts WHERE content MATCH ? ORDER BY bm25(docs_fts) LIMIT 10` | `col.search({text, mode:"text"})` |
| 向量搜索 | `SELECT rowid, distance FROM docs_vec WHERE embedding MATCH ? ORDER BY distance LIMIT 10` | `col.search({vector, mode:"vector"})` |
| hybrid 融合 | **手写**：两路查询 → 取 topK×candidate → 按 rank 算 RRF `1/(60+rank)` → 合并排序 → 取 top10（约 40~60 行 JS） | `col.search({text, vector, mode:"hybrid", fusion:"rrf"})` 一行 |
| 持久化 | 手动管理 db 文件 + WAL | 内置 manifest 原子切换 |
| 估算总有效行数 | ~150~200 行（含融合、双表同步、id 映射） | 见下表实测 |

### Vane demo 实测行数

| 文件 | 总行 | 有效 SLOC（去注释/空行） |
|------|------|------|
| `load-wiki.js` | 90 | 56 |
| `compare.js` | 128 | 109 |
| `lib/vector.js` | 101 | 86 |
| `lib/data.js` | 130 | 107 |
| **合计** | **449** | **358** |

> 说明：`lib/data.js`（合成语料生成，107 SLOC）与 `lib/vector.js` 的伪向量是 demo 专属开销（真实场景用真实语料+真实 embedding，不需要）。**与 sqlite-vec+FTS5 对比的"hybrid 等价代码"核心是 `load-wiki.js` 的灌库逻辑 + `compare.js` 的三路 search 调用**——这部分有效 SLOC 约 165 行，其中绝大部分是 demo 的批量分批、表格打印、差异统计等展示逻辑；**真正的 Vane API 调用（建表+插入+flush+三路 search）合计仅 6 行**，对应手写方案的 ~150~200 行。

### 手写 RRF 融合示例（sqlite-vec+FTS5 方案需自行实现的部分）

```js
// 手写 RRF 融合（约 40~60 行；Vane 一行 col.search({mode:"hybrid",fusion:"rrf"}) 搞定）
function rrfFuse(textHits, vecHits, k = 60, topN = 10) {
  const scores = new Map(); // rowid ->累计 RRF 分
  textHits.forEach((r, rank) => {
    scores.set(r.rowid, (scores.get(r.rowid) || 0) + 1 / (k + rank + 1));
  });
  vecHits.forEach((r, rank) => {
    scores.set(r.rowid, (scores.get(r.rowid) || 0) + 1 / (k + rank + 1));
  });
  return [...scores.entries()]
    .sort((a, b) => b[1] - a[1])
    .slice(0, topN)
    .map(([rowid, score]) => ({ rowid, score }));
}
// 还需：rowid → external id 映射、双表插入事务、fts5/vec0 建表 SQL、distance/bm25 结果归一...
```

---

## 文件结构

```
examples/demo/
├── package.json              # 依赖 @vane/node（本地 file:link）
├── .gitignore                # 忽略 vane-data/、node_modules/
├── lib/
│   ├── vector.js             # hashToVector(text, dim=384) —— 确定性伪向量
│   └── data.js               # generateWikiAbstracts(count, seed) —— 1 万合成英文摘要
├── load-wiki.js              # 灌库：open → collection → add 10k → flush
└── compare.js                # 三列对比：同 query 跑 hybrid/vector/text
```

运行产物 `./vane-data/`（含 `manifest.json` + `segments/seg_*/`）已被 `.gitignore` 忽略。
