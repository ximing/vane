/// <reference types="vite/client" />

// @vane-rs/dict-zh 的 .bin 词典文件作 vite asset URL 导入。
declare module '*.bin' {
  const src: string;
  export default src;
}
