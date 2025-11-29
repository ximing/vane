import DocsLayout from '../components/DocsLayout';
import CodeBlock from '../components/CodeBlock';
import Callout from '../components/Callout';
import './Examples.css';

const NODE_RUN = `cd examples/demo
npm install            # local link @vane-rs/node
npm run load           # load 10k docs → ./vane-data/
npm run compare        # three-column ranking comparison
npm run smoke:vector   # pseudo-vector module self-check
npm run smoke:data     # data generator self-check`;

const BROWSER_RUN = `# Prerequisites: Rust (wasm32-unknown-unknown target) + wasm-bindgen CLI
# (+ wasm-opt, optional), Node.js ≥ 18 (e2e smoke test), Python 3

# At the repository root — produces demo/pkg/ (JS glue, simd/scalar wasm,
# dict.bin + sha256_prefix.bin)
bash demo/build.sh

cd demo
python3 -m http.server 8765
# open http://localhost:8765/index.html`;

const GO_PREREQ = `# 1. Build the static lib (or download libvane_ffi-<lib_dir>.a from GitHub Releases)
cargo build --release -p vane-ffi

# 2. Place it where cgo expects it (os-arch subdirectory of bindings/go/lib/)
mkdir -p bindings/go/lib/$(go env GOOS)-$(go env GOARCH)
cp target/release/libvane_ffi.a bindings/go/lib/$(go env GOOS)-$(go env GOARCH)/

# 3. Run the example
cd bindings/go && go run ./example`;

