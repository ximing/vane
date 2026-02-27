use std::fs;
use std::io::{Cursor, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use vane::config::load_config;
use vane::service::{install_user_service_at, service_paths_for};
use vane::wizard::{run_init, InitAnswers};

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn serial_lock() -> std::sync::MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn with_embed_fail_allowed<T>(f: impl FnOnce() -> T) -> T {
    let prev = std::env::var("VANE_ALLOW_EMBED_FAIL").ok();
    std::env::set_var("VANE_ALLOW_EMBED_FAIL", "1");
    let out = f();
    match prev {
        Some(v) => std::env::set_var("VANE_ALLOW_EMBED_FAIL", v),
        None => std::env::remove_var("VANE_ALLOW_EMBED_FAIL"),
    }
    out
}

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
        "vi{}-{n}-{:x}",
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
}

#[test]
fn assume_writes_config_toml_with_chosen_exclude() {
    let _guard = serial_lock();
    let tmp = tempfile_dir();
    assert_isolated(&tmp);

    let answers = InitAnswers {
        provider: "ollama".into(),
        model: "nomic-embed-text".into(),
        base_url: "http://127.0.0.1:1".into(),
        exclude: vec!["**/node_modules/**".into(), "**/secret/**".into()],
        images: false,
        install_service: false,
        ..InitAnswers::default()
    };
    let mut out = Vec::new();
    with_embed_fail_allowed(|| {
        run_init(&tmp, Cursor::new(""), &mut out, Some(answers)).expect("run_init assume");
    });

    let cfg = load_config(&tmp).expect("load written config");
    assert!(
        cfg.exclude.iter().any(|e| e.contains("secret")),
        "chosen extra exclude should be written, got {:?}",
        cfg.exclude
    );
    assert!(
        cfg.exclude.iter().any(|e| e.contains("node_modules")),
        "chosen exclude should keep node_modules, got {:?}",
        cfg.exclude
    );
    assert_eq!(cfg.defaults.embed.provider, "ollama");
    assert_eq!(cfg.defaults.embed.model, "nomic-embed-text");
    let image = cfg
        .types
        .iter()
        .find(|t| t.extractor == "image")
        .expect("image type rule");
    assert!(
        !image.enabled,
        "images=false should leave image type disabled"
    );
    let raw = fs::read_to_string(tmp.join("config").join("config.toml")).expect("read config");
    assert!(
        !raw.contains("api_key"),
        "ollama init should not write api_key, got {raw}"
    );
}

#[test]
fn assume_openai_compat_writes_api_key_to_global_config() {
    let _guard = serial_lock();
    let tmp = tempfile_dir();
    assert_isolated(&tmp);

    let answers = InitAnswers {
        provider: "openai_compat".into(),
        model: "qwen3.7-text-embedding".into(),
        base_url: "http://127.0.0.1:1/compatible-mode/v1".into(),
        api_key: Some("sk-test-from-init".into()),
        dim: Some(1024),
        exclude: vec!["**/.git/**".into()],
        images: false,
        install_service: false,
        ..InitAnswers::default()
    };
    let mut out = Vec::new();
    with_embed_fail_allowed(|| {
        run_init(&tmp, Cursor::new(""), &mut out, Some(answers)).expect("run_init assume");
    });

    let cfg = load_config(&tmp).expect("load written config");
    assert_eq!(cfg.defaults.embed.provider, "openai_compat");
    assert_eq!(
        cfg.defaults.embed.api_key.as_deref(),
        Some("sk-test-from-init")
    );
    assert_eq!(cfg.defaults.embed.dim, Some(1024));
    let path = tmp.join("config").join("config.toml");
    let raw = fs::read_to_string(&path).expect("read config");
    assert!(
        raw.contains("api_key"),
        "global config should persist api_key, got {raw}"
    );
    assert!(
        raw.contains("dim = 1024"),
        "global config should persist dim, got {raw}"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&path)
            .expect("stat config")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(
            mode, 0o600,
            "config with a secret must be 0600, got {mode:o}"
        );
    }
}

#[test]
fn interactive_openai_compat_asks_for_api_key() {
    let _guard = serial_lock();
    let tmp = tempfile_dir();
    assert_isolated(&tmp);
    // Probe hits 127.0.0.1:1 (connection refused) so init does not wait on a real API.
    let stdin = [
        "openai_compat",
        "qwen3.7-text-embedding",
        "http://127.0.0.1:1/compatible-mode/v1",
        "sk-typed-in-wizard",
        "1024",
        "markdown",
        "800",
        "100",
        "40",
        "",
        "",
        "",
        "n",
        "n",
    ]
    .join("\n")
        + "\n";
    let mut out = Vec::new();
    with_embed_fail_allowed(|| {
        run_init(&tmp, Cursor::new(stdin), &mut out, None).expect("interactive init");
    });
    let printed = String::from_utf8_lossy(&out);
    assert!(
        printed.contains("API key"),
        "wizard must prompt for an API key, got {printed}"
    );
    let cfg = load_config(&tmp).expect("load written config");
    assert_eq!(
        cfg.defaults.embed.api_key.as_deref(),
        Some("sk-typed-in-wizard")
    );
    assert_eq!(cfg.defaults.embed.dim, Some(1024));
    assert!(
        printed.contains("Vector dimension"),
        "wizard must prompt for vector dimension, got {printed}"
    );
    assert_eq!(cfg.defaults.chunk.max_chars, 800);
    assert_eq!(cfg.defaults.chunk.overlap_chars, 100);
    assert_eq!(cfg.defaults.chunk.min_chars, 40);
}

