const test = require('ava');
const { open, VaneError } = require('..');
const os = require('node:os');

function tmp(suffix) {
  return `${os.tmpdir()}/vane-err-${Date.now()}-${suffix}-${Math.random().toString(36).slice(2, 8)}`;
}

test('delete rejects E_UNSUPPORTED (code -10)', async (t) => {
  const db = await open(tmp('del'), {});
  const col = await db.collection(
    'c',
    { fields: [{ name: 'v', type: 'vector', dim: 2 }] },
    {}
  );
  await t.throwsAsync(col.delete(['x']), {
    instanceOf: VaneError,
    code: -10,
    name: 'E_UNSUPPORTED',
  });
  await db.close();
});

test('reindex rejects E_UNSUPPORTED (code -10)', async (t) => {
  const db = await open(tmp('ri'), {});
  const col = await db.collection(
    'c',
    { fields: [{ name: 'v', type: 'vector', dim: 2 }] },
    {}
  );
  await t.throwsAsync(col.reindex(), {
    instanceOf: VaneError,
    code: -10,
    name: 'E_UNSUPPORTED',
  });
  await db.close();
});

test('export rejects E_UNSUPPORTED (code -10)', async (t) => {
  const db = await open(tmp('exp'), {});
  await t.throwsAsync(db.export('/tmp/vane-export-dest'), {
    instanceOf: VaneError,
    code: -10,
    name: 'E_UNSUPPORTED',
  });
  await db.close();
});

test('dim mismatch rejects E_SCHEMA (code -2)', async (t) => {
  const db = await open(tmp('dim'), {});
  const col = await db.collection(
    'c',
    { fields: [{ name: 'v', type: 'vector', dim: 3 }] },
    {}
  );
  await t.throwsAsync(col.add([{ id: 'a', vector: [1, 2] }]), {
    instanceOf: VaneError,
    code: -2,
  });
  await db.close();
});

test('search with filter rejects E_INVALID_ARG (code -11)', async (t) => {
  const db = await open(tmp('flt'), {});
  const col = await db.collection(
    'c',
    { fields: [{ name: 'v', type: 'vector', dim: 2 }] },
    {}
  );
  await col.add([{ id: 'a', vector: [1, 0] }]);
  await col.flush();
  await t.throwsAsync(
    col.search({ vector: [1, 0], filter: { x: 1 } }),
    { instanceOf: VaneError, code: -11 }
  );
  await db.close();
});
