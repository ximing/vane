const test = require('ava');
const { open } = require('..');
const os = require('node:os');

function tmp(suffix) {
  return `${os.tmpdir()}/vane-af-${Date.now()}-${suffix}-${Math.random().toString(36).slice(2, 8)}`;
}

const SCHEMA = {
  fields: [
    { name: 'title', type: 'text' },
    { name: 'v', type: 'vector', dim: 3, metric: 'cosine' },
  ],
};

test('add returns AddReport, flush resolves', async (t) => {
  const db = await open(tmp('add'), { autoCommit: 'off' });
  const col = await db.collection('docs', SCHEMA, {});
  const r = await col.add([
    { id: 'a', text: 'hello world', vector: [1, 0, 0] },
    { id: 'b', text: 'hello rust', vector: [0, 1, 0] },
  ]);
  t.is(r.accepted, 2);
  t.true(r.visibleAfterFlush);
  await col.flush();
  await db.close();
});

test('search hybrid returns ranked hits after flush', async (t) => {
  const db = await open(tmp('search'), { autoCommit: 'off' });
  const col = await db.collection('docs', SCHEMA, {});
  await col.add([
    { id: 'a', text: 'hello world', vector: [1, 0, 0] },
    { id: 'b', text: 'hello rust', vector: [0, 1, 0] },
  ]);
  await col.flush();

  const hits = await col.search({
    text: 'hello',
    vector: [1, 0, 0],
    topK: 5,
    mode: 'hybrid',
  });
  t.is(hits.length, 2);
  t.is(hits[0].id, 'a'); // 文档 a 同时命中 text+vector
  t.true(typeof hits[0].score === 'number');
  t.true(hits[0].fields === null || typeof hits[0].fields === 'object');
  await db.close();
});

test('search vector-only', async (t) => {
  const db = await open(tmp('vec'), { autoCommit: 'off' });
  const col = await db.collection('docs', SCHEMA, {});
  await col.add([
    { id: 'a', text: 'x', vector: [1, 0, 0] },
    { id: 'b', text: 'y', vector: [0, 1, 0] },
  ]);
  await col.flush();
  const hits = await col.search({ vector: [1, 0, 0], topK: 2, mode: 'vector' });
  t.is(hits[0].id, 'a');
  await db.close();
});

test('search text-only', async (t) => {
  const db = await open(tmp('txt'), { autoCommit: 'off' });
  const col = await db.collection('docs', SCHEMA, {});
  await col.add([
    { id: 'a', text: 'rust programming', vector: [1, 0, 0] },
    { id: 'b', text: 'go programming', vector: [0, 1, 0] },
  ]);
  await col.flush();
  const hits = await col.search({ text: 'rust', topK: 3, mode: 'text' });
  t.true(hits.map((h) => h.id).includes('a'));
  await db.close();
});
