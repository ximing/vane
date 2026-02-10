import { Link } from 'react-router-dom';
import DocsLayout from '../../components/DocsLayout';
import LangTabs from '../../components/LangTabs';
import CodeBlock from '../../components/CodeBlock';
import Callout from '../../components/Callout';
import './api.css';

const IDL = `Collection.search(query: SearchQuery) -> Result<[Hit]>

SearchQuery := {
  text?: string, vector?: [f32],        // at least one; both = hybrid
  topK?: u32 = 10,                      // max 1000
  mode?: "hybrid" | "vector" | "text",  // inferred from inputs when omitted
  fusion?: "rrf" | { linear: { alpha: f32, norm: "minmax" } },   // default "rrf"
  filter?: Filter,                       // §8.3 — see the known-gap note below
  candidateMultiplier?: u32 = 3          // RRF: each path takes topK × multiplier
}

Hit    := { id: string, score: f32, fields?: {…} }
Filter := { <scalarField>: { eq?: v, in?: [v], gte?: v, lte?: v } }   // AND across fields`;

const NODE_SNIPPET = `// col.search(query): Promise<Hit[]>
const hits = await col.search({
  text: 'hello',
  vector: [1.0, 0.0, 0.0, 0.0],
  topK: 3,
  mode: 'hybrid',   // 'vector' | 'text' | 'hybrid'
  fusion: 'rrf',    // default; or { linear: { alpha: 0.5 } }
});
// hits = [{ id, score, fields }, ...]`;

const GO_SNIPPET = `// func (c *Collection) Search(query SearchQuery) ([]Hit, error)
hits, err := col.Search(vane.SearchQuery{
    Text:   "hello",
    Vector: []float32{1.0, 0.0, 0.0, 0.0},
    TopK:   3,
    Mode:   "hybrid",
    Fusion: "rrf",
})
if err != nil {
    log.Fatalf("search: %v", err)
}
for _, h := range hits {
    fmt.Printf("hit: id=%s score=%.4f\n", h.ID, h.Score)
}`;

const BROWSER_SNIPPET = `// 假设 vane 已由 createVane({dictData}) 创建，col = await vane.collection(...)
import type { Hit } from '@vane-rs/web';

// vane.search(col, query): Promise<Hit[]> — 无需 JSON.parse（worker 内部已反序列化）
const hits: Hit[] = await vane.search(col, {
  text: 'hello',
  vector: [1.0, 0.0, 0.0, 0.0],
  topK: 3,
  mode: 'hybrid',
});
// hits = [{ id, score, fields }, ...]`;

