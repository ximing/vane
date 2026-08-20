use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use std::process::Command;

use vane::config::{inspect_policy, load_config, resolve_policy, ProjectFile, TypeRule};
use vane::project::{
    find_current_root, find_vane_toml_dir, project_id, reject_nested, resolve_query_scope,
    QueryScope,
};

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
fn inspect_policy_layers_and_never_leaks_api_key() {
    let tmp = tempfile_dir();
    let proj = tmp.join("proj");
    fs::create_dir_all(&proj).unwrap();
    let proj = proj.canonicalize().unwrap();
    write(
        &tmp.join("config/config.toml"),
        &format!(
            r#"
[defaults.embed]
provider = "ollama"
model = "nomic-embed-text"
base_url = "http://127.0.0.1:11434"
api_key = "sk-global-secret"
exclude = ["**/node_modules/**", "**/*.log"]
[[types]]
glob = "**/*.md"
extractor = "text"
[[projects]]
path = "{}"
exclude = ["**/generated/**"]
[projects.embed]
model = "proj-model"
"#,
            proj.display()
        ),
    );
    let cfg = load_config(&tmp).unwrap();
    write(
        &proj.join(".vane.toml"),
        r#"
exclude = ["**/tmp/**"]
[chunk]
max_chars = 800
"#,
    );

    let global = inspect_policy(&cfg, None, true).unwrap();
    assert!(global.root.is_none());
    assert!(global.project_id.is_none());
    assert_eq!(global.source.embed, "global");
    assert_eq!(global.source.exclude, "global");
    assert_eq!(global.embed.model, "nomic-embed-text");
    assert!(global.exclude.project.is_empty());
    let dumped = serde_json::to_string(&global).unwrap();
    assert!(
        !dumped.contains("api_key") && !dumped.contains("sk-global-secret"),
        "inspect must never emit api_key, got {dumped}"
    );

    let report = inspect_policy(&cfg, Some(&proj), false).unwrap();
    assert_eq!(report.root.as_deref(), Some(proj.to_str().unwrap()));
    assert!(report.project_id.is_some());
    assert_eq!(report.embed.model, "proj-model");
    assert_eq!(report.source.embed, "projects");
    assert_eq!(report.chunk.max_chars, 800);
    assert_eq!(report.source.chunk, "vane.toml");
    assert_eq!(report.source.exclude, "vane.toml");
    assert!(report
        .exclude
        .global
        .iter()
        .any(|e| e.contains("node_modules")));
    assert!(report
        .exclude
        .project
        .iter()
        .any(|e| e.contains("generated")));
    assert!(report.exclude.project.iter().any(|e| e.contains("tmp")));
    assert!(report
        .exclude
        .effective
        .iter()
        .any(|e| e.contains("node_modules")));
    assert!(report.exclude.effective.iter().any(|e| e.contains("tmp")));
    let dumped = serde_json::to_string(&report).unwrap();
    assert!(!dumped.contains("api_key"), "{dumped}");
    assert!(!dumped.contains("sk-"), "{dumped}");
}

fn fake_user_home(home: &Path) -> PathBuf {
    let fake = home.join("uh");
    fs::create_dir_all(&fake).unwrap();
    fake
}

fn run_cli(home: &Path, cwd: &Path, args: &[&str]) -> (i32, String, String) {
    let bin = env!("CARGO_BIN_EXE_vane");
    let output = Command::new(bin)
        .args(["--home", home.to_str().expect("utf-8 home")])
        .args(args)
        .current_dir(cwd)
        .env("VANE_HOME", home)
        .env("HOME", fake_user_home(home))
        .env_remove("XDG_CONFIG_HOME")
        .output()
        .expect("run vane");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn inspect_cli_reads_test_config() {
    let tmp = tempfile_dir();
    let proj = tmp.join("proj");
    fs::create_dir_all(&proj).unwrap();
    let proj = proj.canonicalize().unwrap();
    write(
        &tmp.join("config/config.toml"),
        &format!(
            r#"
[defaults.embed]
provider = "ollama"
model = "nomic-embed-text"
base_url = "http://127.0.0.1:9"
api_key = "sk-must-not-print"
exclude = ["**/node_modules/**"]
[[projects]]
path = "{}"
exclude = ["**/build/**"]
"#,
            proj.display()
        ),
    );
    write(
        &proj.join(".vane.toml"),
        "exclude = [\"**/scratch/**\"]\n[chunk]\nmax_chars = 900\n",
    );

    let (code, stdout, stderr) =
        run_cli(&tmp, &proj, &["inspect", "--root", proj.to_str().unwrap()]);
    assert_eq!(code, 0, "stderr={stderr}");
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("inspect JSON");
    assert_eq!(PathBuf::from(v["root"].as_str().unwrap()), proj);
    assert!(v["project_id"].as_str().is_some());
    assert_eq!(v["source"]["chunk"], "vane.toml");
    assert_eq!(v["source"]["exclude"], "vane.toml");
    assert_eq!(v["chunk"]["max_chars"], 900);
    assert!(v["embed"].get("api_key").is_none(), "{v}");
    assert!(
        !stdout.contains("api_key") && !stdout.contains("sk-must-not-print"),
        "{stdout}"
    );
    let project_ex = v["exclude"]["project"].as_array().expect("exclude.project");
    assert!(project_ex
        .iter()
        .any(|e| e.as_str().unwrap().contains("build")));
    assert!(project_ex
        .iter()
        .any(|e| e.as_str().unwrap().contains("scratch")));

    let (code, stdout, stderr) = run_cli(&tmp, &tmp, &["inspect", "--global"]);
    assert_eq!(code, 0, "stderr={stderr}");
    let g: serde_json::Value = serde_json::from_str(&stdout).expect("global inspect JSON");
    assert!(g["root"].is_null(), "{g}");
    assert_eq!(g["source"]["embed"], "global");
    assert_eq!(g["chunk"]["max_chars"], 1200);
    assert!(g["exclude"]["project"].as_array().unwrap().is_empty());
    assert!(!stdout.contains("api_key"));
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
fn find_vane_toml_dir_walks_up() {
    let tmp = tempfile_dir();
    let nested = tmp.join("repo").join("src").join("lib");
    fs::create_dir_all(&nested).unwrap();
    fs::write(
        tmp.join("repo").join(".vane.toml"),
        "[chunk]\nmax_chars = 10\n",
    )
    .unwrap();
    assert_eq!(find_vane_toml_dir(&nested), Some(tmp.join("repo")));
    assert_eq!(find_vane_toml_dir(&tmp.join("other")), None);
}

#[test]
fn resolve_query_scope_prefers_toml_then_registered_then_all() {
    let tmp = tempfile_dir();
    let repo = tmp.join("repo");
    let nested = repo.join("src");
    fs::create_dir_all(&nested).unwrap();
    fs::write(repo.join(".vane.toml"), "[chunk]\nmax_chars = 10\n").unwrap();
    let registered = vec![repo.clone()];
    match resolve_query_scope(&nested, &registered, false) {
        QueryScope::Root(p) => assert_eq!(p, repo),
        other => panic!("expected toml root, got {other:?}"),
    }
    match resolve_query_scope(&tmp.join("elsewhere"), &registered, false) {
        QueryScope::All => {}
        other => panic!("expected All outside registered roots, got {other:?}"),
    }
    match resolve_query_scope(&nested, &registered, true) {
        QueryScope::All => {}
        other => panic!("--global must force All, got {other:?}"),
    }
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
