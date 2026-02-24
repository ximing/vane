use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use vane::cas::Cas;
use vane::config::load_config;
use vane::extract::CanonicalDoc;
use vane::gc::{gc_all, gc_project, gc_ttl, LiveKeySet};
use vane::index::{project_db_prev_path, project_dir, state_path, ProjectState};
use vane::live::{live_path, LiveFile, LiveSet};
use vane::project::project_id;

const DAY: u64 = 86_400;

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
        "vane-gc-test-{}-{}-{}",
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

fn write_config(home: &Path, roots: &[&Path]) {
    let cfg = home.join("config").join("config.toml");
    fs::create_dir_all(cfg.parent().unwrap()).unwrap();
    let mut body = String::from(
        r#"
[defaults.embed]
provider = "mock"
model = "test"
base_url = "http://127.0.0.1"

[gc]
cas_retain_days = 365
"#,
    );
    for root in roots {
        body.push_str(&format!("\n[[projects]]\npath = \"{}\"\n", root.display()));
    }
    fs::write(&cfg, body).unwrap();
}

fn sample_doc(path: &str, text: &str) -> CanonicalDoc {
    CanonicalDoc {
        text: text.into(),
        headings: Vec::new(),
        path: path.into(),
        chunk_index: 0,
        start_byte: 0,
        end_byte: text.len() as u64,
        modality: "text".into(),
        extractor: "text".into(),
    }
}

fn seed_cas(cas: &Cas, extract_key: &str, embed_key: &str, last_seen: u64) {
    let docs = vec![sample_doc("notes.md", "hello cas")];
    cas.put_extract(extract_key, &docs).unwrap();
    cas.put_embed(embed_key, &[0.1, 0.2, 0.3, 0.4]).unwrap();
    cas.touch(
        extract_key,
        std::slice::from_ref(&embed_key.to_string()),
        last_seen,
    );
}

fn write_live(home: &Path, project_id: &str, extract_key: Option<&str>) {
    let mut live = LiveSet::default();
    if let Some(ek) = extract_key {
        live.files.insert(
            "notes.md".into(),
            LiveFile {
                content_sha256: "abc".into(),
                extract_key: ek.to_string(),
                chunk_count: 1,
            },
        );
    }
    live.save_for_project(home, project_id).unwrap();
}

fn write_state(home: &Path, project_id: &str, root: &Path) {
    let state = ProjectState {
        root_path: Some(root.display().to_string()),
        embed_model_id: Some("mock:test:4".into()),
        dim: Some(4),
        ..ProjectState::default()
    };
    state.save_atomic(&state_path(home, project_id)).unwrap();
}

fn make_root(home: &Path, name: &str) -> PathBuf {
    let root = home.join("src").join(name);
    fs::create_dir_all(&root).unwrap();
    let src = root.join("notes.md");
    fs::write(&src, "hello source").unwrap();
    root.canonicalize().unwrap_or(root)
}

fn lives_from(extract_keys: &[&str]) -> LiveKeySet {
    let mut keys = LiveKeySet::default();
    for k in extract_keys {
        keys.insert_extract((*k).to_string());
    }
    keys
}

#[test]
fn gc_project_keeps_shared_extract_key() {
    let home = tempfile_dir();
    let root_a = make_root(&home, "a");
    let root_b = make_root(&home, "b");
    write_config(&home, &[&root_a, &root_b]);
    let pid_a = project_id(&root_a);
    let pid_b = project_id(&root_b);
    write_state(&home, &pid_a, &root_a);
    write_state(&home, &pid_b, &root_b);

    let cas = Cas::new(home.join("rag").join("cas"));
    let k = "shared-extract-k";
    let embed = "shared-embed-k";
    seed_cas(&cas, k, embed, 1_700_000_000);

    // A dropped the file; B still references K.
    write_live(&home, &pid_a, None);
    write_live(&home, &pid_b, Some(k));

    let report = gc_project(&home, &root_a, &lives_from(&[k]), &cas).unwrap();
    assert_eq!(report.extract_deleted, 0, "B still pins shared extract {k}");
    assert!(
        cas.get_extract(k).is_some(),
        "shared extract must survive gc_project(A)"
    );
    assert!(
        cas.get_embed(embed).is_some(),
        "shared embed must survive gc_project(A)"
    );
    assert!(root_a.join("notes.md").is_file(), "must not delete source");
    assert!(root_b.join("notes.md").is_file(), "must not delete source");
}

