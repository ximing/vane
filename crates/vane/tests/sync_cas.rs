use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use vane::cas::Cas;
use vane::config::{ChunkConfig, EmbedConfig, ResolvedPolicy, TypeRule};
use vane::embed::{embed_model_id, Embedder, MockEmbedder};
use vane::error::VaneCliError;
use vane::index::{doc_id, open_or_create, state_path, ProjectState};
use vane::live::LiveSet;
use vane::project::project_id;
use vane::sync::{reconcile_project, SyncCtx, SyncReport};
use vane_core::api::{FusionSpec, SearchMode, SearchQuery};

struct TempHome {
    path: PathBuf,
}

fn tempfile_dir() -> TempHome {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "vane-sync-cas-test-{}-{}-{}",
        std::process::id(),
        nanos,
        n
    ));
    fs::create_dir_all(&path).unwrap();
    // Isolation: never touch the user's real ~/.vane.
    std::env::set_var("VANE_HOME", &path);
    TempHome { path }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let tmp = std::env::temp_dir();
        if self.path.starts_with(&tmp) && self.path != tmp {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

impl std::ops::Deref for TempHome {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.path
    }
}

fn policy() -> ResolvedPolicy {
    ResolvedPolicy {
        embed: EmbedConfig {
            provider: "mock".into(),
            model: "test".into(),
            base_url: "http://127.0.0.1".into(),
            api_key: None,
        },
        chunk: ChunkConfig {
            split: "markdown".into(),
            max_chars: 1200,
            overlap_chars: 200,
            min_chars: 50,
        },
        exclude: vec!["**/node_modules/**".into()],
        types: vec![TypeRule {
            glob: "**/*.{md,mdx,txt,rst,org,html}".into(),
            extractor: "text".into(),
            enabled: true,
        }],
    }
}

fn mock_embedder(dim: u32) -> MockEmbedder {
    MockEmbedder {
        dim,
        fail: false,
        calls: Arc::new(Mutex::new(Vec::new())),
    }
}

struct PanicOnEmbed {
    dim: u32,
}

impl Embedder for PanicOnEmbed {
    fn probe_dim(&self) -> Result<u32, VaneCliError> {
        Ok(self.dim)
    }

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, VaneCliError> {
        panic!("embed must not be called on CAS hit / rename; texts={texts:?}");
    }
}

fn text_query(q: &str) -> SearchQuery {
    SearchQuery {
        text: Some(q.into()),
        vector: None,
        top_k: 8,
        mode: SearchMode::Text,
        fusion: FusionSpec::Rrf,
        filter: None,
        candidate_multiplier: 3,
    }
}

const AUTH_BODY: &str =
    "# 鉴权\n\n如何做登录鉴权，这是一段足够长的说明文字以便通过最小切片长度。\n";

