// @vane-rs/dict-zh 的 .bin 词典文件作 webpack asset URL 导入。
// webpack 5 asset module（type: 'asset/resource'）将 .bin 解析为资源 URL 字符串。
declare module '*.bin' {
  const src: string;
  export default src;
}
