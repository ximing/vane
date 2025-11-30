import DocsLayout from '../../components/DocsLayout';
import CodeBlock from '../../components/CodeBlock';
import Callout from '../../components/Callout';
import HybridPipeline from '../../components/HybridPipeline';
import './HybridSearch.css';

export default function HybridSearch() {
  return (
    <DocsLayout>
      <article className="hs-page">
        <h1>Hybrid Search</h1>
        <p className="hs-lede">
          Every hybrid query in Vane runs down two independent recall paths — a
          vector path and a text path — and fuses both candidate sets into one
          ranked list. This guide explains the two paths, the default RRF
          fusion, and the knobs that shape recall.
        </p>

        <h2 id="two-recall-paths">Two recall paths</h2>
        <p>
          The <strong>vector path</strong> runs a per-segment HNSW search in
          parallel and merges the results, ranked by vector distance. When a
          pre-filter leaves fewer than <code>2 × topK</code> candidates, the
          path falls back to a brute-force exact scan so recall stays at 100%
          for highly selective filters.
        </p>
        <p>
          The <strong>text path</strong> runs Block-Max WAND top-k over the
          BM25 inverted index (k1&nbsp;=&nbsp;1.2, b&nbsp;=&nbsp;0.75, frozen
          into the storage format), ranked by BM25 score. Block-level maximum
          scores let WAND skip whole posting blocks that cannot make the
          top-k.
        </p>
        <HybridPipeline />

        <h2 id="rrf-fusion">RRF fusion</h2>
        <p>Fusion defaults to Reciprocal Rank Fusion:</p>
        <p className="hs-formula">
          <code>score(d) = Σ_path 1 / (k + rank_path(d))</code>, with{' '}
          <code>k = 60</code> (frozen).
        </p>
        <p>
          RRF only looks at each document&apos;s <em>rank</em> on each path,
          never at raw scores — so the incomparable scales of vector distance
          and BM25 never meet. It needs zero tuning, which is why the default
          API path intentionally exposes no <code>alpha</code>.
        </p>

        <h2 id="search-modes">Search modes</h2>
        <table className="hs-table">
          <thead>
            <tr>
              <th>mode</th>
              <th>Recall path</th>
              <th>Ranked by</th>
            </tr>
          </thead>
          <tbody>
            <tr>
              <td>
                <code>vector</code>
              </td>
              <td>
                HNSW per-segment parallel search → merge; brute-force fallback
                when candidates &lt; 2×topK
              </td>
              <td>vector distance</td>
            </tr>
            <tr>
              <td>
                <code>text</code>
              </td>
              <td>Block-Max WAND top-k</td>
              <td>BM25</td>
            </tr>
            <tr>
              <td>
                <code>hybrid</code>
              </td>
              <td>
                both paths, each taking <code>topK × candidateMultiplier</code>{' '}
                candidates
              </td>
              <td>fusion</td>
            </tr>
          </tbody>
        </table>
        <p>
          When <code>mode</code> is omitted it is inferred from the inputs:
          passing both <code>text</code> and <code>vector</code> means hybrid;
          an explicit <code>mode</code> always wins over inference.
        </p>

        <h2 id="candidate-multiplier">Candidate multiplier</h2>
        <p>
          In hybrid mode each path recalls{' '}
          <code>topK × candidateMultiplier</code> candidates (default{' '}
          <code>3</code>) before fusion. With the defaults that means 30
          vector candidates and 30 text candidates compete for the final
          top-10. Raising the multiplier widens the fusion pool — better
          recall at higher latency; lowering it does the opposite.{' '}
          <code>topK</code> itself defaults to <code>10</code> and is capped at{' '}
          <code>1000</code>.
        </p>

        <h2 id="linear-fusion">Linear fusion</h2>
        <p>
          For an explicit weighted blend, pass{' '}
          <code>fusion: {'{ linear: { alpha, norm: "minmax" } }'}</code>. The
          fused score is{' '}
          <code>
            alpha × norm(vector_score) + (1 - alpha) × norm(bm25_score)
          </code>
          , where <code>minmax</code> normalizes each path&apos;s scores
          against the candidate set of the current query — it is the only
          supported normalization.
        </p>
        <CodeBlock
          lang="js"
          title="linear-fusion.js"
          code={`const hits = await col.search({
  text: 'incremental segment merge',
  vector: embed('incremental segment merge'),
  topK: 10,
  mode: 'hybrid',
  fusion: { linear: { alpha: 0.5, norm: 'minmax' } },
  candidateMultiplier: 3,
});`}
        />
        <Callout type="warning" title="Linear scores are not comparable across corpora">
          Because <code>minmax</code> normalization is relative to the current
          query&apos;s candidate set, a linear-fusion score means nothing
          outside that query — scores are not comparable across corpora, and
          not even across queries. Choosing and validating <code>alpha</code>{' '}
          is the caller&apos;s responsibility. If you don&apos;t have a tuning
          harness, stay on the RRF default.
        </Callout>
      </article>
    </DocsLayout>
  );
}
