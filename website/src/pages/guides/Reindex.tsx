import DocsLayout from '../../components/DocsLayout';
import CodeBlock from '../../components/CodeBlock';
import LangTabs from '../../components/LangTabs';
import Callout from '../../components/Callout';
import './Reindex.css';

export default function Reindex() {
  return (
    <DocsLayout>
      <article className="rx-page">
        <h1>Custom Dict &amp; Reindex</h1>
        <p className="rx-lede">
          A collection has exactly one active tokenizer identity at any moment.
          Changing the custom dictionary is therefore a staged, explicit
          process — never a silent mid-stream switch.
        </p>

        <h2 id="state-machine">State machine</h2>
        <figure className="rx-diagram">
          <svg
            viewBox="0 0 720 220"
            role="img"
            aria-label="State machine: Stable to PendingReindex via setUserDict, PendingReindex to Rebuilding via reindex, Rebuilding back to Stable when the rebuild completes with an atomic manifest switch, and PendingReindex back to Stable when the staged dict is abandoned."
          >
            <defs>
              <marker
                id="rx-arrow"
                viewBox="0 0 10 10"
                refX={9}
                refY={5}
                markerWidth={7}
                markerHeight={7}
                orient="auto-start-reverse"
              >
                <path className="rx-arrow-head" d="M 0 0 L 10 5 L 0 10 z" />
              </marker>
            </defs>

            {/* states */}
            <rect className="rx-state" x={30} y={60} width={140} height={48} rx={8} />
            <text className="rx-state-text" x={100} y={89} textAnchor="middle">
              Stable
            </text>
            <rect className="rx-state rx-state--staged" x={290} y={60} width={150} height={48} rx={8} />
            <text className="rx-state-text" x={365} y={89} textAnchor="middle">
              PendingReindex
            </text>
            <rect className="rx-state rx-state--staged" x={540} y={60} width={150} height={48} rx={8} />
            <text className="rx-state-text" x={615} y={89} textAnchor="middle">
              Rebuilding
            </text>

            {/* Stable -> PendingReindex */}
            <path className="rx-edge" d="M 170 84 L 284 84" markerEnd="url(#rx-arrow)" />
            <text className="rx-edge-label" x={227} y={72} textAnchor="middle">
              setUserDict()
            </text>

            {/* PendingReindex -> Rebuilding */}
            <path className="rx-edge" d="M 440 84 L 534 84" markerEnd="url(#rx-arrow)" />
            <text className="rx-edge-label" x={487} y={72} textAnchor="middle">
              reindex()
            </text>

            {/* Rebuilding -> Stable (completion, atomic switch) */}
            <path
              className="rx-edge"
              d="M 615 108 C 615 178, 100 178, 100 112"
              markerEnd="url(#rx-arrow)"
            />
            <text className="rx-edge-label" x={357} y={172} textAnchor="middle">
              rebuild completes → atomic manifest switch
            </text>

            {/* PendingReindex -> Stable (abandon) */}
            <path
              className="rx-edge rx-edge--abandon"
              d="M 340 60 C 320 12, 145 12, 122 56"
              markerEnd="url(#rx-arrow)"
            />
            <text className="rx-edge-label" x={231} y={24} textAnchor="middle">
              abandon: setUserDict() again
            </text>
          </svg>
        </figure>

        <table className="rx-table">
          <thead>
            <tr>
              <th>Transition</th>
              <th>Trigger</th>
              <th>What holds while in the state</th>
            </tr>
          </thead>
          <tbody>
            <tr>
              <td>Stable → PendingReindex</td>
              <td>
                <code>setUserDict()</code> stages a new dictionary
              </td>
              <td>
                New writes and all queries keep using the <strong>old</strong>{' '}
                tokenizer identity; search responses carry{' '}
                <code>needsReindex: true</code>
              </td>
            </tr>
            <tr>
              <td>PendingReindex → Rebuilding</td>
              <td>
                <code>reindex()</code>
              </td>
              <td>
                Every segment is rebuilt with the new tokenizer via the segment
                merge pipeline; old segments keep serving reads until the very
                end
              </td>
            </tr>
            <tr>
              <td>Rebuilding → Stable</td>
              <td>rebuild completes</td>
              <td>
                A single atomic manifest switch puts the new dictionary into
                effect
              </td>
            </tr>
            <tr>
              <td>PendingReindex → Stable</td>
              <td>
                abandon — calling <code>setUserDict()</code> again overwrites
                the staged dictionary
              </td>
              <td>Nothing was ever rebuilt; the old identity stays active</td>
            </tr>
          </tbody>
        </table>

        <Callout type="note" title="Forbidden by design">
          Vane never mixes old and new tokenizer identities in one search,
          never triggers a full rebuild automatically, and never merges
          multiple dictionary versions at query time.
        </Callout>

        <h2 id="set-user-dict">setUserDict is staged</h2>
        <p>
          <code>setUserDict</code> does <strong>not</strong> take effect
          immediately — it only stages the new dictionary and moves the
          collection to <code>PendingReindex</code>. Entries are either bare
          strings (default frequency = the built-in dictionary&apos;s highest
          frequency, so the DAG prefers them) or{' '}
          <code>{'{ term, freq }'}</code> objects. User terms always win over
          built-in ones, and among user terms the higher frequency wins;
          ambiguity resolution stays exactly stock jieba. The cap is 100,000
          entries — beyond that the call fails with{' '}
          <code>E_DICT_TOO_LARGE</code>.
        </p>
        <CodeBlock
          lang="js"
          title="stage-dict.mjs"
          code={`await col.setUserDict([
  '布地奈德',                        // bare term → highest built-in frequency
  { term: 'PD-1抑制剂', freq: 100 }, // explicit frequency
]);
// Still serving reads and writes with the OLD tokenizer identity.`}
        />

        <h2 id="needs-reindex">The needsReindex flag</h2>
        <p>
          While the collection is in <code>PendingReindex</code>, every search
          response carries <code>needsReindex: true</code>. Nothing is rebuilt
          on its own — the flag exists so your application can surface a
          &ldquo;rebuild recommended&rdquo; hint and let an operator (or a
          maintenance job) call <code>reindex()</code> at a convenient time.
          Until then, writes and queries proceed normally on the old tokenizer
          identity.
        </p>
        <CodeBlock
          lang="js"
          title="reindex.mjs"
          code={`const handle = await col.reindex();
console.log(handle.progress()); // 0..1, pollable
await handle.wait();            // or block until the atomic switch`}
        />

        <h2 id="best-practices">Best practices</h2>
        <p>
          Collect your domain terms <em>before</em> building the library and
          pass them as <code>userDict</code> at collection creation — the
          segments are then built with the right tokenizer identity from the
          start, and no reindex is ever needed. Reindexing a large library is
          cheap but not free.
        </p>
        <LangTabs
          node={
            <CodeBlock
              lang="js"
              title="create-with-dict.mjs"
              code={`const col = await db.collection('docs', schema, {
  tokenizer: 'jieba',
  userDict: ['布地奈德', { term: 'PD-1抑制剂', freq: 100 }],
});`}
            />
          }
          go={
            <CodeBlock
              lang="go"
              title="create_with_dict.go"
              code={`col, err := db.Collection("docs", schema, &vane.CollectionOptions{
    Tokenizer: "jieba",
    UserDict: []vane.UserDictEntry{
        {Term: "布地奈德", Freq: 1000000},
        {Term: "PD-1抑制剂", Freq: 100},
    },
})`}
            />
          }
          browser={
            <CodeBlock
              lang="js"
              title="create-with-dict.js"
              code={`const col = await db.collection('docs', schema, {
  tokenizer: 'jieba',
  userDict: ['布地奈德', { term: 'PD-1抑制剂', freq: 100 }],
});`}
            />
          }
        />
      </article>
    </DocsLayout>
  );
}
