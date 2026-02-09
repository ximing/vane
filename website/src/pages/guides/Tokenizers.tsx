import { Link } from 'react-router-dom';
import DocsLayout from '../../components/DocsLayout';
import CodeBlock from '../../components/CodeBlock';
import LangTabs from '../../components/LangTabs';
import Callout from '../../components/Callout';
import './Tokenizers.css';

export default function Tokenizers() {
  return (
    <DocsLayout>
      <article className="tok-page">
        <h1>Tokenizers</h1>
        <p className="tok-lede">
          The tokenizer is chosen per collection and shapes how every text
          field feeds BM25. Vane ships three built-in tokenizers; Chinese text
          gets first-class treatment via the bundled jieba-lite dictionary.
        </p>

        <h2 id="built-in-tokenizers">Built-in tokenizers</h2>
        <table className="tok-table">
          <thead>
            <tr>
              <th>Tokenizer</th>
              <th>Pipeline</th>
              <th>Needs a dictionary?</th>
            </tr>
          </thead>
          <tbody>
            <tr>
              <td>
                <code>standard</code>
              </td>
              <td>
                Unicode word break → lowercase → Porter stemmer (Latin/digit
                runs)
              </td>
              <td>No</td>
            </tr>
            <tr>
              <td>
                <code>cjk_bigram</code>
              </td>
              <td>
                CJK runs → character bigrams; non-CJK runs use the{' '}
                <code>standard</code> pipeline
              </td>
              <td>No</td>
            </tr>
            <tr>
              <td>
                <code>jieba</code>
              </td>
              <td>
                Prefix-DAG max-probability segmentation + HMM for
                out-of-vocabulary words
              </td>
              <td>Yes — bundled jieba-lite (~200k entries)</td>
            </tr>
          </tbody>
        </table>
        <p>
          Set it via <code>CollectionOptions.tokenizer</code> when the
          collection is created (default <code>standard</code>). Changing the
          tokenizer or its dictionary later requires a reindex — see{' '}
          <Link to="/guides/reindex">Custom Dict &amp; Reindex</Link>.
        </p>

        <h2 id="choosing">Choosing a tokenizer</h2>
        <ul className="tok-decision">
          <li>
            <strong>Mostly English or code?</strong> → <code>standard</code>.
            Lowercasing plus Porter stemming gives you the usual
            English-language recall.
          </li>
          <li>
            <strong>Chinese-heavy text?</strong> → <code>jieba</code>. Real word
            segmentation (with HMM for unknown words) beats bigrams on both
            precision and index size.
          </li>
          <li>
            <strong>CJK text but no dictionary available</strong> (slim builds,
            offline without the dict)? → <code>cjk_bigram</code>. It needs no
            dictionary data and still gives reasonable CJK recall.
          </li>
        </ul>

        <h2 id="mixed-text">Mixed CJK/Latin text</h2>
        <p>
          All tokenizers follow one unified rule: text is first split into runs
          at Unicode script boundaries; CJK runs go through{' '}
          <code>jieba</code>/<code>cjk_bigram</code>, Latin/digit runs go
          through lowercase + stemmer. Token positions increase continuously
          across the whole document — cross-language phrase queries depend on
          this invariant.
        </p>
        <div className="tok-flow" aria-label="Tokenization example for mixed text">
          <div className="tok-flow__row">
            <span className="tok-flow__stage">input</span>
            <span className="tok-chip tok-chip--raw">快速 sorting 算法</span>
          </div>
          <div className="tok-flow__row">
            <span className="tok-flow__stage">script runs</span>
            <span className="tok-chip">快速</span>
            <span className="tok-chip">sorting</span>
            <span className="tok-chip">算法</span>
          </div>
          <div className="tok-flow__row">
            <span className="tok-flow__stage">tokens (jieba)</span>
            <span className="tok-chip tok-chip--token">
              快速 <em>@0</em>
            </span>
            <span className="tok-chip tok-chip--token">
              sort <em>@1</em>
            </span>
            <span className="tok-chip tok-chip--token">
              算法 <em>@2</em>
            </span>
          </div>
        </div>
        <p>
          The CJK runs are segmented by jieba; the Latin run is lowercased and
          stemmed (<code>sorting</code> → <code>sort</code>); positions 0, 1, 2
          stay continuous across the language switch.
        </p>

        <h2 id="dict-distribution">Dictionary distribution</h2>
        <p>
          The jieba-lite dictionary (~200k high-frequency entries plus all
          single characters, zstd-compressed double-array trie) reaches each
          platform through a different channel:
        </p>
        <table className="tok-table">
          <thead>
            <tr>
              <th>Platform</th>
              <th>Channel</th>
              <th>Constraints</th>
            </tr>
          </thead>
          <tbody>
            <tr>
              <td>Node.js</td>
              <td>
                <code>@vane/dict-zh</code> platform-independent data package, a
                regular dependency of the main package — auto-loaded on{' '}
                <code>open</code>; <code>@vane/slim</code> is the dict-free
                variant
              </td>
              <td>
                No postinstall downloads; package ≤ 1.5&nbsp;MB gzip (CI gate)
              </td>
            </tr>
            <tr>
              <td>Go</td>
              <td>
                <code>go:embed dict.bin.gz</code>; call{' '}
                <code>db.LoadDict(dict.DictBytes())</code> after <code>Open</code>
                ; <code>//go:build vane_nodict</code> build tag trims it;{' '}
                <code>vane.DictVersion()</code> reports the version
              </td>
              <td>Embedded binary increment &lt; 2&nbsp;MB (CI gate)</td>
            </tr>
            <tr>
              <td>Browser (WASM)</td>
              <td>
                Inline <code>dictData</code> from <code>@vane-rs/dict-zh</code>
                (zero-CDN, transferable); jsdelivr CDN fallback when{' '}
                <code>dictData</code> is omitted
              </td>
              <td>
                On fetch failure, automatically degrades to{' '}
                <code>cjk_bigram</code> with a console warning — never throws
              </td>
            </tr>
          </tbody>
        </table>
        <p>
          The dictionary is versioned independently of the library by calendar
          version (<code>YYYY.MM</code>). Before every release, the dictionary
          hashes pinned by all three channels are checked for consistency — a
          mismatch blocks the release.
        </p>

        <h2 id="custom-dict">Custom dictionary</h2>
        <p>
          Inject domain terms at collection creation or at runtime. A bare
          string entry gets the highest built-in frequency (so the DAG prefers
          it); a <code>{'{ term, freq }'}</code> entry sets the frequency
          explicitly. User terms always win over built-in ones; the cap is
          100,000 entries (<code>E_DICT_TOO_LARGE</code> beyond that).
        </p>
        <LangTabs
          node={
            <CodeBlock
              lang="js"
              title="user-dict.mjs"
              code={`await col.setUserDict([
  '布地奈德',                        // bare term → highest built-in frequency
  { term: 'PD-1抑制剂', freq: 100 }, // explicit frequency
]);`}
            />
          }
          go={
            <CodeBlock
              lang="go"
              title="user_dict.go"
              code={`col, err := db.Collection("docs", schema, &vane.CollectionOptions{
    Tokenizer: "jieba",
    // Go passes explicit frequencies; the bare-string default
    // (highest built-in frequency) is a JS-side form.
    UserDict: []vane.UserDictEntry{
        {Term: "布地奈德", Freq: 1000000},
        {Term: "PD-1抑制剂", Freq: 100},
    },
})`}
            />
          }
          browser={
            <>
              <CodeBlock
                lang="ts"
                title="create-with-dict.ts"
                code={`// vane.collection(name, schema, opts): Promise<number>
// 域词须在 collection 创建时传入 userDict（Web 端无运行时 setUserDict）
const col = await vane.collection('docs', schema, {
  tokenizer: 'jieba',
  userDict: [
    '布地奈德',                        // bare term → highest built-in frequency
    { term: 'PD-1抑制剂', freq: 100 }, // explicit frequency
  ],
});`}
              />
              <Callout type="gap" title="Web has no runtime setUserDict">
                The <code>@vane-rs/web</code> <code>Vane</code> interface does not
                expose <code>setUserDict</code>. Domain terms must be passed as{' '}
                <code>userDict</code> at collection creation time; to change them
                later, drop the collection and recreate, or use the Node / Go
                binding. The base dictionary is loaded once at{' '}
                <code>createVane</code> via <code>dictData</code>, not per
                collection.
              </Callout>
            </>
          }
        />
        <Callout type="note" title="setUserDict stages, it does not apply">
          A runtime <code>setUserDict</code> call only stages the new
          dictionary — writes and queries keep using the old tokenizer identity
          until you call <code>reindex()</code>. The full state machine is in{' '}
          <Link to="/guides/reindex">Custom Dict &amp; Reindex</Link>.
        </Callout>
      </article>
    </DocsLayout>
  );
}
