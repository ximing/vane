use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use vane::cas::Cas;
use vane::config::{ChunkConfig, EmbedConfig, ResolvedPolicy, TypeRule};
use vane::embed::{embed_model_id, Embedder, MockEmbedder};
use vane::extract::CanonicalDoc;
use vane::index::{
    doc_id, index_doc, maybe_compact, open_existing, open_or_create, project_db_path,
    should_compact, ProjectIndex,
};
use vane::live::{live_path, LiveFile, LiveSet};
use vane::progress::{load_progress, load_skips, ProgressPhase, SkipFileReason};
use vane::project::project_id;
use vane::sync::{reconcile_project, SyncCtx};
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
        "vane-index-live-test-{}-{}-{}",
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

fn mock_embedder(dim: u32) -> MockEmbedder {
    MockEmbedder {
        dim,
        fail: false,
        calls: Arc::new(Mutex::new(Vec::new())),
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

fn chunk(path: &str, text: &str, idx: u32) -> CanonicalDoc {
    CanonicalDoc {
        text: text.into(),
        headings: Vec::new(),
        path: path.into(),
        chunk_index: idx,
        start_byte: 0,
        end_byte: text.len() as u64,
        modality: "text".into(),
        extractor: "text".into(),
    }
}

#[test]
fn live_set_roundtrip_atomic() {
    let tmp = tempfile_dir();
    let project = "projdeadbeef0001";
    let path = live_path(&tmp, project);

    let mut live = LiveSet::default();
    live.files.insert(
        "docs/auth.md".into(),
        LiveFile {
            content_sha256: "abc".into(),
            extract_key: "ek1".into(),
            chunk_count: 3,
        },
    );
    live.save_atomic(&path).unwrap();

    assert!(path.exists());
    assert!(!path.with_extension("json.tmp").exists());

    let loaded = LiveSet::load(&path).unwrap();
    assert_eq!(loaded.files.len(), 1);
    let f = loaded.files.get("docs/auth.md").unwrap();
    assert_eq!(f.content_sha256, "abc");
    assert_eq!(f.extract_key, "ek1");
    assert_eq!(f.chunk_count, 3);

    let missing = LiveSet::load(&tmp.join("no-such-live.json")).unwrap();
    assert!(missing.files.is_empty());
}

#[test]
fn doc_id_is_project_path_hash_chunk() {
    assert_eq!(
        doc_id("deadbeefcafebabe", "docs/auth.md", 2),
        "deadbeefcafebabe:docs/auth.md#2"
    );
}

#[test]
fn maybe_compact_thresholds() {
    assert!(should_compact(1000, 0, 0));
    assert!(should_compact(1000, 10_000, 1));
    assert!(should_compact(1, 10, 2));
    assert!(!should_compact(999, 10, 1));
    assert!(!should_compact(0, 0, 0));
    assert!(!should_compact(1, 0, 1));
}

#[test]
fn add_flush_text_search_then_delete() {
    let tmp = tempfile_dir();
    // Isolation: this test's home is a unique temp dir, never ~/.vane.
    // Do not re-read VANE_HOME here — other tests in this process also set it.
    assert!(tmp.starts_with(std::env::temp_dir()));
    assert!(!tmp.starts_with(dirs_home().join(".vane")));

    let project_id = "cafebabedeadbeef";
    let dim = 4;
    let model_id = embed_model_id("mock", "test", dim);
    let embedder = mock_embedder(dim);

    let idx = open_or_create(&tmp, project_id, dim, &model_id).unwrap();
    assert_eq!(idx.dim(), dim);
    assert_eq!(idx.model_id(), model_id);
    assert!(project_db_path(&tmp, project_id).exists());

    let docs = [
        chunk("docs/auth.md", "API > 鉴权\n如何做登录鉴权", 0),
        chunk("docs/intro.md", "欢迎使用本机文档检索", 0),
    ];
    let texts: Vec<String> = docs.iter().map(|d| d.text.clone()).collect();
    let vectors = embedder.embed(&texts).unwrap();
    let root = "/proj";
    let vane_docs: Vec<_> = docs
        .iter()
        .zip(vectors)
        .map(|(d, v)| index_doc(project_id, root, d, Some(v)))
        .collect();
    idx.add_docs(&vane_docs).unwrap();
    idx.flush().unwrap();

    let hits = idx.search(&text_query("鉴权")).unwrap();
    assert!(
        hits.iter()
            .any(|h| h.id == doc_id(project_id, "docs/auth.md", 0)),
        "text search should find the auth chunk, hits={:?}",
        hits.iter().map(|h| &h.id).collect::<Vec<_>>()
    );

    idx.flush().unwrap();
    drop(idx);
    let reopened = open_existing(&tmp, project_id, dim, &model_id, false)
        .expect("reopen after drop must see flushed db");
    let hits2 = reopened.search(&text_query("鉴权")).unwrap();
    assert!(
        hits2
            .iter()
            .any(|h| h.id == doc_id(project_id, "docs/auth.md", 0)),
        "reopened db must still find the auth chunk"
    );
    let gone_id = doc_id(project_id, "docs/auth.md", 0);
    reopened.delete_ids(std::slice::from_ref(&gone_id)).unwrap();
    reopened.flush().unwrap();
    let after = reopened.search(&text_query("鉴权")).unwrap();
    assert!(
        after.iter().all(|h| h.id != gone_id),
        "deleted id must not appear after flush, hits={:?}",
        after.iter().map(|h| &h.id).collect::<Vec<_>>()
    );

    maybe_compact(&reopened, 1, 1, 0).unwrap();

    drop(reopened);
    let reopened = open_or_create(&tmp, project_id, dim, &model_id).unwrap();
    let intro = reopened.search(&text_query("欢迎")).unwrap();
    assert!(
        intro
            .iter()
            .any(|h| h.id == doc_id(project_id, "docs/intro.md", 0)),
        "reopened index should still find surviving docs"
    );
}

fn dirs_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn mixed_policy() -> ResolvedPolicy {
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
        types: vec![
            TypeRule {
                glob: "**/*.{md,mdx,txt,rst,org,html}".into(),
                extractor: "text".into(),
                enabled: true,
            },
            TypeRule {
                glob: "**/*.pdf".into(),
                extractor: "pdf".into(),
                enabled: true,
            },
        ],
    }
}

#[test]
fn mixed_tree_records_skip_and_progress() {
    let tmp = tempfile_dir();
    assert!(tmp.starts_with(std::env::temp_dir()));
    assert!(!tmp.starts_with(dirs_home().join(".vane")));

    let root = tmp.join("proj");
    fs::create_dir_all(root.join("docs")).unwrap();
    fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
    let body = "# 鉴权\n\n如何做登录鉴权，这是一段足够长的说明文字以便通过最小切片长度。\n";
    fs::write(root.join("docs/auth.md"), body).unwrap();
    fs::write(root.join("docs/bad.txt"), [0xff, 0xfe, 0x00]).unwrap();
    fs::write(root.join("spec.pdf"), b"%PDF-1.4\n").unwrap();
    fs::write(
        root.join("node_modules/pkg/index.js"),
        b"module.exports=1;\n",
    )
    .unwrap();
    let root = root.canonicalize().unwrap();
    let pid = project_id(&root);
    let dim = 4;
    let model = embed_model_id("mock", "test", dim);
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

    let report = reconcile_project(&mut ctx, &root, &mixed_policy()).unwrap();
    assert!(
        report.scanned >= 1,
        "mixed tree should scan markdown: {report:?}"
    );
    assert_eq!(report.added, 1, "only auth.md should be added: {report:?}");
    assert!(
        report.skipped >= 2,
        "utf-8 + unsupported pdf must be skipped: {report:?}"
    );
    assert!(report.embedded >= 1, "auth.md should embed: {report:?}");

    assert!(root.join("docs/auth.md").is_file());
    assert!(root.join("docs/bad.txt").is_file());
    assert!(root.join("spec.pdf").is_file());
    assert!(root.join("node_modules/pkg/index.js").is_file());

    let progress_path = tmp.join("run").join("progress.json");
    assert!(
        progress_path.is_file(),
        "progress.json must be written under the test home"
    );
    assert!(progress_path.starts_with(&*tmp));
    assert!(!progress_path.starts_with(dirs_home().join(".vane")));
    let progress = load_progress(&tmp).expect("parse progress.json");
    assert_eq!(progress.phase, ProgressPhase::Idle);
    assert_eq!(progress.project_id, pid);
    assert!(progress.skipped >= 2);

    let log = load_skips(&tmp, &pid);
    assert!(
        log.files
            .iter()
            .any(|f| f.path == "docs/bad.txt" && f.reason == SkipFileReason::InvalidUtf8),
        "invalid utf-8 must be recorded: {:?}",
        log.files
    );
    assert!(
        log.files
            .iter()
            .any(|f| { f.path == "spec.pdf" && f.reason == SkipFileReason::ExtractorUnsupported }),
        "unsupported pdf must be recorded: {:?}",
        log.files
    );
    assert!(
        log.files.iter().all(|f| !f.path.contains("node_modules")),
        "excluded trees must not be dumped: {:?}",
        log.files
    );
}

// Touch ProjectIndex in the import so a missing type fails at compile time.
#[allow(dead_code)]
fn _keep_type(idx: &ProjectIndex) {
    let _ = idx.dim();
}
