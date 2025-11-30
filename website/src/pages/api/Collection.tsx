import { Link } from 'react-router-dom';
import DocsLayout from '../../components/DocsLayout';
import LangTabs from '../../components/LangTabs';
import CodeBlock from '../../components/CodeBlock';
import './api.css';

const IDL = `Db.collection(name: string, schema: Schema & CollectionOptions) -> Collection
// idempotent: same name + same schema returns the existing collection

Db.collections() -> [string]

Field :=
  | { type: "text" }                                              // BM25 inverted index
  | { type: "vector", dim: u32, metric: "cosine" | "l2" | "dot" }
  | { type: "scalar", kind: "int" | "float" | "bool" | "keyword" } // filterable

CollectionOptions := {
  tokenizer?: "standard" | "cjk_bigram" | "jieba"   // default "standard"
  userDict?: [ string | { term: string, freq?: u32 } ]
  // M2 WASM: dictData?: bytes — inline dictionary injection (offline / self-hosted)
}`;

const NODE_SNIPPET = `// db.collection(name, schema, opts?): Promise<VaneCollection>
const col = await db.collection('docs', {
  fields: [
    { name: 'title', type: 'text' },
    { name: 'body',  type: 'text' },
    { name: 'vec',   type: 'vector', dim: 384, metric: 'cosine' },
    { name: 'lang',  type: 'scalar', kind: 'keyword' },
  ],
}, { tokenizer: 'jieba' });`;

const GO_SNIPPET = `// func (db *Db) Collection(name string, schema Schema,
//                          opts *CollectionOptions) (*Collection, error)
schema := vane.Schema{Fields: []vane.SchemaField{
    {Name: "title", Type: "text"},
    {Name: "body",  Type: "text"},
    {Name: "vec",   Type: "vector", Dim: 384, Metric: "cosine"},
    {Name: "lang",  Type: "scalar", Kind: "keyword"},
}}
col, err := db.Collection("docs", schema, &vane.CollectionOptions{
    Tokenizer: "jieba",
})
if err != nil {
    log.Fatalf("collection: %v", err)
}`;

const BROWSER_SNIPPET = `// worker.collection(name, schema, opts): Promise<u32> (collection handle)
const col = await worker.collection('docs', {
  fields: [
    { name: 'title', type: 'text' },
    { name: 'body',  type: 'text' },
    { name: 'vec',   type: 'vector', dim: 384, metric: 'cosine' },
    { name: 'lang',  type: 'scalar', kind: 'keyword' },
  ],
}, { tokenizer: 'jieba' });`;

export default function ApiCollection() {
  return (
    <DocsLayout>
      <div className="api-page">
        <h1>collection</h1>
        <p className="api-lead">
          Creates a collection with a declared schema, or returns the existing one when
          name and schema match (idempotent). <code>db.collections()</code> lists the
          names of all collections in the database.
        </p>

        <pre className="api-idl">
          <code>{IDL}</code>
        </pre>

        <LangTabs
          node={<CodeBlock code={NODE_SNIPPET} lang="js" title="collection.mjs" />}
          go={<CodeBlock code={GO_SNIPPET} lang="go" title="collection.go" />}
          browser={<CodeBlock code={BROWSER_SNIPPET} lang="js" title="collection.js" />}
        />

        <h2 id="schema">Schema</h2>
        <p>
          A schema is a list of named fields, declared at creation time. After creation
          only appendix-style extension is allowed (adding new fields); modifying or
          removing existing fields is forbidden.
        </p>
        <div className="api-table-wrap">
          <table className="api-table">
            <thead>
              <tr>
                <th>Field type</th>
                <th>Shape</th>
                <th>Constraints</th>
              </tr>
            </thead>
            <tbody>
              <tr>
                <td><code>text</code></td>
                <td className="api-table__wide"><code>{'{ type: "text" }'}</code></td>
                <td className="api-table__wide">
                  Feeds the BM25 inverted index. Multiple text fields are allowed, and
                  text fields may be omitted entirely — a pure-vector collection is legal.
                </td>
              </tr>
              <tr>
                <td><code>vector</code></td>
                <td className="api-table__wide">
                  <code>{'{ type: "vector", dim: u32, metric: "cosine" | "l2" | "dot" }'}</code>
                </td>
                <td className="api-table__wide">
                  <strong>Exactly one</strong> vector field per collection (M0–M2 limit).
                  <code>dim</code> ≤ 4096. <code>metric</code> defaults to{' '}
                  <code>"cosine"</code>.
                </td>
              </tr>
              <tr>
                <td><code>scalar</code></td>
                <td className="api-table__wide">
                  <code>{'{ type: "scalar", kind: "int" | "float" | "bool" | "keyword" }'}</code>
                </td>
                <td className="api-table__wide">
                  Filterable metadata. Any number of scalar fields; <code>kind</code> is
                  one of the four listed values.
                </td>
              </tr>
            </tbody>
          </table>
        </div>
        <p>
          A schema that violates these constraints (two vector fields,{' '}
          <code>dim &gt; 4096</code>, an unknown <code>metric</code> or <code>kind</code>)
          is rejected with <code>E_SCHEMA</code> (-2). See{' '}
          <Link to="/api/errors">Error Codes</Link>.
        </p>

        <h2 id="collection-options">CollectionOptions</h2>
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
                <td className="api-table__wide"><code>"standard" | "cjk_bigram" | "jieba"</code></td>
                <td><code>"standard"</code></td>
                <td className="api-table__wide">
                  Built-in tokenizer identity for all text fields. Recorded per segment;
                  changing it requires a reindex. See{' '}
                  <Link to="/guides/tokenizers">Tokenizers</Link>.
                </td>
              </tr>
              <tr>
                <td><code>userDict</code></td>
                <td className="api-table__wide">
                  <code>[ string | {'{ term: string, freq?: u32 }'} ]</code>
                </td>
                <td>—</td>
                <td className="api-table__wide">
                  Domain terms injected into the jieba dictionary at creation. A bare
                  string gets the highest built-in frequency; an object sets an explicit{' '}
                  <code>freq</code>. Over 100k entries fails with{' '}
                  <code>E_DICT_TOO_LARGE</code> (-7). See{' '}
                  <Link to="/guides/reindex">Custom Dict &amp; Reindex</Link>.
                </td>
              </tr>
              <tr>
                <td><code>dictData</code></td>
                <td><code>bytes</code></td>
                <td>—</td>
                <td className="api-table__wide">
                  M2 WASM only: inline jieba dictionary bytes for offline or self-hosted
                  deployments, bypassing the CDN fetch.
                </td>
              </tr>
            </tbody>
          </table>
        </div>

        <h3>Error handling</h3>
        <p>
          Schema violations fail with <code>E_SCHEMA</code> (-2); a query-time tokenizer
          identity that differs from a segment's fails with{' '}
          <code>E_TOKENIZER_MISMATCH</code> (-6), signalling a pending reindex;
          declaring <code>tokenizer: "jieba"</code> without a loaded dictionary fails with{' '}
          <code>E_DICT_UNAVAILABLE</code> (-8) on Node/Go (the browser auto-degrades
          instead). Node rejects with <code>VaneError</code>, Go returns{' '}
          <code>*vane.VaneError</code>. See <Link to="/api/errors">Error Codes</Link>.
        </p>
      </div>
    </DocsLayout>
  );
}