#[test]
fn reinit_keeps_previous_when_answers_empty() {
    let _guard = serial_lock();
    let tmp = tempfile_dir();
    assert_isolated(&tmp);
    let first = InitAnswers {
        provider: "openai_compat".into(),
        model: "keep-me".into(),
        base_url: "http://127.0.0.1:1/v1".into(),
        api_key: Some("sk-keep".into()),
        dim: Some(1024),
        max_chars: 900,
        install_service: false,
        ..InitAnswers::default()
    };
    with_embed_fail_allowed(|| {
        run_init(&tmp, Cursor::new(""), &mut Vec::new(), Some(first)).unwrap();
    });
    let stdin = [
        "",  // provider default openai_compat
        "",  // model keep-me
        "",  // url
        "",  // api key keep
        "",  // dim 1024
        "",  // split
        "",  // max_chars 900
        "",  // overlap
        "",  // min
        "",  // first root
        "",  // uncheck
        "",  // extra
        "",  // images
        "n", // service
    ]
    .join("\n")
        + "\n";
    with_embed_fail_allowed(|| {
        run_init(&tmp, Cursor::new(stdin), &mut Vec::new(), None).expect("re-init");
    });
    let cfg = load_config(&tmp).unwrap();
    assert_eq!(cfg.defaults.embed.model, "keep-me");
    assert_eq!(cfg.defaults.embed.api_key.as_deref(), Some("sk-keep"));
    assert_eq!(cfg.defaults.embed.dim, Some(1024));
    assert_eq!(cfg.defaults.chunk.max_chars, 900);
}

#[test]
fn write_project_toml_has_chunk_not_api_key() {
    let tmp = tempfile_dir();
    assert_isolated(&tmp);
    let root = tmp.join("repo");
    fs::create_dir_all(&root).unwrap();
    let path = vane::wizard::write_project_toml(
        &root,
        &vane::config::ChunkConfig {
            split: "plain".into(),
            max_chars: 400,
            overlap_chars: 40,
            min_chars: 20,
        },
        false,
    )
    .unwrap();
    let body = fs::read_to_string(&path).unwrap();
    assert!(body.contains("max_chars = 400"), "{body}");
    assert!(!body.contains("api_key"), "{body}");
}

#[test]
fn missing_init_blocks_query() {
    let tmp = tempfile_dir();
    assert_isolated(&tmp);
    let bin = env!("CARGO_BIN_EXE_vane");
    let output = Command::new(bin)
        .args([
            "--home",
            tmp.to_str().expect("utf-8 home"),
            "query",
            "hello",
        ])
        .env("VANE_HOME", &*tmp)
        .env("HOME", fake_user_home(&tmp))
        .output()
        .expect("run vane query");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not initialized"),
        "query without init should mention missing init, got {stderr:?}"
    );
}

#[test]
fn uninstall_is_idempotent_if_plist_absent() {
    let tmp = tempfile_dir();
    assert_isolated(&tmp);
    let fake_home = fake_user_home(&tmp);
    let paths = service_paths_for(&fake_home);
    assert!(
        !paths.unit_path.exists(),
        "test must not see a pre-existing unit at {}",
        paths.unit_path.display()
    );

    uninstall_user_service_via_cli(&tmp, &fake_home);
    uninstall_user_service_via_cli(&tmp, &fake_home);
}

