import { Link } from 'react-router-dom';
import DocsLayout from '../../components/DocsLayout';
import LangTabs from '../../components/LangTabs';
import CodeBlock from '../../components/CodeBlock';
import './api.css';

const IDL = `open(path: string, opts?: OpenOptions) -> Db
Db.collection(name: string, schema: Schema & CollectionOptions) -> Collection  // idempotent
Db.collections() -> [string]
Db.export(destPath: string) -> Result<()>
Db.close() -> Result<()>

Collection.add(docs: [Document]) -> Result<AddReport>   // batch idempotent upsert
Collection.flush() -> Result<()>                        // visibility boundary
Collection.search(query: SearchQuery) -> Result<[Hit]>
Collection.delete(ids: [string]) -> Result<u64>         // tombstone count
Collection.compact() -> Result<()>
Collection.reindex() -> Result<ReindexHandle>`;

const NODE_SNIPPET = `import vane from '@vane-rs/node';
const { open } = vane;

const db = await open('./mydb');
const col = await db.collection('docs', schema, { tokenizer: 'standard' });
await col.add(docs);
await col.flush();
const hits = await col.search({ text: 'hello', topK: 10 });
await db.close();`;

const GO_SNIPPET = `db, err := vane.Open("./mydb", nil) // nil opts = defaults
if err != nil { log.Fatal(err) }
defer db.Close()

col, err := db.Collection("docs", schema, &vane.CollectionOptions{Tokenizer: "standard"})
if err != nil { log.Fatal(err) }

_ = col.Add(docs)
_ = col.Flush()
hits, err := col.Search(vane.SearchQuery{Text: "hello", TopK: 10})`;

const BROWSER_SNIPPET = `import { createVane } from '@vane-rs/web';
import type { Schema, Hit } from '@vane-rs/web';
import dictBinUrl from '@vane-rs/dict-zh/dict.bin';

// npm install @vane-rs/web @vane-rs/dict-zh
const dictData = new Uint8Array(await (await fetch(dictBinUrl)).arrayBuffer());
const vane = await createVane({ vfs: 'opfs', dbPath: 'vane.db', dictData });

await vane.open();
const col = await vane.collection('docs', {
  fields: [
    { name: 'body', type: 'text' },
    { name: 'vec', type: 'vector', dim: 4, metric: 'cosine' },
  ],
}, { tokenizer: 'jieba' });

await vane.add(col, [
  { id: 'a', text: 'hello world', vector: [1, 0, 0, 0] },
  { id: 'b', text: 'foo bar baz', vector: [0, 1, 0, 0] },
]);
await vane.flush(col);
const hits: Hit[] = await vane.search(col, {
  text: 'hello', vector: [1, 0, 0, 0], topK: 3, mode: 'hybrid',
});
await vane.close();`;

const NODE_ERR = `import vane, { VaneError } from '@vane-rs/node';

try {
  await col.search({ text: 'hello', topK: 2000 });
} catch (err) {
  if (err instanceof VaneError) {
    console.error(err.code, err.name); // -11, 'E_INVALID_ARG'
  }
}`;

const GO_ERR = `hits, err := col.Search(vane.SearchQuery{Text: "hello", TopK: 2000})
var ve *vane.VaneError
if errors.As(err, &ve) {
    fmt.Println(ve.Code, ve.Message) // -11, "..."
}`;

const BROWSER_ERR = `try {
  await vane.search(col, { text: 'hello', topK: 2000 });
} catch (err) {
  // Rejected value's text carries the SPEC §10 error name.
  console.error(String(err)); // contains 'E_INVALID_ARG'
}`;

