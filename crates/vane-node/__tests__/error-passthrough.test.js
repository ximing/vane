const test = require('ava');
const { open, VaneError } = require('..');
const os = require('node:os');

function tmp(suffix) {
  return `${os.tmpdir()}/vane-err-${Date.now()}-${suffix}-${Math.random().toString(36).slice(2, 8)}`;
}

// M1 实装：delete 返回 tombstone 计数（BigInt），不再 reject E_UNSUPPORTED。
test('delete returns tombstone count (M1 实装)', async (t) => {
  const db = await open(tmp('del'), {});
  const col = await db.collection(
    'c',
    { fields: [{ name: 'v', type: 'vector', dim: 2 }] },
    {}
  );
  await col.add([{ id: 'x', vector: [1, 0] }]);
  await col.flush();

  // 删除已存在文档 → 返回新置入 tombstone 数（BigInt 1）。
  const n = await col.delete(['x']);
  t.is(typeof n, 'bigint');
  t.is(n, 1n);

  // 重复删除同一 id → 已在 tombstone 位图，不再计数（返回 0）。
  const n2 = await col.delete(['x']);
  t.is(n2, 0n);

  await db.close();
});

// M1 实装（06-userdict-reindex）：reindex 在 set_user_dict 后返回 VaneReindexHandle，
// progress/wait 可调（M1 同步执行：progress=1.0, wait 立即返回）。
// 未先调 set_user_dict 时（Stable 状态）reject E_INVALID_ARG。
test('reindex returns VaneReindexHandle (M1 实装)', async (t) => {
  const db = await open(tmp('ri'), {});
  const col = await db.collection(
    'c',
    { fields: [{ name: 'v', type: 'vector', dim: 2 }] },
    {}
  );
  await col.add([{ id: 'x', vector: [1, 0] }]);
  await col.flush();

  // Stable 状态下 reindex → E_INVALID_ARG（无待重建词表）。
  await t.throwsAsync(col.reindex(), {
    instanceOf: VaneError,
    code: -11,
  });

  // 注入用户词表 → PendingReindex，reindex 同步完成返回 handle。
  await col.setUserDict(['生造词甲']);
  t.is(await col.dictState(), 'pendingReindex');

  const handle = await col.reindex();
  t.is(typeof handle.progress, 'function');
  t.is(typeof handle.wait, 'function');
  // M1 同步执行：reindex 完成后返回已完成的 handle（progress=1.0）。
  t.is(handle.progress(), 1.0);
  // wait 同步返回（M1 立即完成）。
  handle.wait();

  await db.close();
});

// M2-12 实装：export 打包 VANE_SNAP 快照（不再 reject E_UNSUPPORTED）。
test('export succeeds on DB with flushed collection (M2-12 实装)', async (t) => {
  const db = await open(tmp('exp'), {});
  const col = await db.collection(
    'c',
    { fields: [{ name: 'v', type: 'vector', dim: 2 }] },
    {}
  );
  await col.add([{ id: 'x', vector: [1, 0] }]);
  await col.flush();

  // export 到临时文件 → 成功（Promise<void> resolve，不 throw）。
  const dest = tmp('exp-dest');
  await db.export(dest);

  // 验证产物存在 + VANE_SNAP magic（前 9 字节）。
  const fs = require('node:fs');
  const buf = Buffer.alloc(9);
  const fd = fs.openSync(dest, 'r');
  fs.readSync(fd, buf, 0, 9, 0);
  fs.closeSync(fd);
  t.is(buf.toString('ascii'), 'VANE_SNAP');

  // 清理。
  fs.unlinkSync(dest);
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
