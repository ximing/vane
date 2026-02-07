// Vane Webpack 5 示例配置。
//
// @vane-rs/web 是 ESM 包（package.json "type":"module"），用 new URL(..., import.meta.url)
// 原生处理 wasm/worker asset，init(wasmUrl) 显式 fetch 加载 wasm。
// 设计 §9.3 称 webpack 5 需 experiments.outputModule（ESM 输出），用 init(wasmUrl) 显式
// fetch 可绕过 experiments.asyncWebAssembly 需求——本配置验证此说法。
const path = require('path');
const HtmlWebpackPlugin = require('html-webpack-plugin');

module.exports = {
  // mode 由 CLI --mode flag 设置（build=production, serve=development）
  entry: './src/main.ts',

  // §9.3：ESM 输出。@vane-rs/web 是 ESM 包，worker 需 {type:'module'}。
  // outputModule 使主线程 chunk + worker chunk 均输出为 ESM。
  // 不需要 experiments.asyncWebAssembly——worker 内 init(wasmUrl) 显式 fetch 加载 wasm，
  // 不依赖 webpack 的 wasm 模块导入机制。
  experiments: {
    outputModule: true,
  },

  output: {
    filename: 'index.js',
    path: path.resolve(__dirname, 'dist'),
    clean: true,
    // wasm + bin asset 产出路径（new URL + import 均用此模板）
    assetModuleFilename: 'assets/[name][ext]',
  },

  resolve: {
    extensions: ['.ts', '.js'],
  },

  module: {
    rules: [
      {
        test: /\.ts$/,
        use: 'ts-loader',
        exclude: /node_modules/,
      },
      {
        // §9.4：webpack 5 asset module 处理 .wasm + .bin。
        // new URL('./x.wasm', import.meta.url) 由 webpack 5 原生识别为 asset，
        // 此规则额外覆盖 import dictBinUrl from '.../*.bin' 的直接导入。
        test: /\.(wasm|bin)$/,
        type: 'asset/resource',
      },
    ],
  },

  plugins: [
    new HtmlWebpackPlugin({
      template: './index.html',
      // experiments.outputModule 产出 ESM（import.meta.url），需 type="module" 加载。
      // 默认 'defer' 注入 <script defer>，ESM 代码会 SyntaxError。
      scriptLoading: 'module',
    }),
  ],

  // wasm 产物较大，关闭性能提示（最小示例，非生产优化）
  performance: {
    hints: false,
  },

  devServer: {
    static: {
      directory: path.join(__dirname, 'dist'),
    },
    compress: true,
    port: 8080,
    hot: true,
  },
};
