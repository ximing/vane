import { Link } from 'react-router-dom';
import DocsLayout from '../../components/DocsLayout';
import LangTabs from '../../components/LangTabs';
import CodeBlock from '../../components/CodeBlock';
import Callout from '../../components/Callout';
import './api.css';

const IDL = `open(path: string, opts?: OpenOptions) -> Db

OpenOptions := {
  persistence?: "persistent" | "best-effort"   // default "persistent";
                                               // WASM maps to navigator.storage.persist()
  autoCommit?: { intervalMs?: u32 = 1000, maxDocs?: u32 = 1000 } | "off"   // default on
  pageCacheMb?: u32 = 32
}`;

const NODE_SNIPPET = `import vane from '@vane-rs/node';
const { open } = vane;

// open(path: string, opts?: OpenOptions): Promise<VaneDb>
const db = await open('./mydb', {
  autoCommit: 'off',   // we control visibility ourselves via flush()
  pageCacheMb: 64,
});`;

const GO_SNIPPET = `import "github.com/ximing/vane/bindings/go"

// func Open(path string, opts *OpenOptions) (*Db, error)
db, err := vane.Open("./mydb", &vane.OpenOptions{
    AutoCommit:  "off", // or map[string]interface{}{"intervalMs": 1000, "maxDocs": 1000}
    PageCacheMB: 64,
})
if err != nil {
    log.Fatalf("open: %v", err)
}
defer db.Close()`;

const BROWSER_SNIPPET = `import init, { VaneWorker } from './pkg/vane_wasm.js';

await init();
// In the browser the database lives inside a Dedicated Worker; open() goes
// through the VaneWorker glue and the path is a logical name inside the VFS.
const worker = await VaneWorker.create({ vfs: 'opfs', dbPath: 'vane.db' });
await worker.open('vane.db', {
  persistence: 'persistent', // requests navigator.storage.persist()
});`;

export default function ApiOpen() {
  return (
    <DocsLayout>
      <div className="api-page">
        <h1>open</h1>
        <p className="api-lead">
          Opens (or creates) a database directory and returns a <code>Db</code> handle.
          All collections, segments and the write-ahead log live under this path.
        </p>

        <pre className="api-idl">
          <code>{IDL}</code>
        </pre>

        <h2 id="signature">Signature</h2>
        <LangTabs
          node={<CodeBlock code={NODE_SNIPPET} lang="js" title="open.mjs" />}
          go={<CodeBlock code={GO_SNIPPET} lang="go" title="open.go" />}
          browser={<CodeBlock code={BROWSER_SNIPPET} lang="js" title="open.js" />}
        />

        <h2 id="open-options">OpenOptions</h2>
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
                <td className="api-table__wide"><code>"persistent" | "best-effort"</code></td>
                <td><code>"persistent"</code></td>
                <td className="api-table__wide">
                  Durability request. On WASM this maps to{' '}
                  <code>navigator.storage.persist()</code>; on native it selects how hard
                  the VFS pushes data to stable storage.
                </td>
              </tr>
              <tr>
                <td><code>autoCommit</code></td>
                <td className="api-table__wide">
                  <code>{'{ intervalMs?: u32, maxDocs?: u32 }'} | "off"</code>
                </td>
                <td className="api-table__wide">on (<code>intervalMs: 1000</code>, <code>maxDocs: 1000</code>)</td>
                <td className="api-table__wide">
                  Automatically flushes pending writes every <code>intervalMs</code>{' '}
                  milliseconds or every <code>maxDocs</code> documents, whichever comes
                  first. Pass <code>"off"</code> to make <code>flush()</code> the only
                  visibility boundary.
                </td>
              </tr>
              <tr>
                <td><code>pageCacheMb</code></td>
                <td><code>u32</code></td>
                <td><code>32</code></td>
                <td className="api-table__wide">
                  Size of the internal page cache in megabytes.
                </td>
              </tr>
            </tbody>
          </table>
        </div>

        <Callout type="note" title="Visibility">
          <code>add()</code> returning is not the same as searchable. Writes become
          atomically visible at <code>flush()</code> — or at the next auto-commit tick.
          See <Link to="/guides/persistence">Persistence &amp; Visibility</Link> for the
          full timing model.
        </Callout>

        <h3>Error handling</h3>
        <p>
          A path that cannot be read or written fails with <code>E_IO</code> (-1). On the
          browser, a missing storage capability (no OPFS with no IndexedDB fallback
          enabled) fails with <code>E_UNSUPPORTED</code> (-10). Node rejects the returned
          Promise with a <code>VaneError</code>; Go returns a <code>*vane.VaneError</code>;
          the browser worker rejects with the same error name. See{' '}
          <Link to="/api/errors">Error Codes</Link>.
        </p>
      </div>
    </DocsLayout>
  );
}
