// T7：首页 SearchDemo 预计算数据生成器（主路径）。
// 用本地构建的 crates/vane-node 真实跑 hybrid/vector/text 三种 mode，
// 输出严格符合 DemoData 契约（website/src/components/contracts.ts）的 JSON。
//
// 用法：
//   node website/scripts/gen-demo-data.mjs          # 重新生成 src/data/demo-results.json
//   node website/scripts/gen-demo-data.mjs --check  # 只校验现有 JSON 的 shape（含 provenance 枚举）
//
// 前置：crates/vane-node 已构建（npm install && npx napi build --platform）。
// 无 npm 依赖：纯 node，通过 createRequire 加载本地 vane-node。

import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import * as path from 'node:path';
import * as fs from 'node:fs';
import * as os from 'node:os';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const WEBSITE_ROOT = path.resolve(__dirname, '..');
const REPO_ROOT = path.resolve(WEBSITE_ROOT, '..');
const OUT_PATH = path.join(WEBSITE_ROOT, 'src', 'data', 'demo-results.json');
const TOP_K = 5;

// ---------------------------------------------------------------------------
// 语料：~30 条中英混合文档（README/SPEC 摘句改写 + 自撰）。
// 4 维伪向量按主题轴设计：
//   axis0 = 向量/embedding 语义，axis1 = BM25/全文/分词，
//   axis2 = hybrid/融合/RRF，   axis3 = 存储/持久化/平台绑定。
// 每篇文档在其主题轴上占主导，保证 vector 列与 text 列排序有讲得通的差异。
// ---------------------------------------------------------------------------

