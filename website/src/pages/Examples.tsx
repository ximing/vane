import { Link } from 'react-router-dom';
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

const BROWSER_VITE_RUN = `cd examples/vite
npm install
npm run dev
# open http://localhost:5173/`;

const BROWSER_WEBPACK_RUN = `cd examples/webpack
npm install
npm run serve
# open http://localhost:8080/`;

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
          <h2 id="browser-vite-webpack">Browser: vite + webpack integration</h2>
          <p className="examples__path">
            <code>examples/vite/</code> ·{' '}
            <a href="https://github.com/ximing/vane/tree/main/examples/vite">
              github.com/ximing/vane → examples/vite
            </a>
            <br />
            <code>examples/webpack/</code> ·{' '}
            <a href="https://github.com/ximing/vane/tree/main/examples/webpack">
              github.com/ximing/vane → examples/webpack
            </a>
          </p>
          <h3>What it does</h3>
          <p>
            Two minimal end-to-end projects that import <code>@vane-rs/web</code> from npm,
            inline the dictionary via <code>@vane-rs/dict-zh</code>, and run a hybrid search
            loop in the browser — no local Rust toolchain, no wasm-bindgen CLI, no build
            script. Vite 6+ and webpack 5 both recognize the{' '}
            <code>new Worker(new URL(..., import.meta.url))</code> idiom natively, so the
            worker is emitted as a separate ESM chunk automatically.
          </p>
          <h3>How to run — vite</h3>
          <CodeBlock code={BROWSER_VITE_RUN} lang="bash" title="terminal" />
          <h3>How to run — webpack</h3>
          <CodeBlock code={BROWSER_WEBPACK_RUN} lang="bash" title="terminal" />
          <Callout type="note" title="Bundler configuration">
            Both examples ship with the minimal config you actually need (Vite:{' '}
            <code>assetsInclude: ['**/*.bin']</code>; webpack:{' '}
            <code>experiments.outputModule</code> + asset rules). See the{' '}
            <Link to="/guides/web-integration">Web Integration</Link> guide for the
            rationale and the common pitfalls (file:// origin,{' '}
            <code>scriptLoading</code>, dictData detach).
          </Callout>
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
