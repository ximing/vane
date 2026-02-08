import { Link } from 'react-router-dom';
import DocsLayout from '../../components/DocsLayout';
import CodeBlock from '../../components/CodeBlock';
import Callout from '../../components/Callout';
import './api.css';

// ── Type signature source of truth: bindings/web/src/types.ts ──────────────
// 签名直接复制自 types.ts，不臆造、不简化到失真。

const CREATE_VANE_SIG = `export async function createVane(opts?: VaneWorkerOpts): Promise<Vane>`;

const CREATE_VANE_EXAMPLE = `import { createVane } from '@vane-rs/web';
import dictBinUrl from '@vane-rs/dict-zh/dict.bin';

// 1. 加载词典字节（@vane-rs/dict-zh 本地引用，零 CDN）
const dictData = new Uint8Array(
  await (await fetch(dictBinUrl)).arrayBuffer(),
);

// 2. 创建 Vane 实例：内部 new Worker + postMessage create + dictData transferable
//    dictData 以 transferable 零拷贝移交 worker，transfer 后主线程不可再访问该 buffer
const vane = await createVane({
  vfs: 'opfs',
  dbPath: 'vane.db',
  dictData,
});

// 3. 打开数据库 + 创建 collection
await vane.open();
const col = await vane.collection('docs', {
  fields: [{ name: 'text', type: 'text' }],
}, { tokenizer: 'jieba' });

await vane.close();`;

const VANE_INTERFACE_SIG = `export interface Vane {
  // 打开数据库。path 默认 "vane.db"，应与 createVane 的 dbPath 一致。
  open(path?: string, opts?: OpenOptions): Promise<void>;

  // 创建或获取 collection。返回 collection 句柄（u32）。
  // 同名 collection 的 schema 必须一致（幂等）。
  collection(name: string, schema: Schema, opts?: CollectionOptions): Promise<number>;

  // 追加文档。返回 accepted 数量（可能因 schema 约束少于传入数）。
  // col 是 collection() 返回的句柄。
  add(col: number, docs: Doc[]): Promise<number>;

  // 刷新缓冲区，持久化段（可见性边界）。
  flush(col: number): Promise<void>;

  // 搜索。返回 Hit[]（worker 内 JSON 序列化，主线程反序列化）。
  search(col: number, query: SearchQuery): Promise<Hit[]>;

  // 删除文档。返回已删除数量。
  delete(col: number, ids: string[]): Promise<number>;

  // 触发段合并。
  compact(col: number): Promise<void>;

  // 触发 reindex（同步执行）。返回 progress（0.0–1.0，1.0 表示已完成）。
  reindex(col: number): Promise<number>;

  // 导出数据库快照到 VFS 容器内虚拟路径。配合 readFile() 读回字节下载。
  export(dest: string): Promise<void>;

  // 读 VFS 容器内指定虚拟路径的文件字节（配合 export 后下载）。
  readFile(path: string): Promise<Uint8Array>;

  // 关闭 Worker（flush 所有 collection + 注销句柄 + terminate worker 线程）。
  // close() 后再调用任何方法 reject（句柄注销）。
  close(): Promise<void>;
}`;

const VANE_HANDLES_EXAMPLE = `import { createVane } from '@vane-rs/web';

const vane = await createVane({ vfs: 'opfs', dbPath: 'vane.db' });
await vane.open();

// collection() 返回 Promise<number>——这是 collection 句柄
const col = await vane.collection('docs', {
  fields: [
    { name: 'text', type: 'text' },
    { name: 'vec', type: 'vector', dim: 64 },
  ],
});

// 所有后续 verb 第一参数是 col: number
await vane.add(col, [{ id: 'd1', text: 'hello', vector: [0.1, 0.2] }]);
await vane.flush(col);
const hits = await vane.search(col, { text: 'hello', topK: 10 });
await vane.delete(col, ['d1']);
await vane.compact(col);

await vane.close();`;

const WORKER_OPTS_SIG = `export type VfsKind = 'opfs' | 'idb' | 'memory';

export interface VaneWorkerOpts {
  vfs?: VfsKind;
  dbPath?: string;
  dictData?: Uint8Array | ArrayBuffer;
  dictUrl?: string;
  dictSha256?: string;
}`;

