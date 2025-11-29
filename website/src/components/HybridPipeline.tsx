import { useEffect, useRef, useState } from 'react';
import './HybridPipeline.css';

/**
 * Hybrid search pipeline diagram: a query fans out to the two recall paths
 * (HNSW vector search / Block-Max WAND BM25 scan), both candidate sets flow
 * into RRF (k = 60) fusion, and a ranked list comes out.
 *
 * Pure inline SVG + CSS keyframes — no animation library. All animations are
 * paused until the figure scrolls into the viewport (IntersectionObserver)
 * and pause again when it leaves. With prefers-reduced-motion the diagram
 * renders in its final, fully-lit state with no motion at all.
 */
export default function HybridPipeline() {
  const rootRef = useRef<HTMLElement>(null);
  const [active, setActive] = useState(false);

  useEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    if (typeof IntersectionObserver === 'undefined') {
      setActive(true);
      return;
    }
    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          setActive(entry.isIntersecting);
        }
      },
      { threshold: 0.35 },
    );
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  return (
    <figure
      ref={rootRef}
      className={active ? 'hybrid-pipeline is-active' : 'hybrid-pipeline'}
    >
      <svg
        viewBox="0 0 720 340"
        role="img"
        aria-label="A hybrid query splits into a vector path through the HNSW graph and a text path through a Block-Max WAND posting scan; both feed RRF fusion with k = 60, producing one ranked list."
      >
        {/* lane labels */}
        <text className="hp-label" x={190} y={34}>
          vector path · HNSW
        </text>
        <text className="hp-label" x={190} y={222}>
          text path · Block-Max WAND
        </text>

        {/* query box */}
        <rect className="hp-box" x={16} y={150} width={88} height={44} rx={8} />
        <text className="hp-box-text" x={60} y={177} textAnchor="middle">
          query
        </text>

        {/* split edges */}
        <path className="hp-edge hp-anim hp-edge--split" pathLength={1} d="M104 168 C 145 168, 150 80, 205 80" />
        <path className="hp-edge hp-anim hp-edge--split" pathLength={1} d="M104 176 C 145 176, 150 264, 195 264" />

        {/* HNSW graph: static faint edges + animated nodes */}
        <path className="hp-graph-edge" d="M225 80 L 295 52" />
        <path className="hp-graph-edge" d="M295 52 L 365 84" />
        <path className="hp-graph-edge" d="M365 84 L 435 58" />
        <path className="hp-graph-edge" d="M225 80 C 280 110, 320 110, 365 84" />
        <circle className="hp-node hp-anim hp-node--1" cx={225} cy={80} r={11} />
        <circle className="hp-node hp-anim hp-node--2" cx={295} cy={52} r={11} />
        <circle className="hp-node hp-anim hp-node--3" cx={365} cy={84} r={11} />
        <circle className="hp-node hp-anim hp-node--4" cx={435} cy={58} r={11} />

        {/* posting list + scan bar */}
        {[0, 1, 2, 3, 4, 5, 6, 7].map((i) => (
          <rect
            key={i}
            className="hp-posting"
            x={205 + i * 30}
            y={250}
            width={22}
            height={28}
            rx={3}
          />
        ))}
        <rect className="hp-scan hp-anim" x={205} y={245} width={26} height={38} rx={4} />

        {/* converge edges */}
        <path className="hp-edge hp-anim hp-edge--conv" pathLength={1} d="M446 66 C 495 66, 515 150, 540 162" />
        <path className="hp-edge hp-anim hp-edge--conv" pathLength={1} d="M437 264 C 490 264, 515 195, 540 182" />

        {/* RRF fusion node */}
        <circle className="hp-rrf hp-anim" cx={575} cy={172} r={36} />
        <text className="hp-rrf-text hp-anim" x={575} y={168} textAnchor="middle">
          RRF
        </text>
        <text className="hp-rrf-text hp-anim" x={575} y={186} textAnchor="middle">
          k = 60
        </text>

        {/* output edges */}
        <path className="hp-edge hp-anim hp-edge--out" pathLength={1} d="M607 149 C 626 149, 630 117, 646 117" />
        <path className="hp-edge hp-anim hp-edge--out" pathLength={1} d="M611 172 L 646 172" />
        <path className="hp-edge hp-anim hp-edge--out" pathLength={1} d="M607 195 C 626 195, 630 227, 646 227" />

        {/* ranked list */}
        <g className="hp-hit hp-anim hp-hit--1">
          <rect className="hp-hit-box" x={646} y={106} width={58} height={22} rx={4} />
          <text className="hp-hit-text" x={675} y={121} textAnchor="middle">
            #1
          </text>
        </g>
        <g className="hp-hit hp-anim hp-hit--2">
          <rect className="hp-hit-box" x={646} y={161} width={58} height={22} rx={4} />
          <text className="hp-hit-text" x={675} y={176} textAnchor="middle">
            #2
          </text>
        </g>
        <g className="hp-hit hp-anim hp-hit--3">
          <rect className="hp-hit-box" x={646} y={216} width={58} height={22} rx={4} />
          <text className="hp-hit-text" x={675} y={231} textAnchor="middle">
            #3
          </text>
        </g>
      </svg>
      <figcaption className="hybrid-pipeline__caption">
        One query, two recall paths running in parallel; RRF (k&nbsp;=&nbsp;60)
        fuses both candidate sets into a single ranked list.
      </figcaption>
    </figure>
  );
}
