import DocsLayout from '../../components/DocsLayout';
import CodeBlock from '../../components/CodeBlock';
import Callout from '../../components/Callout';
import './WebIntegration.css';

export default function WebIntegration() {
  return (
    <DocsLayout>
      <article className="webint-page">
        <h1>Web Integration (vite/webpack)</h1>
        <p className="webint-lede">
          <code>@vane-rs/web</code> is an ESM npm package that ships a
          SIMD/scalar dual-variant wasm module, a Web Worker, and the Chinese
          dictionary as a transferable <code>Uint8Array</code>. It loads
          entirely from your bundler output — no CDN, no clone, no build step
          for the consumer. This guide covers installation, the vite and
          webpack 5 configuration you actually need, the import-to-search
          flow, and the pitfalls that bite in dev.
        </p>

        <h2 id="install">Installation</h2>
        <p>
          Both packages are plain npm tarballs. <code>@vane-rs/web</code>{' '}
          declares <code>@vane-rs/dict-zh</code> as an{' '}
          <code>optionalDependency</code>, so a single install pulls the
          dictionary too — zero CDN, zero manual download.
        </p>
        <CodeBlock
          lang="bash"
          title="install"
          code={`npm install @vane-rs/web @vane-rs/dict-zh`}
        />
        <p>
          The exports map exposes three entry points: the main thread API at{' '}
          <code>@vane-rs/web</code>, the worker at{' '}
          <code>@vane-rs/web/worker</code>, and the SIMD probe at{' '}
          <code>@vane-rs/web/probe</code>. You only import the first one
          directly — the worker and probe are resolved internally by{' '}
          <code>createVane</code> via <code>new URL(..., import.meta.url)</code>.
        </p>

        <h2 id="vite">Vite configuration</h2>
        <p>
          Vite 6+ natively understands the{' '}
          <code>new URL(&apos;./x.wasm&apos;, import.meta.url)</code> +{' '}
          <code>new Worker(new URL(...))</code> pattern that{' '}
          <code>@vane-rs/web</code> uses, so there is <strong>no</strong>{' '}
          <code>vite-plugin-wasm</code> and <strong>no</strong> worker plugin
          to install. The only non-zero config item is{' '}
          <code>assetsInclude</code> for the <code>.bin</code> dictionary —
          Vite&apos;s default <code>assetsInclude</code> already covers{' '}
          <code>*.wasm</code> but not <code>*.bin</code>.
        </p>
        <CodeBlock
          lang="ts"
          title="vite.config.ts"
          code={`import { defineConfig } from 'vite';

// assetsInclude：将 @vane-rs/dict-zh 的 .bin 词典文件识别为静态 asset。
// vite 默认 assetsInclude 含 *.wasm 但不含 *.bin，需显式声明。
// 这是唯一的非零配置项，与 wasm/worker 无关。
export default defineConfig({
  assetsInclude: ['**/*.bin'],
});`}
        />
        <table className="webint-table">
          <thead>
            <tr>
              <th>Config item</th>
              <th>Purpose</th>
              <th>Required</th>
            </tr>
          </thead>
          <tbody>
            <tr>
              <td>
                <code>assetsInclude: [&apos;**/*.bin&apos;]</code>
              </td>
              <td>
                Treat the <code>.bin</code> dictionary as a static asset so{' '}
                <code>import dictBinUrl from &apos;.../*.bin&apos;</code>{' '}
                resolves to a URL
              </td>
              <td>Yes (Vite omits <code>.bin</code> by default)</td>
            </tr>
            <tr>
              <td>wasm plugin</td>
              <td>&mdash;</td>
              <td>No (<code>new URL</code> is native)</td>
            </tr>
            <tr>
              <td>worker plugin</td>
              <td>&mdash;</td>
              <td>No (Vite 6+ handles <code>new Worker(new URL(...))</code>)</td>
            </tr>
          </tbody>
        </table>

        <h2 id="webpack">Webpack 5 configuration</h2>
        <p>
          <code>@vane-rs/web</code> is an ESM package (<code>"type":
          "module"</code>), and its worker must be created with{' '}
          <code>{'{ type: "module" }'}</code>. Webpack 5 therefore needs{' '}
          <code>experiments.outputModule</code> to emit ESM chunks for both the
          main thread and the worker. It does <strong>not</strong> need{' '}
          <code>experiments.asyncWebAssembly</code>: the worker loads wasm via{' '}
          <code>init(wasmUrl)</code> with an explicit <code>fetch</code>, so
          webpack only has to emit the <code>.wasm</code> file as an asset —{' '}
          <code>new URL</code> + the <code>asset/resource</code> rule covers
          that.
        </p>
        <CodeBlock
          lang="js"
          title="webpack.config.js"
          code={`const path = require('path');
const HtmlWebpackPlugin = require('html-webpack-plugin');

module.exports = {
  entry: './src/main.ts',

  // ESM 输出：@vane-rs/web 是 ESM 包，worker 需 {type:'module'}。
  // 不需要 asyncWebAssembly——worker 内 init(wasmUrl) 显式 fetch 加载 wasm。
  experiments: { outputModule: true },

  output: {
    filename: 'index.js',
    path: path.resolve(__dirname, 'dist'),
    clean: true,
    assetModuleFilename: 'assets/[name][ext]',
  },

  resolve: { extensions: ['.ts', '.js'] },

  module: {
    rules: [
      { test: /\\.ts$/, use: 'ts-loader', exclude: /node_modules/ },
      // .wasm + .bin 作 asset module（.bin 直接导入需此规则）
      { test: /\\.(wasm|bin)$/, type: 'asset/resource' },
    ],
  },

  plugins: [
    new HtmlWebpackPlugin({
      template: './index.html',
      // ESM 产出用 import.meta.url，须 type="module"（默认 defer 会 SyntaxError）
      scriptLoading: 'module',
    }),
  ],

  performance: { hints: false },
};`}
        />
        <table className="webint-table">
          <thead>
            <tr>
              <th>Config item</th>
              <th>Purpose</th>
              <th>Required</th>
            </tr>
          </thead>
          <tbody>
            <tr>
              <td>
                <code>experiments.outputModule: true</code>
              </td>
              <td>ESM output for main + worker chunks</td>
              <td>Yes</td>
            </tr>
            <tr>
              <td>
                <code>{'{ test: /\\.(wasm|bin)$/, type: "asset/resource" }'}</code>
              </td>
              <td>
                Emit <code>.wasm</code> and <code>.bin</code> as asset URLs
              </td>
              <td>Yes (the <code>.bin</code> import needs it)</td>
            </tr>
            <tr>
              <td>
                <code>HtmlWebpackPlugin.scriptLoading: &apos;module&apos;</code>
              </td>
              <td>
                Inject <code>&lt;script type="module"&gt;</code> for the ESM
                bundle
              </td>
              <td>Yes (default <code>defer</code> throws on <code>import.meta</code>)</td>
            </tr>
            <tr>
              <td>
                <code>experiments.asyncWebAssembly</code>
              </td>
              <td>&mdash;</td>
              <td>No (<code>init(wasmUrl)</code> bypasses webpack wasm imports)</td>
            </tr>
            <tr>
              <td>wasm / worker plugin</td>
              <td>&mdash;</td>
              <td>No (<code>new URL</code> + <code>new Worker(new URL(...))</code> are native)</td>
            </tr>
          </tbody>
        </table>

        <h2 id="usage">Import and retrieval</h2>
        <p>
          The end-to-end flow is the same in vite and webpack: import{' '}
          <code>createVane</code> and the dictionary URL, fetch the{' '}
          <code>.bin</code> bytes into a <code>Uint8Array</code>, hand it to{' '}
          <code>createVane</code> as <code>dictData</code>, then drive the
          usual <code>open &rarr; collection &rarr; add &rarr; flush &rarr;
          search</code> chain. Vane does not bundle an embedding model — the{' '}
          <code>embed()</code> placeholder below stands in for whatever
          embedding API you actually use.
        </p>
        <CodeBlock
          lang="ts"
          title="main.ts"
          code={`import { createVane } from '@vane-rs/web';
import type { Schema, Hit } from '@vane-rs/web';
import dictBinUrl from '@vane-rs/dict-zh/dict.bin';

// 1. 加载词典字节（@vane-rs/dict-zh 本地引用，零 CDN）
const dictData = new Uint8Array(
  await (await fetch(dictBinUrl)).arrayBuffer(),
);

// 2. 创建 Vane 实例（memory VFS；生产可用 'opfs' 持久化）
//    dictData 以 transferable 零拷贝移交 worker，transfer 后主线程不可再访问
const vane = await createVane({
  vfs: 'memory',
  dbPath: 'vane.db',
  dictData,
});

// 3. 打开数据库 + 创建 collection（jieba 分词）
await vane.open();
const schema: Schema = {
  fields: [
    { name: 'text', type: 'text' },
    { name: 'vec', type: 'vector', dim: 64, metric: 'cosine' },
  ],
};
const col = await vane.collection('docs', schema, { tokenizer: 'jieba' });

// 4. 灌入文档并 flush
await vane.add(col, [
  { id: 'd1', text: '向量检索入门指南', vector: embed('向量检索入门指南') },
  { id: 'd2', text: 'BM25 文本检索算法原理', vector: embed('BM25 文本检索算法原理') },
]);
await vane.flush(col);

// 5. 混合检索（文本 + 向量 → RRF 融合）
const hits: Hit[] = await vane.search(col, {
  text: '检索',
  vector: embed('检索'),
  topK: 10,
  mode: 'hybrid',
});

await vane.close();`}
        />

        <h2 id="dictdata">Dictionary: inline first, CDN fallback</h2>
        <p>
          <code>dictData</code> always takes priority: when you pass a{' '}
          <code>Uint8Array</code>, it is moved into the worker as a
          transferable with zero copy and zero network hop. If you omit{' '}
          <code>dictData</code> and <code>dictUrl</code> both,{' '}
          <code>@vane-rs/web</code> falls back to a jsdelivr CDN URL for the
          published dictionary tarball — convenient for quick experiments, but
          it reintroduces a runtime network dependency you probably do not
          want in production. Pass <code>dictData</code> explicitly to stay
          fully offline.
        </p>
        <CodeBlock
          lang="ts"
          title="dict-data.ts"
          code={`// 优先：dictData 内联（本地包，零 CDN，零拷贝 transfer）
import dictBinUrl from '@vane-rs/dict-zh/dict.bin';
import dictSha256Url from '@vane-rs/dict-zh/sha256_prefix.bin';

const dictData = new Uint8Array(
  await (await fetch(dictBinUrl)).arrayBuffer(),
);
// sha256 前缀（8 字节 → 16 hex 字符）用于 worker 内 verify_sha256_prefix 校验
const dictSha256 = Array.from(
  new Uint8Array(await (await fetch(dictSha256Url)).arrayBuffer()),
)
  .map((b) => b.toString(16).padStart(2, '0'))
  .join('');

const vane = await createVane({ vfs: 'memory', dictData, dictSha256 });

// fallback：dictUrl 从 CDN 加载（jsdelivr npm，仅未提供 dictData 时使用）
const vaneCdn = await createVane({
  vfs: 'memory',
  dictUrl: 'https://cdn.jsdelivr.net/npm/@vane-rs/dict-zh@2026.8.0/dict.bin',
});`}
        />

        <h2 id="worker">The worker is internal</h2>
        <p>
          You never construct the worker yourself. <code>createVane</code>{' '}
          encapsulates the standard ESM worker idiom and posts the{' '}
          <code>create</code> message with a <code>Promise</code> boundary:
        </p>
        <CodeBlock
          lang="ts"
          title="inside createVane (informational)"
          code={`// @vane-rs/web 内部实现（用户无需手写）：
const worker = new Worker(
  new URL('./worker.js', import.meta.url),
  { type: 'module' },
);
// worker 内自动：SIMD 探针 → 选 wasm 变体 → init(wasmUrl) → VaneWorker.create(opts)`}
        />
        <p>
          What you do need to provide is a bundler that resolves{' '}
          <code>new URL('./worker.js', import.meta.url)</code> to a separate
          ESM chunk and rewrites <code>import.meta.url</code> correctly inside
          that chunk. Vite 6+ and webpack 5 (with{' '}
          <code>outputModule</code>) both do this natively — that is the only
          reason the configurations above are so short.
        </p>

        <h2 id="gotchas">Common pitfalls</h2>
        <div className="webint-callouts">
          <Callout type="warning" title="file:// will not work">
            Web Workers, OPFS, and ESM <code>import.meta.url</code> all require
            an <code>http://</code> or <code>https://</code> origin. Opening
            <code> index.html</code> directly from disk throws a security/cross-origin
            error. Always develop through <code>vite dev</code> (default port 5173)
            or <code>webpack serve</code> (default port 8080), and serve
            production builds from a static host.
          </Callout>
          <Callout type="note" title="No wasm or worker plugins needed">
            <code>@vane-rs/web</code> uses the <code>new URL(..., import.meta.url)</code>{' '}
            asset idiom plus <code>init(wasmUrl)</code> with an explicit fetch
            inside the worker. Vite 6+ and webpack 5 both recognize this
            pattern natively. Do <strong>not</strong> add{' '}
            <code>vite-plugin-wasm</code>, <code>experiments.asyncWebAssembly</code>,
            or a dedicated worker loader — they are unnecessary and can
            conflict with the inline asset strategy.
          </Callout>
          <Callout type="warning" title="webpack: scriptLoading must be 'module'">
            With <code>experiments.outputModule</code>, the main chunk is ESM
            and references <code>import.meta.url</code>. <code>HtmlWebpackPlugin</code>{' '}
            injects <code>&lt;script defer&gt;</code> by default, which throws a{' '}
            <code>SyntaxError</code> on <code>import.meta</code>. Set{' '}
            <code>scriptLoading: &apos;module&apos;</code> so the tag becomes{' '}
            <code>&lt;script type="module"&gt;</code>.
          </Callout>
          <Callout type="note" title="dictData is transferable — the main-thread buffer detaches">
            When you pass <code>dictData</code>, the underlying{' '}
            <code>ArrayBuffer</code> is transferred to the worker with zero
            copy. After <code>createVane</code> resolves, the{' '}
            <code>Uint8Array</code> on the main thread is{' '}
            <strong>detached</strong> and reading it throws. If you need to
            keep a copy on the main thread, pass <code>dictData.slice()</code>{' '}
            (or <code>new Uint8Array(buf.slice(0))</code>) instead of the
            original buffer.
          </Callout>
          <Callout type="note" title="SIMD/scalar wasm is auto-probed">
            <code>@vane-rs/web</code> ships two wasm variants —{' '}
            <code>vane_wasm_simd.wasm</code> and <code>vane_wasm_bg.wasm</code>{' '}
            (the non-SIMD fallback). The worker runs a runtime{' '}
            <code>simd128Supported()</code> probe and picks the right variant;
            you never configure SIMD by hand. Both variants are emitted as
            assets by vite and webpack automatically.
          </Callout>
        </div>
      </article>
    </DocsLayout>
  );
}
