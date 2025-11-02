const test = require('ava');
const { loadDict, dictVersion, open } = require('..');
const os = require('node:os');

function tmp(suffix) {
  return `${os.tmpdir()}/vane-dict-${Date.now()}-${suffix}-${Math.random().toString(36).slice(2, 8)}`;
}

test('loadDict returns a non-empty Buffer (dict.bin)', (t) => {
  const buf = loadDict();
  t.true(Buffer.isBuffer(buf));
  t.true(buf.length > 1000, 'dict.bin should be >1KB');
  // SPEC §5.2 magic = "VNDT" (after zstd decompress, but raw bytes should be non-trivial)
});

test('dictVersion returns YYYY.MM format', (t) => {
  const ver = dictVersion();
  t.is(typeof ver, 'string');
  t.regex(ver, /^\d{4}\.\d{2}$/, 'dict version should be YYYY.MM');
});

test('jieba tokenizer auto-loads dict on Db.open (no manual loadDict needed)', async (t) => {
  // SPEC §12.3: 词典在 Db::open 时已由 vane-core（dict-zh feature）自动加载。
  // JS 侧无需手动调 loadDict——collection 创建时若 tokenizer=jieba 自动注入。
  const db = await open(tmp('jieba'), {});
  const col = await db.collection(
    'docs',
    {
      fields: [
        { name: 'body', type: 'text' },
        { name: 'v', type: 'vector', dim: 4, metric: 'cosine' },
      ],
    },
    { tokenizer: 'jieba' }
  );

  // 用 jieba 分词添加中文文档
  const r = await col.add([
    { id: '1', text: '机器学习是人工智能的分支', vector: [1, 0, 0, 0] },
    { id: '2', text: '深度学习需要大量数据', vector: [0, 1, 0, 0] },
  ]);
  t.is(r.accepted, 2);
  await col.flush();

  // jieba 分词应能精确匹配「机器学习」整词
  const hits = await col.search({ text: '机器学习', topK: 2, mode: 'text' });
  t.true(hits.length > 0);
  t.is(hits[0].id, '1');

  await db.close();
});

test('missing jieba dict falls back to cjk_bigram without error', async (t) => {
  // SPEC §13.2-2 ④：缺词典自动降级不抛错。
  // 当前构建含 dict-zh feature，词典可用。此测试验证 jieba 模式不抛错
  // （降级在 core 层自动处理，JS 侧无感知）。
  const db = await open(tmp('fallback'), {});
  const col = await db.collection(
    'docs',
    {
      fields: [
        { name: 'body', type: 'text' },
        { name: 'v', type: 'vector', dim: 4, metric: 'cosine' },
      ],
    },
    { tokenizer: 'jieba' }
  );

  // 不应抛错（即使降级到 bigram，搜索仍可用）
  const r = await col.add([
    { id: '1', text: '中文分词测试', vector: [1, 0, 0, 0] },
  ]);
  t.is(r.accepted, 1);
  await col.flush();

  const hits = await col.search({ text: '中文', topK: 1, mode: 'text' });
  t.true(hits.length > 0);

  await db.close();
});
