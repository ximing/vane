import DocsLayout from '../../components/DocsLayout';
import LangTabs from '../../components/LangTabs';
import CodeBlock from '../../components/CodeBlock';
import Callout from '../../components/Callout';
import './api.css';

const IDL = `// Error surface contract (SPEC §9 / §10)
// Node:     Promise rejects with VaneError { code, name, message }
// Go:       (T, error) — error is *vane.VaneError { Code, Message }
// Browser:  Vane Promise rejects; the message carries the E_* name
// C ABI:    int32_t return code (0 = OK, negative = code);
//           details via vane_last_error_message(h) -> char*`;

const NODE_SNIPPET = `import vane, { VaneError } from '@vane-rs/node';

try {
  await col.add([{ id: 'x', text: 'hi', vector: [1.0, 2.0] }]); // wrong dim
} catch (err) {
  if (err instanceof VaneError) {
    console.error(err.code, err.name, err.message);
    // -2  'E_SCHEMA'  'vector dim mismatch: expected 4, got 2'
  }
}`;

const GO_SNIPPET = `import (
    "errors"
    "log"

    "github.com/ximing/vane/bindings/go"
)

err := col.Add([]vane.Doc{{ID: "x", Text: "hi", Vector: []float32{1, 2}}})
var ve *vane.VaneError
if errors.As(err, &ve) {
    switch ve.Code {
    case vane.ESchema:        // -2
        // fix the document shape
    case vane.EDictTooLarge:  // -7
        // trim the user dictionary
    default:
        log.Fatalf("vane error %d: %s", ve.Code, ve.Message)
    }
}`;

const BROWSER_SNIPPET = `// 假设 vane 已由 createVane({dictData}) 创建，col = await vane.collection(...)
try {
  await vane.add(col, [{ id: 'x', text: 'hi', vector: [1.0, 2.0] }]); // wrong dim
} catch (err) {
  // The rejected value's text carries the SPEC §10 error name.
  const msg = String(err);
  if (msg.includes('E_SCHEMA')) {
    // fix the document shape
  }
}
// Note: dictionary problems never reject here — with dictData inlined
// from @vane-rs/dict-zh there is no CDN fetch to fail; without dictData
// the browser auto-degrades jieba → cjk_bigram with a console warning.`;

export default function ApiErrors() {
  return (
    <DocsLayout>
      <div className="api-page">
        <h1>Error Codes</h1>
        <p className="api-lead">
          One error-code table, passed through unchanged on all three bindings — codes are
          never swallowed or renumbered (SPEC §10). A return of <code>0</code> means OK;
          every negative code is listed below.
        </p>

        <pre className="api-idl">
          <code>{IDL}</code>
        </pre>

        <h2 id="error-codes">Error codes</h2>
        <div className="api-table-wrap">
          <table className="api-table">
            <thead>
              <tr>
                <th>Code</th>
                <th>Name</th>
                <th>Meaning</th>
              </tr>
            </thead>
            <tbody>
              <tr>
                <td><code>-1</code></td>
                <td><code>E_IO</code></td>
                <td className="api-table__wide">VFS-layer read/write failure.</td>
              </tr>
              <tr>
                <td><code>-2</code></td>
                <td><code>E_SCHEMA</code></td>
                <td className="api-table__wide">
                  Schema violation (wrong vector dimension, wrong field type, unknown
                  field).
                </td>
              </tr>
              <tr>
                <td><code>-3</code></td>
                <td><code>E_NOT_FOUND</code></td>
                <td className="api-table__wide">Collection or document id does not exist.</td>
              </tr>
              <tr>
                <td><code>-4</code></td>
                <td><code>E_CORRUPT</code></td>
                <td className="api-table__wide">
                  Segment/manifest checksum failed (magic, version, sha256).
                </td>
              </tr>
              <tr>
                <td><code>-5</code></td>
                <td><code>E_VERSION</code></td>
                <td className="api-table__wide">
                  Format version is newer than this build and no migrator exists.
                </td>
              </tr>
              <tr>
                <td><code>-6</code></td>
                <td><code>E_TOKENIZER_MISMATCH</code></td>
                <td className="api-table__wide">
                  Query-time tokenizer identity differs from the segment's (signals a
                  reindex state).
                </td>
              </tr>
              <tr>
                <td><code>-7</code></td>
                <td><code>E_DICT_TOO_LARGE</code></td>
                <td className="api-table__wide">User dictionary exceeds 100k entries.</td>
              </tr>
              <tr>
                <td><code>-8</code></td>
                <td><code>E_DICT_UNAVAILABLE</code></td>
                <td className="api-table__wide">
                  jieba dictionary not loaded while <code>tokenizer: "jieba"</code> is
                  declared. Never raised on WASM — see{' '}
                  <a href="#wasm-note">the WASM note</a>.
                </td>
              </tr>
              <tr>
                <td><code>-9</code></td>
                <td><code>E_BUSY</code></td>
                <td className="api-table__wide">
                  A reindex/compact is in progress; the operation conflicts with it.
                </td>
              </tr>
              <tr>
                <td><code>-10</code></td>
                <td><code>E_UNSUPPORTED</code></td>
                <td className="api-table__wide">
                  Platform capability missing (e.g. no OPFS and no IndexedDB fallback
                  enabled).
                </td>
              </tr>
              <tr>
                <td><code>-11</code></td>
                <td><code>E_INVALID_ARG</code></td>
                <td className="api-table__wide">
                  Illegal argument (<code>topK &gt; 1000</code>, filter applied to a
                  non-scalar field, etc.).
                </td>
              </tr>
            </tbody>
          </table>
        </div>

        <h2 id="wasm-note">WASM note: E_DICT_UNAVAILABLE is unreachable</h2>
        <Callout type="note" title="Browser dictionary degradation">
          On the browser side, <code>E_DICT_UNAVAILABLE</code> (-8) can never be raised by
          design. When the jieba dictionary cannot be fetched or verified, the WASM
          binding automatically degrades to <code>cjk_bigram</code> with a console warning
          instead of failing (SPEC §10 note). Declaring <code>tokenizer: "jieba"</code>{' '}
          in the browser is therefore always safe — you get jieba when the dictionary is
          available and bigram search when it is not. When <code>dictData</code> is
          inlined from <code>@vane-rs/dict-zh</code> there is no network fetch at
          all, so the CDN failure path is unreachable too.
        </Callout>

        <h2 id="handling">Handling errors</h2>
        <p>
          Same codes, three notations: Node rejects Promises with a{' '}
          <code>VaneError</code> subclass (<code>err.code</code> / <code>err.name</code>);
          Go returns <code>(T, error)</code> with <code>*vane.VaneError</code> carrying{' '}
          <code>Code</code> / <code>Message</code> plus per-code constants such as{' '}
          <code>vane.ESchema</code>; the browser worker rejects with a value whose text
          carries the error name.
        </p>
        <LangTabs
          node={<CodeBlock code={NODE_SNIPPET} lang="js" title="errors.mjs" />}
          go={<CodeBlock code={GO_SNIPPET} lang="go" title="errors.go" />}
          browser={<CodeBlock code={BROWSER_SNIPPET} lang="ts" title="errors.ts" />}
        />
      </div>
    </DocsLayout>
  );
}
