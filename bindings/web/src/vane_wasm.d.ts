// 类型桥接：让 src/*.ts 能 import './vane_wasm.js' 的类型。
//
// 实际声明在 dist/vane_wasm.d.ts（wasm-bindgen 生成），此文件仅用于 tsc 编译期
// 解析。tsc 不将输入 .d.ts 发射到 outDir，故 dist/vane_wasm.d.ts 保留 wasm-bindgen
// 版本不被覆盖。
//
// ⚠️ 编译前置依赖：dist/vane_wasm.d.ts 必须先由 build-web.sh 的 wasm-bindgen 步骤
// 产出，否则 tsc 报 TS2307 Cannot find module './vane_wasm.js'。
export * from '../dist/vane_wasm.js';
export { default } from '../dist/vane_wasm.js';
