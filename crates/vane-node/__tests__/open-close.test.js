const test = require('ava');
const { open, VaneError } = require('..');
const os = require('node:os');

function tmp(suffix) {
  return `${os.tmpdir()}/vane-oc-${Date.now()}-${suffix}-${Math.random().toString(36).slice(2, 8)}`;
}

test('open + close + collections empty', async (t) => {
  const db = await open(tmp('a'), {});
  t.truthy(db);
  t.deepEqual(db.collections(), []);
  await db.close();
});

test('collection with zero vector fields rejects E_SCHEMA (code -2)', async (t) => {
  const db = await open(tmp('b'), {});
  await t.throwsAsync(
    db.collection('c', { fields: [{ name: 'title', type: 'text' }] }, {}),
    { instanceOf: VaneError, code: -2, name: 'E_SCHEMA' }
  );
  await db.close();
});

test('collection with bad opts (missing fields) rejects E_INVALID_ARG', async (t) => {
  const db = await open(tmp('c'), {});
  await t.throwsAsync(db.collection('c', {}, {}), { instanceOf: VaneError, code: -11 });
  await db.close();
});
