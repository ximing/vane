# Vane Sidecar Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a native Rust CLI + one daemon + MCP bridge that indexes registered local folders into per-project Vane collections, reuses extract/embed CAS across branch switches, and serves hybrid search to agents.

**Architecture:** `crates/vane` is a native-only binary crate. It never changes `vane-core` contracts. One process: watch thread + single writer thread + JSON-line Unix socket. Live working set goes into Vane; historical vectors stay in CAS. Config is global defaults + per-root `.vane.toml` overrides.

**Tech Stack:** Rust 2021, `vane-core` (`dict-zh`, `zstd-encode`, `executor-native`), clap, toml, serde_json, sha2, notify, ureq (rustls, no openssl), std threads. No tokio, no regex crate, no globset.

**Spec:** `docs/superpowers/specs/2026-08-19-vane-sidecar-design.md`

## Global Constraints

- Do not modify `vane-core` public API, persistence format, or Won't-have.
- Do not add `tokio`, `openssl`, `regex`, `native-tls` to this crate or as wrappers except as already allowed in `deny.toml`.
- Home dir: `--home` > `VANE_HOME` > `~/.vane`. Tests must set `VANE_HOME` to a temp dir.
- Glob matching is implemented in-crate (`glob_match`); do not depend on `globset` (it pulls `regex`).
- HTTP: `ureq` with `default-features = false`, features `json` + `tls` (rustls).
- WASM CI jobs stay `-p vane-core` / `-p vane-wasm` only; do not add `crates/vane` to wasm32 jobs.
- Single Vane writer thread; searches are concurrent reads.
- `cargo fmt`, `cargo clippy -p vane --all-targets -- -D warnings`, `cargo test -p vane`.

## File map

Create all of these under `crates/vane/` unless noted.

| Path | Responsibility |
|------|----------------|
| `Cargo.toml` (workspace) | Add member + default-member `crates/vane` |
| `crates/vane/Cargo.toml` | Package, bin+lib, native deps |
| `crates/vane/src/lib.rs` | Module tree |
| `crates/vane/src/main.rs` | clap CLI |
| `crates/vane/src/home.rs` | Home dir resolution |
| `crates/vane/src/error.rs` | `VaneCliError` |
| `crates/vane/src/config.rs` | Load/merge TOML |
| `crates/vane/src/project.rs` | `project_id`, current root from cwd |
| `crates/vane/src/glob_match.rs` | `**` / `{a,b}` glob |
| `crates/vane/src/classify.rs` | exclude ∪ types match |
| `crates/vane/src/chunk.rs` | markdown/plain splitter |
| `crates/vane/src/extract.rs` | text + image → `CanonicalDoc` |
| `crates/vane/src/cas.rs` | extract + embed file CAS + last_seen |
| `crates/vane/src/gc.rs` | unreferenced + TTL CAS cleanup |
| `crates/vane/src/embed.rs` | `Embedder` trait, mock, ollama, openai_compat |
| `crates/vane/src/dirty.rs` | retry queue + backoff |
| `crates/vane/src/live.rs` | `live.json` atomic |
| `crates/vane/src/index.rs` | open/add/delete/flush/compact/search one project db |
| `crates/vane/src/rrf.rs` | cross-project RRF |
| `crates/vane/src/watch.rs` | notify, skip excluded dirs |
| `crates/vane/src/log.rs` | daily `daemon.YYYY-MM-DD.log` + prune |
| `crates/vane/src/ipc.rs` | JSON-line request/response |
| `crates/vane/src/daemon.rs` | flock, threads, methods |
| `crates/vane/src/mcp.rs` | stdio MCP proxy |
| `crates/vane/src/service.rs` | launchd / systemd user unit |
| `crates/vane/src/wizard.rs` | `vane init` stdin wizard |
| `crates/vane/SKILL.md` | Agent instructions |
| `crates/vane/tests/*.rs` | integration tests (`VANE_HOME` temp) |
| `deny.toml` | no change unless a new license appears; do not wrap regex |

---

### Task 1: Crate skeleton and home directory

**Files:**
- Modify: `Cargo.toml` (workspace `members` and `default-members`)
- Create: `crates/vane/Cargo.toml`
- Create: `crates/vane/src/lib.rs`, `main.rs`, `home.rs`, `error.rs`
- Test: `crates/vane/tests/home.rs`

**Interfaces:**
- Consumes: none
- Produces:
  - `pub fn resolve_home(cli_home: Option<&Path>, env_vane_home: Option<&OsStr>, fallback_home: &Path) -> PathBuf`
  - `pub struct VaneCliError { pub message: String }` with `Display` + `std::error::Error`

- [ ] **Step 1: Write the failing test**

