use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn serial_lock() -> std::sync::MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

use serde_json::Value;

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
        "vdi{}-{n}-{:x}",
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

fn fake_user_home(home: &Path) -> PathBuf {
    let fake = home.join("fake-user-home");
    fs::create_dir_all(&fake).unwrap();
    fake
}

fn spawn_ollama(dim: usize) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake ollama");
    let addr = listener.local_addr().expect("local_addr");
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            handle_ollama(&mut stream, dim);
        }
    });
    format!("http://{addr}")
}

fn handle_ollama(stream: &mut TcpStream, dim: usize) {
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
    let embedding = vec!["0.1"; dim].join(",");
    let payload = format!(r#"{{"embedding":[{embedding}]}}"#);
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
        payload.len()
    );
    let _ = stream.write_all(resp.as_bytes());
    let _ = stream.flush();
}

const DOC: &str = "# 鉴权\n\n如何做登录鉴权，这是一段足够长的说明文字以便通过最小切片长度。\n";

fn write_config(home: &Path, project: &Path, base_url: &str) {
    let cfg = home.join("config").join("config.toml");
    fs::create_dir_all(cfg.parent().unwrap()).unwrap();
    fs::write(
        &cfg,
        format!(
            r#"
[defaults.embed]
provider = "ollama"
model = "nomic-embed-text"
base_url = "{base_url}"

[[projects]]
path = "{}"
"#,
            project.display()
        ),
    )
    .unwrap();
}

fn spawn_daemon(home: &Path) -> DaemonProcess {
    let bin = env!("CARGO_BIN_EXE_vane");
    let mut child = Command::new(bin)
        .args(["daemon", "--home", home.to_str().expect("utf-8 home")])
        .env("VANE_HOME", home)
        .env("HOME", fake_user_home(home))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn vane daemon");
    let sock = socket_path(home);
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(8) {
        if let Some(status) = child.try_wait().unwrap() {
            let mut err = String::new();
            if let Some(mut pipe) = child.stderr.take() {
                let _ = pipe.read_to_string(&mut err);
            }
            panic!("daemon exited early {status}: {err}");
        }
        if UnixStream::connect(&sock).is_ok() {
            return DaemonProcess { child };
        }
        thread::sleep(Duration::from_millis(20));
    }
    let _ = child.kill();
    let _ = child.wait();
    panic!("daemon socket not ready");
}

fn rpc(home: &Path, method: &str, params: Value) -> Value {
    let mut stream = UnixStream::connect(socket_path(home)).expect("connect sock");
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    let req = serde_json::json!({"id": "1", "method": method, "params": params});
    writeln!(stream, "{req}").unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap_or_else(|_| panic!("rpc json: {line}"))
}

fn search_hits(home: &Path, query: &str) -> Vec<Value> {
    let v = rpc(
        home,
        "search",
        serde_json::json!({"query": query, "top_k": 8}),
    );
    assert!(
        v.get("error").is_none() || v["error"].is_null(),
        "search error: {v}"
    );
    v["result"].as_array().cloned().unwrap_or_default()
}

fn wait_hits(home: &Path, query: &str, timeout: Duration) -> Vec<Value> {
    let start = Instant::now();
    let mut last = Vec::new();
    while start.elapsed() < timeout {
        last = search_hits(home, query);
        if !last.is_empty() {
            return last;
        }
        thread::sleep(Duration::from_millis(100));
    }
    last
}

#[test]
fn daemon_startup_indexes_registered_markdown_and_search_hits() {
    let _serial = serial_lock();
    let tmp = tempfile_dir();
    assert!(tmp.starts_with(std::env::temp_dir()));
    let project = tmp.join("proj");
    fs::create_dir_all(project.join("docs")).unwrap();
    fs::write(project.join("docs/auth.md"), DOC).unwrap();
    let base = spawn_ollama(4);
    write_config(&tmp, &project, &base);
    let _daemon = spawn_daemon(&tmp);

    let hits = wait_hits(&tmp, "鉴权", Duration::from_secs(8));
    assert!(
        !hits.is_empty(),
        "startup reconcile must index docs/auth.md so search returns hits, got {hits:?}"
    );
    let path = hits[0]["path"].as_str().unwrap_or("");
    assert!(
        path.ends_with("docs/auth.md") || path == "docs/auth.md",
        "hit path {path}"
    );
}

#[test]
fn daemon_watch_indexes_new_file() {
    let _serial = serial_lock();
    let tmp = tempfile_dir();
    let project = tmp.join("proj");
    fs::create_dir_all(project.join("docs")).unwrap();
    fs::write(project.join("docs/auth.md"), DOC).unwrap();
    let base = spawn_ollama(4);
    write_config(&tmp, &project, &base);
    let _daemon = spawn_daemon(&tmp);
    let _ = wait_hits(&tmp, "鉴权", Duration::from_secs(8));

    fs::write(
        project.join("docs/release.md"),
        "# 发版\n\n发版检查清单需要足够长的一段正文才能通过最小切片。\n",
    )
    .unwrap();
    let hits = wait_hits(&tmp, "发版检查清单", Duration::from_secs(8));
    assert!(
        !hits.is_empty(),
        "watch batch must reconcile new markdown into the index, got {hits:?}"
    );
}

#[test]
fn daemon_skips_project_file_that_contains_api_key() {
    let _serial = serial_lock();
    let tmp = tempfile_dir();
    let project = tmp.join("proj");
    fs::create_dir_all(project.join("docs")).unwrap();
    fs::write(project.join("docs/auth.md"), DOC).unwrap();
    fs::write(
        project.join(".vane.toml"),
        "[embed]\napi_key = \"sk-secret\"\nmodel = \"nomic-embed-text\"\n",
    )
    .unwrap();
    let base = spawn_ollama(4);
    write_config(&tmp, &project, &base);
    let _daemon = spawn_daemon(&tmp);
    thread::sleep(Duration::from_millis(800));
    let hits = search_hits(&tmp, "鉴权");
    assert!(
        hits.is_empty(),
        "secret-bearing .vane.toml must not be loaded as global overlay; got {hits:?}"
    );
}
