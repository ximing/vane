const test = require('ava');
const { open } = require('..');
const os = require('node:os');

function tmp(suffix) {
  return `${os.tmpdir()}/vane-inspect-${Date.now()}-${suffix}-${Math.random().toString(36).slice(2, 8)}`;
}

test('db.stats() returns DbStats with correct fields', async (t) => {
  const db = await open(tmp('stats'), { autoCommit: 'off' });
  const col = await db.collection(
    'docs',
    {
      fields: [
        { name: 'body', type: 'text' },
        { name: 'v', type: 'vector', dim: 4, metric: 'cosine' },
      ],
    },
    { tokenizer: 'standard' }
  );

  await col.add([
    { id: '1', text: 'rust programming', vector: [0.9, 0.1, 0.0, 0.0] },
    { id: '2', text: 'go programming', vector: [0.1, 0.9, 0.0, 0.0] },
  ]);
  await col.flush();

  const stats = await db.stats();
  t.truthy(stats.dbPath);
  t.true(Array.isArray(stats.collections));
  t.is(stats.collections.length, 1);

  const colStats = stats.collections[0];
  t.is(colStats.name, 'docs');
  t.is(colStats.segmentCount, 1);
  t.is(colStats.totalDocs, 2);
  t.is(colStats.liveDocs, 2);
  t.is(colStats.tombstonedDocs, 0);
  t.true(colStats.indexBytes > 0);
  t.is(colStats.dictState, 'stable');
  t.is(typeof colStats.tokenizerId, 'string');
  t.is(colStats.tokenizerId.length, 64);
  t.is(colStats.health, 'healthy');

  t.is(typeof stats.dictAvailable, 'boolean');
  t.true(stats.executorKind === 'serial' || stats.executorKind === 'rayon');

  await db.close();
});

test('db.segmentInfo() returns SegmentInfo[] with correct fields', async (t) => {
  const db = await open(tmp('seg'), { autoCommit: 'off' });
  const col = await db.collection(
    'docs',
    {
      fields: [
        { name: 'body', type: 'text' },
        { name: 'v', type: 'vector', dim: 2, metric: 'cosine' },
      ],
    },
    { tokenizer: 'standard' }
  );

  await col.add([
    { id: 'a', text: 'hello world', vector: [1.0, 0.0] },
    { id: 'b', text: 'foo bar', vector: [0.0, 1.0] },
  ]);
  await col.flush();

  const segments = await db.segmentInfo();
  t.true(Array.isArray(segments));
  t.is(segments.length, 1);

  const seg = segments[0];
  t.true(typeof seg.ulid === 'string');
  t.true(seg.ulid.length > 0);
  t.is(seg.docCount, 2);
  t.is(seg.docidBase, 0);
  t.is(seg.tombstonedCount, 0);

  t.truthy(seg.formatVersions);
  t.true(seg.formatVersions.header > 0);
  t.true(seg.formatVersions.vectors > 0);

  t.truthy(seg.fileSizes);
  t.true(seg.fileSizes.header > 0);
  t.true(seg.fileSizes.vectors > 0);

  t.is(seg.health, 'healthy');

  await db.close();
});

test('db.stats() on empty DB returns empty collections', async (t) => {
  const db = await open(tmp('empty'), { autoCommit: 'off' });
  const stats = await db.stats();
  t.truthy(stats.dbPath);
  t.true(Array.isArray(stats.collections));
  t.is(stats.collections.length, 0);

  const segments = await db.segmentInfo();
  t.true(Array.isArray(segments));
  t.is(segments.length, 0);

  await db.close();
});

test('db.stats() after delete reflects tombstoned docs', async (t) => {
  const db = await open(tmp('tomb'), { autoCommit: 'off' });
  const col = await db.collection(
    'docs',
    {
      fields: [
        { name: 'body', type: 'text' },
        { name: 'v', type: 'vector', dim: 2, metric: 'cosine' },
      ],
    },
    { tokenizer: 'standard' }
  );

  await col.add([
    { id: 'a', text: 'hello', vector: [1.0, 0.0] },
    { id: 'b', text: 'world', vector: [0.0, 1.0] },
  ]);
  await col.flush();

  await col.delete(['a']);
  await col.flush();

  const stats = await db.stats();
  const colStats = stats.collections[0];
  t.is(colStats.totalDocs, 2);
  t.is(colStats.tombstonedDocs, 1);
  t.is(colStats.liveDocs, 1);

  await db.close();
});
