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
        first_root: None,
        exclude: vec!["**/node_modules/**".into(), "**/secret/**".into()],
        images: false,
        install_service: false,
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
