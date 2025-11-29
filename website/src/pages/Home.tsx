import { useEffect, useState } from 'react';
import { Link } from 'react-router-dom';
import CodeBlock from '../components/CodeBlock';
import Footer from '../components/Footer';
import LangTabs from '../components/LangTabs';
import SearchDemo from '../components/SearchDemo';
import TopBar from '../components/TopBar';
import './Home.css';

const GITHUB_URL = 'https://github.com/ximing/vane';

/* ------------------------------------------------------------------ */
/* Hero terminal — pure JS typing loop over static fake data.          */
/* ------------------------------------------------------------------ */

interface ScriptLine {
  text: string;
  kind: 'cmd' | 'out';
}

const SCRIPT: ScriptLine[] = [
  { text: 'npm install @vane-rs/node', kind: 'cmd' },
  { text: 'added 1 package in 2s', kind: 'out' },
  { text: 'node search.mjs', kind: 'cmd' },
  { text: '  id  score     text', kind: 'out' },
  { text: '  a   0.0321    hello world', kind: 'out' },
  { text: '  c   0.0297    hello foo', kind: 'out' },
  { text: '  b   0.0188    foo bar baz', kind: 'out' },
];

const TYPE_MS = 45;
const LINE_PAUSE_MS = 320;
const CMD_END_PAUSE_MS = 500;
const RESTART_PAUSE_MS = 5000;

function prefersReducedMotion(): boolean {
  return (
    typeof window !== 'undefined' &&
    window.matchMedia('(prefers-reduced-motion: reduce)').matches
  );
}