export default function ApiSearch() {
  return (
    <DocsLayout>
      <div className="api-page">
        <h1>search</h1>
        <p className="api-lead">
          One query object drives all three recall paths — BM25 text, vector similarity,
          or both fused. What you pass decides what runs.
        </p>

        <pre className="api-idl">
          <code>{IDL}</code>
        </pre>

        <LangTabs
          node={<CodeBlock code={NODE_SNIPPET} lang="js" title="search.mjs" />}
          go={<CodeBlock code={GO_SNIPPET} lang="go" title="search.go" />}
          browser={<CodeBlock code={BROWSER_SNIPPET} lang="ts" title="search.ts" />}
        />

        <h2 id="search-query">SearchQuery</h2>
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
                <td><code>text</code></td>
                <td><code>string</code></td>
                <td>—</td>
                <td className="api-table__wide">
                  Input for the BM25 path. At least one of <code>text</code> /{' '}
                  <code>vector</code> is required; passing both means hybrid.
                </td>
              </tr>
              <tr>
                <td><code>vector</code></td>
                <td><code>[f32]</code></td>
                <td>—</td>
                <td className="api-table__wide">
                  Input for the vector path. Dimension must equal the schema{' '}
                  <code>dim</code>.
                </td>
              </tr>
              <tr>
                <td><code>topK</code></td>
                <td><code>u32</code></td>
                <td><code>10</code></td>
                <td className="api-table__wide">
                  Number of hits to return. Maximum 1000; larger values fail with{' '}
                  <code>E_INVALID_ARG</code> (-11).
                </td>
              </tr>
              <tr>
                <td><code>mode</code></td>
                <td className="api-table__wide"><code>"hybrid" | "vector" | "text"</code></td>
                <td className="api-table__wide">inferred from inputs</td>
                <td className="api-table__wide">
                  Which recall paths run. When omitted it is inferred from the inputs; an
                  explicit value wins.
                </td>
              </tr>
              <tr>
                <td><code>fusion</code></td>
                <td className="api-table__wide">
                  <code>"rrf" | {'{ linear: { alpha: f32, norm: "minmax" } }'}</code>
                </td>
                <td><code>"rrf"</code></td>
                <td className="api-table__wide">
                  How the two paths merge in hybrid mode. RRF is zero-tuning;{' '}
                  <code>linear</code> blends with a weight <code>alpha</code> after minmax
                  normalization.
                </td>
              </tr>
              <tr>
                <td><code>filter</code></td>
                <td><code>Filter</code></td>
                <td>—</td>
                <td className="api-table__wide">
                  Pre-filter on scalar fields. Implemented in the core but not yet exposed
                  by the bindings — see <a href="#filter">filter</a> below.
                </td>
              </tr>
              <tr>
                <td><code>candidateMultiplier</code></td>
                <td><code>u32</code></td>
                <td><code>3</code></td>
                <td className="api-table__wide">
                  In hybrid mode each path recalls <code>topK × candidateMultiplier</code>{' '}
                  candidates before fusion.
                </td>
              </tr>
            </tbody>
          </table>
        </div>

        <h2 id="hits">Hits</h2>
        <p>
          Each hit is <code>{'{ id: string, score: f32, fields?: {…} }'}</code> — the
          external document id, a score, and optionally the stored fields. Under the
          default <code>rrf</code> fusion the score is the RRF score (
          <code>Σ 1/(60 + rank)</code>); under <code>linear</code> fusion it is the
          normalized blend. Linear scores are normalized per query and are{' '}
          <strong>not comparable across corpora</strong>.
        </p>

        <h2 id="modes">Modes &amp; candidate multiplier</h2>
        <div className="api-table-wrap">
          <table className="api-table">
            <thead>
              <tr>
                <th>mode</th>
                <th>Recall path</th>
                <th>Ranked by</th>
              </tr>
            </thead>
            <tbody>
              <tr>
                <td><code>vector</code></td>
                <td className="api-table__wide">
                  HNSW per-segment parallel search, merged; brute-force fallback when
                  candidates &lt; 2×topK
                </td>
                <td>vector distance</td>
              </tr>
              <tr>
                <td><code>text</code></td>
                <td>Block-Max WAND top-k</td>
                <td>BM25</td>
              </tr>
              <tr>
                <td><code>hybrid</code></td>
                <td className="api-table__wide">
                  Both paths, each taking <code>topK × candidateMultiplier</code>{' '}
                  candidates
                </td>
                <td>fusion (RRF by default)</td>
              </tr>
            </tbody>
          </table>
        </div>
        <p>
          For the fusion math (RRF k=60, linear alpha) and the recall quality gates, see{' '}
          <Link to="/guides/hybrid-search">Hybrid Search</Link>.
        </p>

        <h2 id="filter">filter</h2>
        <Callout type="gap">
          Metadata filtering is implemented in the Rust core (<code>vane-core</code>) and
          covered by the <code>pre_filter</code> test suite, but it is{' '}
          <strong>not yet exposed through the Node, Go, or browser binding query
          parsers</strong> — they reject <code>filter</code> today (known gap, v0.1.x).
          Until the bindings catch up, filtering is available via the Rust core API
          directly. This is tracked as a binding-completeness gap, not a core limitation.
        </Callout>
        <p>
          The core semantics (SPEC §8.3), for reference: a filter is{' '}
          <code>{'{ <scalarField>: { eq?: v, in?: [v], gte?: v, lte?: v } }'}</code>, AND
          across fields — OR/NOT are not supported (M0–M2). It compiles to a roaring
          bitmap applied as a <em>pre-filter</em> inside the HNSW walk and the WAND scan;
          when the bitmap cardinality drops below 2×topK the vector path falls back to a
          brute-force exact scan (100% recall). Filtering a non-scalar field fails with{' '}
          <code>E_INVALID_ARG</code> (-11).
        </p>

        <h3>Error handling</h3>
        <p>
          <code>topK &gt; 1000</code>, a query with neither <code>text</code> nor{' '}
          <code>vector</code>, or a vector with the wrong dimension fails fast —{' '}
          <code>E_INVALID_ARG</code> (-11) or <code>E_SCHEMA</code> (-2). A query-time
          tokenizer identity that differs from a segment's fails with{' '}
          <code>E_TOKENIZER_MISMATCH</code> (-6), signalling a pending reindex. Node
          rejects with <code>VaneError</code>, Go returns <code>*vane.VaneError</code>.
          See <Link to="/api/errors">Error Codes</Link>.
        </p>
      </div>
    </DocsLayout>
  );
}
