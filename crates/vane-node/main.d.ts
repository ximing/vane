// @vane/node 类型声明。main.js 的 JS 侧胶水（VaneError + open）+
// napi-rs 生成的原生类（VaneDb / VaneCollection，见 index.d.ts）。

import { VaneDb, VaneCollection } from './index';

export { VaneDb, VaneCollection };

/** SPEC §10：reject 携带 VaneError，含 .code（透传 core 错误码）与 .name。 */
export declare class VaneError extends Error {
  code: number;
  name: string;
}

/** 顶层便捷函数：打开 Db（StdFsVfs）。返回 Promise<VaneDb>。 */
export declare function open(path: string, opts?: Record<string, unknown>): Promise<VaneDb>;
