// demo 用伪向量（hash-based），生产用真实 embedding API（如 OpenAI/Cohere 本地模型）。
// SPEC Won't-have：Vane 不内置 embedding。本 demo 用 sha256 n-gram bucket 映射到 384 维，
// L2 归一化后送入 vane vector 路。同文本同向量（确定性）；共享词项的文本 cosine 相似度更高。

import { createHash } from 'node:crypto';
import { pathToFileURL } from 'node:url';

/**
 * 将文本映射到固定维度（默认 384）的 L2 归一化伪向量。
 *
 * 设计：
 * - 小写化 → `/[a-z0-9]+/g` 切词
 * - 每个 unigram token：sha256(token) 取前 4 字节为 uint32 → bucket = h % dim，vec[bucket] += 1.0
 * - 每个 bigram（相邻两词）：sha256(w_i + ' ' + w_{i+1}) → bucket = h % dim，vec[bucket] += 0.5
 * - L2 归一化（零向量保持全 0）
 *
 * @param {string} text
 * @param {number} dim 默认 384
 * @returns {number[]} L2 归一化后的 Float32 等价数组（普通 number[]，便于 JSON 序列化进 binding）
 */
export function hashToVector(text, dim = 384) {
  const vec = new Float64Array(dim);
  const tokens = String(text).toLowerCase().match(/[a-z0-9]+/g) || [];
  if (tokens.length === 0) {
    // 全 0 向量，归一化后仍为全 0
    return Array.from(vec);
  }
  // unigram TF
  for (const tok of tokens) {
    const h = sha256U32(tok);
    const bucket = h % dim;
    vec[bucket] += 1.0;
  }
  // bigram TF（权重减半，避免淹没 unigram 信号）
  for (let i = 0; i < tokens.length - 1; i++) {
    const h = sha256U32(tokens[i] + ' ' + tokens[i + 1]);
    const bucket = h % dim;
    vec[bucket] += 0.5;
  }
  // L2 归一化
  let sumSq = 0;
  for (let i = 0; i < dim; i++) sumSq += vec[i] * vec[i];
  if (sumSq > 0) {
    const norm = Math.sqrt(sumSq);
    for (let i = 0; i < dim; i++) vec[i] /= norm;
  }
  return Array.from(vec);
}

/** sha256(s) 前 4 字节 → uint32（小端读取，确定性，与字节序无关因为固定取前 4 字节）。 */
function sha256U32(s) {
  const buf = createHash('sha256').update(s, 'utf8').digest();
  // 取前 4 字节为 uint32（小端）
  return (
    (buf[0] |
      (buf[1] << 8) |
      (buf[2] << 16) |
      (buf[3] << 24)) >>>
    0
  );
}

/** 余弦相似度（向量已 L2 归一化时即点积）。 */
export function cosine(a, b) {
  let s = 0;
  const n = Math.min(a.length, b.length);
  for (let i = 0; i < n; i++) s += a[i] * b[i];
  return s;
}

// ---- inline smoke 自检 ----
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const v1 = hashToVector('hello world');
  const v1b = hashToVector('hello world');
  const v2 = hashToVector('hello there world');
  const v3 = hashToVector('zzzzzz qqqqqq yyyyyy'); // 与 v1 无共享词
  const asserts = [];
  asserts.push(['len=384', v1.length === 384]);
  asserts.push(['deterministic', arraysEqual(v1, v1b)]);
  const cos12 = cosine(v1, v2);
  const cos13 = cosine(v1, v3);
  asserts.push(['shared-words cos>0', cos12 > 0]);
  asserts.push(['no-shared cos≈0', Math.abs(cos13) < 0.01]);
  let norm = 0;
  for (const x of v1) norm += x * x;
  asserts.push(['norm≈1', Math.abs(norm - 1.0) < 1e-6]);
  let ok = true;
  for (const [name, pass] of asserts) {
    console.log(`${pass ? 'OK' : 'FAIL'}  ${name}`);
    if (!pass) ok = false;
  }
  process.exit(ok ? 0 : 1);
}

function arraysEqual(a, b) {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    if (Math.abs(a[i] - b[i]) > 1e-12) return false;
  }
  return true;
}
