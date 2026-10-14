// 按导出名修正 __wbindgen_externrefs，不写死文件偏移。
//
// 0.4.1 的坏包把这个 table export 指到了 min=max 的 funcref（index 0）。
// 胶水 __wbindgen_init_externref_table 对它 table.grow(4)，引擎抛
// RangeError。funcref 是 0x70，externref 是 0x6f。table index 含 import
// 段里的表，再接 table 段里定义的表。

import { readFileSync, writeFileSync, renameSync } from 'node:fs';

export const EXPORT_NAME = '__wbindgen_externrefs';
export const EXTERNREF = 0x6f;
export const FUNCREF = 0x70;
const TABLE_EXPORT = 0x01;

const textDecoder = new TextDecoder('utf-8', { fatal: true });

export function readUleb(buf, pos) {
  let result = 0;
  let shift = 0;
  const start = pos;
  while (pos < buf.length) {
    const byte = buf[pos++];
    result += (byte & 0x7f) * 2 ** shift;
    if ((byte & 0x80) === 0) {
      if (!Number.isSafeInteger(result)) {
        throw new Error(`uleb128 at ${start} is not a safe integer`);
      }
      return { value: result, pos };
    }
    shift += 7;
    if (shift > 28) {
      throw new Error(`uleb128 at ${start} is longer than 5 bytes`);
    }
  }
  throw new Error(`truncated uleb128 at ${start}`);
}

export function encodeUleb(value) {
  if (!Number.isInteger(value) || value < 0 || value > Number.MAX_SAFE_INTEGER) {
    throw new Error(`cannot encode uleb128 ${value}`);
  }
  const out = [];
  do {
    let byte = value % 128;
    value = Math.floor(value / 128);
    if (value !== 0) byte |= 0x80;
    out.push(byte);
  } while (value !== 0);
  return Buffer.from(out);
}

function readName(buf, pos) {
  const len = readUleb(buf, pos);
  const end = len.pos + len.value;
  if (end > buf.length) throw new Error(`truncated name at ${pos}`);
  let name;
  try {
    name = textDecoder.decode(buf.subarray(len.pos, end));
  } catch {
    throw new Error(`name at ${pos} is not utf-8`);
  }
  return { name, pos: end };
}

function readLimits(buf, pos, kind) {
  if (pos >= buf.length) throw new Error(`truncated ${kind} limits`);
  const flags = buf[pos++];
  // bit0 max, bit1 shared (memory only), bit2 64-bit index.
  const allowed = kind === 'memory' ? 0b111 : 0b101;
  if ((flags & ~allowed) !== 0) {
    throw new Error(`unsupported ${kind} limits flags 0x${flags.toString(16)}`);
  }
  const min = readUleb(buf, pos);
  pos = min.pos;
  let max = null;
  if ((flags & 0x01) !== 0) {
    const maxR = readUleb(buf, pos);
    max = maxR.value;
    pos = maxR.pos;
  }
  return { flags, min: min.value, max, shared: (flags & 0x02) !== 0, pos };
}

function readTableType(buf, pos) {
  if (pos >= buf.length) throw new Error('truncated table type');
  const reftype = buf[pos++];
  if (reftype !== EXTERNREF && reftype !== FUNCREF) {
    throw new Error(
      `unsupported table reftype 0x${reftype.toString(16)} (expected externref 0x6f or funcref 0x70)`,
    );
  }
  const limits = readLimits(buf, pos, 'table');
  return { reftype, flags: limits.flags, min: limits.min, max: limits.max, pos: limits.pos };
}

function readValType(buf, pos) {
  if (pos >= buf.length) throw new Error('truncated valtype');
  const code = buf[pos++];
  const numeric = {
    0x7f: 'i32',
    0x7e: 'i64',
    0x7d: 'f32',
    0x7c: 'f64',
    0x7b: 'v128',
  };
  if (numeric[code]) return { valueType: numeric[code], pos };
  if (code === EXTERNREF) return { valueType: 'externref', pos };
  if (code === FUNCREF) return { valueType: 'funcref', pos };
  throw new Error(`unsupported valtype 0x${code.toString(16)} at ${pos - 1}`);
}

