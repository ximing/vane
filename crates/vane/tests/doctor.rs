use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use vane::doctor::{explain_empty_query, run as run_doctor, CheckLevel};
use vane::home::disk_stats;
use vane::index::{state_path, ProjectState};
use vane::live::{LiveFile, LiveSet};
use vane::project::project_id;

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
    let path = std::env::temp_dir().join(format!(
        "{prefix}{}-{n}-{:x}",
        std::process::id(),
        (nanos % 0xfff) as u16
    ));
    fs::create_dir_all(&path).unwrap();
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

fn fake_user_home(home: &Path) -> PathBuf {
    let fake = home.join("uh");
    fs::create_dir_all(&fake).unwrap();
    fake
}

fn assert_isolated(home: &Path) {
    assert!(
        home.starts_with(std::env::temp_dir()),
        "VANE_HOME must be under the process temp dir, got {}",
        home.display()
    );
    let real = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
        .join(".vane");
    assert!(
        !home.starts_with(&real),
        "test home must not be ~/.vane ({})",
        real.display()
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
    let mut perms = fs::metadata(&cfg).unwrap().permissions();
    perms.set_mode(0o600);
    fs::set_permissions(&cfg, perms).unwrap();
}

fn run_cli(home: &Path, cwd: &Path, args: &[&str]) -> (i32, String, String) {
    let bin = env!("CARGO_BIN_EXE_vane");
    let fake = fake_user_home(home);
    let output = Command::new(bin)
        .args(["--home", home.to_str().expect("utf-8 home")])
        .args(args)
        .current_dir(cwd)
        .env("VANE_HOME", home)
        .env("HOME", &fake)
        .env_remove("XDG_CONFIG_HOME")
        .output()
        .expect("run vane");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
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

fn spawn_daemon(home: &Path) -> DaemonProcess {
    let bin = env!("CARGO_BIN_EXE_vane");
    let fake = fake_user_home(home);
    let sock = home.join("run").join("vane.sock");
    let child = Command::new(bin)
        .args(["daemon", "--home", home.to_str().expect("utf-8 home")])
        .env("VANE_HOME", home)
        .env("HOME", &fake)
        .env_remove("XDG_CONFIG_HOME")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn vane daemon");
    let mut daemon = DaemonProcess { child };
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(8) {
        if let Some(status) = daemon.child.try_wait().unwrap() {
            let mut err = String::new();
            if let Some(mut pipe) = daemon.child.stderr.take() {
                use std::io::Read;
                let _ = pipe.read_to_string(&mut err);
            }
            panic!("daemon exited early {status}: {err}");
        }
        if std::os::unix::net::UnixStream::connect(&sock).is_ok() {
            return daemon;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("daemon socket not ready at {}", sock.display());
}

fn seed_live(home: &Path, root: &Path) {
    let pid = project_id(root);
    let mut live = LiveSet::default();
    live.files.insert(
        "docs/a.md".into(),
        LiveFile {
            content_sha256: "x".into(),
            extract_key: "k".into(),
            chunk_count: 1,
        },
    );
    live.save_for_project(home, &pid).unwrap();
    let state = ProjectState {
        root_path: Some(root.display().to_string()),
        last_reconcile: Some(1),
        ..ProjectState::default()
    };
    state.save_atomic(&state_path(home, &pid)).unwrap();
}

#[test]
fn doctor_missing_config_is_red_exit_one() {
    let tmp = tempfile_dir("vd");
    assert_isolated(&tmp);
    let (code, stdout, stderr) = run_cli(&tmp, &tmp, &["doctor"]);
    assert_eq!(
        code, 1,
        "doctor missing config must exit 1, stderr={stderr}"
    );
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("doctor stdout JSON");
    assert_eq!(v["ok"], false);
    let checks = v["checks"].as_array().expect("checks array");
    let config = checks
        .iter()
        .find(|c| c["id"] == "config")
        .expect("config check");
    assert_eq!(config["level"], "red");
    assert!(
        config["message"].as_str().unwrap_or("").contains("missing")
            || config["fix"].as_str().unwrap_or("").contains("init"),
        "config check should mention missing/init, got {config}"
    );
    assert!(
        checks.iter().any(|c| c["level"] == "red"),
        "expected at least one red check: {v}"
    );
}

#[test]
fn status_piped_json_parseable_when_daemon_down() {
    let tmp = tempfile_dir("vs");
    assert_isolated(&tmp);
    let project = tmp.join("proj");
    fs::create_dir_all(&project).unwrap();
    let project = project.canonicalize().unwrap();
    write_config(&tmp, &[&project]);
    seed_live(&tmp, &project);

    let (code, stdout, stderr) = run_cli(&tmp, &project, &["status"]);
    assert_eq!(code, 0, "status should succeed from disk, stderr={stderr}");
    assert!(
        !stdout.lines().next().unwrap_or("").starts_with('/'),
        "piped status must not prefix JSON with a home path, got {stdout:?}"
    );
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("status stdout JSON");
    assert_eq!(v["running"], false);
    assert!(v.get("home").is_some(), "status JSON needs home: {v}");
    let roots = v["roots"].as_array().expect("roots array");
    assert_eq!(roots.len(), 1);
    assert_eq!(PathBuf::from(roots[0]["path"].as_str().unwrap()), project);
    assert_eq!(roots[0]["live_files"], 1);
    assert!(v.get("dirty_queue_size").is_some());
    assert!(v.get("disk").is_some());
    assert!(v["disk"].get("home_bytes").is_some());
    assert!(v["disk"].get("cas_bytes").is_some());
    let dumped = stdout.to_ascii_lowercase();
    assert!(
        !dumped.contains("api_key"),
        "status must never print api_key: {stdout}"
    );
}

#[test]
fn query_empty_reason_unregistered_cwd() {
    let tmp = tempfile_dir("vq");
    assert_isolated(&tmp);
    let project = tmp.join("proj");
    fs::create_dir_all(project.join("docs")).unwrap();
    let project = project.canonicalize().unwrap();
    write_config(&tmp, &[&project]);
    let _daemon = spawn_daemon(&tmp);

    let (code, stdout, stderr) = run_cli(&tmp, &tmp, &["query", "nothing-here"]);
    assert_eq!(code, 0, "empty query should succeed, stderr={stderr}");
    let v: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("query stdout JSON array");
    assert!(
        v.as_array().is_some(),
        "piped empty query must be JSON array, got {stdout:?}"
    );
    assert!(v.as_array().unwrap().is_empty());
    assert!(
        stderr.contains("vane add") || stderr.contains("--root") || stderr.contains("--all"),
        "stderr should explain unregistered cwd, got {stderr:?}"
    );
}

#[test]
fn explain_empty_query_reason_order() {
    let tmp = tempfile_dir("ve");
    assert_isolated(&tmp);
    let cwd = tmp.to_path_buf();

    let why = explain_empty_query(&tmp, &cwd, "q", false, None);
    assert_eq!(why.id, "not_initialized");
    assert!(why.message.contains("vane init"));

    let proj = tmp.join("proj");
    fs::create_dir_all(&proj).unwrap();
    let proj = proj.canonicalize().unwrap();
    write_config(&tmp, &[&proj]);

    let why = explain_empty_query(&tmp, &cwd, "q", false, None);
    assert_eq!(why.id, "not_registered", "{}", why.message);
    assert!(
        why.message.contains("vane add") && why.message.contains("--root"),
        "{}",
        why.message
    );

    fs::create_dir_all(tmp.join("run")).unwrap();
    fs::write(
        tmp.join("run").join("progress.json"),
        r#"{"project_id":"p","root":"/x","phase":"embed","scanned":1,"total_estimate":4,"added":0,"embedded":0,"skipped":0,"updated_at":1}"#,
    )
    .unwrap();
    let why = explain_empty_query(&tmp, &proj, "q", false, None);
    assert_eq!(why.id, "still_indexing", "{}", why.message);
    assert!(why.message.contains("embed"), "{}", why.message);
    fs::remove_file(tmp.join("run").join("progress.json")).unwrap();

    let why = explain_empty_query(&tmp, &proj, "q", false, None);
    assert_eq!(
        why.id, "still_indexing",
        "live_files==0 and no last_reconcile"
    );

    let pid = project_id(&proj);
    let state = ProjectState {
        last_reconcile: Some(9),
        ..ProjectState::default()
    };
    state.save_atomic(&state_path(&tmp, &pid)).unwrap();
    seed_live(&tmp, &proj);
    fs::create_dir_all(tmp.join("run")).unwrap();
    fs::write(
        tmp.join("run").join("last_error.json"),
        r#"{"at":1,"message":"embed failed sk-testtoken"}"#,
    )
    .unwrap();
    let why = explain_empty_query(&tmp, &proj, "hello", false, None);
    assert_eq!(why.id, "embedder", "{}", why.message);
    fs::remove_file(tmp.join("run").join("last_error.json")).unwrap();

    let why = explain_empty_query(&tmp, &proj, "node_modules/left-pad/index.js", false, None);
    assert_eq!(why.id, "excluded", "{}", why.message);

    let other = tmp.join("other");
    fs::create_dir_all(&other).unwrap();
    let other = other.canonicalize().unwrap();
    write_config(&tmp, &[&proj, &other]);
    seed_live(&tmp, &other);
    let why = explain_empty_query(&tmp, &proj, "hello", false, None);
    assert_eq!(why.id, "wrong_root", "{}", why.message);

    let empty = tmp.join("empty");
    fs::create_dir_all(&empty).unwrap();
    let empty = empty.canonicalize().unwrap();
    write_config(&tmp, &[&empty]);
    let epid = project_id(&empty);
    ProjectState {
        last_reconcile: Some(3),
        ..ProjectState::default()
    }
    .save_atomic(&state_path(&tmp, &epid))
    .unwrap();
    let why = explain_empty_query(&tmp, &empty, "hello", false, None);
    assert_eq!(why.id, "empty_index", "{}", why.message);
}

#[test]
fn doctor_world_readable_config_is_red() {
    let tmp = tempfile_dir("vw");
    assert_isolated(&tmp);
    write_config(&tmp, &[]);
    let cfg = tmp.join("config").join("config.toml");
    let mut perms = fs::metadata(&cfg).unwrap().permissions();
    perms.set_mode(0o644);
    fs::set_permissions(&cfg, perms).unwrap();

    let report = run_doctor(&tmp);
    let mode = report
        .checks
        .iter()
        .find(|c| c.id == "config_mode")
        .expect("config_mode check");
    assert_eq!(mode.level, CheckLevel::Red);
    assert!(!report.ok);
}

#[test]
fn disk_stats_counts_cas_and_project_db() {
    let tmp = tempfile_dir("ds");
    assert_isolated(&tmp);
    fs::create_dir_all(tmp.join("config")).unwrap();
    fs::write(tmp.join("config").join("config.toml"), "abcd").unwrap();
    fs::create_dir_all(tmp.join("rag").join("cas")).unwrap();
    fs::write(tmp.join("rag").join("cas").join("blob"), "12345").unwrap();
    let db = tmp.join("rag").join("projects").join("abc123").join("db");
    fs::create_dir_all(&db).unwrap();
    fs::write(db.join("leaf"), "xyz").unwrap();

    let stats = disk_stats(&tmp);
    assert_eq!(stats.cas_bytes, 5);
    assert_eq!(stats.projects.len(), 1);
    assert_eq!(stats.projects[0].project_id, "abc123");
    assert_eq!(stats.projects[0].db_bytes, 3);
    assert!(
        stats.home_bytes >= 4 + 5 + 3,
        "home_bytes should include config+cas+db, got {}",
        stats.home_bytes
    );
}

#[test]
fn df_piped_json_lists_home_cas_and_project_db() {
    let tmp = tempfile_dir("df");
    assert_isolated(&tmp);
    write_config(&tmp, &[]);
    fs::create_dir_all(tmp.join("rag").join("cas")).unwrap();
    fs::write(tmp.join("rag").join("cas").join("blob"), "12345").unwrap();
    let db = tmp.join("rag").join("projects").join("abc123").join("db");
    fs::create_dir_all(&db).unwrap();
    fs::write(db.join("leaf"), "xyz").unwrap();

    let (code, stdout, stderr) = run_cli(&tmp, &tmp, &["df"]);
    assert_eq!(code, 0, "df should succeed, stderr={stderr}");
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("df JSON");
    assert_eq!(v["cas_bytes"], 5);
    let projects = v["projects"].as_array().expect("projects");
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0]["project_id"], "abc123");
    assert_eq!(projects[0]["db_bytes"], 3);
    assert!(v["home_bytes"].as_u64().unwrap() >= 5 + 3);
    assert_eq!(v["large"], false);
    assert!(
        !stdout.contains("api_key"),
        "df must never print api_key: {stdout}"
    );
}

#[test]
fn status_redacts_last_error_secrets() {
    let tmp = tempfile_dir("vr");
    assert_isolated(&tmp);
    write_config(&tmp, &[]);
    fs::create_dir_all(tmp.join("run")).unwrap();
    fs::write(
        tmp.join("run").join("last_error.json"),
        r#"{"at":1,"message":"401 sk-proj-secretvalue","api_key":"should-not-leak"}"#,
    )
    .unwrap();
    let v = vane::doctor::status_from_disk(&tmp, false);
    let dumped = v.to_string();
    assert!(!dumped.contains("sk-proj-secretvalue"), "{dumped}");
    assert!(!dumped.contains("should-not-leak"), "{dumped}");
    assert!(
        !dumped.contains("api_key"),
        "status last_error must drop api_key: {dumped}"
    );
}