const DOCS = [
  // --- 向量/embedding 主题（axis0 主导） ---
  { id: 'vec-intro-en', title: 'Vector search 101', axis: 0,
    body: 'Vector search turns documents into dense embeddings and finds nearest neighbors by cosine similarity. It captures semantic similarity even when the exact keywords never appear in the text.' },
  { id: 'vec-intro-zh', title: '向量检索入门', axis: 0,
    body: '向量检索把文档编码为稠密 embedding，再按余弦相似度寻找最近邻。即使查询与文档没有共同关键词，语义相近的内容也能被召回。' },
  { id: 'embed-model-en', title: 'Choosing an embedding model', axis: 0,
    body: 'Picking an embedding model is a trade-off between dimension, latency and multilingual quality. A good embedding model makes semantically similar documents cluster together in vector space.' },
  { id: 'embed-doc-zh', title: '如何为文档生成 embedding', axis: 0,
    body: '为文档生成 embedding 通常调用外部模型，把标题和正文拼接后编码成一个向量。向量维度需要与 collection schema 中声明的 dim 保持一致。' },
  { id: 'hnsw-en', title: 'HNSW index internals', axis: 0,
    body: 'HNSW builds a layered proximity graph over vectors so approximate nearest neighbor queries stay fast as the collection grows. Vane keeps per-segment HNSW indexes that merge during compaction.' },
  { id: 'hnsw-zh', title: 'HNSW 图索引原理', axis: 0,
    body: 'HNSW 在向量集合上构建分层邻近图，让近似最近邻查询在百万级规模下仍然毫秒返回。图索引的构建成本在段合并时摊销。' },
  { id: 'metric-en', title: 'Cosine vs dot product', axis: 0,
    body: 'Cosine similarity normalizes vectors to unit length before comparison, while dot product keeps magnitude information. For normalized embeddings the two metrics rank identically.' },

  // --- BM25/全文/分词主题（axis1 主导） ---
  { id: 'bm25-en', title: 'BM25 ranking explained', axis: 1,
    body: 'BM25 scores a document by term frequency saturation and inverse document frequency. Rare query terms contribute more, and long documents are length-normalized so keyword stuffing fails.' },
  { id: 'bm25-zh', title: 'BM25 全文检索原理', axis: 1,
    body: 'BM25 用词频饱和与逆文档频率为文档打分：查询里的稀有词贡献更大，长文档经过长度归一化，堆关键词刷分不再有效。' },
  { id: 'inverted-en', title: 'Full-text search with inverted index', axis: 1,
    body: 'An inverted index maps every term to the postings list of documents containing it. Full-text search intersects postings lists and ranks candidates with BM25.' },
  { id: 'inverted-zh', title: '倒排索引与全文检索', axis: 1,
    body: '倒排索引把每个词映射到包含它的文档列表。全文检索对查询词求 postings 交集，再用 BM25 为候选文档排序。' },
  { id: 'tokenizer-en', title: 'Tokenizers: stemming and lowercase', axis: 1,
    body: 'The standard tokenizer splits on unicode boundaries, lowercases latin text and applies Porter stemming, so "searching" and "search" hit the same term.' },
  { id: 'jieba-zh', title: '中文分词：jieba 与二元组', axis: 1,
    body: '中文没有空格分词，需要分词器。jieba 用前缀词典做最大概率切分并用 HMM 识别未登录词；cjk_bigram 则把连续汉字切成二元组，无需词典。' },
  { id: 'userdict-zh', title: 'jieba 词典与用户自定义词', axis: 1,
    body: '内置 jieba-lite 词典约二十万词条，与 jieba 原版切分行为一致。领域新词可以通过 userDict 注入，单 token 入索引后短语查询百分百命中。' },

  // --- hybrid/融合主题（axis2 主导） ---
  { id: 'hybrid-en', title: 'Hybrid search: best of both worlds', axis: 2,
    body: 'Hybrid search runs vector and BM25 retrieval in parallel and fuses both ranked lists. It keeps semantic recall while staying precise on exact keyword matches.' },
  { id: 'hybrid-zh', title: '混合检索：向量与关键词融合', axis: 2,
    body: '混合检索并行执行向量召回与 BM25 全文召回，再把两路结果融合。它既保留语义召回能力，又在精确关键词上维持高准确率。' },
  { id: 'rrf-en', title: 'Reciprocal rank fusion deep dive', axis: 2,
    body: 'Reciprocal rank fusion scores each document by the sum of 1/(k+rank) across result lists. RRF needs no score calibration, which makes it the default fusion strategy in Vane.' },
  { id: 'rrf-zh', title: 'RRF 倒数排名融合算法', axis: 2,
    body: 'RRF 按每路排名的倒数求和打分，不需要对向量距离与 BM25 分做归一化校准。两路都命中的文档在融合后排名显著靠前。' },
  { id: 'when-hybrid-en', title: 'When hybrid beats vector-only', axis: 2,
    body: 'Pure vector search misses exact product codes and rare proper nouns; pure text search misses paraphrases. Hybrid fusion wins whenever both signals matter.' },
  { id: 'when-hybrid-zh', title: '什么时候该用混合检索', axis: 2,
    body: '纯向量检索会漏掉精确型号与冷门专有名词，纯全文检索又理解不了同义改写。两类信号都重要时，混合检索的排序质量稳定居优。' },

  // --- 存储/持久化/平台主题（axis3 主导） ---
  { id: 'persist-en', title: 'Embedded databases and persistence', axis: 3, vec: [0.05, 0.08, 0.10, 0.92],
    body: 'An embedded search library persists segments to local disk so data survives restarts. Vane opens a directory and manages manifests, segments and snapshots for you.' },
  { id: 'persist-zh', title: '嵌入式检索库的持久化设计', axis: 3, vec: [0.05, 0.08, 0.10, 0.92],
    body: '嵌入式检索库把段文件持久化到本地磁盘，重启后数据不丢。打开一个目录即可，manifest、段与快照由库自动管理。' },
  { id: 'segment-en', title: 'Segmented storage and compaction', axis: 3, vec: [0.05, 0.10, 0.08, 0.90],
    body: 'Writes land in an in-memory buffer and flush to immutable segments. Background compaction merges small segments and reclaims space from deleted documents.' },
  { id: 'segment-zh', title: '段式存储与合并压缩', axis: 3, vec: [0.05, 0.10, 0.08, 0.90],
    body: '写入先落到内存 buffer，flush 后成为不可变段。后台合并把小段压成大段，同时回收被删除文档占用的空间。' },
  { id: 'wal-en', title: 'WAL and crash recovery', axis: 3, vec: [0.03, 0.06, 0.08, 0.94],
    body: 'A write-ahead log records every add before it reaches a segment. After a crash the engine replays the WAL so acknowledged writes are never lost.' },
  { id: 'wal-zh', title: 'WAL 预写日志与崩溃恢复', axis: 3, vec: [0.03, 0.06, 0.08, 0.94],
    body: '预写日志在文档落段之前先记录每一次写入。崩溃重启后引擎重放 WAL，已确认的写入一条不丢。' },
  { id: 'node-bind-en', title: 'Using Vane from Node.js', axis: 3, vec: [0.10, 0.05, 0.12, 0.88],
    body: 'The Node.js binding is a thin napi-rs addon with prebuilt binaries per platform. Open a database directory, create a collection, add documents and search with one async call.' },
  { id: 'node-bind-zh', title: '在 Node.js 中使用 Vane', axis: 3, vec: [0.10, 0.05, 0.12, 0.88],
    body: 'Node 绑定是 napi-rs 薄壳，各平台提供预编译二进制。打开数据库目录、创建 collection、批量 add 文档，然后一次异步调用完成检索。' },
  { id: 'wasm-en', title: 'Search in the browser with WASM', axis: 3, vec: [0.12, 0.05, 0.08, 0.90],
    body: 'The WASM build runs the same engine inside a Web Worker with an IndexedDB-backed VFS. Browser search stays fully offline once the bundle is cached.' },
  { id: 'wasm-zh', title: '浏览器里的 WASM 检索', axis: 3, vec: [0.12, 0.05, 0.08, 0.90],
    body: 'WASM 构建把同一套引擎跑在 Web Worker 里，底层用 IndexedDB 做虚拟文件系统。资源缓存后浏览器端检索可以完全离线。' },
  { id: 'go-bind-en', title: 'Go bindings via cgo', axis: 3, vec: [0.08, 0.05, 0.10, 0.90],
    body: 'The Go package links the C ABI of vane-ffi statically through cgo. The API mirrors the same six verbs: open, collection, add, flush, search, delete.' },
  { id: 'go-bind-zh', title: 'Go 绑定：cgo 静态链接', axis: 3, vec: [0.08, 0.05, 0.10, 0.90],
    body: 'Go 包通过 cgo 静态链接 vane-ffi 的 C ABI。接口保持同样的六个动词：open、collection、add、flush、search、delete。' },
];