fn write_file(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

fn setup_project(home: &Path) -> (PathBuf, String, u32, String) {
    let root = home.join("proj");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let pid = project_id(&root);
    let dim = 4;
    let model = embed_model_id("mock", "test", dim);
    (root, pid, dim, model)
}

fn report_counts(r: &SyncReport) -> (u64, u64, u64, u64, u64) {
    (r.added, r.deleted, r.unchanged, r.embedded, r.cas_hits)
}

#[test]
fn new_file_embeds_once_second_reconcile_is_noop() {
    let tmp = tempfile_dir();
    assert!(tmp.starts_with(std::env::temp_dir()));
    assert!(!tmp.starts_with(dirs_home().join(".vane")));

    let (root, pid, dim, model) = setup_project(&tmp);
    write_file(&root.join("docs/auth.md"), AUTH_BODY);

    let cas = Cas::new(tmp.join("rag").join("cas"));
    let idx = open_or_create(&tmp, &pid, dim, &model).unwrap();
    let embedder = mock_embedder(dim);
    let mut ctx = SyncCtx {
        home: &tmp,
        project_id: &pid,
        cas: &cas,
        index: &idx,
        embedder: &embedder,
        now: 1_700_000_000,
        dirty: None,
    };

    let first = reconcile_project(&mut ctx, &root, &policy()).unwrap();
    assert_eq!(first.added, 1, "new file should be added");
    assert_eq!(first.deleted, 0);
    assert_eq!(first.unchanged, 0);
    assert_eq!(first.embedded, 1, "new file embeds once");
    assert_eq!(first.cas_hits, 0);
    assert_eq!(embedder.calls.lock().unwrap().len(), 1);

    let second = reconcile_project(&mut ctx, &root, &policy()).unwrap();
    assert_eq!(second.embedded, 0, "second reconcile must not re-embed");
    assert_eq!(second.unchanged, 1);
    assert_eq!(second.added, 0);
    assert_eq!(second.deleted, 0);
    assert_eq!(embedder.calls.lock().unwrap().len(), 1);

    let live = LiveSet::load_for_project(&tmp, &pid).unwrap();
    assert!(live.files.contains_key("docs/auth.md"));
    assert_eq!(live.files["docs/auth.md"].chunk_count, 1);

    let hits = idx.search(&text_query("鉴权")).unwrap();
    assert!(
        hits.iter().any(|h| h.id == doc_id(&pid, "docs/auth.md", 0)),
        "indexed file must be searchable, hits={:?}",
        hits.iter().map(|h| &h.id).collect::<Vec<_>>()
    );
}

#[test]
fn delete_removes_from_search() {
    let tmp = tempfile_dir();
    let (root, pid, dim, model) = setup_project(&tmp);
    write_file(&root.join("docs/auth.md"), AUTH_BODY);

    let cas = Cas::new(tmp.join("rag").join("cas"));
    let idx = open_or_create(&tmp, &pid, dim, &model).unwrap();
    let embedder = mock_embedder(dim);
    let mut ctx = SyncCtx {
        home: &tmp,
        project_id: &pid,
        cas: &cas,
        index: &idx,
        embedder: &embedder,
        now: 1_700_000_000,
        dirty: None,
    };

    reconcile_project(&mut ctx, &root, &policy()).unwrap();
    fs::remove_file(root.join("docs/auth.md")).unwrap();

    let report = reconcile_project(&mut ctx, &root, &policy()).unwrap();
    assert_eq!(report.deleted, 1);
    assert_eq!(report.added, 0);
    assert_eq!(report.embedded, 0);

    let live = LiveSet::load_for_project(&tmp, &pid).unwrap();
    assert!(!live.files.contains_key("docs/auth.md"));

    let hits = idx.search(&text_query("鉴权")).unwrap();
    let gone = doc_id(&pid, "docs/auth.md", 0);
    assert!(
        hits.iter().all(|h| h.id != gone),
        "deleted path must not appear after reconcile, hits={:?}",
        hits.iter().map(|h| &h.id).collect::<Vec<_>>()
    );
}

#[test]
fn rename_does_not_embed() {
    let tmp = tempfile_dir();
    let (root, pid, dim, model) = setup_project(&tmp);
    write_file(&root.join("docs/a.md"), AUTH_BODY);

    let cas = Cas::new(tmp.join("rag").join("cas"));
    let idx = open_or_create(&tmp, &pid, dim, &model).unwrap();
    let embedder = mock_embedder(dim);
    let mut ctx = SyncCtx {
        home: &tmp,
        project_id: &pid,
        cas: &cas,
        index: &idx,
        embedder: &embedder,
        now: 1_700_000_000,
        dirty: None,
    };

    let first = reconcile_project(&mut ctx, &root, &policy()).unwrap();
    assert_eq!(first.embedded, 1);
    assert_eq!(report_counts(&first), (1, 0, 0, 1, 0));

    fs::rename(root.join("docs/a.md"), root.join("docs/b.md")).unwrap();

    let panic_embedder = PanicOnEmbed { dim };
    let mut ctx = SyncCtx {
        home: &tmp,
        project_id: &pid,
        cas: &cas,
        index: &idx,
        embedder: &panic_embedder,
        now: 1_700_000_001,
        dirty: None,
    };
    let renamed = reconcile_project(&mut ctx, &root, &policy()).unwrap();
    assert_eq!(renamed.embedded, 0, "rename must reuse embed CAS");
    assert_eq!(renamed.added, 1);
    assert_eq!(renamed.deleted, 1);
    assert!(renamed.cas_hits >= 1, "rename must hit extract CAS");

    let live = LiveSet::load_for_project(&tmp, &pid).unwrap();
    assert!(live.files.contains_key("docs/b.md"), "live should have B");
    assert!(
        !live.files.contains_key("docs/a.md"),
        "live should not have A"
    );

    let hits = idx.search(&text_query("鉴权")).unwrap();
    assert!(
        hits.iter().any(|h| h.id == doc_id(&pid, "docs/b.md", 0)),
        "renamed path must be searchable as B, hits={:?}",
        hits.iter().map(|h| &h.id).collect::<Vec<_>>()
    );
    assert!(
        hits.iter().all(|h| h.id != doc_id(&pid, "docs/a.md", 0)),
        "old path A must be gone, hits={:?}",
        hits.iter().map(|h| &h.id).collect::<Vec<_>>()
    );
}

#[test]
fn model_mismatch_requires_rebuild() {
    let tmp = tempfile_dir();
    let (root, pid, dim, model) = setup_project(&tmp);
    write_file(&root.join("docs/auth.md"), AUTH_BODY);

    let cas = Cas::new(tmp.join("rag").join("cas"));
    let idx = open_or_create(&tmp, &pid, dim, &model).unwrap();
    let mut state = ProjectState::load(&state_path(&tmp, &pid)).unwrap();
    state.embed_model_id = Some("other:model:8".into());
    state.save_atomic(&state_path(&tmp, &pid)).unwrap();

    let embedder = mock_embedder(dim);
    let mut ctx = SyncCtx {
        home: &tmp,
        project_id: &pid,
        cas: &cas,
        index: &idx,
        embedder: &embedder,
        now: 1_700_000_000,
        dirty: None,
    };
    let err = reconcile_project(&mut ctx, &root, &policy()).unwrap_err();
    assert!(
        err.message.contains("model rebuild required"),
        "expected rebuild error, got {}",
        err.message
    );
}

fn dirs_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}