```rust
// crates/vane/tests/home.rs
use std::path::PathBuf;
use vane::home::resolve_home;

#[test]
fn cli_flag_beats_env_beats_default() {
    let fallback = PathBuf::from("/Users/me/.vane");
    let env = std::ffi::OsString::from("/tmp/env-vane");
    let cli = PathBuf::from("/tmp/cli-vane");
    assert_eq!(
        resolve_home(Some(&cli), Some(&env), &fallback),
        PathBuf::from("/tmp/cli-vane")
    );
    assert_eq!(
        resolve_home(None, Some(&env), &fallback),
        PathBuf::from("/tmp/env-vane")
    );
    assert_eq!(resolve_home(None, None, &fallback), fallback);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p vane --test home -- --nocapture`  
Expected: FAIL compiling (`could not find vane`)

- [ ] **Step 3: Add workspace member and minimal crate**

Workspace `Cargo.toml`: append `"crates/vane"` to both `members` and `default-members`.

```toml
# crates/vane/Cargo.toml
[package]
name = "vane"
version.workspace = true
edition.workspace = true
license.workspace = true
publish = false

[lib]
name = "vane"
path = "src/lib.rs"

[[bin]]
name = "vane"
path = "src/main.rs"

[dependencies]
vane-core = { workspace = true, features = ["dict-zh", "zstd-encode", "executor-native"] }
serde = { workspace = true }
serde_json = { workspace = true }
sha2 = { workspace = true }
clap = { version = "4", features = ["derive", "env"] }
toml = "0.8"
notify = "6"
ureq = { version = "2.12", default-features = false, features = ["json", "tls"] }
```

`src/home.rs`:

```rust
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

pub fn resolve_home(
    cli_home: Option<&Path>,
    env_vane_home: Option<&OsStr>,
    fallback_home: &Path,
) -> PathBuf {
    if let Some(p) = cli_home {
        return p.to_path_buf();
    }
    if let Some(p) = env_vane_home {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    fallback_home.to_path_buf()
}

pub fn default_fallback() -> PathBuf {
    dirs_next_or_home().join(".vane")
}

fn dirs_next_or_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}
```

Do **not** add the `dirs` crate unless clippy forces it; `HOME` is enough on macOS/Linux (v1 platforms).

`src/main.rs` for this task: clap with global `--home` and a `status` subcommand that prints the resolved home and exits 1 with "not initialized" if `config/config.toml` is missing.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p vane --test home`  
Expected: PASS  
Also: `cargo clippy -p vane --all-targets -- -D warnings`

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml crates/vane
git commit -m "feat(vane): add native sidecar crate and home dir resolution"
```

---

### Task 2: Config load and merge

**Files:**
- Create: `crates/vane/src/config.rs`, `project.rs`
- Test: `crates/vane/tests/config_merge.rs`
- Modify: `crates/vane/src/lib.rs`

**Interfaces:**
- Consumes: `resolve_home`
- Produces:
  - `pub struct Config { pub defaults: Defaults, pub exclude: Vec<String>, pub types: Vec<TypeRule>, pub projects: Vec<ProjectEntry> }`
  - `pub struct TypeRule { pub glob: String, pub extractor: String, pub enabled: bool }`
  - `pub struct ResolvedPolicy { pub embed: EmbedConfig, pub chunk: ChunkConfig, pub exclude: Vec<String>, pub types: Vec<TypeRule> }`
  - `pub fn load_config(home: &Path) -> Result<Config, VaneCliError>`
  - `pub struct LogConfig { pub retain_days: u32 }` — default `3`, reject `< 1`; not overridable by `.vane.toml`
  - `pub struct GcConfig { pub cas_retain_days: u32 }` — default `365`, reject `< 1`; not overridable by `.vane.toml`
  - `pub fn resolve_policy(cfg: &Config, root: &Path, project_file: Option<&ProjectFile>) -> Result<ResolvedPolicy, VaneCliError>`
  - `pub fn project_id(canonical_root: &Path) -> String` — SHA-256 of realpath bytes, hex first 16 chars
  - `pub fn find_current_root(cwd: &Path, roots: &[PathBuf]) -> Option<PathBuf>` — longest registered prefix

Merge rules (copy from spec): exclude = union; types/include = replace if project wrote them; embed/chunk = field-level overlay. `api_key` in `.vane.toml` → error.

- [ ] **Step 1: Write the failing test**

```rust
use std::fs;
use std::path::PathBuf;
use vane::config::{load_config, resolve_policy, ProjectFile};

fn write(path: &std::path::Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

#[test]
fn exclude_unions_and_types_replace() {
    let tmp = tempfile_dir();
    write(
        &tmp.join("config/config.toml"),
        r#"
exclude = ["**/node_modules/**", "**/*.log"]
[[types]]
glob = "**/*.md"
extractor = "text"
[[projects]]
path = "/proj"
"#,
    );
    let cfg = load_config(&tmp).unwrap();
    let pf = ProjectFile {
        exclude: vec!["**/generated/**".into()],
        include: Some(vec!["**/*.rst".into()]),
        types: None,
        embed: None,
        chunk: None,
    };
    let pol = resolve_policy(&cfg, &PathBuf::from("/proj"), Some(&pf)).unwrap();
    assert!(pol.exclude.iter().any(|e| e.contains("node_modules")));
    assert!(pol.exclude.iter().any(|e| e.contains("generated")));
    assert_eq!(pol.types.len(), 1);
    assert_eq!(pol.types[0].glob, "**/*.rst");
}

#[test]
fn project_api_key_is_rejected() {
    let err = ProjectFile::parse_toml("api_key = \"sk-x\"\n").unwrap_err();
    assert!(err.message.contains("api_key"));
}
```

