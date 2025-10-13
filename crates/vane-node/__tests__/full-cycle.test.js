const test = require('ava');
const { open } = require('..');
const os = require('node:os');

function tmp(suffix) {
  return `${os.tmpdir()}/vane-full-${Date.now()}-${suffix}-${Math.random().toString(36).slice(2, 8)}`;
}

test('full cycle: open→collection→add→flush→search→close', async (t) => {
  const db = await open(tmp('full'), { autoCommit: 'off' });
  const col = await db.collection(
    'wiki',
    {
      fields: [
        { name: 'title', type: 'text' },
        { name: 'body', type: 'text' },
        { name: 'v', type: 'vector', dim: 4, metric: 'cosine' },
      ],
    },
    { tokenizer: 'standard' }
  );

  const r = await col.add([
    { id: '1', text: 'rust programming language', vector: [0.9, 0.1, 0.0, 0.0] },
    { id: '2', text: 'go programming language', vector: [0.1, 0.9, 0.0, 0.0] },
    { id: '3', text: 'rust memory safety', vector: [0.8, 0.2, 0.0, 0.0] },
  ]);
  t.is(r.accepted, 3);
  await col.flush();

  // vector-only
  const vhits = await col.search({ vector: [0.9, 0.1, 0, 0], topK: 2, mode: 'vector' });
  t.is(vhits[0].id, '1');

  // text-only
  const thits = await col.search({ text: 'rust', topK: 3, mode: 'text' });
  t.true(thits.map((h) => h.id).includes('1'));

  // hybrid
  const hhits = await col.search({
    text: 'rust',
    vector: [0.9, 0.1, 0, 0],
    topK: 3,
    mode: 'hybrid',
  });
  t.is(hhits[0].id, '1');

  t.deepEqual(db.collections(), ['wiki']);
  await db.close();
});