const SCHEMA_SIG = `export type VectorMetric = 'cosine' | 'l2' | 'dot';

export interface TextFieldSchema {
  name: string;
  type: 'text';
}

export interface VectorFieldSchema {
  name: string;
  type: 'vector';
  dim: number;        // 向量维度（必填）
  metric?: VectorMetric;  // 默认 "cosine"
}

export interface ScalarFieldSchema {
  name: string;
  type: 'scalar';
  kind?: 'int' | 'float' | 'bool' | 'keyword';  // 默认 "keyword"
}

/** 字段定义（判别联合：type 决定可选字段） */
export type FieldSchema = TextFieldSchema | VectorFieldSchema | ScalarFieldSchema;

export interface Schema {
  fields: FieldSchema[];
}`;

const SCHEMA_EXAMPLE = `import type { Schema } from '@vane-rs/web';

const schema: Schema = {
  fields: [
    // 文本字段 → BM25 倒排索引
    { name: 'title', type: 'text' },
    { name: 'body', type: 'text' },
    // 向量字段 → HNSW 索引（dim 必填，metric 默认 cosine）
    { name: 'vec', type: 'vector', dim: 384, metric: 'cosine' },
    // 标量字段 → 可过滤元数据
    { name: 'lang', type: 'scalar', kind: 'keyword' },
    { name: 'year', type: 'scalar', kind: 'int' },
  ],
};`;

const DOC_SIG = `export interface Doc {
  id: string;
  text?: string;
  vector?: number[];
  meta?: Record<string, number | boolean | string>;
}`;

const DOC_EXAMPLE = `// id 必填；text/vector/meta 可选（但至少与 schema 字段对应）
await vane.add(col, [
  {
    id: 'd1',
    text: '向量检索入门指南',
    vector: [0.1, 0.2, /* ... 384 维 */],
    meta: { lang: 'zh', year: 2025 },
  },
  { id: 'd2', text: 'BM25 文本检索算法' },  // 纯文本，无向量
]);`;

const AUX_TYPES = `export type SearchMode = 'hybrid' | 'vector' | 'text' | 'auto';
export type FusionSpec = 'rrf' | { linear: { alpha?: number } };
export type AutoCommit = 'off' | { intervalMs?: number; maxDocs?: number };`;

const SEARCH_QUERY_SIG = `export interface SearchQuery {
  text?: string;
  vector?: number[];
  topK?: number;
  mode?: SearchMode;
  fusion?: FusionSpec;
  candidateMultiplier?: number;
}`;

const HIT_SIG = `export interface Hit {
  id: string;
  score: number;
  fields: Record<string, string> | null;
}`;

const OPEN_OPTS_SIG = `export type PersistenceMode = 'persistent' | 'best-effort';

export interface OpenOptions {
  persistence?: PersistenceMode;
  autoCommit?: AutoCommit;
  pageCacheMb?: number;
}`;

const COLLECTION_OPTS_SIG = `export type TokenizerKind = 'jieba' | 'cjk_bigram' | 'standard';
export type UserDictEntry = string | { term: string; freq: number };

export interface CollectionOptions {
  tokenizer?: TokenizerKind;
  userDict?: UserDictEntry[];
  autoCommit?: AutoCommit;
}`;

const SIMD_PROBE_SIG = `// @vane-rs/web/probe
export const SIMD128_TEST_MODULE: Uint8Array;

export function simd128Supported(): boolean;`;

const SIMD_PROBE_EXAMPLE = `import { simd128Supported } from '@vane-rs/web/probe';

// 可选：主线程提前探测 SIMD128 支持情况（worker init 内部已自动调用）
const hasSimd = simd128Supported();
console.log('SIMD128 supported:', hasSimd);
// true  → worker 加载 vane_wasm_simd.wasm
// false → worker 加载 vane_wasm_bg.wasm（scalar 变体）`;

const DICT_INLINE = `// 优先：dictData 内联（@vane-rs/dict-zh 本地包，零 CDN，零拷贝 transfer）
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

const vane = await createVane({ vfs: 'memory', dictData, dictSha256 });`;