Put `tempfile_dir()` as a tiny helper using `std::env::temp_dir()` + unique name; do not add the `tempfile` crate if you can avoid it. Cleaning the dir in `Drop` is required.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p vane --test config_merge`  
Expected: FAIL (module not found)

- [ ] **Step 3: Implement load + merge**

Default `TypeRule.enabled = true` when the key is absent. `include = ["g"]` compiles to `TypeRule { glob: g, extractor: "text", enabled: true }`. If both `include` and `[[types]]` exist in the project file, `[[types]]` wins.

Nested roots: `fn reject_nested(existing: &[PathBuf], new: &Path) -> Result<(), VaneCliError>` used later by `add`. Implement it here and unit-test: `/a` vs `/a/b` both directions fail; `/a` vs `/ab` ok.

- [ ] **Step 4: Run tests**

Run: `cargo test -p vane --test config_merge`  
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/vane
git commit -m "feat(vane): merge global and project sidecar config"
```

---

### Task 3: Glob match and classify

**Files:**
- Create: `crates/vane/src/glob_match.rs`, `classify.rs`
- Test: `crates/vane/src/glob_match.rs` (cfg test) and `crates/vane/tests/classify.rs`

**Interfaces:**
- Consumes: `ResolvedPolicy`
- Produces:
  - `pub fn glob_match(pattern: &str, path: &str) -> bool` — POSIX relative path, `/` separators
  - `pub enum SkipReason { Excluded, NoType, Disabled }`
  - `pub fn classify(rel_path: &str, policy: &ResolvedPolicy) -> Result<&TypeRule, SkipReason>`
  - `pub fn should_watch_dir(rel_dir: &str, policy: &ResolvedPolicy) -> bool` — false if any exclude glob would match all files under that dir (`rel_dir` itself or `rel_dir/**`)

Glob must support `*`, `**`, `?`, and `{md,txt}` braces. No `regex` crate.

- [ ] **Step 1: Write the failing tests**

```rust
use vane::glob_match::glob_match;

#[test]
fn double_star_and_braces() {
    assert!(glob_match("**/*.md", "docs/a.md"));
    assert!(glob_match("**/node_modules/**", "app/node_modules/x/y.js"));
    assert!(glob_match("**/*.{md,txt}", "a.txt"));
    assert!(!glob_match("**/*.md", "a.rs"));
    assert!(!glob_match("**/.git/**", "docs/git-notes.md"));
}
```

```rust
#[test]
fn classify_exclude_wins() {
    // policy with exclude **/*.log and type **/* extractor text
    // "a.log" => Excluded
    // "a.md" => text
    // "a.png" with no matching enabled type => NoType
}
```

- [ ] **Step 2: Run tests, expect FAIL**

Run: `cargo test -p vane glob_match classify`

- [ ] **Step 3: Implement matcher**

Recursive `**` implementation: split pattern on `/`, walk path segments. Brace expansion happens first: `{a,b}` → try each expansion. Keep the function under ~150 lines.

`should_watch_dir("node_modules", policy)` is false when exclude contains `**/node_modules/**` or `node_modules/**`.

- [ ] **Step 4: Tests PASS**

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(vane): glob matching and file classification"
```

---

### Task 4: Markdown/plain chunker

**Files:**
- Create: `crates/vane/src/chunk.rs`
- Test: `crates/vane/tests/chunk.rs`

**Interfaces:**
- Consumes: `ChunkConfig { split, max_chars, overlap_chars, min_chars }`
- Produces:
  - `pub struct Chunk { pub text: String, pub headings: Vec<String>, pub start_byte: usize, pub end_byte: usize, pub chunk_index: u32 }`
  - `pub fn chunk_text(src: &str, cfg: &ChunkConfig) -> Result<Vec<Chunk>, VaneCliError>`
  - `pub fn chunk_strategy_id(cfg: &ChunkConfig, extractor_ver: &str) -> String`

`text` for each chunk MUST start with the breadcrumb line when headings exist (`API > 鉴权\n` + body). Overlap copies previous body tail only, not the breadcrumb. Character counts are Unicode scalars. Whole-file shorter than `min_chars` still yields one chunk.

- [ ] **Step 1: Write failing tests** covering: heading split + breadcrumb; overflow split on blank lines; short file kept; invalid `overlap >= max` rejected; `chunk_strategy_id` changes when `max_chars` changes.

- [ ] **Step 2: Run, expect FAIL**

Run: `cargo test -p vane --test chunk`

- [ ] **Step 3: Implement** ATX (`#`–`######`) and Setext (`===`/`---`) headings. HTML is caller’s problem (extract.rs uses `split=plain` for `.html`).

