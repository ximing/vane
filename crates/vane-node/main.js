// @vane/node JS 入口：VaneError 包装 + 便捷 API（SPEC §9.3 / §10）。
// 纯胶水：无检索逻辑（I-8）。所有 Promise reject 被包成 VaneError(.code/.name)。
//
// napi-rs 生成 `index.js`（平台 require 切换 loader）+ `index.d.ts`（类型）。
// 本文件 require 生成的 loader，再叠加 VaneError 包装层。
'use strict';

const native = require('./index.js');

// VaneError 包装（SPEC §10）：reason 编码 "{code}:{name}:{msg}"，解析回 VaneError 子类。
class VaneError extends Error {
  constructor(message, code, name) {
    super(message);
    this.code = code;
    this.name = name;
  }
}

function wrapErr(p) {
  return p.catch((e) => {
    const msg = e && e.message ? e.message : String(e);
    const m = /^(-?\d+):(\w+):([\s\S]*)$/.exec(msg);
    if (m) throw new VaneError(m[3], Number(m[1]), m[2]);
    throw e;
  });
}

// 给 napi 导出的 class 原型方法套 wrapErr（异步方法返回 Promise）。
function wrapMethods(cls, methods) {
  for (const m of methods) {
    const orig = cls.prototype[m];
    if (typeof orig !== 'function') continue;
    // eslint-disable-next-line no-loop-func
    cls.prototype[m] = function (...args) {
      return wrapErr(orig.apply(this, args));
    };
  }
}

const { VaneDb, VaneCollection } = native;

if (VaneDb) wrapMethods(VaneDb, ['collection', 'close', 'export']);
if (VaneCollection) {
  wrapMethods(VaneCollection, ['add', 'flush', 'search', 'delete', 'reindex']);
}

module.exports = {
  VaneError,
  // open 为顶层便捷函数；VaneDb.open 是 AsyncTask（返回 Promise）。
  open: (path, opts = {}) => wrapErr(VaneDb.open(path, opts)),
  VaneDb,
  VaneCollection,
};
