import DocsLayout from '../components/DocsLayout';
import LangTabs from '../components/LangTabs';
import CodeBlock from '../components/CodeBlock';
import Callout from '../components/Callout';
import { Link } from 'react-router-dom';
import './QuickStart.css';

const NODE_INSTALL = `npm install @vane-rs/node`;

const GO_INSTALL = `# 1. Build the static lib (or download libvane_ffi-<lib_dir>.a from GitHub Releases)
cargo build --release -p vane-ffi

# 2. Place it where cgo expects it (os-arch subdirectory of bindings/go/lib/)
mkdir -p bindings/go/lib/$(go env GOOS)-$(go env GOARCH)
cp target/release/libvane_ffi.a bindings/go/lib/$(go env GOOS)-$(go env GOARCH)/

# 3. Add to your module
go get github.com/ximing/vane/bindings/go`;

const BROWSER_INSTALL = `npm install @vane-rs/web @vane-rs/dict-zh`;

const NODE_OPEN = `import vane from '@vane-rs/node';
const { open } = vane;

// Open a database directory (created if missing). autoCommit: 'off' means we
// control the visibility boundary ourselves with flush().
const db = await open('./mydb', { autoCommit: 'off' });`;

const GO_OPEN = `import (
	"fmt"
	"log"

	"github.com/ximing/vane/bindings/go"
	"github.com/ximing/vane/bindings/go/dict"
)

db, err := vane.Open("./mydb", nil) // nil opts = defaults
if err != nil { log.Fatalf("Open: %v", err) }
defer db.Close()

// Load the bundled jieba dictionary (embedded in the dict package).
if b, err := dict.DictBytes(); err == nil {
	_ = db.LoadDict(b) // on failure, jieba degrades to standard — collection creation won't fail
}`;

const BROWSER_OPEN = `import { createVane } from '@vane-rs/web';
import dictBinUrl from '@vane-rs/dict-zh/dict.bin';

// npm install @vane-rs/web @vane-rs/dict-zh
const dictData = new Uint8Array(await (await fetch(dictBinUrl)).arrayBuffer());
const vane = await createVane({ vfs: 'opfs', dbPath: 'vane.db', dictData });
await vane.open();`;

const NODE_INDEX = `// Declare a schema: one text field + one vector field. (One vector field per collection.)
const col = await db.collection('docs', {
  fields: [
    { name: 'body', type: 'text' },
    { name: 'vec',  type: 'vector', dim: 4, metric: 'cosine' },
  ],
}, { tokenizer: 'standard' });

// Batch upsert by id. Returns { accepted, visibleAfterFlush }.
await col.add([
  { id: 'a', text: 'hello world',  vector: [1.0, 0.0, 0.0, 0.0] },
  { id: 'b', text: 'foo bar baz',  vector: [0.0, 1.0, 0.0, 0.0] },
  { id: 'c', text: 'hello foo',    vector: [0.7, 0.3, 0.0, 0.0] },
]);
await col.flush();                       // data is now searchable`;

const GO_INDEX = `schema := vane.Schema{Fields: []vane.SchemaField{
	{Name: "body", Type: "text"},
	{Name: "vec",  Type: "vector", Dim: 4, Metric: "cosine"},
}}
col, err := db.Collection("docs", schema, &vane.CollectionOptions{Tokenizer: "jieba"})
if err != nil { log.Fatalf("Collection: %v", err) }
defer col.Close()

_ = col.Add([]vane.Doc{
	{ID: "a", Text: "hello world", Vector: []float32{1.0, 0.0, 0.0, 0.0}},
	{ID: "b", Text: "foo bar baz", Vector: []float32{0.0, 1.0, 0.0, 0.0}},
	{ID: "c", Text: "hello foo",   Vector: []float32{0.7, 0.3, 0.0, 0.0}},
})
_ = col.Flush()`;

const BROWSER_INDEX = `// vane.collection(name, schema, opts): Promise<number> (collection 句柄)
const col = await vane.collection('docs', {
  fields: [
    { name: 'body', type: 'text' },
    { name: 'vec', type: 'vector', dim: 4, metric: 'cosine' },
  ],
}, { tokenizer: 'jieba' });

// vane.add(col, docs): Promise<number> (accepted count)
await vane.add(col, [
  { id: 'a', text: 'hello world', vector: [1.0, 0.0, 0.0, 0.0] },
  { id: 'b', text: 'foo bar baz', vector: [0.0, 1.0, 0.0, 0.0] },
  { id: 'c', text: 'hello foo', vector: [0.7, 0.3, 0.0, 0.0] },
]);
await vane.flush(col); // data is now searchable`;

