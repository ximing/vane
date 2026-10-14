#!/usr/bin/env node
// 不启动浏览器。合成模块覆盖 0.4.1 的 Table.grow 失败，再检查 bindings/web/dist/。
// dist 不存在或缺少 simd/scalar 时失败，不跳过。
//
//   cd bindings/web && npm test
//   node bindings/web/scripts/check-externref-table.mjs

import { mkdtempSync, readFileSync, writeFileSync, statSync, rmSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  EXTERNREF,
  FUNCREF,
  EXPORT_NAME,
  describeExternref,
  fixExternrefExport,
  fixExternrefFile,
  parseModule,
  stubImportObject,
  assertExternrefGrows,
  tableExports,
} from './externref-table.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const distDir = path.resolve(here, '..', 'dist');
const buildHint = 'bash bindings/web/scripts/build-web.sh';

function fail(message) {
  throw new Error(message);
}

function assert(cond, message) {
  if (!cond) fail(message);
}

function uleb(value) {
  const out = [];
  do {
    let byte = value & 0x7f;
    value = Math.floor(value / 128);
    if (value !== 0) byte |= 0x80;
    out.push(byte);
  } while (value !== 0);
  return Buffer.from(out);
}

function section(id, payload) {
  return Buffer.concat([Buffer.from([id]), uleb(payload.length), payload]);
}

function wasmName(value) {
  const bytes = Buffer.from(value);
  return Buffer.concat([uleb(bytes.length), bytes]);
}

function limits(min, max) {
  if (max == null) return Buffer.concat([Buffer.from([0x00]), uleb(min)]);
  return Buffer.concat([Buffer.from([0x01]), uleb(min), uleb(max)]);
}

function tableType(reftype, min, max) {
  return Buffer.concat([Buffer.from([reftype]), limits(min, max)]);
}

function moduleBytes({ custom, imports, tables, exports }) {
  const parts = [Buffer.from([0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00])];
  if (custom) parts.push(section(0, custom));
  if (imports) parts.push(section(2, imports));
  parts.push(section(4, Buffer.concat([uleb(tables.length), ...tables])));
  const exportParts = [uleb(exports.length)];
  for (const entry of exports) {
    exportParts.push(wasmName(entry.name), Buffer.from([entry.kind]), entry.indexBytes ?? uleb(entry.index));
  }
  parts.push(section(7, Buffer.concat(exportParts)));
  return Buffer.concat(parts);
}

function tableExport(name, index, indexBytes) {
  return { name, kind: 0x01, index, indexBytes };
}

function instantiate(buf) {
  return new WebAssembly.Instance(new WebAssembly.Module(buf), stubImportObject(buf));
}

function assertGrowsAsExternref(buf, expectedMin) {
  const table = instantiate(buf).exports[EXPORT_NAME];
  assert(table instanceof WebAssembly.Table, 'export is not a table');
  assert(table.length === expectedMin, `initial length ${table.length} != externref min ${expectedMin}`);
  const offset = table.grow(4);
  assert(table.length === expectedMin + 4, 'grow(4) did not stick');
  const obj = { marker: 'externref' };
  table.set(offset, obj);
  assert(table.get(offset) === obj, 'stored value is not an externref');
}

function customSectionPayload(mod) {
  const custom = mod.sections.filter((item) => item.id === 0);
  assert(custom.length === 1, 'expected one custom section');
  return custom[0].payload;
}

