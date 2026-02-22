use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use vane::cas::Cas;
use vane::embed::{embed_model_id, Embedder, MockEmbedder};
use vane::extract::CanonicalDoc;
use vane::index::{doc_id, index_doc, open_or_create};
use vane::live::{LiveFile, LiveSet};
use vane::project::project_id;
use vane::rrf::rrf_merge;
use vane::search::{read_by_path, search_all, search_project, snippet, ProjectSearch, SearchHit};

struct TempHome {
    path: PathBuf,
}

fn tempfile_dir(prefix: &str) -> TempHome {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    // Unix socket paths are short on macOS; keep the home compact.
    let path = std::env::temp_dir().join(format!(
        "{prefix}{}-{n}-{:x}",
        std::process::id(),
        (nanos % 0xfff) as u16
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

fn fake_user_home(home: &Path) -> PathBuf {
    let fake = home.join("fake-user-home");
    fs::create_dir_all(&fake).unwrap();
    fake
}

fn mock(dim: u32, fail: bool) -> MockEmbedder {
    MockEmbedder {
        dim,
        fail,
        calls: Arc::new(Mutex::new(Vec::new())),
    }
}

fn chunk(path: &str, text: &str, headings: &[&str], idx: u32) -> CanonicalDoc {
    CanonicalDoc {
        text: text.into(),
        headings: headings.iter().map(|s| (*s).to_string()).collect(),
        path: path.into(),
        chunk_index: idx,
        start_byte: 0,
        end_byte: text.len() as u64,
        modality: "text".into(),
        extractor: "text".into(),
    }
}

fn put_live_and_cas(home: &Path, project_id: &str, docs: &[CanonicalDoc]) {
    let cas = Cas::new(home.join("rag").join("cas"));
    let mut live = LiveSet::default();
    let mut by_path: std::collections::BTreeMap<String, Vec<CanonicalDoc>> =
        std::collections::BTreeMap::new();
    for d in docs {
        by_path.entry(d.path.clone()).or_default().push(d.clone());
    }
    for (path, chunks) in by_path {
        let key = format!("ek-{project_id}-{path}");
        cas.put_extract(&key, &chunks).unwrap();
        live.files.insert(
            path,
            LiveFile {
                content_sha256: "x".into(),
                extract_key: key,
                chunk_count: chunks.len() as u32,
            },
        );
    }
    live.save_for_project(home, project_id).unwrap();
}

fn index_project(home: &Path, root: &Path, dim: u32, docs: &[CanonicalDoc]) -> (String, String) {
    let pid = project_id(root);
    let model = embed_model_id("mock", "test", dim);
    let embedder = mock(dim, false);
    let idx = open_or_create(home, &pid, dim, &model).unwrap();
    let texts: Vec<String> = docs.iter().map(|d| d.text.clone()).collect();
    let vectors = embedder.embed(&texts).unwrap();
    let root_s = root.to_string_lossy();
    let vane_docs: Vec<_> = docs
        .iter()
        .zip(vectors)
        .map(|(d, v)| index_doc(&pid, &root_s, d, Some(v)))
        .collect();
    idx.add_docs(&vane_docs).unwrap();
    idx.flush().unwrap();
    put_live_and_cas(home, &pid, docs);
    drop(idx);
    (pid, model)
}

#[test]
fn snippet_strips_breadcrumb_and_caps_240_scalars() {
    let body: String = (0..300).map(|_| '鉴').collect();
    let text = format!("API > 鉴权\n{body}");
    let snip = snippet(&text);
    assert!(
        !snip.contains("API > 鉴权"),
        "breadcrumb must be stripped: {snip}"
    );
    assert_eq!(snip.chars().count(), 240);
    assert!(snip.chars().all(|c| c == '鉴'));

    let plain = "没有面包屑的纯文本";
    assert_eq!(snippet(plain), plain);
}

#[test]
fn empty_index_returns_empty_hits() {
    let tmp = tempfile_dir("se");
    assert_isolated(&tmp);
    let root = tmp.join("empty");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let pid = project_id(&root);
    let dim = 4;
    let model = embed_model_id("mock", "test", dim);
    let idx = open_or_create(&tmp, &pid, dim, &model).unwrap();
    let cas = Cas::new(tmp.join("rag").join("cas"));
    let live = LiveSet::default();
    let embedder = mock(dim, false);
    let hits = search_project(
        &ProjectSearch {
            index: &idx,
            embedder: &embedder,
            cas: &cas,
            live: &live,
            root: root.to_str().unwrap(),
            extractor: None,
        },
        "anything",
        8,
    )
    .unwrap();
    assert!(hits.is_empty(), "empty index must return [], got {hits:?}");
}

#[test]
fn embed_fail_falls_back_to_bm25_degraded() {
    let tmp = tempfile_dir("se");
    assert_isolated(&tmp);
    let root = tmp.join("proj");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();

    let docs = [chunk(
        "docs/auth.md",
        "API > 鉴权\n如何做登录鉴权，这是一段足够长的说明文字以便检索命中。",
        &["API", "鉴权"],
        0,
    )];
    let (pid, model) = index_project(&tmp, &root, 4, &docs);
    let _ = model;

    let idx = open_or_create(&tmp, &pid, 4, &embed_model_id("mock", "test", 4)).unwrap();
    let cas = Cas::new(tmp.join("rag").join("cas"));
    let live = LiveSet::load_for_project(&tmp, &pid).unwrap();
    let failing = mock(4, true);

    let hits = search_project(
        &ProjectSearch {
            index: &idx,
            embedder: &failing,
            cas: &cas,
            live: &live,
            root: root.to_str().unwrap(),
            extractor: None,
        },
        "鉴权",
        8,
    )
    .unwrap();
    assert!(
        !hits.is_empty(),
        "BM25 fallback must still find the indexed chunk"
    );
    assert!(
        hits.iter().all(|h| h.degraded),
        "embed failure must mark hits degraded, got {hits:?}"
    );
    assert!(
        hits.iter()
            .any(|h| h.id == doc_id(&pid, "docs/auth.md", 0) && h.path == "docs/auth.md"),
        "expected auth.md hit, got {hits:?}"
    );
    let hit = hits
        .iter()
        .find(|h| h.path == "docs/auth.md")
        .expect("auth hit");
    assert!(!hit.snippet.contains("API > 鉴权"));
    assert!(hit.snippet.contains("登录鉴权"));
    assert_eq!(hit.title, "API > 鉴权");
}

#[test]
fn search_all_rrf_returns_hits_from_two_dims() {
    let tmp = tempfile_dir("se");
    assert_isolated(&tmp);

    let root_a = tmp.join("a");
    let root_b = tmp.join("b");
    fs::create_dir_all(&root_a).unwrap();
    fs::create_dir_all(&root_b).unwrap();
    let root_a = root_a.canonicalize().unwrap();
    let root_b = root_b.canonicalize().unwrap();

    let docs_a = [chunk(
        "docs/alpha.md",
        "跨项目检索文档 alpha-only-token 足够长的说明文字。",
        &[],
        0,
    )];
    let docs_b = [chunk(
        "docs/beta.md",
        "跨项目检索文档 beta-only-token 足够长的说明文字。",
        &[],
        0,
    )];
    let (pid_a, _) = index_project(&tmp, &root_a, 4, &docs_a);
    let (pid_b, _) = index_project(&tmp, &root_b, 8, &docs_b);

    let idx_a = open_or_create(&tmp, &pid_a, 4, &embed_model_id("mock", "test", 4)).unwrap();
    let idx_b = open_or_create(&tmp, &pid_b, 8, &embed_model_id("mock", "test", 8)).unwrap();
    let cas = Cas::new(tmp.join("rag").join("cas"));
    let live_a = LiveSet::load_for_project(&tmp, &pid_a).unwrap();
    let live_b = LiveSet::load_for_project(&tmp, &pid_b).unwrap();
    let emb_a = mock(4, false);
    let emb_b = mock(8, false);
    let root_a_s = root_a.to_string_lossy();
    let root_b_s = root_b.to_string_lossy();

    let hits = search_all(
        &[
            ProjectSearch {
                index: &idx_a,
                embedder: &emb_a,
                cas: &cas,
                live: &live_a,
                root: &root_a_s,
                extractor: None,
            },
            ProjectSearch {
                index: &idx_b,
                embedder: &emb_b,
                cas: &cas,
                live: &live_b,
                root: &root_b_s,
                extractor: None,
            },
        ],
        "检索文档",
        8,
    )
    .unwrap();

    assert!(
        hits.iter().any(|h| h.path == "docs/alpha.md"),
        "RRF --all must include project A, hits={hits:?}"
    );
    assert!(
        hits.iter().any(|h| h.path == "docs/beta.md"),
        "RRF --all must include project B, hits={hits:?}"
    );
}

#[test]
fn rrf_merge_sums_reciprocal_ranks_and_tiebreaks_by_id() {
    fn hit(id: &str, score: f32) -> SearchHit {
        SearchHit {
            id: id.into(),
            path: id.into(),
            root: "/r".into(),
            title: id.into(),
            snippet: String::new(),
            score,
            modality: "text".into(),
            extractor: "text".into(),
            degraded: false,
        }
    }
    let merged = rrf_merge(
        vec![
            vec![hit("a", 0.9), hit("b", 0.8)],
            vec![hit("b", 0.95), hit("c", 0.1)],
        ],
        60,
        10,
    );
    let ids: Vec<&str> = merged.iter().map(|h| h.id.as_str()).collect();
    assert_eq!(ids, vec!["b", "a", "c"]);
    let expected_b = 1.0 / (60.0 + 2.0) + 1.0 / (60.0 + 1.0);
    assert!((merged[0].score - expected_b).abs() < 1e-6);
}

#[test]
fn read_path_returns_all_chunks_ascending() {
    let tmp = tempfile_dir("se");
    assert_isolated(&tmp);
    let root = tmp.join("proj");
    fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let pid = project_id(&root);

    let docs = [
        chunk(
            "docs/long.md",
            "API > 一\n第一块正文足够长。",
            &["API", "一"],
            0,
        ),
        chunk(
            "docs/long.md",
            "API > 二\n第二块正文足够长。",
            &["API", "二"],
            1,
        ),
    ];
    put_live_and_cas(&tmp, &pid, &docs);
    let cas = Cas::new(tmp.join("rag").join("cas"));
    let live = LiveSet::load_for_project(&tmp, &pid).unwrap();

    let chunks = read_by_path(&cas, &live, &pid, &root, "docs/long.md").unwrap();
    assert_eq!(chunks.len(), 2, "path read must return every chunk");
    assert_eq!(chunks[0].chunk_index, 0);
    assert_eq!(chunks[1].chunk_index, 1);
    assert_eq!(chunks[0].id, doc_id(&pid, "docs/long.md", 0));
    assert_eq!(chunks[1].id, doc_id(&pid, "docs/long.md", 1));
    assert!(chunks[0].text.contains("第一块"));
    assert!(chunks[1].text.contains("第二块"));
    assert_eq!(chunks[0].headings, vec!["API", "一"]);
    assert_eq!(
        chunks[0].abs_path,
        root.join("docs/long.md").to_string_lossy()
    );
}

fn write_config(home: &Path, projects: &[&Path]) {
    let cfg = home.join("config").join("config.toml");
    fs::create_dir_all(cfg.parent().unwrap()).unwrap();
    let mut body = String::from(
        r#"
[defaults.embed]
provider = "ollama"
model = "nomic-embed-text"
base_url = "http://127.0.0.1:9"

"#,
    );
    for p in projects {
        body.push_str(&format!("[[projects]]\npath = \"{}\"\n\n", p.display()));
    }
    fs::write(&cfg, body).unwrap();
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
                use std::io::Read;
                let _ = pipe.read_to_string(&mut err);
            }
            panic!("daemon exited early {status}: {err}");
        }
        if std::os::unix::net::UnixStream::connect(&sock).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
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

fn run_query(home: &Path, cwd: &Path, args: &[&str]) -> (i32, String, String) {
    let bin = env!("CARGO_BIN_EXE_vane");
    let output = Command::new(bin)
        .args(["--home", home.to_str().expect("utf-8 home")])
        .args(args)
        .current_dir(cwd)
        .env("VANE_HOME", home)
        .env("HOME", fake_user_home(home))
        .output()
        .expect("run vane query");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn query_without_all_stays_in_current_project() {
    let tmp = tempfile_dir("sq");
    assert_isolated(&tmp);

    let root_a = tmp.join("pa");
    let root_b = tmp.join("pb");
    fs::create_dir_all(root_a.join("docs")).unwrap();
    fs::create_dir_all(root_b.join("docs")).unwrap();
    let root_a = root_a.canonicalize().unwrap();
    let root_b = root_b.canonicalize().unwrap();

    let docs_a = [chunk(
        "docs/alpha.md",
        "项目甲文档 alpha-only-token 足够长的说明文字以便检索。",
        &[],
        0,
    )];
    let docs_b = [chunk(
        "docs/beta.md",
        "项目乙文档 beta-only-token 足够长的说明文字以便检索。",
        &[],
        0,
    )];
    let _ = index_project(&tmp, &root_a, 4, &docs_a);
    let _ = index_project(&tmp, &root_b, 8, &docs_b);
    write_config(&tmp, &[&root_a, &root_b]);

    let _daemon = spawn_daemon(&tmp);

    let (code, stdout, stderr) = run_query(&tmp, &root_a, &["query", "beta-only-token"]);
    assert_eq!(
        code, 0,
        "query should succeed with empty-or-local hits, stderr={stderr}"
    );
    assert!(
        !stdout.contains("docs/beta.md") && !stdout.contains("beta-only-token"),
        "query without --all from project A must not return B-only docs: {stdout}"
    );

    let (code_all, stdout_all, stderr_all) =
        run_query(&tmp, &root_a, &["query", "--all", "beta-only-token"]);
    assert_eq!(
        code_all, 0,
        "query --all should succeed, stderr={stderr_all}"
    );
    assert!(
        stdout_all.contains("docs/beta.md") || stdout_all.contains("beta-only-token"),
        "query --all must include project B: {stdout_all}"
    );
}
