import { useMemo, useState } from 'react';
import rawData from '../data/demo-results.json';
import type { DemoData, DemoHit, DemoQuery } from './contracts';
import './SearchDemo.css';

const DATA = rawData as unknown as DemoData;

type Mode = 'hybrid' | 'vector' | 'text';
const MODES: Mode[] = ['hybrid', 'vector', 'text'];

const PROVENANCE_LABEL =
  DATA.provenance === 'vane-node'
    ? 'pre-computed on real Vane output'
    : 'pre-computed';

/* ------------------------------------------------------------------ */
/* Matching helpers                                                    */
/* ------------------------------------------------------------------ */

function normalize(s: string): string {
  return s.trim().toLowerCase().replace(/\s+/g, ' ');
}

/** Split a query into highlight/match terms (whitespace-separated runs). */
function queryTerms(input: string): string[] {
  return normalize(input).split(' ').filter(Boolean);
}

/**
 * Exact match first (normalized), then fuzzy: either string contains the
 * other after normalization. Returns null when nothing matches.
 */
function findPreset(input: string): DemoQuery | null {
  const n = normalize(input);
  if (!n) return null;
  const exact = DATA.queries.find((q) => normalize(q.q) === n);
  if (exact) return exact;
  return (
    DATA.queries.find((q) => {
      const p = normalize(q.q);
      return p.includes(n) || n.includes(p);
    }) ?? null
  );
}

/* ------------------------------------------------------------------ */
/* Fallback ranking: simple contains re-rank over DemoData.docs        */
/* ------------------------------------------------------------------ */

const SNIPPET_RADIUS = 70;

function makeSnippet(body: string, terms: string[]): string {
  const lower = body.toLowerCase();
  let first = -1;
  for (const t of terms) {
    const i = lower.indexOf(t);
    if (i !== -1 && (first === -1 || i < first)) first = i;
  }
  if (first === -1) {
    return body.length > SNIPPET_RADIUS * 2
      ? body.slice(0, SNIPPET_RADIUS * 2) + '…'
      : body;
  }
  const start = Math.max(0, first - SNIPPET_RADIUS);
  const end = Math.min(body.length, first + SNIPPET_RADIUS);
  return (start > 0 ? '…' : '') + body.slice(start, end) + (end < body.length ? '…' : '');
}

/**
 * Score each doc by counting term occurrences in title + body; rank by
 * number of distinct terms hit, then total occurrences. Top 5.
 */
function fallbackRank(input: string): DemoHit[] {
  const terms = queryTerms(input);
  if (!terms.length) return [];
  return DATA.docs
    .map((d) => {
      const hay = (d.title + ' ' + d.body).toLowerCase();
      let distinct = 0;
      let total = 0;
      for (const t of terms) {
        let idx = 0;
        let count = 0;
        while ((idx = hay.indexOf(t, idx)) !== -1) {
          count += 1;
          idx += t.length;
        }
        if (count > 0) {
          distinct += 1;
          total += count;
        }
      }
      return { d, distinct, total };
    })
    .filter((x) => x.distinct > 0)
    .sort((a, b) => b.distinct - a.distinct || b.total - a.total)
    .slice(0, 5)
    .map((x) => ({
      id: x.d.id,
      title: x.d.title,
      snippet: makeSnippet(x.d.body, terms),
      score: x.total,
    }));
}

/* ------------------------------------------------------------------ */
/* Highlighting: wrap term occurrences in <mark> (--highlight)         */
/* ------------------------------------------------------------------ */

function escapeRegExp(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

function Highlight({ text, terms }: { text: string; terms: string[] }) {
  const parts = useMemo(() => {
    const valid = terms.filter(Boolean);
    if (!valid.length) return null;
    const re = new RegExp(`(${valid.map(escapeRegExp).join('|')})`, 'gi');
    return text.split(re);
  }, [text, terms]);

  if (!parts) return <>{text}</>;
  // With a single capturing group, odd indices are the matched terms.
  return (
    <>
      {parts.map((part, i) =>
        i % 2 === 1 ? <mark key={i}>{part}</mark> : part,
      )}
    </>
  );
}

/* ------------------------------------------------------------------ */
/* Hit card                                                            */
/* ------------------------------------------------------------------ */

function HitCard({ hit, terms }: { hit: DemoHit; terms: string[] }) {
  return (
    <li className="search-demo__hit">
      <div className="search-demo__hit-head">
        <span className="search-demo__hit-title">
          <Highlight text={hit.title} terms={terms} />
        </span>
        <span className="search-demo__hit-score">{hit.score.toFixed(4)}</span>
      </div>
      <p className="search-demo__hit-snippet">
        <Highlight text={hit.snippet} terms={terms} />
      </p>
    </li>
  );
}

/* ------------------------------------------------------------------ */
/* Component                                                           */
/* ------------------------------------------------------------------ */

export default function SearchDemo() {
  const [input, setInput] = useState(DATA.queries[0]?.q ?? '');

  const preset = useMemo(() => findPreset(input), [input]);
  const terms = useMemo(() => queryTerms(input), [input]);
  const hasInput = normalize(input).length > 0;
  const isFallback = hasInput && !preset;
  const fallbackHits = useMemo(
    () => (isFallback ? fallbackRank(input) : []),
    [isFallback, input],
  );

  return (
    <div className="search-demo">
      <span className="search-demo__provenance">{PROVENANCE_LABEL}</span>

      <div className="search-demo__controls">
        <input
          type="search"
          className="search-demo__input"
          value={input}
          onChange={(e) => setInput(e.target.value)}
          placeholder="Search 32 docs — try a preset query below"
          aria-label="Search demo query"
        />
        <div className="search-demo__chips" role="group" aria-label="Preset queries">
          {DATA.queries.map((q) => (
            <button
              key={q.q}
              type="button"
              className={
                'search-demo__chip' +
                (preset === q ? ' search-demo__chip--active' : '')
              }
              onClick={() => setInput(q.q)}
            >
              {q.q}
            </button>
          ))}
        </div>
      </div>

      {preset && (
        <div className="search-demo__columns">
          {MODES.map((mode) => (
            <section key={mode} className="search-demo__column">
              <h3 className="search-demo__column-head">
                <span className="search-demo__mode">{mode}</span> mode
              </h3>
              <ul className="search-demo__hits">
                {preset[mode].map((hit) => (
                  <HitCard key={hit.id} hit={hit} terms={terms} />
                ))}
              </ul>
            </section>
          ))}
        </div>
      )}

      {isFallback && (
        <div className="search-demo__fallback">
          <p className="search-demo__fallback-note">
            No pre-computed results for this query — showing{' '}
            <strong>fallback ranking</strong> (simple text matching over the
            32 demo docs, not real Vane output).
          </p>
          {fallbackHits.length > 0 ? (
            <ul className="search-demo__hits search-demo__hits--fallback">
              {fallbackHits.map((hit) => (
                <HitCard key={hit.id} hit={hit} terms={terms} />
              ))}
            </ul>
          ) : (
            <p className="search-demo__empty">
              No demo doc contains “{input.trim()}”.
            </p>
          )}
        </div>
      )}

      {!hasInput && (
        <p className="search-demo__empty">
          Type a query or pick one of the preset queries above.
        </p>
      )}
    </div>
  );
}