const NODE_SEARCH = `// Hybrid search: BM25(text) + vector similarity, fused with RRF.
const hits = await col.search({
  text: 'hello',
  vector: [1.0, 0.0, 0.0, 0.0],
  topK: 3,
  mode: 'hybrid',                        // 'vector' | 'text' | 'hybrid'
  fusion: 'rrf',                         // default; or { linear: { alpha: 0.5 } }
});
// hits = [{ id, score, fields }, ...]

await db.close();`;

const GO_SEARCH = `hits, _ := col.Search(vane.SearchQuery{
	Text: "hello", Vector: []float32{1.0, 0.0, 0.0, 0.0}, TopK: 3,
})
for _, h := range hits {
	fmt.Printf("hit: id=%s score=%.4f\\n", h.ID, h.Score)
}`;

const BROWSER_SEARCH = `import type { Hit } from '@vane-rs/web';

// vane.search(col, query): Promise<Hit[]> — 无需 JSON.parse
const hits: Hit[] = await vane.search(col, {
  text: 'hello',
  vector: [1.0, 0.0, 0.0, 0.0],
  topK: 3,
  mode: 'hybrid',
  fusion: 'rrf',
});
// hits = [{ id, score, fields }, ...]

await vane.close();`;

export default function QuickStart() {
  return (
    <DocsLayout>
      <div className="quickstart">
        <h1>Quick Start</h1>
        <p>
          Vane is an embedded hybrid search library — one Rust core, exposed to Node.js, Go,
          and the browser. This page walks through install → open → index → search on each
          runtime.
        </p>
        <Callout type="note" title="Local folders and agents">
          To index directories on disk and search them from Claude / Cursor without embedding
          Vane in your own app, use the{' '}
          <Link to="/guides/sidecar">sidecar CLI</Link> instead (
          <code>vane init</code> → <code>vane mcp</code>).
        </Callout>

        <h2 id="choose-your-runtime">Choose your runtime</h2>
        <p>
          Pick a runtime below. Your choice is remembered across every docs page, and all
          tabbed steps on this page switch together.
        </p>
        <LangTabs
          node={
            <p>
              <strong>Node.js</strong> — an npm package with prebuilt napi-rs binaries. The
              bundled <code>jieba</code> dictionary loads automatically on open, so Chinese
              search works out of the box.
            </p>
          }
          go={
            <p>
              <strong>Go</strong> — an idiomatic cgo binding that links a prebuilt static
              library (<code>libvane_ffi.a</code>). The <code>jieba</code> dictionary is
              embedded in the <code>bindings/go/dict</code> package.
            </p>
          }
          browser={
            <p>
              <strong>Browser</strong> — the <code>@vane-rs/web</code> npm package ships a
              SIMD/scalar dual-variant wasm module, a Web Worker, and the Chinese dictionary
              as a transferable <code>Uint8Array</code>. Inline the dictionary via{' '}
              <code>@vane-rs/dict-zh</code> for zero-CDN production builds; a jsdelivr CDN
              fallback exists for quick experiments.
            </p>
          }
        />

        <h2 id="install">1. Install</h2>
        <LangTabs
          node={
            <>
              <CodeBlock code={NODE_INSTALL} lang="bash" title="terminal" />
              <p>
                Prebuilt native binaries are selected automatically via{' '}
                <code>optionalDependencies</code>:
              </p>
              <table>
                <thead>
                  <tr>
                    <th>Platform</th>
                    <th>npm sub-package</th>
                  </tr>
                </thead>
                <tbody>
                  <tr>
                    <td>Linux x64 (glibc)</td>
                    <td>
                      <code>@vane-rs/node-linux-x64-gnu</code>
                    </td>
                  </tr>
                  <tr>
                    <td>macOS arm64</td>
                    <td>
                      <code>@vane-rs/node-darwin-arm64</code>
                    </td>
                  </tr>
                  <tr>
                    <td>macOS x64</td>
                    <td>
                      <code>@vane-rs/node-darwin-x64</code>
                    </td>
                  </tr>
                  <tr>
                    <td>Windows x64 (MSVC)</td>
                    <td>
                      <code>@vane-rs/node-win32-x64-msvc</code>
                    </td>
                  </tr>
                </tbody>
              </table>
              <p>
                You should never need to compile from source on these platforms.
              </p>
            </>
          }
          go={
            <>
              <CodeBlock code={GO_INSTALL} lang="bash" title="terminal" />
              <p>
                Prebuilt static libs cover <code>linux-amd64</code>, <code>linux-arm64</code>,{' '}
                <code>darwin-amd64</code>, and <code>darwin-arm64</code>. Call{' '}
                <code>db.LoadDict(dict.DictBytes())</code> once after <code>Open</code> to enable
                the embedded <code>jieba</code> dictionary. A <code>vane_nodict</code> build tag
                drops the embedded dictionary (the tokenizer degrades to{' '}
                <code>cjk_bigram</code>).
              </p>
              <Callout type="warning" title="wazero status: stub / deferred">
                <p>
                  <code>CGO_ENABLED=0</code> builds are not supported on the default cgo path. A{' '}
                  <code>wazero</code> build tag exists, but in v0.1.x it is a{' '}
                  <strong>stub</strong> — the pure-Go variant is not implemented and is{' '}
                  <strong>deferred</strong> (see the README “Status” section). Use the cgo path
                  with a prebuilt static library.
                </p>
              </Callout>
            </>
          }
          browser={
            <>
              <CodeBlock code={BROWSER_INSTALL} lang="bash" title="terminal" />
              <p>
                <code>@vane-rs/web</code> ships SIMD128 + scalar dual wasm variants; a runtime
                probe inside the worker picks the right one. The dictionary ships as a separate{' '}
                <code>@vane-rs/dict-zh</code> package — inline it via <code>dictData</code> for
                zero-CDN production builds, or let <code>@vane-rs/web</code> fall back to a
                jsdelivr CDN URL for quick experiments (degrades to <code>cjk_bigram</code>{' '}
                offline without error).
              </p>
            </>
          }
        />

        <h2 id="open">2. Open a database</h2>
        <LangTabs
          node={<CodeBlock code={NODE_OPEN} lang="js" title="quickstart.js" />}
          go={<CodeBlock code={GO_OPEN} lang="go" title="main.go" />}
          browser={
            <>
              <p>
                <code>createVane</code> spawns a Web Worker that wraps the wasm engine with a{' '}
                <code>postMessage</code> Promise boundary, OPFS persistence (IndexedDB fallback),
                and a transferable <code>dictData</code> dictionary. See the{' '}
                <Link to="/guides/web-integration">Web Integration</Link> guide for vite / webpack
                configuration.
              </p>
              <CodeBlock code={BROWSER_OPEN} lang="ts" title="open.ts" />
              <p>
                You must access the page over <code>http://</code> or <code>https://</code> —
                under <code>file://</code>, Worker / OPFS / ES modules are restricted.
              </p>
            </>
          }
        />

        <h2 id="index">3. Index documents</h2>
        <LangTabs
          node={<CodeBlock code={NODE_INDEX} lang="js" title="quickstart.js" />}
          go={<CodeBlock code={GO_INDEX} lang="go" title="main.go" />}
          browser={<CodeBlock code={BROWSER_INDEX} lang="ts" title="index.ts" />}
        />

        <h2 id="search">4. Search</h2>
        <LangTabs
          node={<CodeBlock code={NODE_SEARCH} lang="js" title="quickstart.js" />}
          go={<CodeBlock code={GO_SEARCH} lang="go" title="main.go" />}
          browser={<CodeBlock code={BROWSER_SEARCH} lang="ts" title="search.ts" />}
        />

        <h2 id="about-the-demo-vectors">About the demo vectors</h2>
        <Callout type="note" title="About the demo vectors">
          <p>
            Vane does not include an embedding model — vectors are supplied by the caller, and
            Vane stores, indexes, and fuses whatever you give it. The examples on this page use
            deterministic pseudo-vectors (tiny dummy vectors or hash buckets) so they run
            offline as-is. For production, wire in a real embedding provider — OpenAI, ollama,
            or transformers.js in the browser — in a few lines; see the <code>examples/</code>{' '}
            directory in the repository.
          </p>
        </Callout>
      </div>
    </DocsLayout>
  );
}