function runSynthetic() {
  const funcrefFixed = tableType(FUNCREF, 4, 4);
  const externref = tableType(EXTERNREF, 2, null);
  const marker = Buffer.concat([
    wasmName('marker'),
    Buffer.from('keep-me'),
    Buffer.from(EXPORT_NAME),
  ]);

  const bad = moduleBytes({
    custom: marker,
    tables: [funcrefFixed, externref],
    exports: [tableExport(EXPORT_NAME, 0), tableExport('__wbindgen_externrefs_extra', 0)],
  });
  const badTable = instantiate(bad).exports[EXPORT_NAME];
  let grewBad = false;
  try {
    badTable.grow(4);
    grewBad = true;
  } catch (err) {
    assert(err instanceof RangeError, `expected RangeError, got ${err}`);
  }
  assert(!grewBad, 'min=max funcref table.grow should fail');

  const fixed = fixExternrefExport(bad);
  assert(fixed.rewritten, 'bad export index was not rewritten');
  assert(fixed.from === 0 && fixed.info.tableIndex === 1, 'expected rewrite 0 -> 1');
  assert(!fixed.buf.equals(bad), 'rewritten module is identical to the bad module');
  const fixedCustom = customSectionPayload(parseModule(fixed.buf));
  assert(fixedCustom.equals(marker), 'custom section was modified while rewriting the export');
  assertGrowsAsExternref(fixed.buf, 2);
  const listed = tableExports(fixed.buf);
  assert(listed.find((entry) => entry.name === EXPORT_NAME)?.index === 1, 'exact export name was not retargeted');
  assert(
    listed.find((entry) => entry.name === '__wbindgen_externrefs_extra')?.index === 0,
    'a different export name was rewritten',
  );

  const already = moduleBytes({
    tables: [funcrefFixed, externref],
    exports: [tableExport(EXPORT_NAME, 1)],
  });
  const same = fixExternrefExport(already);
  assert(!same.rewritten && same.buf.equals(already), 'correct export was rewritten');

  // externref 在 table 0 时不能把 index 改成 1。
  const swapped = moduleBytes({
    tables: [externref, funcrefFixed],
    exports: [tableExport(EXPORT_NAME, 1)],
  });
  const swappedFixed = fixExternrefExport(swapped);
  assert(swappedFixed.rewritten && swappedFixed.info.tableIndex === 0, 'did not retarget index 1 to externref 0');
  assertGrowsAsExternref(swappedFixed.buf, 2);
  const swappedOk = moduleBytes({
    tables: [externref, funcrefFixed],
    exports: [tableExport(EXPORT_NAME, 0)],
  });
  assert(fixExternrefExport(swappedOk).buf.equals(swappedOk), 'externref at index 0 was rewritten');

  // index 0 的非最短 LEB（两字节）改成一字节的目标 index，段大小要跟着变。
  const wide = moduleBytes({
    tables: [funcrefFixed, externref],
    exports: [tableExport(EXPORT_NAME, 0, Buffer.from([0x80, 0x00]))],
  });
  const wideFixed = fixExternrefExport(wide);
  assert(wideFixed.rewritten && wideFixed.info.exportIndex === 1, 'wide leb was not rewritten to index 1');
  assert(wideFixed.buf.length === wide.length - 1, 'section size did not shrink with the index leb');
  assertGrowsAsExternref(wideFixed.buf, 2);

  // 目标 index >= 128 时导出 leb 从 1 字节变成 2 字节。
  const many = [tableType(FUNCREF, 0, 0)];
  for (let i = 0; i < 128; i++) many.push(tableType(FUNCREF, 0, 0));
  many.push(tableType(EXTERNREF, 3, null));
  const far = moduleBytes({
    tables: many,
    exports: [tableExport(EXPORT_NAME, 0)],
  });
  const farFixed = fixExternrefExport(far);
  assert(farFixed.info.tableIndex === 129, `expected externref at 129, got ${farFixed.info.tableIndex}`);
  assert(farFixed.rewritten && farFixed.buf.length === far.length + 1, 'far index was not widened');
  assertGrowsAsExternref(farFixed.buf, 3);

  const importPayload = Buffer.concat([
    uleb(1),
    wasmName('env'),
    wasmName('t'),
    Buffer.from([0x01]),
    tableType(FUNCREF, 1, 1),
  ]);
  const imported = moduleBytes({
    imports: importPayload,
    tables: [tableType(EXTERNREF, 5, null)],
    exports: [tableExport(EXPORT_NAME, 0)],
  });
  const importedFixed = fixExternrefExport(imported);
  assert(importedFixed.info.tableIndex === 1 && importedFixed.rewritten, 'import did not shift the externref index');
  assertGrowsAsExternref(importedFixed.buf, 5);

  const growableImport = moduleBytes({
    imports: Buffer.concat([
      uleb(1),
      wasmName('env'),
      wasmName('ext'),
      Buffer.from([0x01]),
      tableType(EXTERNREF, 6, null),
    ]),
    tables: [tableType(FUNCREF, 1, 1)],
    exports: [tableExport(EXPORT_NAME, 1)],
  });
  const importTarget = fixExternrefExport(growableImport);
  assert(importTarget.rewritten && importTarget.info.tableIndex === 0, 'growable imported externref was not selected');
  assertGrowsAsExternref(importTarget.buf, 6);

  const dir = mkdtempSync(path.join(tmpdir(), 'vane-externref-'));
  try {
    const wrongPath = path.join(dir, 'wrong.wasm');
    const okPath = path.join(dir, 'ok.wasm');
    writeFileSync(wrongPath, bad);
    writeFileSync(okPath, already);
    const before = statSync(okPath);
    const written = fixExternrefFile(wrongPath);
    assert(written.rewritten && written.tableIndex === 1, 'fixExternrefFile did not rewrite');
    assertGrowsAsExternref(readFileSync(wrongPath), 2);
    const untouched = fixExternrefFile(okPath);
    assert(!untouched.rewritten, 'fixExternrefFile rewrote a correct module');
    const after = statSync(okPath);
    assert(before.mtimeMs === after.mtimeMs, 'correct wasm file was modified');
    assert(readFileSync(okPath).equals(already), 'correct wasm bytes changed');
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }

  for (const [label, buf] of [
    ['missing export', moduleBytes({ tables: [externref], exports: [tableExport('other', 0)] })],
    [
      'two growable externrefs',
      moduleBytes({
        tables: [tableType(EXTERNREF, 1, null), tableType(EXTERNREF, 1, 4)],
        exports: [tableExport(EXPORT_NAME, 0)],
      }),
    ],
    [
      'externref min=max',
      moduleBytes({
        tables: [tableType(EXTERNREF, 2, 2)],
        exports: [tableExport(EXPORT_NAME, 0)],
      }),
    ],
  ]) {
    let threw = false;
    try {
      fixExternrefExport(buf);
    } catch {
      threw = true;
    }
    assert(threw, `${label} should fail the build`);
  }
  console.log('OK synthetic externref export rewrites');
}