// 主题轴 → 4 维伪向量（主导轴 0.92，其余按相关性给少量分量）。
function axisVector(axis) {
  const table = [
    [0.92, 0.10, 0.20, 0.05], // 向量/embedding
    [0.10, 0.92, 0.15, 0.05], // BM25/全文/分词
    [0.25, 0.25, 0.92, 0.05], // hybrid/融合
    [0.05, 0.08, 0.10, 0.92], // 存储/平台
  ];
  return table[axis];
}

// 预置 query：中英文都有；vec 指向对应主题轴。
const QUERIES = [
  { q: '向量检索', vector: [0.92, 0.10, 0.20, 0.05] },
  { q: 'hybrid search fusion', vector: [0.25, 0.25, 0.92, 0.05] },
  { q: 'BM25 ranking', vector: [0.10, 0.92, 0.15, 0.05] },
  { q: '中文分词', vector: [0.10, 0.92, 0.15, 0.05] },
  { q: 'embedding semantic similarity', vector: [0.92, 0.10, 0.20, 0.05] },
  { q: '持久化 崩溃恢复', vector: [0.04, 0.07, 0.09, 0.93] },
  { q: 'WASM 浏览器', vector: [0.12, 0.05, 0.08, 0.90] },
];

// ---------------------------------------------------------------------------
// snippet 截取：优先取含命中词的窗口（查询的 latin 词 / CJK 子串）。
// ---------------------------------------------------------------------------