export function parseModule(buf) {
  if (
    buf.length < 8 ||
    buf[0] !== 0x00 ||
    buf[1] !== 0x61 ||
    buf[2] !== 0x73 ||
    buf[3] !== 0x6d
  ) {
    throw new Error('not a wasm module');
  }
  const version = buf.readUInt32LE(4);
  if (version !== 1) throw new Error(`unsupported wasm version ${version}`);
  const sections = [];
  let pos = 8;
  while (pos < buf.length) {
    const id = buf[pos++];
    if (id > 12) throw new Error(`unknown wasm section id ${id} at ${pos - 1}`);
    const size = readUleb(buf, pos);
    const payloadStart = size.pos;
    const payloadEnd = payloadStart + size.value;
    if (payloadEnd > buf.length) throw new Error(`section ${id} truncated`);
    sections.push({
      id,
      payload: Buffer.from(buf.subarray(payloadStart, payloadEnd)),
      payloadStart,
    });
    pos = payloadEnd;
  }
  return { version, sections };
}

function singleSection(sections, id) {
  const found = sections.filter((section) => section.id === id);
  if (found.length > 1) throw new Error(`duplicate wasm section ${id}`);
  return found[0] ?? null;
}

function parseImportSection(payload) {
  let pos = 0;
  const count = readUleb(payload, pos);
  pos = count.pos;
  const tables = [];
  const imports = [];
  for (let i = 0; i < count.value; i++) {
    const mod = readName(payload, pos);
    pos = mod.pos;
    const nm = readName(payload, pos);
    pos = nm.pos;
    if (pos >= payload.length) throw new Error('truncated import kind');
    const kind = payload[pos++];
    const entry = { module: mod.name, name: nm.name, kind };
    if (kind === 0x00) {
      const typeIndex = readUleb(payload, pos);
      pos = typeIndex.pos;
      entry.kindName = 'func';
    } else if (kind === 0x01) {
      const table = readTableType(payload, pos);
      pos = table.pos;
      entry.kindName = 'table';
      entry.reftype = table.reftype;
      entry.min = table.min;
      entry.max = table.max;
      tables.push({
        index: tables.length,
        reftype: table.reftype,
        flags: table.flags,
        min: table.min,
        max: table.max,
        imported: true,
      });
    } else if (kind === 0x02) {
      const limits = readLimits(payload, pos, 'memory');
      pos = limits.pos;
      entry.kindName = 'memory';
      entry.min = limits.min;
      entry.max = limits.max;
      entry.shared = limits.shared;
    } else if (kind === 0x03) {
      const val = readValType(payload, pos);
      pos = val.pos;
      if (pos >= payload.length) throw new Error('truncated global mutability');
      const mutable = payload[pos++];
      if (mutable !== 0x00 && mutable !== 0x01) {
        throw new Error(`unsupported global mutability 0x${mutable.toString(16)}`);
      }
      entry.kindName = 'global';
      entry.valueType = val.valueType;
      entry.mutable = mutable === 0x01;
    } else {
      throw new Error(`unknown import kind 0x${kind.toString(16)}`);
    }
    imports.push(entry);
  }
  if (pos !== payload.length) {
    throw new Error(`import section has ${payload.length - pos} trailing bytes`);
  }
  return { tables, imports };
}

function parseTableSection(payload, indexBase) {
  let pos = 0;
  const count = readUleb(payload, pos);
  pos = count.pos;
  const tables = [];
  for (let i = 0; i < count.value; i++) {
    const table = readTableType(payload, pos);
    pos = table.pos;
    tables.push({
      index: indexBase + i,
      reftype: table.reftype,
      flags: table.flags,
      min: table.min,
      max: table.max,
      imported: false,
    });
  }
  if (pos !== payload.length) {
    throw new Error(`table section has ${payload.length - pos} trailing bytes`);
  }
  return tables;
}

function parseExports(payload) {
  let pos = 0;
  const count = readUleb(payload, pos);
  pos = count.pos;
  const exports = [];
  for (let i = 0; i < count.value; i++) {
    const nm = readName(payload, pos);
    pos = nm.pos;
    if (pos >= payload.length) throw new Error('truncated export kind');
    const kind = payload[pos++];
    const indexStart = pos;
    const index = readUleb(payload, pos);
    pos = index.pos;
    exports.push({
      name: nm.name,
      kind,
      index: index.value,
      indexStart,
      indexEnd: pos,
    });
  }
  if (pos !== payload.length) {
    throw new Error(`export section has ${payload.length - pos} trailing bytes`);
  }
  return exports;
}

