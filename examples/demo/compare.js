// compare.js：三列排序对比（hybrid / vector / text）。
// 对同一组 query 分别跑三种 mode，打印 top-10 id 对比表，验证融合在起作用（AC3/AC4）。
// demo 用伪向量（hashToVector），生产用真实 embedding API。

import vane from '@vane/node';
const { open } = vane;
import { hashToVector } from './lib/vector.js';
import { DB_PATH, SCHEMA, COLLECTION } from './load-wiki.js';
import { existsSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

// 预设 query（英文短语，与数据词库有重叠以保证有召回）
const QUERIES = [
  'quantum algorithm',
  'renaissance europe',
  'ecosystem dynamics',
  'neural network',
  'ancient philosophy',
];

const TOPK = 10;

/**
 * 对单条 query 跑三种 mode，返回 { query, hybrid, vector, text }（各为 id 数组）。
 * @param {object} col VaneCollection
 * @param {string} query
 */
async function runOne(col, query) {
  const qvec = hashToVector(query, 384);
  // 1. vector-only
  const vHits = await col.search({ vector: qvec, topK: TOPK, mode: 'vector' });
  // 2. text-only
  const tHits = await col.search({ text: query, topK: TOPK, mode: 'text' });
  // 3. hybrid（RRF, k=60 冻结）
  const hHits = await col.search({
    text: query,
    vector: qvec,
    topK: TOPK,
    mode: 'hybrid',
    fusion: 'rrf',
  });
  return {
    query,
    hybrid: hHits.map((h) => h.id),
    vector: vHits.map((h) => h.id),
    text: tHits.map((h) => h.id),
  };
}

/** 计算两列 id 数组的顺序差异计数（按位置比对，不同即 +1）。 */
function diffCount(a, b) {
  const n = Math.max(a.length, b.length);
  let d = 0;
  for (let i = 0; i < n; i++) {
    if (a[i] !== b[i]) d++;
  }
  return d;
}

/** 打印一组结果的三列对比表 + 差异计数。 */
function printResult(r) {
  const padId = (s) => String(s || '').padEnd(13);
  console.log(`query: "${r.query}"`);
  console.log(`${'MODE'.padEnd(8)} ${'top10 ids'}`);
  const row = (label, ids) => {
    console.log(`${label.padEnd(8)} ${ids.map(padId).join(' ').trimEnd()}`);
  };
  row('hybrid', r.hybrid);
  row('vector', r.vector);
  row('text', r.text);
  const dhv = diffCount(r.hybrid, r.vector);
  const dht = diffCount(r.hybrid, r.text);
  const dvt = diffCount(r.vector, r.text);
  console.log(
    `diff  hybrid vs vector=${dhv}  hybrid vs text=${dht}  vector vs text=${dvt}`
  );
  console.log('---');
}

/**
 * 跑全部 query 对比。
 * @param {object} col VaneCollection
 * @returns {Promise<object[]>} 每条 query 的结果
 */
export async function runComparison(col) {
  const results = [];
  for (const q of QUERIES) {
    results.push(await runOne(col, q));
  }
  return results;
}

/** 打印全部结果。 */
export function printAll(results) {
  for (const r of results) printResult(r);
  // AC4 汇总：至少一组 query 的 hybrid 与 vector/text 均不完全相同
  let fusedDiffers = 0;
  for (const r of results) {
    if (diffCount(r.hybrid, r.vector) > 0 && diffCount(r.hybrid, r.text) > 0) {
      fusedDiffers++;
    }
  }
  console.log(
    `summary: ${fusedDiffers}/${results.length} query(ies) show hybrid distinct from both vector and text`
  );
}

async function main() {
  if (!existsSync(DB_PATH)) {
    console.error(
      `vane-data not found at ${DB_PATH}. Run "npm run load" first.`
    );
    process.exit(1);
  }
  const db = await open(DB_PATH, { autoCommit: 'off' });
  // collection 幂等：打开既有 collection（schema/opts 需与建库时一致）
  const col = await db.collection(COLLECTION, SCHEMA, { tokenizer: 'standard' });
  const results = await runComparison(col);
  printAll(results);
  await db.close();
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((e) => {
    console.error(e);
    process.exit(1);
  });
}