export default function ApiOverview() {
  return (
    <DocsLayout>
      <div className="api-page">
        <h1>API Overview</h1>
        <p className="api-lead">
          One language-neutral IDL, three thin bindings. The same six verbs plus four
          management calls appear on every runtime; only casing and error style differ.
          All behavior is implemented and tested in the Rust core — the bindings carry
          no search logic of their own.
        </p>

        <pre className="api-idl">
          <code>{IDL}</code>
        </pre>

        <LangTabs
          node={<CodeBlock code={NODE_SNIPPET} lang="js" title="app.mjs" />}
          go={<CodeBlock code={GO_SNIPPET} lang="go" title="main.go" />}
          browser={<CodeBlock code={BROWSER_SNIPPET} lang="ts" title="main.ts" />}
        />

        <h2 id="verb-table">Verb table</h2>
        <p>
          The complete call surface (SPEC §4.1, frozen since M0) with its four-side
          signature mapping (SPEC §4.3). JS calls are async and return Promises; Go calls
          block and are goroutine-safe. The browser binding exposes the same verbs through
          the <code>@vane-rs/web</code> package's <code>createVane()</code> factory;
          collections are addressed by number handle — <code>collection()</code> returns{' '}
          <code>Promise&lt;number&gt;</code> and every subsequent verb takes{' '}
          <code>col: number</code> as its first argument.
        </p>
        <div className="api-table-wrap">
          <table className="api-table">
            <thead>
              <tr>
                <th>Operation</th>
                <th>IDL</th>
                <th>JS (Node)</th>
                <th>JS (Web)</th>
                <th>Go</th>
              </tr>
            </thead>
            <tbody>
              <tr>
                <td>Open a database</td>
                <td><code>open(path, opts?)</code></td>
                <td className="api-table__wide"><code>open(path, opts)</code> → <code>Promise&lt;VaneDb&gt;</code></td>
                <td className="api-table__wide"><code>createVane(opts)</code> → <code>Promise&lt;Vane&gt;</code>; <code>vane.open(path, opts)</code></td>
                <td className="api-table__wide"><code>vane.Open(path, *OpenOptions) (*Db, error)</code></td>
              </tr>
              <tr>
                <td>Create / open a collection</td>
                <td><code>Db.collection(name, schema)</code></td>
                <td className="api-table__wide"><code>db.collection(name, schema, opts)</code> → <code>Promise&lt;VaneCollection&gt;</code></td>
                <td className="api-table__wide"><code>vane.collection(name, schema, opts)</code> → <code>Promise&lt;number&gt;</code></td>
                <td className="api-table__wide"><code>db.Collection(name, Schema, *CollectionOptions) (*Collection, error)</code></td>
              </tr>
              <tr>
                <td>List collections</td>
                <td><code>Db.collections()</code></td>
                <td className="api-table__wide"><code>db.collections()</code> → <code>string[]</code></td>
                <td className="api-table__wide">— (not yet exposed)</td>
                <td className="api-table__wide">— (not yet exposed)</td>
              </tr>
              <tr>
                <td>Add documents</td>
                <td><code>Collection.add(docs)</code></td>
                <td className="api-table__wide"><code>col.add(docs)</code> → <code>Promise&lt;{'{accepted, visibleAfterFlush}'}&gt;</code></td>
                <td className="api-table__wide"><code>vane.add(col, docs)</code> → <code>Promise&lt;number&gt;</code></td>
                <td className="api-table__wide"><code>col.Add([]Doc) error</code></td>
              </tr>
              <tr>
                <td>Make writes visible</td>
                <td><code>Collection.flush()</code></td>
                <td className="api-table__wide"><code>col.flush()</code> → <code>Promise&lt;void&gt;</code></td>
                <td className="api-table__wide"><code>vane.flush(col)</code> → <code>Promise&lt;void&gt;</code></td>
                <td className="api-table__wide"><code>col.Flush() error</code></td>
              </tr>
              <tr>
                <td>Search</td>
                <td><code>Collection.search(query)</code></td>
                <td className="api-table__wide"><code>col.search(query)</code> → <code>Promise&lt;Hit[]&gt;</code></td>
                <td className="api-table__wide"><code>vane.search(col, query)</code> → <code>Promise&lt;Hit[]&gt;</code></td>
                <td className="api-table__wide"><code>col.Search(SearchQuery) ([]Hit, error)</code></td>
              </tr>
              <tr>
                <td>Delete by id</td>
                <td><code>Collection.delete(ids)</code></td>
                <td className="api-table__wide"><code>col.delete(ids)</code> → <code>Promise&lt;bigint&gt;</code></td>
                <td className="api-table__wide"><code>vane.delete(col, ids)</code> → <code>Promise&lt;number&gt;</code></td>
                <td className="api-table__wide"><code>col.Delete([]string) (uint64, error)</code></td>
              </tr>
              <tr>
                <td>Compact segments</td>
                <td><code>Collection.compact()</code></td>
                <td className="api-table__wide"><code>col.compact()</code> → <code>Promise&lt;void&gt;</code></td>
                <td className="api-table__wide"><code>vane.compact(col)</code> → <code>Promise&lt;void&gt;</code></td>
                <td className="api-table__wide"><code>col.Compact() error</code></td>
              </tr>
              <tr>
                <td>Reindex</td>
                <td><code>Collection.reindex()</code></td>
                <td className="api-table__wide"><code>col.reindex()</code> → <code>Promise&lt;VaneReindexHandle&gt;</code></td>
                <td className="api-table__wide"><code>vane.reindex(col)</code> → <code>Promise&lt;number&gt;</code></td>
                <td className="api-table__wide"><code>col.Reindex() (*ReindexHandle, error)</code></td>
              </tr>
              <tr>
                <td>Export snapshot</td>
                <td><code>Db.export(destPath)</code></td>
                <td className="api-table__wide"><code>db.export(dest)</code> → <code>Promise&lt;void&gt;</code></td>
                <td className="api-table__wide"><code>vane.export(dest)</code> → <code>Promise&lt;void&gt;</code></td>
                <td className="api-table__wide"><code>db.Export(dest) error</code></td>
              </tr>
              <tr>
                <td>Close</td>
                <td><code>Db.close()</code></td>
                <td className="api-table__wide"><code>db.close()</code> → <code>Promise&lt;void&gt;</code></td>
                <td className="api-table__wide"><code>vane.close()</code> → <code>Promise&lt;void&gt;</code></td>
                <td className="api-table__wide"><code>db.Close() error</code> / <code>col.Close() error</code></td>
              </tr>
            </tbody>
          </table>
        </div>
        <p>
          <strong>Web:</strong> the entry point is the <code>createVane()</code> factory
          (encapsulating the Worker); <code>open</code>, <code>collection</code>, and all
          verbs are called on the returned <code>Vane</code> instance. <code>collection()</code>
          returns a <code>number</code> handle and every subsequent verb takes{' '}
          <code>col: number</code> as its first argument. The <code>add</code> and{' '}
          <code>reindex</code> verbs return <code>Promise&lt;number&gt;</code> (accepted count
          / progress) rather than the Node-style object or handle. See{' '}
          <Link to="/api/web">Web API</Link> for the full type reference.
        </p>
        <p>
          All public APIs are thread/goroutine-safe. The write path of a single collection
          is serialized internally (single writer); reads run lock-free and concurrently.
        </p>

        <h2 id="error-style">Error style</h2>
        <p>
          Errors are one contract with three notations (SPEC §9.1/§9.3, §10). Node rejects
          every Promise with a <code>VaneError</code> carrying the numeric <code>code</code> and
          string <code>name</code> from the error-code table. Go returns <code>(T, error)</code> where
          the error is a <code>*vane.VaneError</code> with <code>Code</code> and <code>Message</code> fields.
          Under the hood the C ABI returns an <code>int32_t</code> (0 = OK, negative = error code)
          with details available via <code>vane_last_error_message</code>; the browser worker
          rejects its Promises with a value whose text carries the same error name. Codes
          are passed through unchanged on all three sides — never swallowed or renumbered.
          See <Link to="/api/errors">Error Codes</Link> for the full table.
        </p>
        <LangTabs
          node={<CodeBlock code={NODE_ERR} lang="js" title="errors.mjs" />}
          go={<CodeBlock code={GO_ERR} lang="go" title="errors.go" />}
          browser={<CodeBlock code={BROWSER_ERR} lang="ts" title="errors.ts" />}
        />
      </div>
    </DocsLayout>
  );
}