function collectTables(sections) {
  let tables = [];
  let imports = [];
  const importSec = singleSection(sections, 2);
  if (importSec) {
    const parsed = parseImportSection(importSec.payload);
    tables = parsed.tables;
    imports = parsed.imports;
  }
  const tableSec = singleSection(sections, 4);
  if (tableSec) {
    tables = tables.concat(parseTableSection(tableSec.payload, tables.length));
  }
  return { tables, imports };
}

export function tableExports(buf) {
  const exportSec = singleSection(parseModule(buf).sections, 7);
  if (!exportSec) return [];
  return parseExports(exportSec.payload)
    .filter((entry) => entry.kind === TABLE_EXPORT)
    .map((entry) => ({ name: entry.name, index: entry.index }));
}

export function formatTables(tables) {
  if (tables.length === 0) return '(none)';
  return tables
    .map((table) => {
      const ty =
        table.reftype === EXTERNREF
          ? 'externref'
          : table.reftype === FUNCREF
            ? 'funcref'
            : `0x${table.reftype.toString(16)}`;
      const max = table.max == null ? 'none' : String(table.max);
      return `[${table.index}] ${ty} min=${table.min} max=${max}${table.imported ? ' imported' : ''}`;
    })
    .join(', ');
}

function isGrowable(table) {
  if (table.max != null && table.max < table.min) {
    throw new Error(`table ${table.index} has max ${table.max} < min ${table.min}`);
  }
  return table.max == null || table.max > table.min;
}

function findExternrefExport(exports) {
  const named = exports.filter((entry) => entry.name === EXPORT_NAME);
  if (named.length !== 1) {
    throw new Error(`expected exactly one export named ${EXPORT_NAME}, found ${named.length}`);
  }
  const exp = named[0];
  if (exp.kind !== TABLE_EXPORT) {
    throw new Error(`${EXPORT_NAME} is kind ${exp.kind}, expected table (${TABLE_EXPORT})`);
  }
  return exp;
}

export function describeExternref(buf) {
  const mod = parseModule(buf);
  const { tables, imports } = collectTables(mod.sections);
  const exportSec = singleSection(mod.sections, 7);
  if (!exportSec) throw new Error('wasm module has no export section');
  const exp = findExternrefExport(parseExports(exportSec.payload));
  const candidates = tables.filter((table) => table.reftype === EXTERNREF && isGrowable(table));
  if (candidates.length !== 1) {
    throw new Error(
      `expected exactly one growable externref (0x6f) table, found ${candidates.length}; tables: ${formatTables(tables)}`,
    );
  }
  const target = candidates[0];
  return {
    mod,
    imports,
    tables,
    exp,
    exportIndex: exp.index,
    tableIndex: target.index,
    reftype: target.reftype,
    flags: target.flags,
    min: target.min,
    max: target.max,
    growable: true,
  };
}

export function assertExternrefExport(buf) {
  const info = describeExternref(buf);
  if (info.exportIndex !== info.tableIndex) {
    throw new Error(
      `${EXPORT_NAME} exports table ${info.exportIndex}, but the growable externref table is ${info.tableIndex} (${formatTables(info.tables)})`,
    );
  }
  const exported = info.tables.find((table) => table.index === info.exportIndex);
  if (!exported || exported.reftype !== EXTERNREF) {
    throw new Error(`table ${info.exportIndex} is not externref (0x6f)`);
  }
  if (!isGrowable(exported)) {
    throw new Error(
      `table ${info.exportIndex} cannot grow (min=${exported.min} max=${exported.max}); refusing a min=max funcref`,
    );
  }
  return info;
}

function encodeModule(mod) {
  const parts = [Buffer.from([0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00])];
  for (const section of mod.sections) {
    parts.push(Buffer.from([section.id]), encodeUleb(section.payload.length), section.payload);
  }
  return Buffer.concat(parts);
}

function rewriteExportIndex(payload, exp, newIndex) {
  const encoded = encodeUleb(newIndex);
  return Buffer.concat([payload.subarray(0, exp.indexStart), encoded, payload.subarray(exp.indexEnd)]);
}