function checkDistFile(file) {
  const buf = readFileSync(file);
  const info = describeExternref(buf);
  assert(
    info.exportIndex === info.tableIndex,
    `${file}: ${EXPORT_NAME} exports table ${info.exportIndex}, growable externref is ${info.tableIndex}`,
  );
  assert(info.reftype === EXTERNREF, `${file}: exported table is not externref 0x6f`);
  assert(info.max == null || info.max > info.min, `${file}: exported table cannot grow (min=${info.min} max=${info.max})`);
  const runtime = assertExternrefGrows(buf);
  assert(
    runtime.lengthBefore === info.min,
    `${file}: instantiated table length ${runtime.lengthBefore} != parsed externref min ${info.min}`,
  );
  const max = info.max == null ? 'none' : String(info.max);
  console.log(
    `OK ${path.basename(file)}: __wbindgen_externrefs table index ${info.tableIndex}, type externref (0x${info.reftype.toString(16)}), limits min=${info.min} max=${max} flags=0x${info.flags.toString(16)}`,
  );
}

function runDist() {
  if (!existsSync(distDir)) {
    fail(`FAIL: ${distDir} 不存在。先跑 ${buildHint}`);
  }
  const required = ['vane_wasm_simd.wasm', 'vane_wasm_scalar.wasm'];
  for (const name of required) {
    const file = path.join(distDir, name);
    if (!existsSync(file)) {
      fail(`FAIL: 缺少 ${file}。先跑 ${buildHint}`);
    }
  }
  const names = [...required];
  const bg = path.join(distDir, 'vane_wasm_bg.wasm');
  if (existsSync(bg)) names.push('vane_wasm_bg.wasm');
  for (const name of names) checkDistFile(path.join(distDir, name));
}

try {
  runSynthetic();
  runDist();
} catch (err) {
  console.error(err.message);
  if (!String(err.message).startsWith('FAIL:')) console.error(err.stack);
  process.exit(1);
}