#[test]
fn gc_project_deletes_unreferenced_extract_immediately() {
    let home = tempfile_dir();
    let root_a = make_root(&home, "a");
    write_config(&home, &[&root_a]);
    let pid_a = project_id(&root_a);
    write_state(&home, &pid_a, &root_a);

    let cas = Cas::new(home.join("rag").join("cas"));
    let k = "lonely-extract-k";
    let embed = "lonely-embed-k";
    seed_cas(&cas, k, embed, 1_700_000_000);
    write_live(&home, &pid_a, None);

    let prev = project_db_prev_path(&home, &pid_a);
    fs::create_dir_all(&prev).unwrap();
    fs::write(prev.join("stale"), b"old").unwrap();

    let report = gc_project(&home, &root_a, &lives_from(&[]), &cas).unwrap();
    assert!(
        report.extract_deleted >= 1,
        "unreferenced extract must be deleted immediately"
    );
    assert!(
        cas.get_extract(k).is_none(),
        "A is the only live; K must go"
    );
    assert!(
        cas.get_embed(embed).is_none(),
        "cascade must drop recorded embed_keys"
    );
    assert!(!prev.exists(), "gc_project must drop db.prev");
    assert!(
        root_a.join("notes.md").is_file(),
        "source file must remain after gc"
    );
}

#[test]
fn gc_ttl_keeps_recent_and_live_deletes_expired() {
    let home = tempfile_dir();
    let cas = Cas::new(home.join("rag").join("cas"));
    let now = 2_000_000_000;

    seed_cas(&cas, "recent-k", "recent-e", now - 10 * DAY);
    seed_cas(&cas, "expired-k", "expired-e", now - 400 * DAY);
    seed_cas(&cas, "live-old-k", "live-old-e", now - 400 * DAY);

    let keep = gc_ttl(&cas, &lives_from(&[]), now, 365);
    assert!(
        cas.get_extract("recent-k").is_some(),
        "10-day-old unreferenced CAS must survive retain_days=365"
    );
    assert!(
        cas.get_extract("expired-k").is_none(),
        "400-day-old unreferenced CAS must be deleted"
    );
    assert!(
        cas.get_embed("expired-e").is_none(),
        "expired extract cascade deletes embed"
    );
    assert!(keep.extract_deleted >= 1);

    // Live set pins even a very old last_seen.
    seed_cas(&cas, "live-old-k", "live-old-e", now - 400 * DAY);
    let _ = gc_ttl(&cas, &lives_from(&["live-old-k"]), now, 365);
    assert!(
        cas.get_extract("live-old-k").is_some(),
        "TTL must skip keys present in live_keys"
    );
    assert!(cas.get_embed("live-old-e").is_some());
}

#[test]
fn gc_never_deletes_user_source_files() {
    let home = tempfile_dir();
    let root = make_root(&home, "docs");
    write_config(&home, &[&root]);
    let pid = project_id(&root);
    write_state(&home, &pid, &root);
    let src = root.join("notes.md");
    assert!(src.is_file());

    let cas = Cas::new(home.join("rag").join("cas"));
    seed_cas(&cas, "src-k", "src-e", 1);
    write_live(&home, &pid, None);

    let _ = gc_project(&home, &root, &lives_from(&[]), &cas).unwrap();
    assert!(
        src.is_file(),
        "gc must never delete files under the project source root"
    );
    assert_eq!(fs::read_to_string(&src).unwrap(), "hello source");
    assert!(
        !project_dir(&home, &pid).join("notes.md").exists(),
        "source is not under rag/"
    );
}

