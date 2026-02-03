/**
 * Vane Vite 示例：验证 @vane-rs/web + @vane-rs/dict-zh 在 vite 中零配置可 import + 检索。
 *
 * 链路（设计 §4.3 用法示例）：
 *   1. import createVane from @vane-rs/web
 *   2. import dict.bin / sha256_prefix.bin from @vane-rs/dict-zh（vite asset url）
 *   3. fetch 词典字节 → createVane({ dictData, dictSha256 }) → worker 内零 CDN
 *   4. open → collection(jieba) → add → flush → search → console.log
 *
 * 运行：npm run dev（浏览器打开 http://localhost:5173）
 */

import { createVane } from '@vane-rs/web';
import type { Schema, Hit } from '@vane-rs/web';
import dictBinUrl from '@vane-rs/dict-zh/dict.bin';
import dictSha256Url from '@vane-rs/dict-zh/sha256_prefix.bin';

// ── 占位 hash 向量（SPEC Won't-have：Vane 不内置 embedding）─────────────────
// 简单 char unigram bucket → 64 维 L2 归一化。同文本同向量、共享字符的文本
// cosine 相似度更高，足以演示向量召回。生产应替换为真实 embedding API。
const DIM = 64;

function hashVector(text: string, dim = DIM): number[] {
  const vec = new Float32Array(dim);
  for (const ch of [...text]) {
    const code = ch.codePointAt(0) || 0;
    vec[code % dim] += 1;
  }
  let norm = 0;
  for (let i = 0; i < dim; i++) norm += vec[i] * vec[i];
  norm = Math.sqrt(norm) || 1;
  for (let i = 0; i < dim; i++) vec[i] /= norm;
  return Array.from(vec);
}

// ── 主流程 ────────────────────────────────────────────────────────────────

async function main(): Promise<void> {
  // 1. 加载词典字节（@vane-rs/dict-zh 本地引用，零 CDN）
  console.log('[vane] 加载词典...');
  const dictData = new Uint8Array(await (await fetch(dictBinUrl)).arrayBuffer());
  const sha256Hex = Array.from(
    new Uint8Array(await (await fetch(dictSha256Url)).arrayBuffer()),
  )
    .map((b) => b.toString(16).padStart(2, '0'))
    .join('');
  console.log(`[vane] 词典加载完成（${dictData.byteLength} 字节），sha256 前缀: ${sha256Hex}`);

  // 2. 创建 Vane 实例（memory VFS，避免 OPFS 权限弹窗；生产可用 'opfs' 持久化）
  console.log('[vane] 创建 Vane 实例（memory VFS）...');
  const vane = await createVane({
    vfs: 'memory',
    dbPath: 'vane.db',
    dictData, // transferable 零拷贝，transfer 后主线程不可再访问
    dictSha256: sha256Hex,
  });

  // 3. 打开数据库 + 创建 collection（jieba 分词）
  await vane.open();
  const schema: Schema = {
    fields: [
      { name: 'text', type: 'text' },
      { name: 'vec', type: 'vector', dim: DIM, metric: 'cosine' },
    ],
  };
  const col = await vane.collection('docs', schema, { tokenizer: 'jieba' });
  console.log(`[vane] collection 创建成功, handle: ${col}`);

  // 4. 灌入中文文档
  const docs = [
    { id: 'd1', text: '向量检索入门指南', vector: hashVector('向量检索入门指南') },
    { id: 'd2', text: 'BM25 文本检索算法原理', vector: hashVector('BM25 文本检索算法原理') },
    { id: 'd3', text: 'RRF 融合排序策略', vector: hashVector('RRF 融合排序策略') },
  ];
  const accepted = await vane.add(col, docs);
  await vane.flush(col);
  console.log(`[vane] 灌入 ${accepted} 篇文档并 flush`);

  // 5. 混合检索（文本 + 向量 → RRF 融合）
  const query = '检索';
  const hits: Hit[] = await vane.search(col, {
    text: query,
    vector: hashVector(query),
    topK: 10,
    mode: 'hybrid',
  });
  console.log(`[vane] 搜索 "${query}" 结果（${hits.length} 条）:`);
  for (const hit of hits) {
    const score = hit.score.toFixed(4);
    console.log(`  ${hit.id}  score=${score}  fields=${JSON.stringify(hit.fields)}`);
  }

  // 6. 关闭
  await vane.close();
  console.log('[vane] 已关闭');

  // 渲染到页面
  const app = document.getElementById('app');
  if (app) {
    app.innerHTML = `
      <h1>Vane Vite 示例</h1>
      <p>搜索"${query}"返回 ${hits.length} 条结果：</p>
      <ul>
        ${hits
          .map(
            (h) =>
              `<li><strong>${h.id}</strong> — score: ${h.score.toFixed(4)}</li>`,
          )
          .join('')}
      </ul>
      <p>详见控制台输出（F12）。</p>
    `;
  }
}

main().catch((err) => {
  console.error('[vane] 错误:', err);
  const app = document.getElementById('app');
  if (app) {
    app.innerHTML = `<p style="color:red">错误: ${err.message}</p>`;
  }
});
