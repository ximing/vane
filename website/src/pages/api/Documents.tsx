import { Link } from 'react-router-dom';
import DocsLayout from '../../components/DocsLayout';
import LangTabs from '../../components/LangTabs';
import CodeBlock from '../../components/CodeBlock';
import './api.css';

const IDL = `Document := { id: string, <field>: value, ... }
// bindings expose the simplified shape { id, text, vector, meta }

Collection.add(docs: [Document]) -> Result<AddReport>   // batch idempotent upsert
Collection.flush() -> Result<()>                        // visibility boundary
Collection.delete(ids: [string]) -> Result<u64>         // returns tombstone count

AddReport := { accepted: u64, visibleAfterFlush: true }`;

const NODE_SNIPPET = `// col.add(docs): Promise<{ accepted, visibleAfterFlush }>
const report = await col.add([
  { id: 'a', text: 'hello world', vector: [1, 0, 0, 0], meta: { lang: 'en' } },
  { id: 'b', text: 'foo bar',     vector: [0, 1, 0, 0], meta: { lang: 'en' } },
]);
// report = { accepted: 2, visibleAfterFlush: true }

await col.flush();                        // atomically visible now
const removed = await col.delete(['b']);  // → 1n: tombstone count`;

const GO_SNIPPET = `// func (c *Collection) Add(docs []Doc) error
err := col.Add([]vane.Doc{
    {ID: "a", Text: "hello world", Vector: []float32{1, 0, 0, 0},
     Meta: map[string]interface{}{"lang": "en"}},
    {ID: "b", Text: "foo bar", Vector: []float32{0, 1, 0, 0},
     Meta: map[string]interface{}{"lang": "en"}},
})

err = col.Flush()                          // atomically visible now
removed, err := col.Delete([]string{"b"})  // → 1: tombstone count`;

const BROWSER_SNIPPET = `// 假设 vane 已由 createVane({dictData}) 创建，col = await vane.collection(...)
// vane.add(col, docs): Promise<number> (accepted count)
const accepted = await vane.add(col, [
  { id: 'a', text: 'hello world', vector: [1, 0, 0, 0], meta: { lang: 'en' } },
  { id: 'b', text: 'foo bar',     vector: [0, 1, 0, 0], meta: { lang: 'en' } },
]);

await vane.flush(col);                        // atomically visible now
const removed = await vane.delete(col, ['b']); // → 1: tombstone count`;

export default function ApiDocuments() {
  return (
    <DocsLayout>
      <div className="api-page">
        <h1>documents</h1>
        <p className="api-lead">
          Writing documents is a three-verb flow: <code>add</code> stages a batch,{' '}
          <code>flush</code> makes it atomically searchable, <code>delete</code> removes
          by id.
        </p>

        <pre className="api-idl">
          <code>{IDL}</code>
        </pre>

        <LangTabs
          node={<CodeBlock code={NODE_SNIPPET} lang="js" title="docs.mjs" />}
          go={<CodeBlock code={GO_SNIPPET} lang="go" title="docs.go" />}
          browser={<CodeBlock code={BROWSER_SNIPPET} lang="ts" title="docs.ts" />}
        />

        <h2 id="document">Document</h2>
        <p>
          In the language-neutral IDL a document is{' '}
          <code>{'{ id: string, <field>: value, ... }'}</code>. The bindings expose the
          simplified shape <code>{'{ id, text, vector, meta }'}</code>, where{' '}
          <code>meta</code> holds scalar values keyed by field name.
        </p>
        <ul>
          <li>
            <code>id</code> is an external string primary key, ≤ 512 bytes, unique within
            the collection. <code>add</code> is an <strong>idempotent upsert by id</strong>:
            re-adding the same id replaces the previous document.
          </li>
          <li>
            Internally each id maps to a <code>u64</code> docid, assigned monotonically
            per segment; the mapping table is persisted with the segment.
          </li>
          <li>A single document serialized must be ≤ 16MB.</li>
          <li>
            The vector dimension must equal the schema <code>dim</code>, or the call fails
            with <code>E_SCHEMA</code> (-2).
          </li>
        </ul>

        <h2 id="add">add</h2>
        <p>
          Batch-upserts documents and returns{' '}
          <code>AddReport := {'{ accepted: u64, visibleAfterFlush: true }'}</code>. The{' '}
          <code>visibleAfterFlush</code> flag is a standing reminder of the visibility
          contract: accepted documents are <em>not</em> searchable yet.
        </p>

        <h2 id="flush">flush</h2>
        <p>
          <code>flush()</code> is the atomic visibility boundary: everything accepted
          before it becomes searchable together, nothing half-visible. With{' '}
          <code>autoCommit</code> on (the default), flushes also happen automatically
          every 1s or 1000 docs. See{' '}
          <Link to="/guides/persistence">Persistence &amp; Visibility</Link> for the full
          timing model and crash-recovery behavior.
        </p>

        <h2 id="delete">delete</h2>
        <p>
          Deletes by external id and returns the number of tombstones written
          (<code>u64</code>). Deletes are recorded as tombstones and become visible at the
          next <code>flush()</code>; the physical space is reclaimed by{' '}
          <Link to="/api/maintenance">compact</Link>.
        </p>

        <h3>Error handling</h3>
        <p>
          A vector whose dimension differs from the schema fails with{' '}
          <code>E_SCHEMA</code> (-2); unknown fields or wrong field types are the same
          error. Node rejects with <code>VaneError</code> (<code>err.code === -2</code>),
          Go returns a <code>*vane.VaneError</code> (<code>ve.Code == -2</code>). See{' '}
          <Link to="/api/errors">Error Codes</Link>.
        </p>
      </div>
    </DocsLayout>
  );
}
