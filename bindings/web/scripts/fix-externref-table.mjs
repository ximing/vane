#!/usr/bin/env node
// wasm-opt 之后、拷进 dist/ 之前调用：
//   node bindings/web/scripts/fix-externref-table.mjs <file.wasm> [more.wasm...]
// 导出 index 已经指向可增长的 externref 表时不改文件。断言失败退出码非 0。

import { fixExternrefFile } from './externref-table.mjs';

const files = process.argv.slice(2);
if (files.length === 0) {
  console.error('usage: node bindings/web/scripts/fix-externref-table.mjs <file.wasm> [more.wasm...]');
  process.exit(1);
}

let failed = false;
for (const file of files) {
  try {
    const info = fixExternrefFile(file);
    const max = info.max == null ? 'none' : String(info.max);
    const action = info.rewritten ? `rewritten ${info.from} -> ${info.tableIndex}` : 'unchanged';
    console.log(
      `OK ${file}: __wbindgen_externrefs table[${info.tableIndex}] externref 0x${info.reftype.toString(16)} min=${info.min} max=${max} flags=0x${info.flags.toString(16)} (${action})`,
    );
  } catch (err) {
    console.error(`FAIL ${file}: ${err.message}`);
    failed = true;
  }
}
if (failed) process.exit(1);
