import { Link } from 'react-router-dom';
import DocsLayout from '../../components/DocsLayout';
import LangTabs from '../../components/LangTabs';
import CodeBlock from '../../components/CodeBlock';
import './api.css';

const IDL = `Collection.compact() -> Result<()>              // manual segment merge
Collection.reindex() -> Result<ReindexHandle>     // rebuild with new tokenizer identity
Db.export(destPath: string) -> Result<()>         // single-file snapshot
Db.close() -> Result<()>

ReindexHandle := { progress(): f32, wait(): Result<()> }   // pollable or blocking`;

const NODE_SNIPPET = `await col.compact();

const handle = await col.reindex();
while (handle.progress() < 1) {
  await new Promise((r) => setTimeout(r, 500));
}
// or simply: await handle.wait();

await db.export('./backup.vane');   // single-file snapshot
await db.close();`;

const GO_SNIPPET = `if err := col.Compact(); err != nil {
    log.Fatalf("compact: %v", err)
}

rh, err := col.Reindex()
if err != nil {
    log.Fatalf("reindex: %v", err)
}
if err := rh.Wait(); err != nil {   // or poll rh.Progress()
    log.Fatalf("reindex wait: %v", err)
}
rh.Close()

if err := db.Export("./backup.vane"); err != nil {
    log.Fatalf("export: %v", err)
}
db.Close()`;

const BROWSER_SNIPPET = `// 假设 vane 已由 createVane({dictData}) 创建，col = await vane.collection(...)
await vane.compact(col);

// Web: reindex runs to completion inside the call and
// resolves with the final progress (1.0 when done).
const progress: number = await vane.reindex(col);

// export writes the snapshot into the VFS container; readFile
// returns its bytes for a Blob download in the main thread.
await vane.export('backup.vane');
const bytes: Uint8Array = await vane.readFile('backup.vane');

await vane.close();`;

export default function ApiMaintenance() {
  return (
    <DocsLayout>
      <div className="api-page">
        <h1>maintenance</h1>
        <p className="api-lead">
          The four management calls: <code>compact</code>, <code>reindex</code>,{' '}
          <code>export</code>, <code>close</code>. They keep a live database healthy,
          portable, and cleanly shut down.
        </p>

        <pre className="api-idl">
          <code>{IDL}</code>
        </pre>

        <LangTabs
          node={<CodeBlock code={NODE_SNIPPET} lang="js" title="maintenance.mjs" />}
          go={<CodeBlock code={GO_SNIPPET} lang="go" title="maintenance.go" />}
          browser={<CodeBlock code={BROWSER_SNIPPET} lang="ts" title="maintenance.ts" />}
        />

        <h2 id="compact">compact</h2>
        <p>
          Manually triggers a segment merge: small segments are merged and tombstoned
          documents are physically reclaimed. Merges also happen automatically — the
          segment count has a hard cap of 10, with small segments (&lt;10k docs) merged
          first — so <code>compact()</code> is for forcing the issue, e.g. after a large
          delete batch. Conflicting with an in-progress reindex/compact fails with{' '}
          <code>E_BUSY</code> (-9).
        </p>

        <h2 id="reindex">reindex</h2>
        <p>
          Rebuilds every segment with a new tokenizer identity — for example after a new
          user dictionary has been staged (<code>setUserDict</code> in the Node
          binding). The rebuild runs in the background and incrementally; old segments
          stay read-only until an atomic switch, so queries keep working throughout. The
          returned{' '}
          <code>ReindexHandle</code> is pollable (<code>progress(): f32</code>) or
          blocking (<code>wait(): Result&lt;()&gt;</code>). On the browser, <code>vane.reindex(col)</code> runs synchronously
          inside the call and resolves with the final progress (<code>1.0</code> when done). Full state machine:{' '}
          <Link to="/guides/reindex">Custom Dict &amp; Reindex</Link>.
        </p>

        <h2 id="export">export</h2>
        <p>
          <code>Db.export(destPath)</code> packs the whole database into a single-file
          snapshot — the backup and migration format. On the browser the snapshot is
          written inside the VFS container; read it back with{' '}
          <code>vane.readFile(dest)</code> (<code>Uint8Array</code>) and hand it to a{' '}
          <code>Blob</code> for download.
        </p>

        <h2 id="close">close</h2>
        <p>
          Flushes pending state and releases the handle. After <code>close</code>, any
          further call on the object fails. In Go, collections have their own{' '}
          <code>col.Close()</code>; in the browser, <code>vane.close()</code>{' '}
          invalidates every handle the worker issued.
        </p>

        <h3>Error handling</h3>
        <p>
          Conflicting maintenance operations fail with <code>E_BUSY</code> (-9); a
          destination that cannot be written fails with <code>E_IO</code> (-1). Node
          rejects with <code>VaneError</code>, Go returns <code>*vane.VaneError</code>.
          See <Link to="/api/errors">Error Codes</Link>.
        </p>
      </div>
    </DocsLayout>
  );
}
