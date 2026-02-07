import { defineConfig } from 'vite';

// @vane-rs/web 设计用 new URL(..., import.meta.url) 原生支持 wasm/worker asset，
// 无需 vite-plugin-wasm 或 worker 插件（vite 6+ 原生识别 new URL + Worker 模式）。
//
// assetsInclude：将 @vane-rs/dict-zh 的 .bin 词典文件识别为静态 asset。
// vite 默认 assetsInclude 含 *.wasm 但不含 *.bin，需显式声明。
// 这是唯一的非零配置项，与 wasm/worker 无关。
export default defineConfig({
  assetsInclude: ['**/*.bin'],
});