const DICT_CDN = `// fallback：未提供 dictData 且未指定 dictUrl 时，
// @vane-rs/web 层自动填入 jsdelivr CDN 默认 URL
const vane = await createVane({ vfs: 'memory' });
// 等价于 createVane({ vfs: 'memory', dictUrl: DEFAULT_DICT_URL })

// 或显式指定 dictUrl + dictSha256（自托管 CDN / 内网镜像）
const vaneSelf = await createVane({
  vfs: 'memory',
  dictUrl: 'https://cdn.example.com/dict-zh/dict.bin',
  dictSha256: 'a1b2c3d4e5f6a7b8',
});`;

const WORKER_INTERNALS = `// @vane-rs/web 内部实现（用户无需手写）：
const worker = new Worker(
  new URL('./worker.js', import.meta.url),
  { type: 'module' },
);
// worker 内自动：SIMD 探针 → 选 wasm 变体 → init(wasmUrl) → VaneWorker.create(opts)

// postMessage Promise 边界：
//   pending Map<number, { resolve, reject }>
//   call(op, payload, transfer) → { op, id, ...payload } → 等 { id, result|error }
//   close() 后 reject 所有后续调用（句柄注销）`;

export default function ApiWeb() {
  return (
    <DocsLayout>
      <div className="api-page">
        <h1>Web (@vane-rs/web)</h1>
        <p className="api-lead">
          The browser binding of Vane, distributed as the{' '}
          <code>@vane-rs/web</code> npm package. It ships a SIMD/scalar
          dual-variant wasm module, a Web Worker, and the Chinese dictionary as
          a transferable <code>Uint8Array</code>. The main-thread API is a
          single <code>createVane</code> factory returning a{' '}
          <code>Vane</code> instance whose every method is a{' '}
          <code>Promise</code> routed to the worker via{' '}
          <code>postMessage</code>. This page is the package-level type
          reference; for installation and bundler configuration see the{' '}
          <Link to="/guides/web-integration">Web Integration</Link> guide.
        </p>

        <h2 id="createvane">createVane</h2>
        <p>
          The factory is the only export you call directly. It encapsulates
          three things: constructing the ESM worker via{' '}
          <code>new Worker(new URL(&apos;./worker.js&apos;, import.meta.url),{' '}
          {'{ type: "module" }'})</code>, posting a <code>create</code> message
          with a <code>postMessage</code> Promise boundary, and transferring{' '}
          <code>dictData</code> zero-copy into the worker. When neither{' '}
          <code>dictData</code> nor <code>dictUrl</code> is provided, the layer
          auto-fills the jsdelivr CDN default URL.
        </p>
        <CodeBlock
          lang="ts"
          title="createVane signature"
          code={CREATE_VANE_SIG}
        />
        <CodeBlock
          lang="ts"
          title="minimal usage"
          code={CREATE_VANE_EXAMPLE}
        />

        <h2 id="vane-interface">Vane interface</h2>
        <p>
          <code>createVane</code> returns a <code>Vane</code> instance. Every
          method is a <code>Promise</code> — internally each call posts a
          message to the worker and awaits the matching response by id. After{' '}
          <code>close()</code> any subsequent call rejects (handle revoked).
        </p>
        <Callout type="warning" title="collection returns a number handle, not an object">
          Unlike the Node and Go bindings where <code>collection()</code>{' '}
          returns a <code>VaneCollection</code> object, the browser binding
          returns <code>Promise&lt;number&gt;</code> — a u32 handle. Every
          subsequent verb (<code>add</code>, <code>flush</code>,{' '}
          <code>search</code>, <code>delete</code>, <code>compact</code>,{' '}
          <code>reindex</code>) takes <code>col: number</code> as its first
          argument. The handle is valid until <code>close()</code>.
        </Callout>
        <CodeBlock
          lang="ts"
          title="Vane interface (types.ts)"
          code={VANE_INTERFACE_SIG}
        />
        <CodeBlock
          lang="ts"
          title="handle usage pattern"
          code={VANE_HANDLES_EXAMPLE}
        />

        <h2 id="vane-workeropts">VaneWorkerOpts</h2>
        <p>
          Options passed to <code>createVane</code>. They configure the VFS
          backend, database path, and dictionary loading strategy. Dictionary
          resolution priority: <code>dictData</code> (inline, zero-copy){' '}
          &rarr; <code>dictUrl</code> (explicit) &rarr; auto-filled CDN default.
        </p>
        <CodeBlock
          lang="ts"
          title="VaneWorkerOpts (types.ts)"
          code={WORKER_OPTS_SIG}
        />
        <div className="api-table-wrap">
          <table className="api-table">
            <thead>
              <tr>
                <th>Field</th>
                <th>Type</th>
                <th>Default</th>
                <th>Meaning</th>
              </tr>
            </thead>
            <tbody>
              <tr>
                <td><code>vfs</code></td>
                <td className="api-table__wide"><code>VfsKind</code></td>
                <td><code>&apos;opfs&apos;</code></td>
                <td className="api-table__wide">
                  VFS backend. <code>&apos;opfs&apos;</code> uses the Origin
                  Private File System; when OPFS is unavailable it
                  auto-degrades to IDB &rarr; memory. <code>&apos;idb&apos;</code>{' '}
                  and <code>&apos;memory&apos;</code> can be set explicitly.
                </td>
              </tr>
              <tr>
                <td><code>dbPath</code></td>
                <td><code>string</code></td>
                <td><code>&apos;vane.db&apos;</code></td>
                <td className="api-table__wide">
                  Database logical path. Under OPFS this is also the filename
                  inside the OPFS root. Should match the <code>path</code>{' '}
                  passed to <code>open()</code>.
                </td>
              </tr>
              <tr>
                <td><code>dictData</code></td>
                <td className="api-table__wide"><code>Uint8Array | ArrayBuffer</code></td>
                <td>—</td>
                <td className="api-table__wide">
                  Dictionary bytes (zstd-compressed <code>dict.bin</code>).
                  Takes priority over <code>dictUrl</code>. Transferred
                  zero-copy to the worker; the main-thread buffer is{' '}
                  <strong>detached</strong> after <code>createVane</code>{' '}
                  resolves.
                </td>
              </tr>
              <tr>
                <td><code>dictUrl</code></td>
                <td><code>string</code></td>
                <td>CDN default</td>
                <td className="api-table__wide">
                  CDN fallback URL for the dictionary. When neither{' '}
                  <code>dictData</code> nor <code>dictUrl</code> is provided,
                  @vane-rs/web auto-fills the jsdelivr URL for{' '}
                  <code>@vane-rs/dict-zh@2026.8.0/dict.bin</code>.
                </td>
              </tr>
              <tr>
                <td><code>dictSha256</code></td>
                <td><code>string</code></td>
                <td>—</td>
                <td className="api-table__wide">
                  16-character hex (sha256 first 8 bytes). Used by the worker&apos;s{' '}
                  <code>verify_sha256_prefix</code> to detect truncated or
                  corrupted dictionary downloads. Recommended whenever{' '}
                  <code>dictUrl</code> is used.
                </td>
              </tr>
            </tbody>
          </table>
        </div>

        <h2 id="schema">Schema &amp; FieldSchema</h2>
        <p>
          A schema is a list of named fields declared at collection creation
          time. <code>FieldSchema</code> is a discriminated union: the{' '}
          <code>type</code> field determines which optional fields are present.
          After creation only appendix-style extension is allowed; modifying or
          removing existing fields is forbidden.
        </p>
        <CodeBlock
          lang="ts"
          title="Schema & FieldSchema (types.ts)"
          code={SCHEMA_SIG}
        />
        <div className="api-table-wrap">
          <table className="api-table">
            <thead>
              <tr>
                <th>Branch</th>
                <th>Discriminator</th>
                <th>Extra fields</th>
                <th>Meaning</th>
              </tr>
            </thead>
            <tbody>
              <tr>
                <td><code>TextFieldSchema</code></td>
                <td><code>type: &apos;text&apos;</code></td>
                <td>—</td>
                <td className="api-table__wide">
                  Feeds the BM25 inverted index. Multiple text fields allowed;
                  a pure-vector collection (no text fields) is legal.
                </td>
              </tr>
              <tr>
                <td><code>VectorFieldSchema</code></td>
                <td><code>type: &apos;vector&apos;</code></td>
                <td className="api-table__wide">
                  <code>dim: number</code> (required),{' '}
                  <code>metric?: VectorMetric</code>
                </td>
                <td className="api-table__wide">
                  HNSW vector index. <strong>Exactly one</strong> vector field
                  per collection (M0–M2 limit). <code>dim</code> &le; 4096;{' '}
                  <code>metric</code> defaults to <code>&apos;cosine&apos;</code>.
                </td>
              </tr>
              <tr>
                <td><code>ScalarFieldSchema</code></td>
                <td><code>type: &apos;scalar&apos;</code></td>
                <td className="api-table__wide">
                  <code>kind?: &apos;int&apos; | &apos;float&apos; | &apos;bool&apos; | &apos;keyword&apos;</code>
                </td>
                <td className="api-table__wide">
                  Filterable metadata. Any number of scalar fields;{' '}
                  <code>kind</code> defaults to <code>&apos;keyword&apos;</code>.
                </td>
              </tr>
            </tbody>
          </table>
        </div>
        <CodeBlock
          lang="ts"
          title="schema with all three field types"
          code={SCHEMA_EXAMPLE}
        />

        <h2 id="doc">Doc</h2>
        <p>
          A document to be added via <code>vane.add(col, docs)</code>.{' '}
          <code>id</code> is required; <code>text</code>, <code>vector</code>,
          and <code>meta</code> are optional but should correspond to declared
          schema fields. <code>meta</code> values support{' '}
          <code>number</code>, <code>boolean</code>, and{' '}
          <code>string</code> (mapped to <code>ScalarValue</code> in the core).
        </p>
        <CodeBlock
          lang="ts"
          title="Doc (types.ts)"
          code={DOC_SIG}
        />
        <CodeBlock
          lang="ts"
          title="add documents"
          code={DOC_EXAMPLE}
        />

        <h2 id="searchquery">SearchQuery</h2>
        <p>
          A single query object drives all three recall paths — BM25 text,
          vector similarity, or both fused. At least one of{' '}
          <code>text</code> / <code>vector</code> must be provided; passing
          both means hybrid. The browser binding does not support the{' '}
          <code>filter</code> field (see the gap note below).
        </p>
        <CodeBlock
          lang="ts"
          title="auxiliary types"
          code={AUX_TYPES}
        />
        <CodeBlock
          lang="ts"
          title="SearchQuery (types.ts)"
          code={SEARCH_QUERY_SIG}
        />
        <div className="api-table-wrap">
          <table className="api-table">
            <thead>
              <tr>
                <th>Field</th>
                <th>Type</th>
                <th>Default</th>
                <th>Meaning</th>
              </tr>
            </thead>
            <tbody>
              <tr>
                <td><code>text</code></td>
                <td><code>string</code></td>
                <td>—</td>
                <td className="api-table__wide">
                  Input for the BM25 path. At least one of{' '}
                  <code>text</code> / <code>vector</code> is required.
                </td>
              </tr>
              <tr>
                <td><code>vector</code></td>
                <td><code>number[]</code></td>
                <td>—</td>
                <td className="api-table__wide">
                  Input for the vector path. Dimension must equal the schema{' '}
                  <code>dim</code>.
                </td>
              </tr>
              <tr>
                <td><code>topK</code></td>
                <td><code>number</code></td>
                <td><code>10</code></td>
                <td className="api-table__wide">
                  Number of hits to return. Maximum 1000; larger values fail
                  with <code>E_INVALID_ARG</code> (-11).
                </td>
              </tr>
              <tr>
                <td><code>mode</code></td>
                <td className="api-table__wide"><code>SearchMode</code></td>
                <td><code>&apos;auto&apos;</code></td>
                <td className="api-table__wide">
                  Which recall paths run. <code>&apos;auto&apos;</code> infers
                  from inputs; an explicit value wins.
                </td>
              </tr>
              <tr>
                <td><code>fusion</code></td>
                <td className="api-table__wide"><code>FusionSpec</code></td>
                <td><code>&apos;rrf&apos;</code></td>
                <td className="api-table__wide">
                  How the two paths merge in hybrid mode.{' '}
                  <code>&apos;rrf&apos;</code> is zero-tuning;{' '}
                  <code>{'{ linear: { alpha } }'}</code> blends with a weight{' '}
                  <code>alpha</code> (default 0.5).
                </td>
              </tr>
              <tr>
                <td><code>candidateMultiplier</code></td>
                <td><code>number</code></td>
                <td><code>3</code></td>
                <td className="api-table__wide">
                  In hybrid mode each path recalls{' '}
                  <code>topK &times; candidateMultiplier</code> candidates
                  before fusion.
                </td>
              </tr>
            </tbody>
          </table>
        </div>
        <div className="api-table-wrap">
          <table className="api-table">
            <thead>
              <tr>
                <th>FusionSpec branch</th>
                <th>Shape</th>
                <th>Meaning</th>
              </tr>
            </thead>
            <tbody>
              <tr>
                <td><code>&apos;rrf&apos;</code></td>
                <td className="api-table__wide">string literal</td>
                <td className="api-table__wide">
                  Reciprocal Rank Fusion (<code>Σ 1/(60 + rank)</code>).
                  Zero-tuning, the default.
                </td>
              </tr>
              <tr>
                <td><code>{'{ linear: { alpha? } }'}</code></td>
                <td className="api-table__wide">
                  <code>{'{ linear: { alpha?: number } }'}</code>
                </td>
                <td className="api-table__wide">
                  Linear blend after minmax normalization.{' '}
                  <code>alpha</code> defaults to 0.5 (vector weight). Scores
                  are per-query normalized and not comparable across corpora.
                </td>
              </tr>
            </tbody>
          </table>
        </div>
        <Callout type="gap" title="filter is not supported on the wasm side">
          The <code>SearchQuery</code> type in <code>@vane-rs/web</code> does
          not include a <code>filter</code> field. Metadata filtering is
          implemented in the Rust core and exposed through the Node and Go
          bindings, but the wasm worker&apos;s{' '}
          <code>parse_search_query</code> rejects <code>filter</code> with a{' '}
          <code>VaneError</code>. Do not pass <code>filter</code> to{' '}
          <code>vane.search()</code>. This is a binding-completeness gap
          tracked for a future release.
        </Callout>

        <h2 id="hit">Hit</h2>
        <p>
          A search result. <code>id</code> is the external document id;{' '}
          <code>score</code> is the fusion score (RRF or linear-normalized);{' '}
          <code>fields</code> is the stored-fields map, which may be{' '}
          <code>null</code> when no fields were stored.
        </p>
        <CodeBlock
          lang="ts"
          title="Hit (types.ts)"
          code={HIT_SIG}
        />

        <h2 id="openoptions">OpenOptions</h2>
        <p>
          Options for <code>vane.open(path, opts)</code>. Controls persistence
          durability, auto-commit cadence, and page cache size.
        </p>
        <CodeBlock
          lang="ts"
          title="OpenOptions (types.ts)"
          code={OPEN_OPTS_SIG}
        />
        <div className="api-table-wrap">
          <table className="api-table">
            <thead>
              <tr>
                <th>Field</th>
                <th>Type</th>
                <th>Default</th>
                <th>Meaning</th>
              </tr>
            </thead>
            <tbody>
              <tr>
                <td><code>persistence</code></td>
                <td className="api-table__wide"><code>PersistenceMode</code></td>
                <td><code>&apos;persistent&apos;</code></td>
                <td className="api-table__wide">
                  <code>&apos;persistent&apos;</code> durably flushes to the
                  VFS; <code>&apos;best-effort&apos;</code> may lose recent
                  writes on tab close.
                </td>
              </tr>
              <tr>
                <td><code>autoCommit</code></td>
                <td className="api-table__wide"><code>AutoCommit</code></td>
                <td className="api-table__wide">
                  <code>{'{ intervalMs: 1000, maxDocs: 1000 }'}</code>
                </td>
                <td className="api-table__wide">
                  Auto-commit cadence. <code>&apos;off&apos;</code> disables
                  auto-commit (call <code>flush</code> manually); an object
                  sets the interval and document-count threshold.
                </td>
              </tr>
              <tr>
                <td><code>pageCacheMb</code></td>
                <td><code>number</code></td>
                <td><code>32</code></td>
                <td className="api-table__wide">
                  Page cache size in megabytes.
                </td>
              </tr>
            </tbody>
          </table>
        </div>
        <div className="api-table-wrap">
          <table className="api-table">
            <thead>
              <tr>
                <th>AutoCommit branch</th>
                <th>Shape</th>
                <th>Meaning</th>
              </tr>
            </thead>
            <tbody>
              <tr>
                <td><code>&apos;off&apos;</code></td>
                <td className="api-table__wide">string literal</td>
                <td className="api-table__wide">
                  No auto-commit. Call <code>flush()</code> to make writes
                  visible.
                </td>
              </tr>
              <tr>
                <td><code>{'{ intervalMs?, maxDocs? }'}</code></td>
                <td className="api-table__wide">
                  <code>{'{ intervalMs?: number; maxDocs?: number }'}</code>
                </td>
                <td className="api-table__wide">
                  Auto-commit when either threshold is reached. Defaults:{' '}
                  <code>intervalMs: 1000</code>, <code>maxDocs: 1000</code>.
                </td>
              </tr>
            </tbody>
          </table>
        </div>

        <h2 id="collectionoptions">CollectionOptions</h2>
        <p>
          Options for <code>vane.collection(name, schema, opts)</code>. Sets the
          tokenizer identity, user dictionary entries, and auto-commit for the
          collection.
        </p>
        <Callout type="note" title="No dictData field here">
          <code>CollectionOptions</code> does not have a <code>dictData</code>{' '}
          field. The dictionary is loaded at <code>createVane</code> time via{' '}
          <code>VaneWorkerOpts.dictData</code> / <code>dictUrl</code>, not per
          collection. <code>userDict</code> here is for additional domain terms
          injected into the jieba dictionary at collection creation.
        </Callout>
        <CodeBlock
          lang="ts"
          title="CollectionOptions (types.ts)"
          code={COLLECTION_OPTS_SIG}
        />
        <div className="api-table-wrap">
          <table className="api-table">
            <thead>
              <tr>
                <th>Field</th>
                <th>Type</th>
                <th>Default</th>
                <th>Meaning</th>
              </tr>
            </thead>
            <tbody>
              <tr>
                <td><code>tokenizer</code></td>
                <td className="api-table__wide"><code>TokenizerKind</code></td>
                <td><code>&apos;standard&apos;</code></td>
                <td className="api-table__wide">
                  Built-in tokenizer identity for all text fields.{' '}
                  <code>&apos;jieba&apos;</code> without a loaded dictionary
                  auto-degrades to <code>&apos;cjk_bigram&apos;</code> (no
                  error). Recorded per segment; changing it requires a reindex.
                </td>
              </tr>
              <tr>
                <td><code>userDict</code></td>
                <td className="api-table__wide"><code>UserDictEntry[]</code></td>
                <td>—</td>
                <td className="api-table__wide">
                  Domain terms injected into the jieba dictionary at creation.
                  A bare string gets the highest built-in frequency; an object{' '}
                  <code>{'{ term, freq }'}</code> sets an explicit frequency.
                  Over 100k entries fails with <code>E_DICT_TOO_LARGE</code>{' '}
                  (-7).
                </td>
              </tr>
              <tr>
                <td><code>autoCommit</code></td>
                <td className="api-table__wide"><code>AutoCommit</code></td>
                <td className="api-table__wide">
                  <code>{'{ intervalMs: 1000, maxDocs: 1000 }'}</code>
                </td>
                <td className="api-table__wide">
                  Same shape as <code>OpenOptions.autoCommit</code>. Per-collection
                  auto-commit cadence.
                </td>
              </tr>
            </tbody>
          </table>
        </div>

        <h2 id="simd-probe">SIMD probe</h2>
        <p>
          <code>@vane-rs/web</code> ships two wasm variants —{' '}
          <code>vane_wasm_simd.wasm</code> and <code>vane_wasm_bg.wasm</code>{' '}
          (the scalar fallback). The worker init runs a runtime{' '}
          <code>simd128Supported()</code> probe and picks the right variant
          automatically; you never configure SIMD by hand. The probe is
          re-exported from <code>@vane-rs/web/probe</code> for advanced users
          who want to check support from the main thread.
        </p>
        <CodeBlock
          lang="ts"
          title="probe signature"
          code={SIMD_PROBE_SIG}
        />
        <CodeBlock
          lang="ts"
          title="optional main-thread probe"
          code={SIMD_PROBE_EXAMPLE}
        />
        <p>
          The probe works by calling{' '}
          <code>WebAssembly.validate(SIMD128_TEST_MODULE)</code> on a minimal
          50-byte module containing a <code>v128.const</code> instruction
          (opcode <code>FD 0C</code>). Only a SIMD128-capable runtime
          validates it successfully; otherwise it returns <code>false</code>{' '}
          (or catches and returns <code>false</code>).
        </p>

        <h2 id="dictdata">dictData &amp; CDN fallback</h2>
        <p>
          Dictionary resolution follows a strict priority chain:{' '}
          <code>dictData</code> (inline, zero-copy transfer) &rarr;{' '}
          <code>dictUrl</code> (explicit URL) &rarr; auto-filled jsdelivr CDN
          default. When <code>dictData</code> is provided, the underlying{' '}
          <code>ArrayBuffer</code> is transferred to the worker with zero copy
          — after <code>createVane</code> resolves the main-thread buffer is{' '}
          <strong>detached</strong> and reading it throws. If a CDN fetch
          fails inside the worker, the tokenizer degrades to bigram mode
          (no error thrown).
        </p>
        <CodeBlock
          lang="ts"
          title="dictData inline (recommended for production)"
          code={DICT_INLINE}
        />
        <CodeBlock
          lang="ts"
          title="dictUrl / CDN fallback"
          code={DICT_CDN}
        />
        <p>
          <code>dictSha256</code> is a 16-character hex string (the first 8
          bytes of the sha256 digest). The worker calls{' '}
          <code>verify_sha256_prefix</code> to detect truncated or corrupted
          downloads before loading the dictionary. It is recommended whenever{' '}
          <code>dictUrl</code> is used over the network; when using{' '}
          <code>dictData</code> from the <code>@vane-rs/dict-zh</code> package
          the <code>sha256_prefix.bin</code> sidecar provides the matching
          value.
        </p>

        <h2 id="worker-internals">Worker internals</h2>
        <p>
          This section is informational — it describes what{' '}
          <code>createVane</code> does internally, not API you call. The
          worker is constructed using the standard ESM worker idiom:
        </p>
        <CodeBlock
          lang="ts"
          title="inside createVane (informational)"
          code={WORKER_INTERNALS}
        />
        <Callout type="note" title="You never write the worker yourself">
          <code>createVane</code> encapsulates the worker construction, the{' '}
          <code>postMessage</code> Promise boundary, the{' '}
          <code>pending</code> Map (id &rarr; {'{ resolve, reject }'}), and the{' '}
          <code>close()</code> handle revocation. The only thing you need to
          provide is a bundler that resolves{' '}
          <code>new URL(&apos;./worker.js&apos;, import.meta.url)</code> to a
          separate ESM chunk — Vite 6+ and webpack 5 (with{' '}
          <code>outputModule</code>) both do this natively. See the{' '}
          <Link to="/guides/web-integration">Web Integration</Link> guide for
          bundler configuration.
        </Callout>
        <p>
          The <code>new URL(&apos;./worker.js&apos;, import.meta.url)</code>{' '}
          pattern is recognized natively by Vite 6+ and webpack 5, which
          emit the worker as a separate ESM chunk and rewrite{' '}
          <code>import.meta.url</code> correctly inside that chunk. Inside the
          worker, the init sequence is: SIMD probe &rarr; select wasm variant
          &rarr; <code>init(wasmUrl)</code> with an explicit{' '}
          <code>fetch</code> &rarr;{' '}
          <code>VaneWorker.create(opts)</code>. The main-thread{' '}
          <code>VaneImpl</code> holds a <code>pending</code> Map keyed by
          message id; each call posts <code>{'{ op, id, ...payload }'}</code>{' '}
          and awaits the matching <code>{'{ id, result|error }'}</code>{' '}
          response. After <code>close()</code> the <code>closed</code> flag is
          set and every subsequent call rejects immediately with{' '}
          <code>&ldquo;vane worker closed&rdquo;</code>.
        </p>
      </div>
    </DocsLayout>
  );
}
