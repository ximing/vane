#!/usr/bin/env node
// M2-06 §8.4 双变体 Jaccard ≥0.99 硬断言。
//
// 接收两份 Jaccard 探针 JSON（simd / scalar），对每条 (q, mode, tier) 查询的
// topK id 集计算 Jaccard，断言 min Jaccard ≥ 0.99。失败退出码 1（CI 阻断）。
//
// 用法：
//   node scripts/wasm-recall-jaccard.mjs <simd.json> <scalar.json>
//   node scripts/wasm-recall-jaccard.mjs --inline '<simd json>' '<scalar json>'
//
// JSON 格式（由 recall_jaccard_probe 测试产出）：
//   [{"q":0,"mode":"vector","tier":0.1,"topk":["d5","d12",...]}, ...]

import { readFileSync } from 'node:fs';

const THRESHOLD = 0.99;

function loadJson(arg) {
  if (arg.startsWith('[') || arg.startsWith('{')) {
    return JSON.parse(arg);
  }
  return JSON.parse(readFileSync(arg, 'utf8'));
}

function jaccard(a, b) {
  const sa = new Set(a);
  const sb = new Set(b);
  if (sa.size === 0 && sb.size === 0) return 1.0;
  let inter = 0;
  for (const x of sa) if (sb.has(x)) inter++;
  const union = sa.size + sb.size - inter;
  return union === 0 ? 1.0 : inter / union;
}

function key(e) {
  return `${e.q}|${e.mode}|${e.tier}`;
}

function main() {
  const args = process.argv.slice(2);
  if (args.length < 2) {
    console.error('usage: wasm-recall-jaccard.mjs <simd.json> <scalar.json>');
    process.exit(2);
  }
  const simd = loadJson(args[0]);
  const scalar = loadJson(args[1]);

  const simdMap = new Map(simd.map((e) => [key(e), e.topk]));
  const scalarMap = new Map(scalar.map((e) => [key(e), e.topk]));

  let minJ = 1.0;
  const diffs = [];
  for (const [k, simdTopk] of simdMap) {
    const scalarTopk = scalarMap.get(k);
    if (!scalarTopk) {
      console.error(`MISSING scalar entry for ${k}`);
      process.exit(1);
    }
    const j = jaccard(simdTopk, scalarTopk);
    if (j < minJ) minJ = j;
    if (j < THRESHOLD) {
      diffs.push({ key: k, j, simd: simdTopk, scalar: scalarTopk });
    }
  }

  console.log(`Jaccard comparison: ${simdMap.size} queries, min Jaccard = ${minJ.toFixed(6)}`);
  if (diffs.length > 0) {
    console.error(`FAIL: ${diffs.length} queries below threshold ${THRESHOLD}:`);
    for (const d of diffs) {
      console.error(
        `  ${d.key} Jaccard=${d.j.toFixed(6)}\n    simd=${JSON.stringify(d.simd)}\n    scalar=${JSON.stringify(d.scalar)}`
      );
    }
    process.exit(1);
  }
  console.log(`PASS: all queries Jaccard >= ${THRESHOLD}`);
}

main();