- [ ] **Step 4: Tests PASS**

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(vane): markdown and plain text chunking"
```

---

### Task 5: Extractors and two-layer CAS

**Files:**
- Create: `crates/vane/src/extract.rs`, `cas.rs`
- Test: `crates/vane/tests/cas.rs`

**Interfaces:**
- Consumes: chunker, sha2
- Produces:
  - `pub struct CanonicalDoc { pub text: String, pub headings: Vec<String>, pub path: String, pub chunk_index: u32, pub start_byte: u64, pub end_byte: u64, pub modality: String, pub extractor: String }`
  - `pub fn extract_text(rel_path: &str, bytes: &[u8], cfg: &ChunkConfig) -> Result<Vec<CanonicalDoc>, VaneCliError>` — invalid UTF-8 → error the caller logs and skips
  - `pub fn extract_image(rel_path: &str, bytes: &[u8]) -> Result<Vec<CanonicalDoc>, VaneCliError>` — one doc; skip if `bytes.len() > 20 * 1024 * 1024`
  - `pub struct Cas { root: PathBuf }`
  - `impl Cas { pub fn get_extract(&self, key: &str) -> Option<Vec<CanonicalDoc>>; pub fn put_extract(&self, key: &str, docs: &[CanonicalDoc]) -> Result<(), VaneCliError>; pub fn get_embed(&self, key: &str) -> Option<Vec<f32>>; pub fn put_embed(&self, key: &str, v: &[f32]) -> Result<(), VaneCliError>; pub fn touch(&self, extract_key: &str, embed_keys: &[String], now: u64); pub fn last_seen(&self, key: &str) -> Option<u64>; }`
  - `pub fn extract_key(file_sha256: &str, extractor: &str, extractor_ver: &str, chunk_strategy_id: &str) -> String`
  - `pub fn embed_key(chunk_text: &str, embed_model_id: &str) -> String`

Atomic CAS writes: temp file in the same directory + rename.

- [ ] **Step 1: Failing tests** — roundtrip extract; same bytes+strategy hit; changing strategy misses; embed key changes with model id; image extractor produces `modality=image` and filename as `text`.

- [ ] **Step 2: Run FAIL**

- [ ] **Step 3: Implement.** Text files > 8 MiB: `extract_text` returns a skip error variant `VaneCliError::skip("too large")` so the writer does not panic.

- [ ] **Step 4: PASS**

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(vane): extractors and content-addressed extract/embed cache"
```

---

### Task 6: Embedder trait, mock, Ollama, OpenAI-compatible

**Files:**
- Create: `crates/vane/src/embed.rs`, `dirty.rs`
- Test: `crates/vane/tests/embed.rs` (mock + HTTP via a thread + `std::net::TcpListener`)

**Interfaces:**
- Consumes: `EmbedConfig { provider, model, base_url, api_key: Option<String> }`
- Produces:
  - `pub trait Embedder: Send { fn probe_dim(&self) -> Result<u32, VaneCliError>; fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, VaneCliError>; }`
  - `pub struct MockEmbedder { pub dim: u32, pub fail: bool, pub calls: Arc<Mutex<Vec<String>>> }`
  - `pub fn embed_model_id(provider: &str, model: &str, dim: u32) -> String` → `provider:model:dim`
  - `pub fn ollama_embedder(cfg: &EmbedConfig) -> OllamaEmbedder`
  - `pub fn openai_embedder(cfg: &EmbedConfig) -> OpenAiCompatEmbedder` — POST `{base_url}/v1/embeddings`, batch ≤ 64
  - `pub struct DirtyQueue` with `push(project_id, path)`, `pop_due(now) -> Vec<DirtyItem>`, backoff 1s..60s

Ollama: `POST {base_url}/api/embeddings` with JSON `{"model","prompt"}` per text (no true batch).

- [ ] **Step 1: Failing tests**
  - Mock `fail=true` → `embed` errors
  - Fake HTTP server returns `{"embedding":[0.1,0.2]}` for Ollama; client `probe_dim` is 2
  - OpenAI fake returns `{"data":[{"embedding":[1.0,0.0,0.0]}]}`; batch of 65 texts results in two HTTP calls (64+1)
  - DirtyQueue: first retry at +1s, then doubles until 60s

- [ ] **Step 2: FAIL**

- [ ] **Step 3: Implement with ureq.** Read `OPENAI_API_KEY` then `VANE_EMBED_API_KEY`. Never log the key.

- [ ] **Step 4: PASS**

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(vane): embedding providers and dirty retry queue"
```

---

### Task 7: Live set + per-project Vane index

**Files:**
- Create: `crates/vane/src/live.rs`, `index.rs`
- Test: `crates/vane/tests/index_live.rs`

**Interfaces:**
- Consumes: `vane_core::api::{Db, Collection, Doc, OpenOptions, CollectionOptions, Schema, FieldDef, Metric, ScalarKind, BuiltinTokenizer, SearchQuery, SearchMode, FusionSpec, Filter, FilterCond, ScalarValue}`
- Produces:
  - `pub struct LiveFile { pub content_sha256: String, pub extract_key: String, pub chunk_count: u32 }`
  - `pub struct LiveSet { pub files: BTreeMap<String, LiveFile> }` with `load`/`save_atomic`
  - `pub struct ProjectIndex { db: Db, col: Collection, dim: u32, model_id: String }`
  - `pub fn open_or_create(home: &Path, project_id: &str, dim: u32, model_id: &str) -> Result<ProjectIndex, VaneCliError>`
  - `pub fn doc_id(project_id: &str, rel_path: &str, chunk_index: u32) -> String` → `{project_id}:{rel_path}#{chunk_index}`
  - `impl ProjectIndex { add_docs, delete_ids, flush, compact, search }`