function queryTerms(q) {
  const terms = [];
  for (const m of q.toLowerCase().matchAll(/[a-z0-9]+/g)) {
    terms.push(m[0]);
    // Porter 词干的常见落点：covers "searching"->"search" 之类的命中
    if (m[0].endsWith('ing') && m[0].length > 4) terms.push(m[0].slice(0, -3));
    if (m[0].endsWith('s') && m[0].length > 3) terms.push(m[0].slice(0, -1));
  }
  for (const m of q.matchAll(/[一-鿿]+/g)) {
    const run = m[0];
    terms.push(run);
    // 长中文 query 再补 2-gram，提高子串命中率
    for (let i = 0; i + 2 <= run.length; i++) terms.push(run.slice(i, i + 2));
  }
  // 长的优先，避免先命中短词截出低信息量片段
  return [...new Set(terms)].sort((a, b) => b.length - a.length);
}

function makeSnippet(doc, q) {
  const haystack = `${doc.title} — ${doc.body}`;
  const lower = haystack.toLowerCase();
  let pos = -1;
  for (const t of queryTerms(q)) {
    pos = lower.indexOf(t.toLowerCase());
    if (pos >= 0) break;
  }
  const WIDTH = 90;
  if (pos < 0) {
    return haystack.length > WIDTH ? haystack.slice(0, WIDTH) + '…' : haystack;
  }
  let start = Math.max(0, pos - 30);
  let end = Math.min(haystack.length, start + WIDTH);
  start = Math.max(0, end - WIDTH);
  // 窗口边界对齐到词边界，避免从 ASCII 单词中间截断
  const isWord = (c) => /[A-Za-z0-9]/.test(c);
  while (start > 0 && start < pos && isWord(haystack[start - 1]) && isWord(haystack[start])) start++;
  // 跳过残留的分隔符（"… — 正文" → "…正文"）
  while (start < end && /[\s—–-]/.test(haystack[start])) start++;
  while (end < haystack.length && isWord(haystack[end - 1]) && isWord(haystack[end])) end--;
  return (start > 0 ? '…' : '') + haystack.slice(start, end) + (end < haystack.length ? '…' : '');
}

// ---------------------------------------------------------------------------
// DemoData shape 校验（--check 模式也可独立跑）。
// ---------------------------------------------------------------------------

function validateDemoData(data) {
  const errors = [];
  const isStr = (v) => typeof v === 'string' && v.length > 0;
  const isNum = (v) => typeof v === 'number' && Number.isFinite(v);

  if (data === null || typeof data !== 'object' || Array.isArray(data)) {
    errors.push('root must be an object');
  } else {
    if (!['vane-node', 'manual'].includes(data.provenance)) {
      errors.push(`provenance must be 'vane-node' | 'manual', got ${JSON.stringify(data.provenance)}`);
    }
    if (!Array.isArray(data.docs) || data.docs.length < 25) {
      errors.push('docs must be an array of ~30 entries (>=25)');
    } else {
      const ids = new Set();
      data.docs.forEach((d, i) => {
        if (!isStr(d.id) || !isStr(d.title) || !isStr(d.body)) {
          errors.push(`docs[${i}] needs non-empty id/title/body strings`);
        }
        if (ids.has(d.id)) errors.push(`docs[${i}].id duplicated: ${d.id}`);
        ids.add(d.id);
      });
    }
    if (!Array.isArray(data.queries) || data.queries.length < 6) {
      errors.push('queries must be an array of >=6 entries');
    } else {
      const docIds = new Set((data.docs || []).map((d) => d.id));
      data.queries.forEach((qq, i) => {
        if (!isStr(qq.q)) errors.push(`queries[${i}].q missing`);
        for (const mode of ['hybrid', 'vector', 'text']) {
          const col = qq[mode];
          if (!Array.isArray(col) || col.length === 0) {
            errors.push(`queries[${i}].${mode} must be a non-empty DemoHit array`);
            continue;
          }
          col.forEach((h, j) => {
            if (!isStr(h.id) || !isStr(h.title) || !isStr(h.snippet) || !isNum(h.score)) {
              errors.push(`queries[${i}].${mode}[${j}] needs id/title/snippet strings + numeric score`);
            }
            if (!docIds.has(h.id)) {
              errors.push(`queries[${i}].${mode}[${j}].id not in docs: ${h.id}`);
            }
          });
        }
      });
    }
  }
  return errors;
}