export function fixExternrefExport(buf) {
  const info = describeExternref(buf);
  if (info.exportIndex === info.tableIndex) {
    assertExternrefExport(buf);
    return { buf, rewritten: false, from: info.exportIndex, info };
  }
  const exportSec = singleSection(info.mod.sections, 7);
  exportSec.payload = rewriteExportIndex(exportSec.payload, info.exp, info.tableIndex);
  const out = encodeModule(info.mod);
  const checked = assertExternrefExport(out);
  if (checked.exportIndex !== info.tableIndex || checked.reftype !== EXTERNREF || !checked.growable) {
    throw new Error('post-rewrite assertion failed for __wbindgen_externrefs');
  }
  return { buf: out, rewritten: true, from: info.exportIndex, info: checked };
}

export function stubImportObject(buf) {
  const { imports } = collectTables(parseModule(buf).sections);
  const obj = {};
  for (const imp of imports) {
    const bucket = (obj[imp.module] ??= {});
    if (imp.kindName === 'func') {
      bucket[imp.name] = () => 0;
    } else if (imp.kindName === 'table') {
      // Node 的 Table descriptor 只认 'anyfunc' / 'externref'，不认 'funcref'。
      const desc = {
        initial: imp.min,
        element: imp.reftype === EXTERNREF ? 'externref' : 'anyfunc',
      };
      if (imp.max != null) desc.maximum = imp.max;
      bucket[imp.name] = new WebAssembly.Table(desc);
    } else if (imp.kindName === 'memory') {
      const desc = { initial: imp.min };
      if (imp.max != null) desc.maximum = imp.max;
      if (imp.shared) desc.shared = true;
      bucket[imp.name] = new WebAssembly.Memory(desc);
    } else if (imp.kindName === 'global') {
      const value = imp.valueType === 'externref' || imp.valueType === 'funcref' ? null : 0;
      const valueType = imp.valueType === 'funcref' ? 'anyfunc' : imp.valueType;
      bucket[imp.name] = new WebAssembly.Global({ value: valueType, mutable: imp.mutable }, value);
    } else {
      throw new Error(`cannot stub import ${imp.module}.${imp.name}`);
    }
  }
  return obj;
}

// 独立于“我们觉得 index 对了”的运行时断言：这张导出表必须能 grow，并且能放下普通对象。
export function assertExternrefGrows(buf) {
  let compiled;
  try {
    compiled = new WebAssembly.Module(buf);
  } catch (err) {
    throw new Error(`wasm failed to compile: ${err.message}`);
  }
  let instance;
  try {
    instance = new WebAssembly.Instance(compiled, stubImportObject(buf));
  } catch (err) {
    throw new Error(`wasm failed to instantiate: ${err.message}`);
  }
  const table = instance.exports[EXPORT_NAME];
  if (!table || typeof table.grow !== 'function') {
    throw new Error(`export ${EXPORT_NAME} is not a WebAssembly.Table`);
  }
  const before = table.length;
  let offset;
  try {
    offset = table.grow(4);
  } catch (err) {
    throw new Error(
      `WebAssembly.Table.grow(4) failed for ${EXPORT_NAME} (length ${before}): ${err.message}. ` +
        'This is the 0.4.1 failure mode when the export points at a min=max funcref table.',
    );
  }
  if (table.length !== before + 4) {
    throw new Error(`table.grow(4) did not increase length (${before} -> ${table.length})`);
  }
  const obj = { vaneExternref: true };
  try {
    table.set(offset, obj);
  } catch (err) {
    throw new Error(`table.set(object) failed; ${EXPORT_NAME} is not an externref table: ${err.message}`);
  }
  if (table.get(offset) !== obj) {
    throw new Error('externref identity was not preserved by __wbindgen_externrefs');
  }
  return { lengthBefore: before, offset };
}

export function fixExternrefFile(file) {
  const original = readFileSync(file);
  const result = fixExternrefExport(original);
  assertExternrefGrows(result.buf);
  if (result.rewritten) {
    const tmp = `${file}.${process.pid}.tmp`;
    writeFileSync(tmp, result.buf);
    renameSync(tmp, file);
  }
  const disk = readFileSync(file);
  if (!disk.equals(result.buf)) {
    throw new Error(`on-disk wasm does not match the checked module: ${file}`);
  }
  const info = assertExternrefExport(disk);
  return {
    file,
    exportIndex: info.exportIndex,
    tableIndex: info.tableIndex,
    reftype: info.reftype,
    flags: info.flags,
    min: info.min,
    max: info.max,
    growable: info.growable,
    rewritten: result.rewritten,
    from: result.from,
  };
}