Schema (exact field names from spec): `body` text, `embedding` vector cosine `dim`, scalars keyword `root`,`path`,`modality`,`extractor`, int `chunk_index`,`start_byte`,`end_byte`.

Collection name: `"docs"`. Tokenizer: `BuiltinTokenizer::Jieba`; if `collection()` returns dict-unavailable, retry `CjkBigram` and record `tokenizer_fallback` in a `state.json` field.

`OpenOptions.auto_commit` / collection auto-commit **off**. Caller flushes.

Compact when deletes in the window ≥ 1000 or dead/live ≥ 0.2. Expose `maybe_compact(&self, deletes: u64, live: u64, dead: u64)`.

- [ ] **Step 1: Failing integration test** with `VANE_HOME` temp dir, `MockEmbedder` dim=4, add two chunks, `flush`, `search` text-only finds the id, `delete` + `flush` removes it. Tokenizer fallback is allowed.

```rust
let q = SearchQuery {
    text: Some("鉴权".into()),
    vector: None,
    top_k: 8,
    mode: SearchMode::Text,
    fusion: FusionSpec::Rrf,
    filter: None,
    candidate_multiplier: 3,
};
```

- [ ] **Step 2: FAIL**

- [ ] **Step 3: Implement `open_or_create` using `StdFsVfs::with_root` or `StdFsVfs::new()` plus db path `home/rag/projects/{id}/db`. Use the same pattern as `crates/vane-node/src/db.rs` (`StdFsVfs::new()` + absolute path).**

- [ ] **Step 4: PASS**

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(vane): per-project live set and Vane collection"
```

---

### Task 8: Working-set sync (add/delete/rename/CAS)

**Files:**
- Create: `crates/vane/src/sync.rs`
- Test: `crates/vane/tests/sync_cas.rs`

**Interfaces:**
- Consumes: classify, extract, cas, embed, live, index
- Produces:
  - `pub fn reconcile_project(ctx: &mut SyncCtx, root: &Path, policy: &ResolvedPolicy) -> Result<SyncReport, VaneCliError>`
  - `pub struct SyncReport { pub added: u64, pub deleted: u64, pub unchanged: u64, pub embedded: u64, pub cas_hits: u64 }`
  - Walk files (no follow-out-of-root symlinks). Hash SHA-256 streaming. Apply §7.3 table.

Rename test: write file A, reconcile, rename to B, reconcile with embedder that panics if `embed` is called; expect `embedded == 0`, live has B not A, search still works.

Model-rebuild flag: if `state.embed_model_id` mismatches index, `reconcile_project` must not take the no-op row; that path is Task 11. For now if mismatch, return error `"model rebuild required"`.

- [ ] **Step 1: Tests** — new file embeds once; second reconcile `embedded==0`; delete removes from search; rename does not embed.

- [ ] **Step 2: FAIL**

- [ ] **Step 3: Implement walk with `should_watch_dir` so `node_modules` is not descended. Skip non-UTF8 and oversized files (log, continue).**

- [ ] **Step 4: PASS**

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(vane): working-set reconcile with CAS reuse"
```

---

### Task 9: Daemon, flock, JSON-line IPC, daily logs

**Files:**
- Create: `crates/vane/src/ipc.rs`, `daemon.rs`, `log.rs`
- Modify: `crates/vane/src/main.rs`
- Test: `crates/vane/tests/daemon_ipc.rs`, `crates/vane/tests/log_rotate.rs`

**Interfaces:**
- Consumes: config, sync, index
- Produces:
  - `pub struct RpcRequest { pub id: String, pub method: String, pub params: serde_json::Value }`
  - `pub struct RpcResponse { pub id: String, pub result: Option<Value>, pub error: Option<RpcError> }`
  - `pub fn serve_forever(home: PathBuf) -> Result<(), VaneCliError>`
  - Methods: `status`, `search` (stub ok until Task 11 if it returns empty list), `read`, `list_roots`, `reload_config`, `add_root`, `remove_root`
  - `fn acquire_pid_lock(home: &Path) -> Result<File, VaneCliError>`
  - `pub struct DailyLogger { ... }`
  - `impl DailyLogger { pub fn open(dir: &Path, retain_days: u32) -> Result<Self, VaneCliError>; pub fn write(&mut self, level: Level, msg: &str); pub fn prune_with_today(&self, today: NaiveDate); }`

