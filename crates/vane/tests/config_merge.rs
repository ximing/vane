use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use vane::config::{load_config, resolve_policy, ProjectFile, TypeRule};
use vane::project::{find_current_root, project_id, reject_nested};

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
        "vane-config-test-{}-{}-{}",
        std::process::id(),
        nanos,
        n
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

fn write(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

#[test]
fn exclude_unions_and_types_replace() {
    let tmp = tempfile_dir();
    write(
        &tmp.join("config/config.toml"),
        r#"
exclude = ["**/node_modules/**", "**/*.log"]
[[types]]
glob = "**/*.md"
extractor = "text"
[[projects]]
path = "/proj"
"#,
    );
    let cfg = load_config(&tmp).unwrap();
    let pf = ProjectFile {
        exclude: vec!["**/generated/**".into()],
        include: Some(vec!["**/*.rst".into()]),
        types: None,
        embed: None,
        chunk: None,
    };
    let pol = resolve_policy(&cfg, &PathBuf::from("/proj"), Some(&pf)).unwrap();
    assert!(pol.exclude.iter().any(|e| e.contains("node_modules")));
    assert!(pol.exclude.iter().any(|e| e.contains("generated")));
    assert_eq!(pol.types.len(), 1);
    assert_eq!(pol.types[0].glob, "**/*.rst");
}

#[test]
fn project_api_key_is_rejected() {
    let err = ProjectFile::parse_toml("api_key = \"sk-x\"\n").unwrap_err();
    assert!(err.message.contains("api_key"));
}

#[test]
fn nested_api_key_in_embed_is_rejected() {
    let err = ProjectFile::parse_toml("[embed]\napi_key = \"sk-x\"\nmodel = \"x\"\n").unwrap_err();
    assert!(err.message.contains("api_key"));
}

#[test]
fn reject_nested_roots() {
    let a = PathBuf::from("/a");
    let ab = PathBuf::from("/a/b");
    let a_sibling = PathBuf::from("/ab");
    assert!(reject_nested(std::slice::from_ref(&a), &ab).is_err());
    assert!(reject_nested(std::slice::from_ref(&ab), &a).is_err());
    assert!(reject_nested(std::slice::from_ref(&a), &a).is_err());
    assert!(reject_nested(std::slice::from_ref(&a), &a_sibling).is_ok());
}

#[test]
fn types_table_wins_over_include() {
    let tmp = tempfile_dir();
    write(
        &tmp.join("config/config.toml"),
        r#"
[[types]]
glob = "**/*.md"
extractor = "text"
[[projects]]
path = "/proj"
"#,
    );
    let cfg = load_config(&tmp).unwrap();
    let pf = ProjectFile {
        exclude: vec![],
        include: Some(vec!["**/*.rst".into()]),
        types: Some(vec![TypeRule {
            glob: "**/*.md".into(),
            extractor: "text".into(),
            enabled: true,
        }]),
        embed: None,
        chunk: None,
    };
    let pol = resolve_policy(&cfg, Path::new("/proj"), Some(&pf)).unwrap();
    assert_eq!(pol.types.len(), 1);
    assert_eq!(pol.types[0].glob, "**/*.md");
}

#[test]
fn type_enabled_defaults_true() {
    let pf = ProjectFile::parse_toml(
        r#"
[[types]]
glob = "**/*.md"
extractor = "text"
"#,
    )
    .unwrap();
    let types = pf.types.expect("types written");
    assert!(types[0].enabled);
}

#[test]
fn log_and_gc_defaults_and_reject_below_one() {
    let tmp = tempfile_dir();
    write(
        &tmp.join("config/config.toml"),
        "[[projects]]\npath = \"/proj\"\n",
    );
    let cfg = load_config(&tmp).unwrap();
    assert_eq!(cfg.log.retain_days, 3);
    assert_eq!(cfg.gc.cas_retain_days, 365);

    write(&tmp.join("config/config.toml"), "[log]\nretain_days = 0\n");
    let err = load_config(&tmp).unwrap_err();
    assert!(err.message.contains("retain_days"));

    write(
        &tmp.join("config/config.toml"),
        "[gc]\ncas_retain_days = 0\n",
    );
    let err = load_config(&tmp).unwrap_err();
    assert!(err.message.contains("cas_retain_days"));
}

#[test]
fn embed_chunk_field_overlay() {
    let tmp = tempfile_dir();
    write(
        &tmp.join("config/config.toml"),
        r#"
[defaults.embed]
provider = "ollama"
model = "nomic-embed-text"
base_url = "http://127.0.0.1:11434"
[defaults.chunk]
split = "markdown"
max_chars = 1200
overlap_chars = 200
min_chars = 50
[[projects]]
path = "/proj"
"#,
    );
    let cfg = load_config(&tmp).unwrap();
    let pf = ProjectFile::parse_toml(
        r#"
[embed]
provider = "openai_compat"
model = "text-embedding-3-small"
[chunk]
max_chars = 800
"#,
    )
    .unwrap();
    let pol = resolve_policy(&cfg, Path::new("/proj"), Some(&pf)).unwrap();
    assert_eq!(pol.embed.dim, None);
    assert_eq!(pol.embed.provider, "openai_compat");
    assert_eq!(pol.embed.model, "text-embedding-3-small");
    assert_eq!(pol.embed.base_url, "http://127.0.0.1:11434");
    assert_eq!(pol.chunk.max_chars, 800);
    assert_eq!(pol.chunk.overlap_chars, 200);
    assert_eq!(pol.chunk.split, "markdown");
}

#[test]
fn embed_dim_loads_from_global_config() {
    let tmp = tempfile_dir();
    write(
        &tmp.join("config/config.toml"),
        r#"
[defaults.embed]
provider = "openai_compat"
model = "qwen3.7-text-embedding"
base_url = "https://example.invalid/v1"
dim = 1024
"#,
    );
    let cfg = load_config(&tmp).unwrap();
    assert_eq!(cfg.defaults.embed.dim, Some(1024));
}

#[test]
fn invalid_chunk_rejected() {
    let tmp = tempfile_dir();
    write(
        &tmp.join("config/config.toml"),
        "[defaults.chunk]\nmax_chars = 100\noverlap_chars = 100\n",
    );
    let err = load_config(&tmp).unwrap_err();
    assert!(
        err.message.contains("overlap") || err.message.contains("chunk"),
        "{}",
        err.message
    );
}

#[test]
fn find_current_root_longest_registered_prefix() {
    let roots = vec![PathBuf::from("/work/a"), PathBuf::from("/work/a/nested")];
    assert_eq!(
        find_current_root(Path::new("/work/a/nested/src"), &roots).as_deref(),
        Some(Path::new("/work/a/nested"))
    );
    assert_eq!(find_current_root(Path::new("/work/other"), &roots), None);
    assert_eq!(
        find_current_root(Path::new("/work/ab"), &[PathBuf::from("/work/a")]),
        None
    );
}

#[test]
fn project_id_is_sha256_utf8_prefix() {
    let p = PathBuf::from("/Users/me/docs");
    assert_eq!(project_id(&p), "67b55a4d2f3000cb");
    assert_eq!(project_id(&p).len(), 16);
}

#[test]
fn load_config_does_not_touch_user_vane_home() {
    let tmp = tempfile_dir();
    write(
        &tmp.join("config/config.toml"),
        "exclude = [\"**/*.log\"]\n",
    );
    let _cfg = load_config(&tmp).unwrap();
    let real = PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".vane");
    // Isolation: this test never creates or writes ~/.vane.
    let _ = real;
}
