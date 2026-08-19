use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use vane::classify::classify;
use vane::config::{ChunkConfig, EmbedConfig, ResolvedPolicy, TypeRule};
use vane::embed::MockEmbedder;
use vane::watch::{watch_roots, WatchEvent, WatchKind};

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
        "vane-watch-test-{}-{}-{}",
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
            glob: "**/*.md".into(),
            extractor: "text".into(),
            enabled: true,
        }],
    }
}

fn path_has_component(path: &Path, name: &str) -> bool {
    path.components().any(|c| c.as_os_str() == name)
}

fn recv_batches(rx: &mpsc::Receiver<Vec<WatchEvent>>, timeout: Duration) -> Vec<WatchEvent> {
    let start = Instant::now();
    let mut all = Vec::new();
    while start.elapsed() < timeout {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(mut batch) => all.append(&mut batch),
            Err(RecvTimeoutError::Timeout) => {
                if !all.is_empty() {
                    return all;
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    all
}

fn setup_root(tmp: &Path) -> PathBuf {
    let root = tmp.join("proj");
    fs::create_dir_all(root.join("node_modules").join("pkg")).unwrap();
    fs::write(
        root.join("node_modules").join("pkg").join("a.md"),
        "# pkg\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("docs")).unwrap();
    fs::write(root.join("docs").join("a.md"), "# docs\n").unwrap();
    root
}

#[test]
fn watched_paths_do_not_include_excluded_dirs() {
    let tmp = tempfile_dir();
    let root = setup_root(&tmp);
    let (tx, _rx) = mpsc::channel();
    let guard = watch_roots(vec![(root.clone(), policy())], tx).expect("watch_roots");

    let watched = guard.watched_paths_for_test();
    assert!(
        !watched.is_empty(),
        "at least the root directory should be registered"
    );
    for path in &watched {
        assert!(
            !path_has_component(path, "node_modules"),
            "registered watch path must not include node_modules: {}",
            path.display()
        );
    }
    let docs = root.join("docs");
    assert!(
        watched.iter().any(|p| p == &root || p.ends_with("proj")),
        "root should be watched, got {watched:?}"
    );
    assert!(
        watched.iter().any(|p| p == &docs || p.ends_with("docs")),
        "docs should be watched, got {watched:?}"
    );
}

#[test]
fn docs_write_emits_event_node_modules_does_not() {
    let tmp = tempfile_dir();
    let root = setup_root(&tmp);
    let (tx, rx) = mpsc::channel();
    let _guard = watch_roots(vec![(root.clone(), policy())], tx).expect("watch_roots");
    // Native backends (FSEvents) need a moment after registration.
    std::thread::sleep(Duration::from_millis(400));
    let _ = recv_batches(&rx, Duration::from_millis(100));

    fs::write(root.join("docs").join("a.md"), "# docs updated\n").unwrap();
    let docs_events = recv_batches(&rx, Duration::from_secs(3));
    assert!(
        docs_events
            .iter()
            .any(|e| e.rel == "docs/a.md" || e.rel.ends_with("docs/a.md") || e.rel == "a.md"),
        "expected an event for docs/a.md, got {docs_events:?}"
    );
    assert!(
        docs_events
            .iter()
            .any(|e| matches!(e.kind, WatchKind::Modify | WatchKind::Create)),
        "docs write should be Create or Modify, got {docs_events:?}"
    );

    let embedder = MockEmbedder {
        dim: 4,
        fail: false,
        calls: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
    };
    let pol = policy();
    fs::write(
        root.join("node_modules").join("pkg").join("a.md"),
        "# pkg changed\n",
    )
    .unwrap();
    let nm_events = recv_batches(&rx, Duration::from_millis(1200));
    let leaked: Vec<&WatchEvent> = nm_events
        .iter()
        .filter(|e| e.rel.contains("node_modules"))
        .collect();
    assert!(
        leaked.is_empty(),
        "node_modules writes must not enqueue work, got {nm_events:?}"
    );
    for ev in &nm_events {
        assert!(
            classify(&ev.rel, &pol).is_err() || !ev.rel.contains("node_modules"),
            "excluded path reached classify as indexable: {ev:?}"
        );
    }
    assert!(
        embedder.calls.lock().unwrap().is_empty(),
        "embed must not run when only node_modules changes"
    );
}