`today` is injected in tests so prune does not depend on the real clock. Production `write` uses local calendar date (`time` crate `OffsetDateTime::now_local`, or `chrono::Local` if `time` local offset is painful). Do not add tracing/tokio.

Log path: `{home}/log/daemon.YYYY-MM-DD.log`. `retain_days=3` and `today=2026-08-19` keeps 19/18/17, deletes `daemon.2026-08-16.log` and undated `daemon.log`.

Writer: `std::sync::mpsc::channel` of `WriterCmd`. Only that thread calls Vane write APIs. Main thread accepts `UnixListener` on `home/run/vane.sock` mode `0o600`.

Second daemon: spawn two `serve_forever` on the same home; the second returns error containing `"already running"`.

- [ ] **Step 1: Tests** — lock; JSON roundtrip `list_roots`; missing config.toml → CLI `status` exit 1. Log rotate: seed `log/daemon.2026-08-16.log`, `daemon.2026-08-17.log`, `daemon.log`; `prune_with_today(2026-08-19)` + `retain_days=3` leaves only the 17th file (and creates today on next write).

- [ ] **Step 2: FAIL**

- [ ] **Step 3: Implement `vane daemon`, `vane start` (spawns daemon if not running — first version may `daemon` in foreground only; `start` writes a note if launchd not installed yet). Socket dir created with `create_dir_all`. Open `DailyLogger` at start and prune; `write` switches file when the local date changes and prunes again. Log IO errors go to stderr and do not panic.**

PID liveness: write pid; if file exists, `libc` kill 0 or `std::fs::read_to_string` + `sysctl` skip — on Unix `nix` is extra; use:

```rust
fn pid_alive(pid: u32) -> bool {
    let r = unsafe { libc::kill(pid as i32, 0) };
    r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}
```

Adding `libc` crate is allowed (not on the deny list). Alternatively parse `/bin/ps` — do not. Use `libc`.

- [ ] **Step 4: PASS**

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(vane): daemon pid lock, JSON-line socket, and daily log rotation"
```

---

### Task 10: Watch with exclude-at-register

**Files:**
- Create: `crates/vane/src/watch.rs`
- Test: `crates/vane/tests/watch_exclude.rs`
- Modify: `daemon.rs` to start the watch thread

**Interfaces:**
- Consumes: notify, `should_watch_dir`
- Produces:
  - `pub fn watch_roots(roots: Vec<(PathBuf, ResolvedPolicy)>, tx: Sender<Vec<WatchEvent>>) -> Result<WatchGuard, VaneCliError>`
  - `pub struct WatchEvent { pub root: PathBuf, pub rel: String, pub kind: WatchKind }` enum Create/Modify/Remove
  - Debounce: 500ms quiet or 2s max, then one batch

Test: create a root with `node_modules/pkg/a.md` and `docs/a.md`; after `watch_roots`, the recorded *registered* watch paths (expose via `WatchGuard::watched_paths_for_test`) must not contain `node_modules`. Write `docs/a.md` and receive an event; writing under `node_modules` must not enqueue work (either no event or classify drops it — assert sync is not called by checking embed calls stay 0 if only node_modules changes).

- [ ] **Step 1: FAILING test** on watched path list

- [ ] **Step 2: FAIL**

- [ ] **Step 3: Implement recursive watch by walking and calling `watcher.watch(dir)` only when `should_watch_dir`. On create-dir events, if the new dir is not excluded, `watch` it too.**

- [ ] **Step 4: PASS** (may be slightly timing-sensitive; wait up to 3s for the event)

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(vane): single watcher that does not descend excluded dirs"
```

---

### Task 11: Search, snippet, BM25 fallback, cross-project RRF, query CLI

**Files:**
- Create: `crates/vane/src/rrf.rs`, `search.rs`
- Modify: `daemon.rs` `search`/`read` methods, `main.rs` `vane query`
- Test: `crates/vane/tests/search.rs`

**Interfaces:**
- Consumes: ProjectIndex, Embedder
- Produces:
  - `pub struct SearchHit { pub id, path, root, title, snippet, score, modality, extractor, degraded: bool }`
  - `pub fn snippet(canonical_text: &str) -> String` — strip first line if it looks like breadcrumb (contains ` > ` or the first line equals `headings.join(" > ")`), then first 240 Unicode scalars
  - `pub fn search_project(...) -> Result<Vec<SearchHit>, VaneCliError>`
  - `pub fn search_all(projects, query, top_k) -> Result<Vec<SearchHit>, VaneCliError>` — unique `embed_model_id` embed once, RRF k=60 on `id`
  - `pub fn rrf_merge(lists: Vec<Vec<SearchHit>>, k: u32, top_k: usize) -> Vec<SearchHit>`
  - `read_by_id` / `read_by_path` — path returns **all** chunks ascending; body from extract CAS

`vane query` default = current project (`find_current_root`). `--all` uses `search_all`. `--root` sets one project.

Empty index → `[]`.

When `embed()` fails: `SearchMode::Text`, `degraded: true`.

