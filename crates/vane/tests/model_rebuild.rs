use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::json;
use vane::cas::Cas;
use vane::config::{ChunkConfig, EmbedConfig, ResolvedPolicy, TypeRule};
use vane::embed::{embed_model_id, serving_embed_config, Embedder, MockEmbedder};
use vane::error::VaneCliError;
use vane::index::{
    doc_id, open_existing, open_or_create, project_db_path, project_db_prev_path, state_path,
    swap_new_db, ProjectState,
};
use vane::ipc;
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
            dim: None,
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
        dim: None,
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
        dirty: None,
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
    assert_eq!(state.embed_base_url.as_deref(), Some("http://127.0.0.1"));
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

    let (root, pid, old_dim, old_model) = setup_indexed(&tmp);
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
        state.rebuild.is_none(),
        "failed rebuild must clear rebuild progress so status is not stuck, got {:?}",
        state.rebuild
    );
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

    // Leftover rebuild progress would make incremental sync refuse the old db.
    let cas = Cas::new(tmp.join("rag").join("cas"));
    let embedder = mock_embedder(old_dim);
    let mut ctx = SyncCtx {
        home: &tmp,
        project_id: &pid,
        cas: &cas,
        index: &reopened,
        embedder: &embedder,
        now: 1_700_000_001,
        dirty: None,
    };
    reconcile_project(&mut ctx, &root, &policy()).expect(
        "failed rebuild must not leave reconcile_project blocked with \"model rebuild required\"",
    );
}

/// Short home path so `$VANE_HOME/run/vane.sock` fits `sockaddr_un` on macOS.
fn tempfile_dir_sock() -> TempHome {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "vm{}-{n}-{:x}",
        std::process::id(),
        (nanos % 0xfff) as u16
    ));
    fs::create_dir_all(&path).unwrap();
    std::env::set_var("VANE_HOME", &path);
    TempHome { path }
}

fn fake_user_home(home: &Path) -> PathBuf {
    let fake = home.join("fake-user-home");
    fs::create_dir_all(&fake).unwrap();
    fake
}

struct FakeOllama {
    base_url: String,
    models: Arc<Mutex<Vec<String>>>,
}

impl FakeOllama {
    fn spawn() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake ollama");
        let addr = listener.local_addr().expect("local_addr");
        let models = Arc::new(Mutex::new(Vec::new()));
        let models_thread = Arc::clone(&models);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let seen = Arc::clone(&models_thread);
                thread::spawn(move || handle_fake_ollama(stream, &seen));
            }
        });
        Self {
            base_url: format!("http://{addr}"),
            models,
        }
    }

    fn models(&self) -> Vec<String> {
        self.models.lock().expect("models").clone()
    }
}

fn handle_fake_ollama(mut stream: TcpStream, seen: &Mutex<Vec<String>>) {
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(2))).ok();
    let mut buf = Vec::new();
    let mut tmp = [0u8; 2048];
    let headers_end;
    loop {
        let n = match stream.read(&mut tmp) {
            Ok(0) => return,
            Ok(n) => n,
            Err(_) => return,
        };
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            headers_end = pos + 4;
            break;
        }
        if buf.len() > 64 * 1024 {
            return;
        }
    }
    let header_text = String::from_utf8_lossy(&buf[..headers_end]);
    let content_len = header_text
        .lines()
        .find_map(|line| {
            let (k, v) = line.split_once(':')?;
            if k.eq_ignore_ascii_case("content-length") {
                v.trim().parse::<usize>().ok()
            } else {
                None
            }
        })
        .unwrap_or(0);
    while buf.len().saturating_sub(headers_end) < content_len {
        let n = match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };
        buf.extend_from_slice(&tmp[..n]);
    }
    let body = &buf[headers_end..buf.len().min(headers_end + content_len)];
    let model = serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("model")?.as_str().map(str::to_string))
        .unwrap_or_default();
    if let Ok(mut models) = seen.lock() {
        models.push(model.clone());
    }
    let dim = if model.contains("8") { 8 } else { 4 };
    let embedding: Vec<f32> = vec![0.1; dim];
    let payload = json!({ "embedding": embedding }).to_string();
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
        payload.len()
    );
    let _ = stream.write_all(resp.as_bytes());
    let _ = stream.flush();
}

fn write_daemon_config(home: &Path, project: &Path, base_url: &str) {
    let cfg = home.join("config").join("config.toml");
    fs::create_dir_all(cfg.parent().unwrap()).unwrap();
    fs::write(
        &cfg,
        format!(
            r#"
[defaults.embed]
provider = "ollama"
model = "test"
base_url = "{base_url}"

[[projects]]
path = "{}"
"#,
            project.display()
        ),
    )
    .unwrap();
}

