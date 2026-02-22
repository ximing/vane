use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use vane::cas::Cas;
use vane::config::{ChunkConfig, EmbedConfig, ResolvedPolicy, TypeRule};
use vane::embed::{embed_model_id, Embedder, MockEmbedder};
use vane::error::VaneCliError;
use vane::index::{doc_id, open_or_create, project_db_path, state_path, ProjectState};
use vane::live::LiveSet;
use vane::project::project_id;
use vane::sync::{rebuild_for_new_model, rebuild_for_new_model_with, reconcile_project, SyncCtx};
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
        "vane-model-rebuild-{}-{}-{}",
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

fn dirs_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn assert_isolated(home: &Path) {
    assert!(home.starts_with(std::env::temp_dir()));
    assert!(!home.starts_with(dirs_home().join(".vane")));
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

fn embed_cfg(model: &str) -> EmbedConfig {
    EmbedConfig {
        provider: "mock".into(),
        model: model.into(),
        base_url: "http://127.0.0.1".into(),
        api_key: None,
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
const INTRO_BODY: &str =
    "# 介绍\n\n欢迎使用本机文档检索，这是一段足够长的说明文字以便通过最小切片长度。\n";

fn write_file(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

fn project_dir(home: &Path, pid: &str) -> PathBuf {
    home.join("rag").join("projects").join(pid)
}

/// Blocks on the first `embed` until the test releases the gate so a search
/// can still hit the old `db/` while `db.new/` is being built.
struct GateEmbedder {
    dim: u32,
    started: Mutex<Option<mpsc::Sender<()>>>,
    go: Mutex<Option<mpsc::Receiver<()>>>,
    calls: Arc<Mutex<Vec<String>>>,
}

impl Embedder for GateEmbedder {
    fn probe_dim(&self) -> Result<u32, VaneCliError> {
        Ok(self.dim)
    }

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, VaneCliError> {
        if let Ok(mut calls) = self.calls.lock() {
            calls.extend(texts.iter().cloned());
        }
        if let Some(tx) = self.started.lock().expect("started").take() {
            let _ = tx.send(());
        }
        if let Some(rx) = self.go.lock().expect("go").take() {
            match rx.recv_timeout(Duration::from_secs(10)) {
                Ok(()) | Err(RecvTimeoutError::Disconnected) => {}
                Err(RecvTimeoutError::Timeout) => {
                    return Err(VaneCliError::new("rebuild gate timed out"));
                }
            }
        }
        let dim = self.dim as usize;
        Ok(texts.iter().map(|_| vec![0.1; dim]).collect())
    }
}

/// `probe_dim` succeeds; `embed` succeeds `ok_embeds` times then errors.
struct FailAfterEmbeds {
    dim: u32,
    ok_embeds: Mutex<u32>,
    calls: Arc<Mutex<Vec<String>>>,
}

impl Embedder for FailAfterEmbeds {
    fn probe_dim(&self) -> Result<u32, VaneCliError> {
        Ok(self.dim)
    }

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, VaneCliError> {
        if let Ok(mut calls) = self.calls.lock() {
            calls.extend(texts.iter().cloned());
        }
        let mut left = self.ok_embeds.lock().expect("ok_embeds");
        if *left == 0 {
            return Err(VaneCliError::new("embed failed mid-rebuild"));
        }
        *left -= 1;
        let dim = self.dim as usize;
        Ok(texts.iter().map(|_| vec![0.2; dim]).collect())
    }
}

fn setup_indexed(home: &Path) -> (PathBuf, String, u32, String) {
    let root = home.join("proj");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let pid = project_id(&root);
    let dim = 4;
    let model = embed_model_id("mock", "test", dim);
    write_file(&root.join("docs/auth.md"), AUTH_BODY);
    write_file(&root.join("docs/intro.md"), INTRO_BODY);

    let cas = Cas::new(home.join("rag").join("cas"));
    let idx = open_or_create(home, &pid, dim, &model).unwrap();
    let embedder = mock_embedder(dim);
    let mut ctx = SyncCtx {
        home,
        project_id: &pid,
        cas: &cas,
        index: &idx,
        embedder: &embedder,
        now: 1_700_000_000,
    };
    let report = reconcile_project(&mut ctx, &root, &policy()).unwrap();
    assert_eq!(report.added, 2);
    assert_eq!(report.embedded, 2);
    idx.flush().unwrap();
    drop(idx);
    (root, pid, dim, model)
}

#[test]
fn rebuild_switches_dim_and_reuses_extract_cas() {
    let tmp = tempfile_dir();
    assert_isolated(&tmp);

    let (_root, pid, old_dim, old_model) = setup_indexed(&tmp);
    let old_idx = open_or_create(&tmp, &pid, old_dim, &old_model).unwrap();
    let before = old_idx.search(&text_query("鉴权")).unwrap();
    assert!(
        before
            .iter()
            .any(|h| h.id == doc_id(&pid, "docs/auth.md", 0)),
        "pre-rebuild search must find the old hit, hits={:?}",
        before.iter().map(|h| &h.id).collect::<Vec<_>>()
    );

    let (started_tx, started_rx) = mpsc::channel();
    let (go_tx, go_rx) = mpsc::channel();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let new_cfg = embed_cfg("test-8");
    let embedder = GateEmbedder {
        dim: 8,
        started: Mutex::new(Some(started_tx)),
        go: Mutex::new(Some(go_rx)),
        calls: Arc::clone(&calls),
    };

    let home = tmp.path.clone();
    let pid_clone = pid.clone();
    let cfg_clone = new_cfg.clone();
    let handle =
        thread::spawn(move || rebuild_for_new_model_with(&home, &pid_clone, &cfg_clone, &embedder));

    started_rx
        .recv_timeout(Duration::from_secs(8))
        .expect("rebuild should reach embed while old db is still live");

    let mid = old_idx.search(&text_query("鉴权")).unwrap();
    assert!(
        mid.iter().any(|h| h.id == doc_id(&pid, "docs/auth.md", 0)),
        "search during rebuild must still return old hits, hits={:?}",
        mid.iter().map(|h| &h.id).collect::<Vec<_>>()
    );
    assert!(
        project_dir(&tmp, &pid).join("db.new").exists(),
        "rebuild must write db.new/ before swap"
    );

    go_tx.send(()).unwrap();
    let report = handle.join().expect("rebuild thread").unwrap();
    assert!(
        report.cas_hits > 0,
        "extract CAS must be reused on model change, report={report:?}"
    );
    assert_eq!(
        report.embedded, 2,
        "each live chunk must be embedded once for the new model, report={report:?}"
    );
    assert_eq!(calls.lock().unwrap().len(), 2);

    drop(old_idx);

    let state = ProjectState::load(&state_path(&tmp, &pid)).unwrap();
    assert_eq!(state.dim, Some(8));
    assert_eq!(
        state.embed_model_id.as_deref(),
        Some(embed_model_id("mock", "test-8", 8).as_str())
    );
    assert!(state.rebuild.is_none());
    assert!(state.reindex_error.is_none());
    assert!(!project_dir(&tmp, &pid).join("db.new").exists());
    assert!(!project_dir(&tmp, &pid).join("db.prev").exists());
    assert!(project_db_path(&tmp, &pid).exists());

    let new_model = embed_model_id("mock", "test-8", 8);
    let new_idx = open_or_create(&tmp, &pid, 8, &new_model).unwrap();
    let after = new_idx.search(&text_query("鉴权")).unwrap();
    assert!(
        after
            .iter()
            .any(|h| h.id == doc_id(&pid, "docs/auth.md", 0)),
        "post-rebuild search must work on the new dim, hits={:?}",
        after.iter().map(|h| &h.id).collect::<Vec<_>>()
    );
    let intro = new_idx.search(&text_query("介绍")).unwrap();
    assert!(
        intro
            .iter()
            .any(|h| h.id == doc_id(&pid, "docs/intro.md", 0)),
        "both live files must be in the rebuilt index"
    );

    let live = LiveSet::load_for_project(&tmp, &pid).unwrap();
    assert_eq!(live.files.len(), 2);

    // Same model is a no-op (no extra embeds). The public wrapper is the CLI entry.
    let _public: fn(&Path, &str, &EmbedConfig) -> Result<(), VaneCliError> = rebuild_for_new_model;
    let noop = rebuild_for_new_model_with(&tmp, &pid, &new_cfg, &mock_embedder(8)).unwrap();
    assert_eq!(noop.embedded, 0);
    assert_eq!(noop.cas_hits, 0);
}

#[test]
fn failed_rebuild_leaves_old_db_queryable() {
    let tmp = tempfile_dir();
    assert_isolated(&tmp);

    let (_root, pid, old_dim, old_model) = setup_indexed(&tmp);
    let old_idx = open_or_create(&tmp, &pid, old_dim, &old_model).unwrap();
    assert!(old_idx
        .search(&text_query("鉴权"))
        .unwrap()
        .iter()
        .any(|h| h.id == doc_id(&pid, "docs/auth.md", 0)));

    let calls = Arc::new(Mutex::new(Vec::new()));
    let embedder = FailAfterEmbeds {
        dim: 8,
        ok_embeds: Mutex::new(1),
        calls,
    };
    let err = rebuild_for_new_model_with(&tmp, &pid, &embed_cfg("test-8"), &embedder).unwrap_err();
    assert!(
        err.message.contains("mid-rebuild") || err.message.contains("embed failed"),
        "expected mid-rebuild embed error, got {}",
        err.message
    );

    let mid = old_idx.search(&text_query("鉴权")).unwrap();
    assert!(
        mid.iter().any(|h| h.id == doc_id(&pid, "docs/auth.md", 0)),
        "old db must stay queryable after a failed rebuild, hits={:?}",
        mid.iter().map(|h| &h.id).collect::<Vec<_>>()
    );
    drop(old_idx);

    let state = ProjectState::load(&state_path(&tmp, &pid)).unwrap();
    assert_eq!(state.dim, Some(old_dim), "failed rebuild must not swap dim");
    assert_eq!(state.embed_model_id.as_deref(), Some(old_model.as_str()));
    assert!(
        state
            .reindex_error
            .as_deref()
            .is_some_and(|m| m.contains("embed") || m.contains("mid-rebuild")),
        "state.reindex_error must record the failure, got {:?}",
        state.reindex_error
    );
    assert!(
        project_db_path(&tmp, &pid).exists(),
        "old db/ must remain after a failed rebuild"
    );

    let reopened = open_or_create(&tmp, &pid, old_dim, &old_model).unwrap();
    let after = reopened.search(&text_query("介绍")).unwrap();
    assert!(
        after
            .iter()
            .any(|h| h.id == doc_id(&pid, "docs/intro.md", 0)),
        "reopened old db must still search after failed rebuild"
    );
}