- [ ] **Step 1: Tests** — snippet 240; embed fail → degraded text hits; two projects different mock dims, `--all` returns both; `read` path returns 2 chunks; `query` without `--all` from cwd inside project A does not return B-only docs.

- [ ] **Step 2: FAIL**

- [ ] **Step 3: Implement RRF: score `1.0 / (k + rank)` with rank 1-based, sum by `id`, sort desc, stable id tie-break.**

Hybrid query when embed works:

```rust
SearchQuery {
    text: Some(q.clone()),
    vector: Some(vec),
    top_k,
    mode: SearchMode::Hybrid,
    fusion: FusionSpec::Rrf,
    filter,
    candidate_multiplier: 3,
}
```

- [ ] **Step 4: PASS**

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(vane): hybrid search, BM25 fallback, and cross-project RRF"
```

---

### Task 12: Model change rebuild

**Files:**
- Modify: `index.rs`, `sync.rs`, `daemon.rs`, `main.rs` (`vane model`)
- Test: `crates/vane/tests/model_rebuild.rs`

**Interfaces:**
- Consumes: probe_dim, CAS extract reuse
- Produces:
  - `pub fn rebuild_for_new_model(home, project_id, new_cfg) -> Result<(), VaneCliError>`
  - Writes `db.new/`, serves old `db/` until swap: `db` → `db.prev`, `db.new` → `db`, delete `db.prev`
  - `state.json`: `{ root_path, embed_model_id, dim, chunk_strategy_id, rebuild: { done, total } | null, reindex_error: Option<String> }`

- [ ] **Step 1: Test** — index with Mock dim=4; switch to Mock dim=8; during rebuild a search still returns old hits; after rebuild, `state.dim==8` and search works; extract CAS hit count > 0 and embed called for each chunk once. Failed rebuild (mock errors mid-way) leaves old db queryable.

- [ ] **Step 2: FAIL**

- [ ] **Step 3: Implement.** Never add a vector whose len != new dim.

- [ ] **Step 4: PASS**

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(vane): rebuild project index when embedding model or dim changes"
```

---

### Task 13: MCP stdio bridge + image `read` + SKILL.md

**Files:**
- Create: `crates/vane/src/mcp.rs`, `crates/vane/SKILL.md`
- Modify: `main.rs` (`vane mcp`)
- Test: `crates/vane/tests/mcp.rs`

**Interfaces:**
- Consumes: IPC client to daemon
- Produces: MCP initialize + tools `search`, `read`, `list_roots`
- Image `read`: if file ≤ 4 MiB, MCP content type image (base64); else path + mime only

MCP is JSON-RPC 2.0 **stdio** (Content-Length framing as in the MCP spec, not the daemon’s JSON lines). Implement a small codec:

- Read headers until `\r\n\r\n`, parse `Content-Length`, read body JSON
- Write `Content-Length: N\r\n\r\n` + body

Do not add an MCP SDK crate unless it has zero banned deps; a 100-line codec is enough.

- [ ] **Step 1: Tests** — spawn daemon in-process or thread, write initialize + `tools/list` to a `vane mcp` child or `handle_mcp_message`; expect the three tool names. `read` on a png fixture ≤ 4 MiB returns image payload.

- [ ] **Step 2: FAIL**

- [ ] **Step 3: Implement SKILL.md** telling agents: call `list_roots`, then `search`, then `read` for full chunk; do not walk the filesystem; daemon must be running.

- [ ] **Step 4: PASS**

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(vane): MCP stdio bridge and image read"
```

---

### Task 14: Init wizard, service install/uninstall, CLI polish

**Files:**
- Create: `crates/vane/src/wizard.rs`, `service.rs`
- Modify: `main.rs`
- Test: `crates/vane/tests/init_service.rs`

**Interfaces:**
- Produces:
  - `pub fn run_init(home: &Path, stdin, stdout, assume: Option<InitAnswers>) -> Result<(), VaneCliError>`
  - `InitAnswers { provider, model, base_url, first_root: Option<PathBuf>, exclude: Vec<String>, images: bool, install_service: bool }`
  - `pub fn install_user_service(home: &Path, vane_bin: &Path) -> Result<(), VaneCliError>`
  - `pub fn uninstall_user_service() -> Result<(), VaneCliError>`

Interactive wizard uses `std::io` (no inquire). Tests call `run_init` with `assume: Some(...)` so they are non-interactive.

macOS plist `~/Library/LaunchAgents/com.vane.daemon.plist` `ProgramArguments = [vane_bin, "daemon", "--home", home]`. Linux: `~/.config/systemd/user/vane.service`. Uninstall removes the file and `launchctl unload` / `systemctl --user disable --now`. Does not delete `VANE_HOME` data.

CLI surface from spec: `init`, `add`, `rm`, `include add|reset`, `exclude add|reset`, `model`, `start`, `stop`, `daemon`, `service uninstall`, `status`, `query`, `mcp`, `gc`. Global `--home`.

- [ ] **Step 1: Tests** — `assume` writes `config.toml` with chosen exclude; missing init blocks `query`; uninstall is idempotent if plist absent.

- [ ] **Step 2: FAIL**

- [ ] **Step 3: Implement remaining CLI by sending IPC methods to a running daemon; if daemon is down, print the spec’s start hint (except `init`/`daemon`/`start`).**

- [ ] **Step 4: PASS** plus `cargo test -p vane`, `cargo clippy -p vane --all-targets -- -D warnings`, `cargo fmt --all -- --check`

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(vane): init wizard and user service install"
```