export default function Examples() {
  return (
    <DocsLayout>
      <div className="examples">
        <h1>Examples</h1>
        <p>
          Three runnable projects in the repository, one per runtime. Each card lists what it
          does, the real prerequisites for running it, and a link to the source.
        </p>

        <section className="examples__card">
          <h2 id="node-ranking-compare">Node: three-way ranking comparison</h2>
          <p className="examples__path">
            <code>examples/demo/</code> ·{' '}
            <a href="https://github.com/ximing/vane/tree/main/examples/demo">
              github.com/ximing/vane → examples/demo
            </a>
          </p>
          <h3>What it does</h3>
          <p>
            Loads 10,000 synthetic English wiki abstracts (deterministic PRNG seed — offline
            reproducible, not a real wiki corpus) into Vane, runs the same queries in{' '}
            <code>hybrid</code> / <code>vector</code> / <code>text</code> modes, and prints the
            top-10 ids as a three-column table to prove RRF fusion actually changes the
            ranking. It closes with a code-volume comparison against a hand-rolled sqlite-vec +
            FTS5 setup.
          </p>
          <h3>How to run</h3>
          <p>
            This is <strong>not</strong> a one-command demo. It depends on{' '}
            <code>@vane-rs/node</code> through a local <code>file:</code> link (see its{' '}
            <code>package.json</code>), so you must build the Node binding first:
          </p>
          <ul>
            <li>Node.js ≥ 18</li>
            <li>
              Build the binding: <code>cd crates/vane-node && napi build --platform --release</code>{' '}
              (produces the <code>*.node</code> binary the demo links against)
            </li>
          </ul>
          <CodeBlock code={NODE_RUN} lang="bash" title="terminal" />
          <h3>Code-volume comparison vs hand-rolled sqlite-vec + FTS5</h3>
          <table>
            <thead>
              <tr>
                <th>Step</th>
                <th>Hand-rolled sqlite-vec + FTS5</th>
                <th>Vane</th>
              </tr>
            </thead>
            <tbody>
              <tr>
                <td>Tables / schema</td>
                <td>
                  FTS5 + vec0 virtual tables, plus a rowid mapping table
                </td>
                <td>
                  <code>db.collection("wiki", schema, opts)</code> — one call
                </td>
              </tr>
              <tr>
                <td>Insert</td>
                <td>Separate inserts into fts / vec / rowid tables, wrapped in a transaction</td>
                <td>
                  <code>col.add(docs)</code>
                </td>
              </tr>
              <tr>
                <td>Flush</td>
                <td>
                  Manual <code>COMMIT</code>
                </td>
                <td>
                  <code>col.flush()</code>
                </td>
              </tr>
              <tr>
                <td>Text search</td>
                <td>
                  SQL with <code>bm25(docs_fts)</code>, <code>MATCH</code>, <code>ORDER BY</code>,{' '}
                  <code>LIMIT</code>
                </td>
                <td>
                  <code>col.search({'{ text, mode: "text" }'})</code>
                </td>
              </tr>
              <tr>
                <td>Vector search</td>
                <td>
                  SQL with <code>embedding MATCH ?</code> + <code>ORDER BY distance</code>
                </td>
                <td>
                  <code>col.search({'{ vector, mode: "vector" }'})</code>
                </td>
              </tr>
              <tr>
                <td>Hybrid fusion</td>
                <td>
                  ~40–60 lines of hand-written RRF (<code>1/(60+rank)</code> merge over two
                  queries)
                </td>
                <td>
                  <code>col.search({'{ text, vector, mode: "hybrid", fusion: "rrf" }'})</code>
                </td>
              </tr>
              <tr>
                <td>Estimated total</td>
                <td>~150–200 lines (fusion, dual-table sync, id mapping)</td>
                <td>6 Vane API calls</td>
              </tr>
            </tbody>
          </table>
          <p>
            <small>
              The demo itself measures 449 total lines / 358 SLOC, but almost all of that is
              batch loading, table printing, and diff statistics. The corpus generator (
              <code>lib/data.js</code>, 107 SLOC) and the pseudo-vector module (
              <code>lib/vector.js</code>, 86 SLOC) are demo-only overhead — a real deployment
              brings its own corpus and embeddings.
            </small>
          </p>
        </section>

        <section className="examples__card">
          <h2 id="browser-markdown-search">Browser: pure-frontend Markdown search</h2>
          <p className="examples__path">
            <code>demo/</code> ·{' '}
            <a href="https://github.com/ximing/vane/tree/main/demo">
              github.com/ximing/vane → demo
            </a>
          </p>
          <img
            className="examples__shot"
            src={`${import.meta.env.BASE_URL}screenshots/browser-markdown-demo.jpg`}
            width={1600}
            height={800}
            alt="Vane browser demo: hybrid search results for the Chinese query 人工智能 over a folder of Markdown files"
            loading="lazy"
          />
          <h3>What it does</h3>
          <ul>
            <li>
              Drag in a folder — recursively parses <code>.md</code> files into{' '}
              <code>{'{id, text, vector}'}</code> documents
            </li>
            <li>
              Chinese search with <code>jieba</code> — dictionary fetched from the jsdelivr gh
              CDN, sha256-verified, cached in OPFS; offline it degrades to bigram without an
              error
            </li>
            <li>Hybrid search — text + vector fused with RRF (placeholder hash vectors)</li>
            <li>OPFS persistence — data survives reloads (IndexedDB fallback)</li>
            <li>
              SIMD dual variants — a runtime <code>WebAssembly.validate</code> probe loads the
              simd or scalar build
            </li>
            <li>
              Export backup — <code>db.export("backup.vane")</code> writes a snapshot into OPFS
            </li>
          </ul>
          <h3>How to run</h3>
          <p>
            The wasm artifacts are build output, so you compile them locally before serving the
            page:
          </p>
          <CodeBlock code={BROWSER_RUN} lang="bash" title="terminal" />
          <p>
            You must access the page over <code>http://localhost</code> — under{' '}
            <code>file://</code>, Worker / OPFS / ES modules are restricted.
          </p>
        </section>

        <section className="examples__card">
          <h2 id="go-open-add-search">Go: open → load dict → add → search</h2>
          <p className="examples__path">
            <code>bindings/go/example/</code> ·{' '}
            <a href="https://github.com/ximing/vane/tree/main/bindings/go/example">
              github.com/ximing/vane → bindings/go/example
            </a>
          </p>
          <h3>What it does</h3>
          <p>
            A minimal cgo end-to-end pass (<code>main.go</code>): open a database in a temp
            directory → load the embedded <code>jieba</code> dictionary → create a collection →
            add documents → flush → search → close.
          </p>
          <h3>How to run</h3>
          <p>
            The Go binding links <code>libvane_ffi.a</code> via cgo, so the static library must
            be in place first (prebuilt libs on GitHub Releases cover{' '}
            <code>linux-amd64</code>, <code>linux-arm64</code>, <code>darwin-amd64</code>,{' '}
            <code>darwin-arm64</code>):
          </p>
          <CodeBlock code={GO_PREREQ} lang="bash" title="terminal" />
          <Callout type="warning" title="cgo path only">
            <p>
              The example carries <code>//go:build !wazero</code> and builds only on the default
              cgo path. In v0.1.x the <code>wazero</code> build tag is a <strong>stub</strong> —
              the pure-Go variant is not implemented and is <strong>deferred</strong> (see the
              README “Status” section).
            </p>
          </Callout>
        </section>

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