#[test]
fn cas_retain_days_zero_rejected_at_config_load() {
    let home = tempfile_dir();
    let cfg = home.join("config").join("config.toml");
    fs::create_dir_all(cfg.parent().unwrap()).unwrap();
    fs::write(&cfg, "[gc]\ncas_retain_days = 0\n").unwrap();
    let err = load_config(&home).unwrap_err();
    assert!(
        err.message.contains("cas_retain_days"),
        "got {}",
        err.message
    );
}

#[test]
fn gc_all_fail_closed_when_config_unreadable() {
    let home = tempfile_dir();
    let root = make_root(&home, "a");
    write_config(&home, &[&root]);
    let pid = project_id(&root);
    write_state(&home, &pid, &root);

    let cas = Cas::new(home.join("rag").join("cas"));
    let k = "pinned-extract-k";
    let embed = "pinned-embed-k";
    seed_cas(&cas, k, embed, 1_700_000_000);
    write_live(&home, &pid, Some(k));

    let leftover = project_dir(&home, "leftover-unregistered");
    fs::create_dir_all(&leftover).unwrap();
    fs::write(leftover.join("marker"), b"keep").unwrap();

    fs::write(home.join("config").join("config.toml"), "not toml [[[").unwrap();

    let err = gc_all(&home, &cas).expect_err("gc_all must fail closed if config cannot be read");
    assert!(
        !err.message.is_empty(),
        "fail-closed error should explain why live union was not built"
    );
    assert!(
        cas.get_extract(k).is_some(),
        "fail-open empty live would wipe every extract"
    );
    assert!(
        cas.get_embed(embed).is_some(),
        "fail-open empty live would wipe every embed"
    );
    assert!(
        project_dir(&home, &pid).exists(),
        "registered project dir must survive unreadable config"
    );
    assert!(
        leftover.join("marker").is_file(),
        "leftover project dir must survive unreadable config"
    );
}

#[test]
fn gc_all_fail_closed_when_one_live_json_unreadable() {
    let home = tempfile_dir();
    let root_a = make_root(&home, "a");
    let root_b = make_root(&home, "b");
    write_config(&home, &[&root_a, &root_b]);
    let pid_a = project_id(&root_a);
    let pid_b = project_id(&root_b);
    write_state(&home, &pid_a, &root_a);
    write_state(&home, &pid_b, &root_b);

    let cas = Cas::new(home.join("rag").join("cas"));
    let k = "shared-live-k";
    let embed = "shared-live-e";
    seed_cas(&cas, k, embed, 1_700_000_000);
    write_live(&home, &pid_a, Some(k));
    write_live(&home, &pid_b, Some(k));
    fs::write(live_path(&home, &pid_b), "{not-json").unwrap();

    let err =
        gc_all(&home, &cas).expect_err("gc_all must fail closed if one live.json cannot be read");
    assert!(
        err.message.contains("live.json") || err.message.contains("parse"),
        "got {}",
        err.message
    );
    assert!(
        cas.get_extract(k).is_some(),
        "partial live union must not wipe CAS"
    );
    assert!(cas.get_embed(embed).is_some());
    assert!(project_dir(&home, &pid_a).exists());
    assert!(project_dir(&home, &pid_b).exists());
}

#[test]
fn gc_project_fail_closed_when_config_unreadable() {
    let home = tempfile_dir();
    let root = make_root(&home, "docs");
    write_config(&home, &[&root]);
    let pid = project_id(&root);
    write_state(&home, &pid, &root);

    let cas = Cas::new(home.join("rag").join("cas"));
    let k = "project-extract-k";
    let embed = "project-embed-k";
    seed_cas(&cas, k, embed, 1_700_000_000);
    write_live(&home, &pid, Some(k));

    fs::write(home.join("config").join("config.toml"), "[[[bad").unwrap();

    gc_project(&home, &root, &lives_from(&[]), &cas)
        .expect_err("gc_project must fail closed if config cannot be read");
    assert!(
        project_dir(&home, &pid).exists(),
        "unreadable config must not treat a registered project as leftover"
    );
    assert!(
        cas.get_extract(k).is_some(),
        "must not sweep CAS when live union cannot be confirmed"
    );
    assert!(cas.get_embed(embed).is_some());
    assert!(root.join("notes.md").is_file());
}
