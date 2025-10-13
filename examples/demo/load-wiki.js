// load-wiki.js：灌库 1 万条合成英文维基摘要到 Vane。
// demo 用伪向量（hashToVector），生产用真实 embedding API（SPEC Won't-have：Vane 不内置 embedding）。
//
// 流程：open → collection("wiki", schema, {tokenizer:"standard"}) → 分批 add(500/批) → flush → close
// 产出：./vane-data/（manifest.json + segments/seg_*/）

import vane from '@vane/node';
const { open } = vane;
import { hashToVector } from './lib/vector.js';
import { generateWikiAbstracts } from './lib/data.js';
import { rmSync, readdirSync, existsSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));

// 库目录（相对 demo 目录解析为绝对路径，避免 CWD 依赖）
export const DB_PATH = resolve(__dirname, 'vane-data');

// S17 裁决：schema 的 text 字段名统一为 'text'（与 Doc.text API 字段对齐）。
export const SCHEMA = {
  fields: [
    { name: 'text', type: 'text' },
    { name: 'embedding', type: 'vector', dim: 384, metric: 'cosine' },
  ],
};

export const COLLECTION = 'wiki';

const BATCH_SIZE = 500;
const DOC_COUNT = 10000;

/**
 * 构建库：删旧库 → open → collection → 分批 add → flush → close。
 * @returns {Promise<{docCount:number, segmentCount:number, ms:number}>}
 */
export async function buildDb() {
  // 1. 删旧库（幂等）
  if (existsSync(DB_PATH)) {
    rmSync(DB_PATH, { recursive: true, force: true });
  }

  const t0 = Date.now();

  // 2. open（autoCommit off：手动 flush 控制可见性边界，SPEC §7.1）
  const db = await open(DB_PATH, { autoCommit: 'off' });

  // 3. collection
  const col = await db.collection(COLLECTION, SCHEMA, { tokenizer: 'standard' });

  // 4. 生成 1 万条摘要
  const docs = generateWikiAbstracts(DOC_COUNT, 42);

  // 5. 分批 add（每条 text + 伪向量 embedding）
  let accepted = 0;
  for (let i = 0; i < docs.length; i += BATCH_SIZE) {
    const batch = docs.slice(i, i + BATCH_SIZE).map((d) => ({
      id: d.id,
      text: d.text,
      vector: hashToVector(d.text, 384), // 伪向量；生产替换为真实 embedding
    }));
    const r = await col.add(batch);
    accepted += r.accepted;
  }

  // 6. flush（可见性边界）
  await col.flush();

  // 7. close
  await db.close();

  const ms = Date.now() - t0;

  // 8. 统计段数
  const segDir = resolve(DB_PATH, 'segments');
  const segmentCount = existsSync(segDir)
    ? readdirSync(segDir).filter((n) => n.startsWith('seg_')).length
    : 0;

  console.log(`loaded ${accepted} docs in ${ms}ms (${segmentCount} segment(s))`);
  return { docCount: accepted, segmentCount, ms };
}

// main 守卫
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  buildDb().catch((e) => {
    console.error(e);
    process.exit(1);
  });
}