fn write_serving_base_url(home: &Path, pid: &str, base_url: &str) {
    let path = state_path(home, pid);
    let mut state = ProjectState::load(&path).unwrap();
    state.embed_base_url = Some(base_url.to_string());
    state.save_atomic(&path).unwrap();
}

fn write_model_overlay(root: &Path, model: &str) {
    fs::write(
        root.join(".vane.toml"),
        format!("[embed]\nmodel = \"{model}\"\n"),
    )
    .unwrap();
}

struct DaemonProcess {
    child: Child,
}

impl Drop for DaemonProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn socket_path(home: &Path) -> PathBuf {
    home.join("run").join("vane.sock")
}

fn wait_for_socket(home: &Path, child: &mut Child, timeout: Duration) {
    let sock = socket_path(home);
    let start = Instant::now();
    while start.elapsed() < timeout {
        if let Some(status) = child.try_wait().unwrap() {
            let mut err = String::new();
            if let Some(mut pipe) = child.stderr.take() {
                let _ = pipe.read_to_string(&mut err);
            }
            panic!("daemon exited early {status}: {err}");
        }
        if std::os::unix::net::UnixStream::connect(&sock).is_ok() {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("daemon socket not ready at {}", sock.display());
}

fn spawn_daemon(home: &Path) -> DaemonProcess {
    let bin = env!("CARGO_BIN_EXE_vane");
    let fake_home = fake_user_home(home);
    let mut child = Command::new(bin)
        .args(["daemon", "--home", home.to_str().expect("utf-8 home")])
        .env("VANE_HOME", home)
        .env("HOME", &fake_home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn vane daemon");
    wait_for_socket(home, &mut child, Duration::from_secs(8));
    DaemonProcess { child }
}

fn daemon_search(home: &Path, root: &Path, query: &str) -> serde_json::Value {
    ipc::rpc_call(
        home,
        "search",
        json!({
            "query": query,
            "root": root.display().to_string(),
            "top_k": 8
        }),
    )
    .unwrap_or_else(|e| panic!("daemon search failed: {e}"))
}

fn assert_old_model_hybrid_hit(hits: &serde_json::Value, pid: &str, rel: &str) {
    let arr = hits
        .as_array()
        .unwrap_or_else(|| panic!("hits array, got {hits}"));
    let want = doc_id(pid, rel, 0);
    let hit = arr
        .iter()
        .find(|h| h["id"] == want)
        .unwrap_or_else(|| panic!("daemon search must return old-db hit {want}, hits={arr:?}"));
    assert_eq!(
        hit["degraded"], false,
        "daemon search must keep hybrid on the serving (old) model, not degrade to BM25 after overlay/dim mismatch, hit={hit}"
    );
}

#[test]
fn daemon_search_uses_old_model_while_rebuild_runs() {
    let tmp = tempfile_dir_sock();
    assert_isolated(&tmp);

    let (root, pid, _old_dim, _old_model) = setup_indexed(&tmp);
    let fake = FakeOllama::spawn();
    write_daemon_config(&tmp, &root, &fake.base_url);
    write_serving_base_url(&tmp, &pid, &fake.base_url);
    // Same order as `vane model`: persist the new overlay before swap.
    write_model_overlay(&root, "test-8");

    let _daemon = spawn_daemon(&tmp);

    let (started_tx, started_rx) = mpsc::channel();
    let (go_tx, go_rx) = mpsc::channel();
    let embedder = GateEmbedder {
        dim: 8,
        started: Mutex::new(Some(started_tx)),
        go: Mutex::new(Some(go_rx)),
        calls: Arc::new(Mutex::new(Vec::new())),
    };
    let home = tmp.path.clone();
    let pid_clone = pid.clone();
    let handle = thread::spawn(move || {
        rebuild_for_new_model_with(&home, &pid_clone, &embed_cfg("test-8"), &embedder)
    });
    started_rx
        .recv_timeout(Duration::from_secs(8))
        .expect("rebuild should reach embed while old db is still live");

    let hits = daemon_search(&tmp, &root, "鉴权");
    assert_old_model_hybrid_hit(&hits, &pid, "docs/auth.md");
    let searched = fake.models();
    assert!(
        searched.iter().any(|m| m == "test") && searched.iter().all(|m| m != "test-8"),
        "daemon search during rebuild must embed with the old serving model, saw {searched:?}"
    );

    go_tx.send(()).unwrap();
    handle.join().expect("rebuild thread").unwrap();
}

#[test]
fn daemon_search_keeps_old_model_after_failed_rebuild() {
    let tmp = tempfile_dir_sock();
    assert_isolated(&tmp);

    let (root, pid, old_dim, old_model) = setup_indexed(&tmp);
    let fake = FakeOllama::spawn();
    write_daemon_config(&tmp, &root, &fake.base_url);
    write_serving_base_url(&tmp, &pid, &fake.base_url);
    write_model_overlay(&root, "test-8");

    let embedder = FailAfterEmbeds {
        dim: 8,
        ok_embeds: Mutex::new(1),
        calls: Arc::new(Mutex::new(Vec::new())),
    };
    let err = rebuild_for_new_model_with(&tmp, &pid, &embed_cfg("test-8"), &embedder).unwrap_err();
    assert!(
        err.message.contains("mid-rebuild") || err.message.contains("embed failed"),
        "expected mid-rebuild embed error, got {}",
        err.message
    );

    let state = ProjectState::load(&state_path(&tmp, &pid)).unwrap();
    assert_eq!(state.dim, Some(old_dim));
    assert_eq!(state.embed_model_id.as_deref(), Some(old_model.as_str()));
    assert!(state.rebuild.is_none());
    assert!(state.reindex_error.is_some());

    let _daemon = spawn_daemon(&tmp);
    let listed = ipc::rpc_call(&tmp, "list_roots", json!({})).unwrap();
    let root_st = &listed["roots"][0];
    assert_eq!(
        root_st["rebuilding"], false,
        "status must not stay rebuilding after a failed rebuild, got {root_st}"
    );
    assert!(
        root_st["reindex_error"]
            .as_str()
            .is_some_and(|m| !m.is_empty()),
        "status must surface reindex_error, got {root_st}"
    );

    let hits = daemon_search(&tmp, &root, "鉴权");
    assert_old_model_hybrid_hit(&hits, &pid, "docs/auth.md");
    let searched = fake.models();
    assert!(
        searched.iter().any(|m| m == "test") && searched.iter().all(|m| m != "test-8"),
        "daemon search after failed rebuild must embed with the old serving model, saw {searched:?}"
    );
}

#[test]
fn open_for_read_does_not_create_db_during_swap() {
    let tmp = tempfile_dir();
    assert_isolated(&tmp);

    let (_root, pid, dim, model) = setup_indexed(&tmp);
    let db = project_db_path(&tmp, &pid);
    let prev = project_db_prev_path(&tmp, &pid);
    assert!(db.is_dir(), "setup must leave a real db/");
    fs::rename(&db, &prev).unwrap();
    assert!(!db.exists(), "precondition: db/ renamed to db.prev");

    let opened = open_existing(&tmp, &pid, dim, &model, false);
    assert!(
        !db.exists(),
        "query/open-for-read must not create_dir_all db/ while it is renamed to db.prev"
    );
    let idx = opened.expect("mid-swap read must open db.prev, not invent an empty db/");
    let hits = idx.search(&text_query("鉴权")).unwrap();
    assert!(
        hits.iter().any(|h| h.id == doc_id(&pid, "docs/auth.md", 0)),
        "search via db.prev must still return old hits, hits={:?}",
        hits.iter().map(|h| &h.id).collect::<Vec<_>>()
    );
}

#[test]
fn swap_new_db_leaves_prev_until_state_is_updated() {
    let tmp = tempfile_dir();
    assert_isolated(&tmp);

    let pid = "swap-order";
    let db = project_db_path(&tmp, pid);
    let new = project_dir(&tmp, pid).join("db.new");
    let prev = project_db_prev_path(&tmp, pid);
    fs::create_dir_all(db.join("old-marker")).unwrap();
    fs::create_dir_all(new.join("new-marker")).unwrap();

    swap_new_db(&tmp, pid).unwrap();

    assert!(
        db.join("new-marker").is_dir(),
        "db.new/ must become db/ on swap"
    );
    assert!(
        prev.join("old-marker").is_dir(),
        "spec §7.4: keep db.prev until state.json records the new embed_model_id/dim"
    );
    assert!(!new.exists(), "db.new/ must be gone after rename onto db/");
}

#[test]
fn serving_embed_config_restores_base_url() {
    let policy = EmbedConfig {
        provider: "openai_compat".into(),
        model: "new-model".into(),
        base_url: "http://new.example:8080".into(),
        api_key: None,
        dim: None,
    };
    let old_id = embed_model_id("ollama", "old-model", 4);
    let cfg = serving_embed_config(&policy, &old_id, Some("http://127.0.0.1:11434"));
    assert_eq!(cfg.provider, "ollama");
    assert_eq!(cfg.model, "old-model");
    assert_eq!(cfg.dim, Some(4));
    assert_eq!(
        cfg.base_url, "http://127.0.0.1:11434",
        "query embedding must keep the serving collection's base_url, not the new overlay"
    );
}