fn uninstall_user_service_via_cli(home: &Path, user_home: &Path) {
    let bin = env!("CARGO_BIN_EXE_vane");
    let output = Command::new(bin)
        .args([
            "--home",
            home.to_str().expect("utf-8 home"),
            "service",
            "uninstall",
        ])
        .env("VANE_HOME", home)
        .env("HOME", user_home)
        .env("XDG_CONFIG_HOME", user_home.join(".config"))
        .output()
        .expect("run vane service uninstall");
    assert!(
        output.status.success(),
        "uninstall should be idempotent if plist is absent, stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn install_writes_unit_under_fake_user_home() {
    let tmp = tempfile_dir();
    assert_isolated(&tmp);
    let user = fake_user_home(&tmp);
    let paths = service_paths_for(&user);
    let vane_bin = PathBuf::from("/opt/vane/bin/vane");
    install_user_service_at(&paths, &tmp, &vane_bin).expect("install");
    assert!(
        paths.unit_path.starts_with(&user),
        "unit must live under fake HOME, got {}",
        paths.unit_path.display()
    );
    let body = fs::read_to_string(&paths.unit_path).expect("read unit");
    assert!(
        body.contains("/opt/vane/bin/vane"),
        "unit should reference vane bin, got {body}"
    );
    assert!(
        body.contains("daemon"),
        "unit should launch daemon, got {body}"
    );
    assert!(
        body.contains(tmp.to_str().unwrap()),
        "unit should pass --home, got {body}"
    );

    vane::service::uninstall_user_service_at(&paths).expect("uninstall once");
    assert!(!paths.unit_path.exists());
    vane::service::uninstall_user_service_at(&paths).expect("uninstall again");
}

#[test]
fn init_success_next_steps_card_lists_query_start_and_mcp() {
    let _guard = serial_lock();
    let tmp = tempfile_dir();
    assert_isolated(&tmp);
    let answers = InitAnswers {
        provider: "ollama".into(),
        model: "nomic-embed-text".into(),
        base_url: "http://127.0.0.1:1".into(),
        install_service: false,
        ..InitAnswers::default()
    };
    with_embed_fail_allowed(|| {
        run_init(&tmp, Cursor::new(""), &mut Vec::new(), Some(answers)).expect("init success");
    });
    assert!(tmp.join("config").join("config.toml").is_file());

    let down = vane::wizard::next_steps_card(false);
    assert!(down.contains("Next steps"), "{down}");
    assert!(down.contains("vane start"), "{down}");
    assert!(down.contains("vane query"), "{down}");
    assert!(down.contains("vane mcp"), "{down}");
    assert!(down.contains("mcp install"), "{down}");
    assert!(down.contains("not running"), "{down}");

    let up = vane::wizard::next_steps_card(true);
    assert!(up.contains("daemon running"), "{up}");
    assert!(up.contains("vane query"), "{up}");
    assert!(up.contains("vane mcp"), "{up}");
    assert!(up.contains("mcp install"), "{up}");
}

fn spawn_http_status(status: u16, body: &str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind 401 server");
    let addr = listener.local_addr().expect("local_addr");
    let body = body.to_string();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            drain_http(&mut stream);
            let payload = body.clone();
            let resp = format!(
                "HTTP/1.1 {status} Denied\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                payload.len()
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
    });
    format!("http://{addr}/v1")
}

fn drain_http(stream: &mut TcpStream) {
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
}

#[test]
fn assume_init_401_without_env_fails() {
    let _guard = serial_lock();
    std::env::remove_var("VANE_ALLOW_EMBED_FAIL");
    let tmp = tempfile_dir();
    assert_isolated(&tmp);

    let base_url = spawn_http_status(
        401,
        r#"{"error":{"message":"Incorrect API key provided: sk-badtoken","type":"invalid_request_error"}}"#,
    );
    let answers = InitAnswers {
        provider: "openai_compat".into(),
        model: "text-embedding-3-small".into(),
        base_url,
        api_key: Some("sk-badtoken".into()),
        dim: None,
        install_service: false,
        ..InitAnswers::default()
    };
    let err = run_init(&tmp, Cursor::new(""), &mut Vec::new(), Some(answers))
        .expect_err("401 assume init must fail closed");
    assert!(
        err.message.contains("401") || err.message.to_ascii_lowercase().contains("embed"),
        "expected 401/embed error, got {}",
        err.message
    );
    assert!(
        !tmp.join("config").join("config.toml").is_file(),
        "fail-closed init must not write config.toml"
    );
    let last = fs::read_to_string(tmp.join("run").join("last_error.json"))
        .expect("last_error.json after failed probe");
    assert!(
        last.contains("401") || last.to_ascii_lowercase().contains("embed"),
        "last_error.json should record the probe failure, got {last}"
    );
    assert!(
        !last.contains("sk-badtoken"),
        "last_error.json must not contain the api key, got {last}"
    );
    let dumped = last.to_ascii_lowercase();
    assert!(
        !dumped.contains("api_key"),
        "must never persist api_key: {last}"
    );
}

#[test]
fn assume_init_401_with_env_continues() {
    let _guard = serial_lock();
    let tmp = tempfile_dir();
    assert_isolated(&tmp);

    let base_url = spawn_http_status(401, r#"{"error":"unauthorized"}"#);
    let answers = InitAnswers {
        provider: "openai_compat".into(),
        model: "text-embedding-3-small".into(),
        base_url,
        api_key: Some("sk-badtoken".into()),
        dim: None,
        install_service: false,
        ..InitAnswers::default()
    };
    with_embed_fail_allowed(|| {
        run_init(&tmp, Cursor::new(""), &mut Vec::new(), Some(answers))
            .expect("VANE_ALLOW_EMBED_FAIL=1 continues after 401");
    });
    assert!(
        tmp.join("config").join("config.toml").is_file(),
        "allow-fail init should still write config"
    );
}
