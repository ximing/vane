use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use vane::daemon::serve_forever;

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
    // Unix socket paths are capped (~104 bytes on macOS). Keep the home short.
    let path = std::env::temp_dir().join(format!(
        "vd{}-{n}-{:x}",
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

fn write_config(home: &Path, project: &Path) {
    let cfg = home.join("config").join("config.toml");
    fs::create_dir_all(cfg.parent().unwrap()).unwrap();
    fs::write(
        &cfg,
        format!(
            r#"
[defaults.embed]
provider = "ollama"
model = "nomic-embed-text"
base_url = "http://127.0.0.1:9"

[[projects]]
path = "{}"
"#,
            project.display()
        ),
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
                use std::io::Read;
                let _ = pipe.read_to_string(&mut err);
            }
            panic!("daemon exited early {status}: {err}");
        }
        if UnixStream::connect(&sock).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("daemon socket not ready at {}", sock.display());
}

fn fake_user_home(home: &Path) -> PathBuf {
    let fake = home.join("fake-user-home");
    fs::create_dir_all(&fake).unwrap();
    fake
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
    wait_for_socket(home, &mut child, Duration::from_secs(5));
    DaemonProcess { child }
}

#[test]
fn missing_config_status_exits_one() {
    let tmp = tempfile_dir();
    let bin = env!("CARGO_BIN_EXE_vane");
    let output = Command::new(bin)
        .args(["--home", tmp.to_str().expect("utf-8 home"), "status"])
        .env("VANE_HOME", &*tmp)
        .env("HOME", fake_user_home(&tmp))
        .output()
        .expect("run vane status");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not initialized"),
        "stderr should mention missing init, got {stderr:?}"
    );
}

#[test]
fn list_roots_json_roundtrip() {
    let tmp = tempfile_dir();
    let project = tmp.join("proj");
    fs::create_dir_all(&project).unwrap();
    write_config(&tmp, &project);
    let _daemon = spawn_daemon(&tmp);

    let mut stream = UnixStream::connect(socket_path(&tmp)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let req = serde_json::json!({"id": "1", "method": "list_roots"});
    writeln!(stream, "{req}").unwrap();

    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    let v: serde_json::Value = serde_json::from_str(&line).expect("json response");
    assert_eq!(v["id"], "1");
    assert!(
        v.get("error").is_none() || v["error"].is_null(),
        "list_roots should succeed, got {v}"
    );
    let roots = v["result"]["roots"].as_array().expect("result.roots array");
    assert_eq!(roots.len(), 1);
    assert_eq!(PathBuf::from(roots[0]["path"].as_str().unwrap()), project);
}

#[test]
fn second_daemon_fails_with_already_running() {
    let tmp = tempfile_dir();
    let project = tmp.join("proj");
    fs::create_dir_all(&project).unwrap();
    write_config(&tmp, &project);
    let _first = spawn_daemon(&tmp);

    // Same-process flock is not exclusive on some Unixes; the child holds
    // the OS lock. A second serve_forever in this process must still fail.
    let err = serve_forever(tmp.path.clone()).expect_err("second daemon must exit");
    assert!(
        err.message.contains("already running"),
        "expected already running, got {}",
        err.message
    );

    let bin = env!("CARGO_BIN_EXE_vane");
    let mut second = Command::new(bin)
        .args(["daemon", "--home", tmp.to_str().expect("utf-8 home")])
        .env("VANE_HOME", &*tmp)
        .env("HOME", fake_user_home(&tmp))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn second vane daemon");
    let start = Instant::now();
    let status = loop {
        if let Some(status) = second.try_wait().unwrap() {
            break status;
        }
        if start.elapsed() > Duration::from_secs(3) {
            let _ = second.kill();
            let _ = second.wait();
            panic!("second daemon did not exit");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(!status.success(), "second daemon must exit non-zero");
    let mut stderr = String::new();
    if let Some(mut pipe) = second.stderr.take() {
        use std::io::Read;
        let _ = pipe.read_to_string(&mut stderr);
    }
    assert!(
        stderr.contains("already running"),
        "second daemon stderr should mention already running, got {stderr:?}"
    );
}

#[test]
fn status_rpc_includes_additive_keys() {
    let tmp = tempfile_dir();
    let project = tmp.join("proj");
    fs::create_dir_all(&project).unwrap();
    write_config(&tmp, &project);
    let _daemon = spawn_daemon(&tmp);

    let mut stream = UnixStream::connect(socket_path(&tmp)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let req = serde_json::json!({"id": "1", "method": "status"});
    writeln!(stream, "{req}").unwrap();

    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    let v: serde_json::Value = serde_json::from_str(&line).expect("json response");
    assert_eq!(v["id"], "1");
    assert!(
        v.get("error").is_none() || v["error"].is_null(),
        "status should succeed, got {v}"
    );
    let result = &v["result"];
    assert_eq!(result["running"], true);
    assert!(result.get("home").is_some(), "{result}");
    assert!(
        result["roots"].is_array(),
        "status.roots must stay an array: {result}"
    );
    assert!(result
        .get("dirty_queue_size")
        .and_then(|x| x.as_u64())
        .is_some());
    assert!(result.get("disk").is_some());
    assert!(result["disk"].get("home_bytes").is_some());
    assert!(result["disk"].get("cas_bytes").is_some());
    assert!(result.get("last_error").is_some());
    let dumped = result.to_string();
    assert!(
        !dumped.contains("api_key"),
        "status must not leak api_key: {dumped}"
    );
}