// ---------------------------------------------------------------------------
// 主流程
// ---------------------------------------------------------------------------

async function main() {
  if (process.argv.includes('--check')) {
    const data = JSON.parse(fs.readFileSync(OUT_PATH, 'utf8'));
    const errors = validateDemoData(data);
    if (errors.length) {
      console.error(`demo-results.json INVALID (${errors.length} errors):`);
      for (const e of errors) console.error(`  - ${e}`);
      process.exit(1);
    }
    console.log(
      `demo-results.json OK: ${data.docs.length} docs, ${data.queries.length} queries, provenance=${data.provenance}`
    );
    return;
  }

  // 包自身目录内无法按名字 self-resolve（无 exports 字段），直接 require 入口文件。
  const require = createRequire(import.meta.url);
  const vane = require(path.join(REPO_ROOT, 'crates', 'vane-node', 'main.js'));

  const dbPath = fs.mkdtempSync(path.join(os.tmpdir(), 'vane-demo-'));
  const db = await vane.open(dbPath, { autoCommit: 'off' });
  try {
    const col = await db.collection(
      'demo',
      {
        fields: [
          { name: 't', type: 'text' },
          { name: 'v', type: 'vector', dim: 4, metric: 'cosine' },
        ],
      },
      { tokenizer: 'jieba' }
    );

    const report = await col.add(
      DOCS.map((d) => ({ id: d.id, text: `${d.title}。${d.body}`, vector: d.vec ?? axisVector(d.axis) }))
    );
    if (report.accepted !== DOCS.length) {
      throw new Error(`add accepted ${report.accepted}, expected ${DOCS.length}`);
    }
    await col.flush();

    const docById = new Map(DOCS.map((d) => [d.id, d]));
    const round = (x) => Math.round(x * 1e6) / 1e6;

    const queries = [];
    for (const { q, vector } of QUERIES) {
      const entry = { q };
      const params = {
        hybrid: { text: q, vector, topK: TOP_K, mode: 'hybrid' },
        vector: { vector, topK: TOP_K, mode: 'vector' },
        text: { text: q, topK: TOP_K, mode: 'text' },
      };
      for (const [mode, p] of Object.entries(params)) {
        const hits = await col.search(p);
        entry[mode] = hits.map((h) => {
          const doc = docById.get(h.id);
          return { id: h.id, title: doc.title, snippet: makeSnippet(doc, q), score: round(h.score) };
        });
      }
      queries.push(entry);
      console.log(
        `query ${JSON.stringify(q)}: hybrid=${entry.hybrid.length} vector=${entry.vector.length} text=${entry.text.length} hits`
      );
    }

    const data = {
      docs: DOCS.map(({ id, title, body }) => ({ id, title, body })),
      queries,
      provenance: 'vane-node',
    };

    const errors = validateDemoData(data);
    if (errors.length) {
      console.error('generated data failed shape validation:');
      for (const e of errors) console.error(`  - ${e}`);
      process.exit(1);
    }

    fs.mkdirSync(path.dirname(OUT_PATH), { recursive: true });
    fs.writeFileSync(OUT_PATH, JSON.stringify(data, null, 2) + '\n');
    console.log(`wrote ${OUT_PATH} (${data.docs.length} docs, ${data.queries.length} queries)`);
  } finally {
    await db.close();
    fs.rmSync(dbPath, { recursive: true, force: true });
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