function Terminal() {
  const [reduced] = useState(prefersReducedMotion);
  const [done, setDone] = useState<ScriptLine[]>([]);
  const [typing, setTyping] = useState('');

  useEffect(() => {
    if (reduced) {
      setDone(SCRIPT);
      setTyping('');
      return;
    }
    let cancelled = false;
    let timer = 0;
    let line = 0;
    let char = 0;

    const step = () => {
      if (cancelled) return;
      if (line >= SCRIPT.length) {
        timer = window.setTimeout(() => {
          if (cancelled) return;
          setDone([]);
          setTyping('');
          line = 0;
          char = 0;
          timer = window.setTimeout(step, 600);
        }, RESTART_PAUSE_MS);
        return;
      }
      const current = SCRIPT[line];
      if (current.kind === 'cmd') {
        char += 1;
        setTyping(current.text.slice(0, char));
        if (char >= current.text.length) {
          timer = window.setTimeout(() => {
            if (cancelled) return;
            setDone((d) => [...d, current]);
            setTyping('');
            line += 1;
            char = 0;
            timer = window.setTimeout(step, LINE_PAUSE_MS);
          }, CMD_END_PAUSE_MS);
        } else {
          timer = window.setTimeout(step, TYPE_MS);
        }
      } else {
        setDone((d) => [...d, current]);
        line += 1;
        timer = window.setTimeout(step, LINE_PAUSE_MS);
      }
    };

    timer = window.setTimeout(step, 600);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [reduced]);

  return (
    <div className="term">
      <div className="term__bar">
        <span className="term__dots" aria-hidden="true">
          <i className="term__dot" />
          <i className="term__dot" />
          <i className="term__dot" />
        </span>
        <span className="term__title">terminal</span>
      </div>
      <div className="term__body">
        {done.map((l, i) =>
          l.kind === 'cmd' ? (
            <div key={i} className="term__line">
              <span className="term__prompt">$</span> {l.text}
            </div>
          ) : (
            <div key={i} className="term__line term__line--out">
              {l.text}
            </div>
          ),
        )}
        {!reduced && (
          <div className="term__line">
            <span className="term__prompt">$</span> {typing}
            <span className="term__cursor" aria-hidden="true" />
          </div>
        )}
      </div>
    </div>
  );
}

/* ------------------------------------------------------------------ */
/* Feature grid icons — hand-drawn inline SVG line art, no emoji.      */
/* ------------------------------------------------------------------ */

const iconProps = {
  viewBox: '0 0 32 32',
  width: 28,
  height: 28,
  fill: 'none',
  stroke: 'currentColor',
  strokeWidth: 1.5,
  strokeLinecap: 'round',
  strokeLinejoin: 'round',
  'aria-hidden': true,
} as const;

function IconRuntimes() {
  return (
    <svg {...iconProps}>
      <rect x="12" y="12" width="8" height="8" rx="1" />
      <rect x="2" y="2" width="6" height="6" rx="1" />
      <rect x="24" y="2" width="6" height="6" rx="1" />
      <rect x="2" y="24" width="6" height="6" rx="1" />
      <rect x="24" y="24" width="6" height="6" rx="1" />
      <path d="M8 5h4M27 8v4M8 27h4M27 24v-4M12 16H8M20 16h4M16 12V8M16 20v4" />
    </svg>
  );
}

function IconHybrid() {
  return (
    <svg {...iconProps}>
      <path d="M6 6v6a4 4 0 0 0 4 4h4" />
      <path d="M26 6v6a4 4 0 0 1-4 4h-4" />
      <path d="M16 16v10" />
      <path d="M12 22l4 4 4-4" />
    </svg>
  );
}

function IconChinese() {
  return (
    <svg {...iconProps}>
      <rect x="3" y="5" width="12" height="12" rx="1" />
      <rect x="17" y="15" width="12" height="12" rx="1" />
      <path d="M6 9h6M9 8v8" />
      <path d="M20 24l3-6 3 6M21 22.5h4" />
    </svg>
  );
}

function IconEmbedded() {
  return (
    <svg {...iconProps}>
      <ellipse cx="16" cy="7" rx="10" ry="3.5" />
      <path d="M6 7v9c0 1.9 4.5 3.5 10 3.5S26 17.9 26 16V7" />
      <path d="M6 16v9c0 1.9 4.5 3.5 10 3.5S26 26.9 26 25v-9" />
    </svg>
  );
}

const FEATURES = [
  {
    icon: <IconRuntimes />,
    title: 'One core, four runtimes',
    body: 'The same Rust engine powers Node (napi-rs), Go (cgo static lib), and the browser (wasm-bindgen + Web Worker). Bindings are thin shells — no logic is duplicated across languages.',
  },
  {
    icon: <IconHybrid />,
    title: 'Hybrid by default',
    body: 'mode: "hybrid" runs vector + BM25 in parallel and fuses with RRF (k = 60), with zero tuning. Recall@10 ≥ 0.95 is a hard CI gate.',
  },
  {
    icon: <IconChinese />,
    title: 'First-class Chinese',
    body: 'A jieba tokenizer (DAG + HMM, ~200k-word trimmed dictionary) ships alongside standard and cjk_bigram. Mixed CJK/Latin text is segmented correctly; inject your own domain terms with userDict.',
  },
  {
    icon: <IconEmbedded />,
    title: 'Embedded & durable',
    body: 'Directory-based segments + an atomically-switched manifest, crash-safe WAL, and a single-file export() snapshot. No server, no GPU, no mmap.',
  },
];

/* ------------------------------------------------------------------ */
/* Quickstart snippets (trimmed from README "Quick start").            */
/* ------------------------------------------------------------------ */

const NODE_SNIPPET = `import vane from '@vane-rs/node';
const { open } = vane;

const db = await open('./mydb', { autoCommit: 'off' });

const col = await db.collection('docs', {
  fields: [
    { name: 'body', type: 'text' },
    { name: 'vec', type: 'vector', dim: 4, metric: 'cosine' },
  ],
}, { tokenizer: 'standard' });

await col.add([
  { id: 'a', text: 'hello world', vector: [1.0, 0.0, 0.0, 0.0] },
  { id: 'b', text: 'foo bar baz', vector: [0.0, 1.0, 0.0, 0.0] },
  { id: 'c', text: 'hello foo',   vector: [0.7, 0.3, 0.0, 0.0] },
]);
await col.flush(); // data is now searchable

const hits = await col.search({
  text: 'hello', vector: [1.0, 0.0, 0.0, 0.0],
  topK: 3, mode: 'hybrid', fusion: 'rrf',
});`;

const GO_SNIPPET = `db, err := vane.Open("./mydb", nil)
if err != nil { log.Fatalf("Open: %v", err) }
defer db.Close()

// Load the bundled jieba dictionary.
if b, err := dict.DictBytes(); err == nil {
	_ = db.LoadDict(b)
}

schema := vane.Schema{Fields: []vane.SchemaField{
	{Name: "body", Type: "text"},
	{Name: "vec", Type: "vector", Dim: 4, Metric: "cosine"},
}}
col, _ := db.Collection("docs", schema,
	&vane.CollectionOptions{Tokenizer: "jieba"})
defer col.Close()

_ = col.Add([]vane.Doc{
	{ID: "a", Text: "hello world", Vector: []float32{1.0, 0.0, 0.0, 0.0}},
	{ID: "b", Text: "foo bar baz", Vector: []float32{0.0, 1.0, 0.0, 0.0}},
})
_ = col.Flush()

hits, _ := col.Search(vane.SearchQuery{
	Text: "hello", Vector: []float32{1.0, 0.0, 0.0, 0.0}, TopK: 3,
})`;

const BROWSER_SNIPPET = `# Build the wasm dual variants + JS glue + dict.bin
bash demo/build.sh

cd demo && python3 -m http.server 8765
# open http://localhost:8765/ — drag in a folder
# of .md files and search, fully in-browser`;

/* ------------------------------------------------------------------ */
/* Comparison table (README "What is Vane").                           */
/* ------------------------------------------------------------------ */

const COMPARISON_ROWS = [
  {
    want: 'Vector + text search, in-process',
    stack: 'sqlite-vec + FTS5, hand-rolled fusion glue',
    giveUp: 'Atomic hybrid ranking, one filter model, ~200 lines of plumbing',
  },
  {
    want: 'Browser-side semantic search',
    stack: 'A pure-JS engine',
    giveUp: 'Performance ceiling, no Rust core to reuse on Node/Go',
  },
  {
    want: 'Chinese-aware tokenization',
    stack: 'Tantivy + a tokenizer crate',
    giveUp: 'A browser build, or a second engine for the client',
  },
];

/* ------------------------------------------------------------------ */
/* Performance gates (README "Performance" / SPEC §13.1).              */
/* ------------------------------------------------------------------ */

const PERF_STATS = [
  {
    value: '< 50 ms',
    label: 'Hybrid topK=10 P99 — HNSW path',
    basis: '100k docs × 384 dims, native / Node',
  },
  {
    value: '< 150 ms',
    label: 'Hybrid topK=10 P99 — brute-force fallback',
    basis: 'Same 100k × 384d native setup',
  },
  {
    value: '≥ 0.95',
    label: 'Hybrid recall@10 vs brute-force dual-path + RRF baseline',
    basis: 'Hard CI gate',
  },
  {
    value: '≥ 5,000 docs/s',
    label: 'Batch add throughput, index build included',
    basis: 'Same 100k × 384d native setup',
  },
  {
    value: '≤ 800 KB',
    label: 'core wasm, gzip',
    basis: 'Tokenizer code included, dictionary data excluded — hard CI size gate',
  },
];

/* ------------------------------------------------------------------ */
/* Page                                                                */
/* ------------------------------------------------------------------ */

export default function Home() {
  return (
    <div className="home">
      <TopBar />

      {/* 1. Hero */}
      <section className="home__hero">
        <div className="home__container home__hero-grid">
          <div className="home__hero-copy">
            <h1 className="home__title">
              Vector + BM25 hybrid retrieval, embedded.
            </h1>
            <p className="home__lede">
              Vane is a lightweight hybrid retrieval library built on a single
              Rust core that embeds into desktop, Node.js, Go, and the browser.
              It pairs segment HNSW vector search with Block-Max WAND BM25, and
              fuses the two with RRF — sqlite-vec's embedded shape,
              Tantivy-grade text search, and unified hybrid ranking in one
              library.
            </p>
            <div className="home__badges">
              <span className="home__badge">Node.js</span>
              <span className="home__badge">Go</span>
              <span className="home__badge">Browser</span>
            </div>
            <div className="home__ctas">
              <Link to="/quickstart" className="home__btn home__btn--primary">
                Get Started
              </Link>
              <a
                href={GITHUB_URL}
                target="_blank"
                rel="noopener noreferrer"
                className="home__btn home__btn--secondary"
              >
                GitHub
              </a>
            </div>
          </div>
          <div className="home__hero-term">
            <Terminal />
          </div>
        </div>
      </section>

      {/* 2. Feature grid */}
      <section className="home__section">
        <div className="home__container">
          <h2 className="home__h2">Why Vane</h2>
          <div className="home__features">
            {FEATURES.map((f) => (
              <article key={f.title} className="home__feature">
                <div className="home__feature-icon">{f.icon}</div>
                <h3 className="home__feature-title">{f.title}</h3>
                <p className="home__feature-body">{f.body}</p>
              </article>
            ))}
          </div>
        </div>
      </section>

      {/* 3. Three-language quickstart */}
      <section className="home__section home__section--alt">
        <div className="home__container">
          <h2 className="home__h2">Quick start, three runtimes</h2>
          <p className="home__section-lede">
            The examples below use 4-dimensional dummy vectors so they run
            as-is. In production, replace <code>vector</code> with real
            embeddings from your model — Vane indexes and searches whatever you
            give it.
          </p>
          <LangTabs
            node={<CodeBlock code={NODE_SNIPPET} lang="js" title="search.mjs" />}
            go={<CodeBlock code={GO_SNIPPET} lang="go" title="main.go" />}
            browser={
              <CodeBlock code={BROWSER_SNIPPET} lang="bash" title="shell" />
            }
          />
        </div>
      </section>

      {/* 4. Comparison table */}
      <section className="home__section">
        <div className="home__container">
          <h2 className="home__h2">What you stop giving up</h2>
          <p className="home__section-lede">
            Vane exists because the obvious alternatives each give up something:
          </p>
          <div className="home__table-wrap">
            <table className="home__table">
              <thead>
                <tr>
                  <th>You want…</th>
                  <th>Typical stack</th>
                  <th>What you give up</th>
                </tr>
              </thead>
              <tbody>
                {COMPARISON_ROWS.map((r) => (
                  <tr key={r.want}>
                    <td>{r.want}</td>
                    <td>{r.stack}</td>
                    <td>{r.giveUp}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      </section>

      {/* 5. Architecture */}
      <section className="home__section home__section--alt">
        <div className="home__container">
          <h2 className="home__h2">One Rust core, thin bindings</h2>
          <div className="home__arch">
            <svg
              viewBox="0 0 760 300"
              role="img"
              aria-label="Architecture diagram: Node, Go, and the C ABI on the left and the browser on the right all call into a single vane-core Rust engine containing the VFS layer, immutable segments with manifest switch, HNSW plus Block-Max WAND BM25, and RRF fusion."
              className="home__arch-svg"
            >
              {/* left bindings */}
              <g className="arch-box">
                <rect x="10" y="28" width="160" height="52" rx="6" />
                <text x="90" y="49" textAnchor="middle" className="arch-label">
                  Node
                </text>
                <text x="90" y="66" textAnchor="middle" className="arch-sub">
                  napi-rs
                </text>
              </g>
              <g className="arch-box">
                <rect x="10" y="124" width="160" height="52" rx="6" />
                <text x="90" y="145" textAnchor="middle" className="arch-label">
                  Go
                </text>
                <text x="90" y="162" textAnchor="middle" className="arch-sub">
                  cgo / static .a
                </text>
              </g>
              <g className="arch-box">
                <rect x="10" y="220" width="160" height="52" rx="6" />
                <text x="90" y="241" textAnchor="middle" className="arch-label">
                  C ABI
                </text>
                <text x="90" y="258" textAnchor="middle" className="arch-sub">
                  FFI
                </text>
              </g>
              {/* right binding */}
              <g className="arch-box">
                <rect x="590" y="112" width="160" height="76" rx="6" />
                <text x="670" y="141" textAnchor="middle" className="arch-label">
                  Browser
                </text>
                <text x="670" y="158" textAnchor="middle" className="arch-sub">
                  wasm-bindgen + Worker
                </text>
                <text x="670" y="174" textAnchor="middle" className="arch-sub">
                  OPFS / IDB
                </text>
              </g>
              {/* connectors */}
              <path className="arch-line" d="M170 54 H210 V150 H230" />
              <path className="arch-line" d="M170 150 H230" />
              <path className="arch-line" d="M170 246 H210 V150 H230" />
              <path className="arch-line" d="M530 150 H590" />
              {/* core */}
              <g className="arch-core">
                <rect x="230" y="40" width="300" height="220" rx="6" />
                <text x="380" y="68" textAnchor="middle" className="arch-title">
                  vane-core
                </text>
                <text x="380" y="86" textAnchor="middle" className="arch-sub">
                  one Rust engine, no mmap
                </text>
                <text x="252" y="116" className="arch-item">
                  · VFS trait: std-fs / OPFS / IDB / mem
                </text>
                <text x="252" y="146" className="arch-item">
                  · immutable segments + manifest switch
                </text>
                <text x="252" y="176" className="arch-item">
                  · HNSW (per-seg) + Block-Max WAND BM25
                </text>
                <text x="252" y="206" className="arch-item">
                  · RRF fusion, pre-filter bitmaps
                </text>
                <text x="252" y="236" className="arch-item">
                  · crash-safe WAL, export() snapshot
                </text>
              </g>
            </svg>
          </div>
        </div>
      </section>

      {/* 6. Performance gates */}
      <section className="home__section">
        <div className="home__container">
          <h2 className="home__h2">Performance gates</h2>
          <p className="home__section-lede">
            Hard numeric gates enforced in CI — targets, not marketing numbers.
          </p>
          <div className="home__stats">
            {PERF_STATS.map((s) => (
              <div key={s.label} className="home__stat">
                <div className="home__stat-value">{s.value}</div>
                <div className="home__stat-label">{s.label}</div>
                <div className="home__stat-basis">{s.basis}</div>
              </div>
            ))}
          </div>
        </div>
      </section>

      {/* 7. Won't-have statement */}
      <section className="home__section home__section--alt">
        <div className="home__container">
          <blockquote className="home__quote">
            <p>
              Vane does not generate embeddings, run models, or speak
              SQL/distributed. It is a retrieval library — fast, embeddable, and
              predictable.
            </p>
          </blockquote>
        </div>
      </section>

      {/* 8. SearchDemo */}
      <section className="home__section" id="live-demo">
        <div className="home__container">
          <h2 className="home__h2">Live demo</h2>
          <p className="home__section-lede">
            Three ranking modes on the same 32-doc corpus — hybrid (RRF fusion
            of vector + BM25) against each path alone.
          </p>
          <SearchDemo />
        </div>
      </section>

      {/* 9. Footer */}
      <Footer />
    </div>
  );
}
