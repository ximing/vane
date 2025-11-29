import DocsLayout from '../../components/DocsLayout';
import CodeBlock from '../../components/CodeBlock';
import './Persistence.css';

export default function Persistence() {
  return (
    <DocsLayout>
      <article className="per-page">
        <h1>Persistence &amp; Visibility</h1>
        <p className="per-lede">
          Vane stores data in immutable segments behind an atomically switched
          manifest. That one design decision explains the directory layout, the
          flush visibility boundary, crash recovery, and deletion alike.
        </p>

        <h2 id="directory-layout">Directory layout</h2>
        <CodeBlock
          lang="bash"
          title="my-db/"
          code={`my-db/
├── manifest.json          # version, segment list, collections
├── wal.log                # thin WAL: segment add/remove + tombstone meta-ops
└── segments/
    └── seg_<ulid>/
        ├── header.bin     # magic | format_version | tokenizer_id | docid_range | tombstone bitmap
        ├── vectors.bin    # f32, fixed-width, in docid order
        ├── hnsw.bin       # per-segment HNSW graph
        ├── inverted.bin   # term dictionary blocks + posting blocks
        ├── scalars.col    # columnar scalar blocks, partitioned by field
        └── stored.bin     # raw docs / JSON meta (format v2: zstd block compression)`}
        />
        <p>
          Every file starts with a 4-byte <code>magic</code> and a 4-byte{' '}
          <code>format_version</code>, versioned <em>per file</em> rather than
          per database — format changes bump a version and ship a migrator or
          dual-mode read. Opening a segment only reads the header and manifest;{' '}
          <code>vectors.bin</code>, <code>stored.bin</code>, and{' '}
          <code>hnsw.bin</code> are loaded lazily on first access.
        </p>

        <h2 id="flush-visibility">Flush is the visibility boundary</h2>
        <p>
          <code>add()</code> returning does <strong>not</strong> mean the
          documents are searchable — they sit in an in-memory buffer.{' '}
          <code>flush()</code> builds a new segment and atomically switches the
          manifest, making the batch visible to new snapshots in one step: the
          vector and BM25 indexes appear together in the same manifest switch,
          never one without the other.
        </p>
        <div
          className="per-timeline"
          aria-label="Timeline: add returns with data buffered in memory and not searchable; flush builds the segment and switches the manifest; new snapshots then see the batch atomically."
        >
          <div className="per-stage">
            <p className="per-stage__title">add() returns</p>
            <p className="per-stage__body">
              buffered in memory — <strong>not yet searchable</strong>
            </p>
          </div>
          <div className="per-stage__arrow" aria-hidden="true">
            →
          </div>
          <div className="per-stage per-stage--boundary">
            <p className="per-stage__title">flush()</p>
            <p className="per-stage__body">
              segment built, files synced, WAL appended, manifest renamed
            </p>
          </div>
          <div className="per-stage__arrow" aria-hidden="true">
            →
          </div>
          <div className="per-stage">
            <p className="per-stage__title">new snapshot</p>
            <p className="per-stage__body">
              batch <strong>atomically visible</strong> — vector + BM25
              together
            </p>
          </div>
        </div>

        <h2 id="auto-commit">Auto-commit</h2>
        <p>
          Auto-commit is on by default and flushes when{' '}
          <strong>either</strong> <code>intervalMs = 1000</code> has elapsed{' '}
          <strong>or</strong> <code>maxDocs = 1000</code> buffered documents is
          reached — whichever comes first. Pass <code>&quot;off&quot;</code> to
          disable it and call <code>flush()</code> yourself.
        </p>
        <CodeBlock
          lang="js"
          title="open-options.mjs"
          code={`const db = await VaneDb.open('./my-db', {
  autoCommit: { intervalMs: 1000, maxDocs: 1000 }, // the defaults
  // autoCommit: 'off',                            // manual flush only
});`}
        />

        <h2 id="tombstones-compact">Tombstones &amp; compaction</h2>
        <p>
          <code>delete(ids)</code> appends tombstones — they enter the WAL
          immediately and take effect with the segment at the next flush.
          Tombstoned documents are filtered at query time and only physically
          removed by <code>compact()</code> or automatic tiered merging.
        </p>
        <ul className="per-list">
          <li>
            HNSW graphs are never modified in place; they are rebuilt from
            scratch when segments merge. There is no separate &ldquo;graph
            rebuild&rdquo; API.
          </li>
          <li>
            The segment count is hard-capped at 10 — exceeding it forces a
            merge — and small segments (&lt; 10,000 docs) merge first.
          </li>
          <li>
            Merging is an incremental, sliceable background task: it never
            blocks reads (snapshots are immutable) and can only delay writes.
          </li>
          <li>
            Sustained trickle-write workloads should batch ≥ 100 documents per{' '}
            <code>add()</code> call.
          </li>
        </ul>

        <h2 id="export">Export</h2>
        <p>
          <code>db.export(dest)</code> packs the whole database into a
          single-file snapshot — handy for backups, shipping a prebuilt index,
          or moving a library between machines.
        </p>
        <CodeBlock
          lang="js"
          title="export.mjs"
          code={`await db.export('./backup.vane');   // Node / browser handle
// Go: err := db.Export("./backup.vane")`}
        />

        <h2 id="crash-recovery">Crash recovery</h2>
        <p>
          A flush writes the new segment files, syncs each one, appends the WAL
          record, writes the new manifest to a temporary file, syncs it, and
          finally <code>rename</code>s it into place. Recovery follows from
          that ordering:
        </p>
        <ul className="per-list">
          <li>
            The manifest always points at the last complete state — a crash
            mid-flush simply never switched it.
          </li>
          <li>
            WAL replay only restores tombstone appends and segment add/remove
            meta-operations.
          </li>
          <li>
            Half-written segment files (ULID not referenced by the manifest)
            are garbage-collected at startup.
          </li>
          <li>
            No mmap anywhere: native and browser share the same explicit
            read + page-cache code path, so recovery behaves identically on
            every platform.
          </li>
        </ul>
      </article>
    </DocsLayout>
  );
}
