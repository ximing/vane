# Vane

![License](https://img.shields.io/badge/license-Apache--2.0-blue)
![CI](https://github.com/ximing/vane/actions/workflows/ci.yml/badge.svg)
![Rust](https://img.shields.io/badge/Rust-stable-orange)
[![Docs](https://img.shields.io/badge/docs-ximing.github.io%2Fvane-3b9eff)](https://ximing.github.io/vane/)

**English** | [中文](README.zh-CN.md)

Vane is a lightweight **vector + BM25 hybrid retrieval library** built on a single Rust
core that embeds into desktop, Node.js, Go, and the browser. It pairs segment HNSW vector
search with Block-Max WAND BM25, and fuses the two with RRF — sqlite-vec's embedded shape,
Tantivy-grade text search, and unified hybrid ranking in one library.

- **One core, four runtimes.** The same Rust engine powers Node (napi-rs), Go (cgo static
  lib), and the browser (wasm-bindgen + Web Worker). Bindings are thin shells — no logic
  is duplicated across languages.
- **Hybrid by default.** `mode: "hybrid"` runs vector + BM25 in parallel and fuses with
  RRF (`k = 60`), with zero tuning. Recall@10 ≥ 0.95 is a hard CI gate.
- **First-class Chinese.** A `jieba` tokenizer (DAG + HMM, ~200k-word trimmed dictionary)
  ships alongside `standard` and `cjk_bigram`. Mixed CJK/Latin text is segmented correctly;
  inject your own domain terms with `userDict`.
- **Embedded & durable.** Directory-based segments + an atomically-switched manifest,
  crash-safe WAL, and a single-file `export()` snapshot. No server, no GPU, no mmap.
- **No built-in embeddings.** Vane stores, indexes, and fuses vectors you provide. Wire in
  OpenAI / ollama / transformers.js in a few lines (see `examples/`).

> Vane does not generate embeddings, run models, or speak SQL/distributed. It is a
> retrieval library — fast, embeddable, and predictable.

---

## Table of contents

- [What is Vane](#what-is-vane)
- [Features](#features)
- [Install](#install)
  - [Node.js](#nodejs) · [Go](#go) · [Browser](#browser) · [Build from source](#build-from-source)
- [Quick start](#quick-start)
  - [Node.js](#quick-start-nodejs) · [Go](#quick-start-go) · [Browser](#quick-start-browser)
- [API reference](#api-reference)
  - [Schema & documents](#schema--documents) · [Tokenizers](#tokenizers) · [Search modes & fusion](#search-modes--fusion) · [Custom dictionary & reindex](#custom-dictionary--reindex) · [Filtering](#filtering)
- [Architecture](#architecture)
- [Performance](#performance)
- [Status](#status)
- [Examples](#examples)
- [Contributing](#contributing)
- [License](#license)

---

## What is Vane

Vane is a **hybrid retrieval library** you embed into your own process. You hand it
documents — each a text field, a vector, and optional scalar metadata — and it builds an
inverted index (BM25) and a vector index (HNSW) over the same data. At query time you ask
with text, a vector, or both, and Vane returns ranked hits with the two signals fused.

It exists because the obvious alternatives each give up something:

| You want… | Typical stack | What you give up |
|---|---|---|
| vector + text search, in-process | sqlite-vec **+** FTS5, hand-rolled fusion glue | atomic hybrid ranking, one filter model, ~200 lines of plumbing |
| a browser-side semantic search | a pure-JS engine | performance ceiling, no Rust core to reuse on Node/Go |
| Chinese-aware tokenization | Tantivy + a tokenizer crate | a browser build, or a second engine for the client |

Vane's bet is that one Rust core, kept mmap-free and platform-clean, can serve the desktop
(Node), server (Go), and browser (WASM) from the same code path — with BM25 and vector
retrieval designed to be fused from the start rather than bolted together.

Typical scenarios: an AI agent's local memory store, an edge/on-device RAG retrieval
layer, or a privacy-preserving in-browser semantic search over a notes/PKM library.

## Features

- **Collections** — create / list / delete; a document is `id` + `text` + `vector` + JSON metadata.
- **Vector index** — segment HNSW with adaptive brute-force fallback when the filtered
  candidate set is tiny. Metrics: `cosine` / `l2` / `dot`. Dim up to 4096.
- **BM25** — Block-Max WAND top-k over a posting list with 128-doc skip blocks; `k1=1.2`,
  `b=0.75`. Multiple text fields per collection.
- **Hybrid fusion** — RRF by default (`k=60`, no calibration); optional
  `{ linear: { alpha } }` with min-max normalization.
- **Tokenizers** — `standard` (unicode + lowercase + Porter stemmer), `cjk_bigram`
  (dictionary-free CJK fallback), `jieba` (precise Chinese segmentation).
- **Custom dictionary** — inject domain terms at collection creation or via `setUserDict`,
  then `reindex()` to apply atomically. User terms always win over the built-in dictionary.
- **Metadata filtering** — `eq` / `in` / `gte` / `lte` over scalar fields, AND-combined,
  pushed into the HNSW walk and WAND scan (pre-filter, not post-filter). *See
  [Filtering](#filtering) for current binding coverage.*
- **Persistence** — directory of immutable segments + `manifest.json` switched atomically
  via `rename`; thin WAL for crash recovery; `export()` for a single-file snapshot.
- **Deletes & compaction** — tombstone bitmaps; `compact()` physically reclaims space.
- **Visibility** — `flush()` is the atomic boundary after which new reads see the data;
  auto-commit defaults on (1s or 1000 docs).
- **Concurrency** — single writer, lock-free concurrent reads; all public APIs are
  thread/goroutine-safe.

## Install

### Node.js

```bash
npm install @vane-rs/node
```

Prebuilt native binaries are selected automatically via `optionalDependencies` for:

| Platform | npm sub-package |
|---|---|
| Linux x64 (glibc) | `@vane-rs/node-linux-x64-gnu` |
| macOS arm64 | `@vane-rs/node-darwin-arm64` |
| macOS x64 | `@vane-rs/node-darwin-x64` |
| Windows x64 (MSVC) | `@vane-rs/node-win32-x64-msvc` |

You should never need to compile from source on these platforms. The bundled `jieba`
dictionary loads automatically on open — Chinese search works out of the box.

### Go

The Go binding links a prebuilt static library (`libvane_ffi.a`) per platform via cgo.

```bash
# 1. Build the static lib (or download libvane_ffi-<lib_dir>.a from GitHub Releases)
cargo build --release -p vane-ffi

# 2. Place it where cgo expects it (os-arch subdirectory of bindings/go/lib/)
mkdir -p bindings/go/lib/$(go env GOOS)-$(go env GOARCH)
cp target/release/libvane_ffi.a bindings/go/lib/$(go env GOOS)-$(go env GOARCH)/

# 3. Add to your module
go get github.com/ximing/vane/bindings/go
```

Prebuilt static libs cover `linux-amd64`, `linux-arm64`, `darwin-amd64`, `darwin-arm64`.
The `jieba` dictionary is embedded in the `bindings/go/dict` package — call
`db.LoadDict(dict.DictBytes())` once after `Open`. A `vane_nodict` build tag drops the
embedded dictionary (degrades to `cjk_bigram`).

> `CGO_ENABLED=0` builds are not supported on the cgo path; use the `wazero` build tag for a
> pure-Go (slower) variant.

### Browser

```bash
npm install @vane-rs/web @vane-rs/dict-zh
```

The `@vane-rs/web` package ships a SIMD/scalar dual-variant wasm module and a Web Worker
as ESM assets; `@vane-rs/dict-zh` ships the Chinese dictionary as a transferable
`Uint8Array`. Vite 6+ and webpack 5 (with `outputModule`) recognize the
`new URL('./worker.js', import.meta.url)` pattern natively — no wasm or worker plugins
needed. See the
[Web Integration guide](https://ximing.github.io/vane/guides/web-integration)
for bundler configuration.

### Build from source

```bash
# Build everything
cargo build --release --workspace

# Full test suite + quality gates used by CI
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace --all-features

# WASM baseline (core must stay free of std::fs / mmap)
cargo check --target wasm32-unknown-unknown -p vane-core
```

## Quick start

The examples below use 4-dimensional dummy vectors so they run as-is. In production, replace
`vector` with real embeddings from your model — Vane indexes and searches whatever you give it.

### Quick start: Node.js

```js
import vane from '@vane-rs/node';
const { open } = vane;

// Open a database directory (created if missing). autoCommit: 'off' means we
// control the visibility boundary ourselves with flush().
const db = await open('./mydb', { autoCommit: 'off' });

// Declare a schema: one text field + one vector field. (One vector field per collection.)
const col = await db.collection('docs', {
  fields: [
    { name: 'body', type: 'text' },
    { name: 'vec',  type: 'vector', dim: 4, metric: 'cosine' },
  ],
}, { tokenizer: 'standard' });

// Batch upsert by id. Returns { accepted, visibleAfterFlush }.
await col.add([
  { id: 'a', text: 'hello world',  vector: [1.0, 0.0, 0.0, 0.0] },
  { id: 'b', text: 'foo bar baz',  vector: [0.0, 1.0, 0.0, 0.0] },
  { id: 'c', text: 'hello foo',    vector: [0.7, 0.3, 0.0, 0.0] },
]);
await col.flush();                       // data is now searchable

// Hybrid search: BM25(text) + vector similarity, fused with RRF.
const hits = await col.search({
  text: 'hello',
  vector: [1.0, 0.0, 0.0, 0.0],
  topK: 3,
  mode: 'hybrid',                        // 'vector' | 'text' | 'hybrid'
  fusion: 'rrf',                         // default; or { linear: { alpha: 0.5 } }
});
// hits = [{ id, score, fields }, ...]

await db.close();
```

### Quick start: Go

```go
package main

import (
	"fmt"
	"log"

	"github.com/ximing/vane/bindings/go"
	"github.com/ximing/vane/bindings/go/dict"
)

func main() {
	db, err := vane.Open("./mydb", nil) // nil opts = defaults
	if err != nil { log.Fatalf("Open: %v", err) }
	defer db.Close()

	// Load the bundled jieba dictionary (embedded in the dict package).
	if b, err := dict.DictBytes(); err == nil {
		_ = db.LoadDict(b) // on failure, jieba degrades to standard — collection creation won't fail
	}

	schema := vane.Schema{Fields: []vane.SchemaField{
		{Name: "body", Type: "text"},
		{Name: "vec",  Type: "vector", Dim: 4, Metric: "cosine"},
	}}
	col, err := db.Collection("docs", schema, &vane.CollectionOptions{Tokenizer: "jieba"})
	if err != nil { log.Fatalf("Collection: %v", err) }
	defer col.Close()

	_ = col.Add([]vane.Doc{
		{ID: "a", Text: "hello world", Vector: []float32{1.0, 0.0, 0.0, 0.0}},
		{ID: "b", Text: "foo bar baz", Vector: []float32{0.0, 1.0, 0.0, 0.0}},
		{ID: "c", Text: "hello foo",   Vector: []float32{0.7, 0.3, 0.0, 0.0}},
	})
	_ = col.Flush()

	hits, _ := col.Search(vane.SearchQuery{
		Text: "hello", Vector: []float32{1.0, 0.0, 0.0, 0.0}, TopK: 3,
	})
	for _, h := range hits {
		fmt.Printf("hit: id=%s score=%.4f\n", h.ID, h.Score)
	}
}
```

### Quick start: Browser

```bash
npm install @vane-rs/web @vane-rs/dict-zh
```

```ts
import { createVane } from '@vane-rs/web';
import type { Schema, Hit } from '@vane-rs/web';
import dictBinUrl from '@vane-rs/dict-zh/dict.bin';

// Load the dictionary (inline, zero-copy transfer to the worker)
const dictData = new Uint8Array(await (await fetch(dictBinUrl)).arrayBuffer());
const vane = await createVane({ vfs: 'opfs', dbPath: 'vane.db', dictData });

await vane.open();
const col = await vane.collection('docs', {
  fields: [
    { name: 'body', type: 'text' },
    { name: 'vec',  type: 'vector', dim: 4, metric: 'cosine' },
  ],
}, { tokenizer: 'jieba' });

await vane.add(col, [
  { id: 'a', text: 'hello world', vector: [1.0, 0.0, 0.0, 0.0] },
  { id: 'b', text: 'foo bar baz', vector: [0.0, 1.0, 0.0, 0.0] },
]);
await vane.flush(col);
const hits: Hit[] = await vane.search(col, {
  text: 'hello', vector: [1.0, 0.0, 0.0, 0.0], topK: 3, mode: 'hybrid',
});
await vane.close();
```

See [`examples/vite/`](examples/vite/) and [`examples/webpack/`](examples/webpack/) for
complete bundler-configured examples, or the
[Web Integration guide](https://ximing.github.io/vane/guides/web-integration) for the full
Worker / persistence / SIMD story.

## API reference

The same six verbs (+ four management calls) appear across all bindings; only the casing
and error style differ (JS `Promise`/`VaneError`; Go `(T, error)`).

| Operation | Node.js | Web | Go |
|---|---|---|---|
| Open a database | `open(path, opts)` → `VaneDb` | `createVane(opts)` → `Vane`; `vane.open(path)` | `vane.Open(path, *OpenOptions)` |
| Create / open a collection | `db.collection(name, schema, opts)` | `vane.collection(name, schema, opts)` → `number` | `db.Collection(name, Schema, *CollectionOptions)` |
| List collections | `db.collections()` | — (not yet exposed) | — (not yet exposed) |
| Add documents (batch upsert) | `col.add(docs)` → `{accepted}` | `vane.add(col, docs)` → `number` | `col.Add([]Doc)` |
| Make writes visible | `col.flush()` | `vane.flush(col)` | `col.Flush()` |
| Search | `col.search(query)` → `Hit[]` | `vane.search(col, query)` → `Hit[]` | `col.Search(SearchQuery)` |
| Delete by id | `col.delete(ids)` | `vane.delete(col, ids)` → `number` | `col.Delete([]string)` |
| Trigger segment compaction | `col.compact()` | `vane.compact(col)` | `col.Compact()` |
| Rebuild with a new tokenizer/dict | `col.reindex()` → handle | `vane.reindex(col)` → `number` | `col.Reindex()` |
| Single-file snapshot export | `db.export(dest)` | `vane.export(dest)` | `db.Export(dest)` |
| Close | `db.close()` | `vane.close()` | `db.Close()` |

> **Web:** `collection()` returns a `number` handle; every subsequent verb takes `col` as
> its first argument. See <https://ximing.github.io/vane/api/web> for the full type
> reference.

### Schema & documents

A schema is a list of named fields. There is exactly one `vector` field per collection;
`text` fields feed BM25; `scalar` fields are filterable.

```js
{
  fields: [
    { name: 'title', type: 'text' },
    { name: 'body',  type: 'text' },
    { name: 'vec',   type: 'vector', dim: 384, metric: 'cosine' }, // metric: cosine|l2|dot
    { name: 'lang',  type: 'scalar', kind: 'keyword' },           // kind: int|float|bool|keyword
  ],
}
```

A document is `{ id, text, vector, meta }` where `meta` holds scalar values keyed by field
name. `id` is an external string primary key (≤ 512 bytes); `add` is an idempotent upsert by
`id`. Vectors must match the schema `dim`, or the call errors with `E_SCHEMA`.

### Tokenizers

Set per collection via `CollectionOptions.tokenizer`:

| Tokenizer | What it does | Needs a dictionary? |
|---|---|---|
| `standard` | Unicode word break → lowercase → Porter stemmer (Latin/digit runs) | No |
| `cjk_bigram` | CJK runs → character bigrams; non-CJK runs use the `standard` pipeline | No |
| `jieba` | Prefix-DAG max-probability segmentation + HMM for out-of-vocabulary words | Yes (bundled) |

Mixed CJK/Latin text is split at Unicode-script boundaries; CJK runs go through
`jieba`/`cjk_bigram` and Latin/digit runs through lowercase+stemmer, with token positions
continuous across the whole document (so cross-language phrase queries stay correct).

Dictionary loading, per platform:
- **Node** — auto-loaded on `open` (the `@vane-rs/node` package bundles `dict.bin`).
- **Go** — `db.LoadDict(dict.DictBytes())` after `Open`.
- **Browser** — fetched from a CDN, sha256-verified, cached in OPFS; falls back to
  `cjk_bigram` (with a console warning) when unavailable. Inline `dictData` injection is
  supported for offline/self-hosted deployments.

### Search modes & fusion

| `mode` | Recall path | Ranked by |
|---|---|---|
| `vector` | HNSW per-segment parallel search → merge; brute-force fallback when candidates < 2×topK | vector distance |
| `text` | Block-Max WAND top-k | BM25 |
| `hybrid` | both paths, each taking `topK × candidateMultiplier` candidates | fusion |

Fusion defaults to **RRF** (`score = Σ 1/(60 + rank)`, zero tuning). For an explicit
weighted blend, pass `fusion: { linear: { alpha: 0.5 } }` — note that linear scores are
normalized per-query and are **not comparable across corpora**; tuning is your
responsibility. The default path intentionally exposes no `alpha`.

### Custom dictionary & reindex

Inject domain terms at creation or at runtime:

```js
await col.setUserDict([
  '布地奈德',                       // bare term → highest built-in frequency
  { term: 'PD-1抑制剂', freq: 100 }, // explicit frequency
]);
```

`setUserDict` **stages** the new dictionary — the collection enters a `pendingReindex`
state, but all writes and queries keep using the **old** tokenizer identity until you call
`reindex()`. This avoids silent inconsistencies between new and old segments. `reindex()`
rebuilds every segment with the new tokenizer (background, incremental; old segments stay
read-only until an atomic switch), then the new dictionary takes effect. Poll
`reindexHandle.progress()` or `await reindexHandle.wait()`.

> Best practice: collect your domain terms before building the library and pass them as
> `userDict` at collection creation. Reindexing a large library is cheap but not free.

### Filtering

Metadata filtering (`eq` / `in` / `gte` / `lte`, AND across fields, applied as a pre-filter
inside the HNSW walk and WAND scan, with brute-force fallback at low selectivity) is
implemented in the Rust core (`vane-core`) and covered by the `pre_filter` test suite.

**It is not yet exposed through the Node, Go, or browser binding query parsers** (they
reject `filter` today). Until the bindings catch up, filtering is available via the Rust
core API directly. This is tracked as a binding-completeness gap, not a core limitation.

## Architecture

```
                    ┌─────────────────────────────────────────┐
   Node (napi-rs) ──┤                                         │
   Go (cgo/.a)   ───┤   vane-core  (one Rust engine)          ├── Browser (wasm-bindgen + Worker)
   C ABI (FFI)   ───┤   • VFS trait: std-fs / OPFS / IDB / mem │
                    │   • immutable segments + manifest switch  │
                    │   • HNSW (per-seg) + Block-Max WAND BM25  │
                    │   • RRF fusion, pre-filter bitmaps        │
                    └─────────────────────────────────────────┘
```

- **One core, no mmap.** All I/O goes through a `Vfs` trait; `vane-core` is forbidden from
  touching `std::fs`/`std::net`/mmap (a hard CI gate from day one). Native and browser share
  the same code path — explicit reads into an LRU page cache, not mmap.
- **Immutable segments.** A write buffers in memory; `flush` builds a new segment and
  atomically switches the manifest via `rename`. Reads hold an immutable snapshot.
- **Segmented HNSW.** Each segment owns one HNSW graph; deletes are tombstones; graphs are
  rebuilt only at segment merge (never in place). Multi-segment searches run in parallel
  (rayon on native, serial on wasm) and merge.
- **Thin bindings.** Node connects to the core directly (no C ABI, no tokio); Go wraps the
  C ABI through cgo; the browser wraps a handle-based wasm surface in a Worker. Behavior
  tests live in the core; the bindings only convert and dispatch.

## Performance

Native / Node, 100k documents × 384 dims:

| Metric | Target |
|---|---|
| Hybrid topK=10, P99 latency | < 50 ms (HNSW); < 150 ms (brute force) |
| Batch add throughput | ≥ 5,000 docs/s (incl. index build) |
| Cold open of a 100k library | metadata < 1 s; first vector query < 3 s |
| Resident memory (full load) | < 500 MB (< 200 MB with SQ8) |
| Hybrid recall@10 | ≥ 0.95 vs brute-force dual-path + RRF baseline (hard CI gate) |
| Browser (WASM) | latency relaxed ~3–5×; SIMD128 variant auto-selected at runtime |

Size gates (hard CI): core wasm ≤ 800 KB gzip (tokenizer code incl., dict data excl.);
full-feature ≤ 1.2 MB; Chinese dictionary ≤ 1.5 MB per channel.

## Status

**v0.2.0** — the core engine is feature-complete through milestones M0–M3:

- ✅ Core API, VFS, tokenizer (standard/cjk_bigram/jieba), BM25, segment HNSW, RRF fusion
- ✅ Persistence (segments + manifest + WAL), tombstone delete, compaction, snapshot export
- ✅ Pre-filter bitmaps in core, SQ8 quantization, rayon parallel executor
- ✅ `setUserDict` / `reindex` state machine, Chinese dictionary distribution (Node/Go/WASM)
- ✅ Bindings: Node (napi-rs, 4 platforms), Go (cgo, 4 platforms + wazero stub), Browser
  (@vane-rs/web npm package — wasm-bindgen + Worker, OPFS/IDB, SIMD dual variants)
- ✅ `@vane-rs/web` + `@vane-rs/dict-zh` npm packages (ESM, vite 6+/webpack 5 native,
  dictData inline transfer, zero CDN)

Known gaps: `filter` is wired in core but not yet exposed through the binding query parsers
(see [Filtering](#filtering)); musl/linux-arm64/winx64-arm Node prebuilts and the wazero
pure-Go path are deferred.

## Examples

- [`examples/demo/`](examples/demo/) — Node: load 10k synthetic wiki abstracts and compare
  hybrid / vector / text rankings side by side (with a code-volume contrast vs a hand-rolled
  sqlite-vec + FTS5 setup).
- [`examples/vite/`](examples/vite/) — Browser (vite): `@vane-rs/web` + `@vane-rs/dict-zh`
  end-to-end — createVane → open → collection(jieba) → add → search (recommended).
- [`examples/webpack/`](examples/webpack/) — Browser (webpack 5): same flow with
  `experiments.outputModule` + asset/resource config.
- [`bindings/go/example/`](bindings/go/example/) — Go: open → load dict → add → search.

The examples use deterministic **pseudo-vectors** (hash buckets) so they run without an
embedding model. Swap in a real embedding provider for production — that wiring is
intentionally out of scope for Vane itself.

## Contributing

Vane is developed against two contracts: [`docs/REQUIREMENTS.md`](docs/REQUIREMENTS.md)
(what & why) and [`docs/SPEC.md`](docs/SPEC.md) (precise interfaces, file formats, error
codes, and numeric gates). Implementation must not silently drift from them — when changing
public API, persistence format, error codes, or cross-language behavior, update the SPEC and
the corresponding tests first.

Local checks before opening a PR:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cd crates/vane-node && npm test          # Node binding tests
cd bindings/go && go test ./...         # Go binding tests (after building vane-ffi)
```

## License

Apache-2.0. The patent grant keeps embedded use legally simple.
