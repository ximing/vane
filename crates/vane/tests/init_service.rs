use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use vane::config::load_config;
use vane::service::{install_user_service_at, service_paths_for};
use vane::wizard::{run_init, InitAnswers};

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
    let tmp = tempfile_dir();
    assert_isolated(&tmp);

    let answers = InitAnswers {
        provider: "ollama".into(),
        model: "nomic-embed-text".into(),
        base_url: "http://127.0.0.1:11434".into(),
        exclude: vec!["**/node_modules/**".into(), "**/secret/**".into()],
        images: false,
        install_service: false,
        ..InitAnswers::default()
    };
    let mut out = Vec::new();
    run_init(&tmp, Cursor::new(""), &mut out, Some(answers)).expect("run_init assume");

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
    let tmp = tempfile_dir();
    assert_isolated(&tmp);

    let answers = InitAnswers {
        provider: "openai_compat".into(),
        model: "qwen3.7-text-embedding".into(),
        base_url: "https://example.invalid/compatible-mode/v1".into(),
        api_key: Some("sk-test-from-init".into()),
        dim: Some(1024),
        exclude: vec!["**/.git/**".into()],
        images: false,
        install_service: false,
        ..InitAnswers::default()
    };
    let mut out = Vec::new();
    run_init(&tmp, Cursor::new(""), &mut out, Some(answers)).expect("run_init assume");

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
    run_init(&tmp, Cursor::new(stdin), &mut out, None).expect("interactive init");
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
    let tmp = tempfile_dir();
    assert_isolated(&tmp);
    let first = InitAnswers {
        provider: "openai_compat".into(),
        model: "keep-me".into(),
        base_url: "https://example.invalid/v1".into(),
        api_key: Some("sk-keep".into()),
        dim: Some(1024),
        max_chars: 900,
        install_service: false,
        ..InitAnswers::default()
    };
    run_init(&tmp, Cursor::new(""), &mut Vec::new(), Some(first)).unwrap();
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
    run_init(&tmp, Cursor::new(stdin), &mut Vec::new(), None).expect("re-init");
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