---

### Task 16: CAS garbage collection

**Files:**
- Create: `crates/vane/src/gc.rs`
- Modify: `cas.rs` (`touch`, `last_seen`, store `embed_keys` next to extract), `daemon.rs` (startup + daily TTL, IPC `gc`), `sync.rs` (touch on live keep/add), `main.rs` (`vane gc`)
- Test: `crates/vane/tests/gc.rs`

**Interfaces:**
- Consumes: live.json of all registered projects, Cas, LogConfig-style `GcConfig { cas_retain_days: u32 }` default 365
- Produces:
  - `pub fn gc_ttl(cas: &Cas, live_keys: &LiveKeySet, now: u64, retain_days: u32) -> GcReport`
  - `pub fn gc_unreferenced(cas: &Cas, live_keys: &LiveKeySet) -> GcReport` — delete extract/embed not in the set
  - `pub fn gc_project(home, root, all_lives, cas) -> GcReport` — compact project db, drop `db.prev`, then `gc_unreferenced` using **union of all remaining lives** (other projects still pin shared keys)
  - Never deletes files under the project source root. Tests must assert source files still exist.

`now` and `retain_days` are injected. `retain_days=365` and `last_seen = now - 366*86400` → deleted; in live set → kept even if last_seen is old (refresh first in reconcile, but TTL function must also skip keys present in `live_keys`).

- [ ] **Step 1: Tests**
  - Two projects share extract key K; `gc_project(A)` after A drops the file keeps K because B live still has it.
  - A is the only live; file removed from A's live; `gc_project(A)` deletes K immediately.
  - K not in any live, `last_seen` 10 days ago, `cas_retain_days=365` → TTL keeps; `last_seen` 400 days ago → TTL deletes.
  - Source file path still on disk after gc.
  - `cas_retain_days=0` rejected at config load.

- [ ] **Step 2: FAIL**

- [ ] **Step 3: Implement.** Daemon runs `gc_ttl` on start and when local date changes (same hook as log prune). IPC method `gc` with `{root? , all?}`.

- [ ] **Step 4: PASS**

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(vane): gc unreferenced CAS and TTL unused embeddings"
```

---

### Task 15: Workspace gates

**Files:**
- Modify: `.github/workflows/ci.yml` only if `cargo test --workspace` / clippy need an exclude (should not; vane is native)
- Modify: `scripts/check-no-std-fs.sh` if it greps the whole workspace — **must not** flag `crates/vane` (sidecar is allowed `std::fs`). Read the script; if it scans all crates, limit it to `vane-core` and `vane-wasm` as today.

**Interfaces:** none new

- [ ] **Step 1: Run** `bash scripts/check-no-std-fs.sh` and `cargo deny check` from repo root. If deny fails on ureq/notify licenses, add only an allowed license already in the allow list; do not add openssl.

- [ ] **Step 2: If check-no-std-fs fails on crates/vane, change the script to skip that package (exact paths `crates/vane-core` and `crates/vane-wasm`).**

- [ ] **Step 3: `cargo test -p vane && cargo clippy --workspace --exclude vane-fuzz --all-targets -- -D warnings`**

- [ ] **Step 4: Commit only if scripts/deny changed**

```bash
git commit -am "chore(ci): keep sidecar native-only and deny-clean"
```

---

## Self-review vs spec

| Spec section | Task |
|--------------|------|
| §3 home / VANE_HOME | 1 |
| §4 config merge, nested roots, secrets | 2 |
| §5 classify, extractors, size limits | 3, 5, 8 |
| §6 chunking | 4 |
| §7 CAS, live, rename, git=files | 5, 8 |
| §7.5 gc unreferenced + TTL | 16 |
| §7.4 model rebuild | 12 |
| §8 schema, jieba, compact | 7 |
| §9 daemon, flock, watch exclude, embed backoff, daily logs | 6, 9, 10 |
| §9.5 IPC JSON lines, no ACL | 9 |
| §10 CLI + init + service uninstall | 14 |
| §11 MCP, snippet, read-all-chunks, search defaults | 11, 13 |
| §12 errors / BM25 degraded | 11 |
| §13 PDF reserved (warn + skip unknown extractor) | 3 (`classify` skips unknown extractor with log) |
| §14 tests | each task |
| Image extractor enabled flag | 5, 14 |

Unknown extractor names in `[[types]]` (`pdf`, `docx`, `pptx`): classify logs a warning and skips the file (spec §5.2). Implement that in Task 3.

No placeholders remain. Types named in later tasks (`CanonicalDoc`, `ResolvedPolicy`, `SearchHit`, `RpcRequest`) are produced in earlier tasks.
