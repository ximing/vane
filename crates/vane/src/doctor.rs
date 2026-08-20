use std::fs::{self, File};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{json, Value};

use crate::classify::{classify, SkipReason};
use crate::config::{load_config, resolve_policy, Config, ProjectFile};
use crate::dirty::{dirty_path, DirtyQueue};
use crate::embed::embedder_from_config;
use crate::error::VaneCliError;
use crate::home::disk_stats;
use crate::index::{state_path, ProjectState};
use crate::live::LiveSet;
use crate::project::{find_current_root, project_id};

const DISK_YELLOW_BYTES: u64 = 1 << 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckLevel {
    Green,
    Yellow,
    Red,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorCheck {
    pub id: String,
    pub level: CheckLevel,
    pub message: String,
    pub fix: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    pub ok: bool,
    pub checks: Vec<DoctorCheck>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmptyQueryWhy {
    pub id: &'static str,
    pub message: String,
}

pub fn run(home: &Path) -> DoctorReport {
    let mut checks = Vec::new();
    let cfg = check_config(home, &mut checks);
    check_socket(home, &mut checks);
    check_daemon(home, &mut checks);
    check_service(&mut checks);
    if let Some(cfg) = cfg.as_ref() {
        check_embed(cfg, &mut checks);
        check_roots(cfg, &mut checks);
    }
    check_disk(home, &mut checks);
    let ok = !checks.iter().any(|c| c.level == CheckLevel::Red);
    DoctorReport { ok, checks }
}

pub fn status_from_disk(home: &Path, running: bool) -> Value {
    let dirty = DirtyQueue::load(&dirty_path(home));
    let mut roots = Vec::new();
    if let Ok(cfg) = load_config(home) {
        for proj in &cfg.projects {
            roots.push(root_status_object(home, &proj.path, &dirty));
        }
    }
    json!({
        "home": home.display().to_string(),
        "roots": roots,
        "running": running,
        "dirty_queue_size": dirty.len() as u64,
        "last_error": last_error_value(home),
        "disk": disk_stats(home),
    })
}

pub fn enrich_status_roots(home: &Path, roots: &mut Value) {
    let dirty = DirtyQueue::load(&dirty_path(home));
    let Some(arr) = roots.as_array_mut() else {
        return;
    };
    for root in arr {
        let pid = root
            .get("project_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if pid.is_empty() {
            continue;
        }
        if let Ok(state) = ProjectState::load(&state_path(home, &pid)) {
            if let Some(err) = state.last_error.as_deref() {
                root["last_error"] = json!(redact_secrets(err));
            }
        }
        if let Some(n) = crate::progress::skip_count(home, &pid) {
            root["skip_count"] = json!(n);
        }
        root["dirty_queue_size"] = json!(dirty.len_for(&pid) as u64);
    }
}

pub fn last_error_value(home: &Path) -> Value {
    read_last_error(home).unwrap_or(Value::Null)
}

pub fn last_error_path(home: &Path) -> PathBuf {
    home.join("run").join("last_error.json")
}

/// Persist `$VANE_HOME/run/last_error.json` and optional `state.json` last_error.
/// Secrets are redacted. Does not write `reindex_error`.
pub fn write_last_error(
    home: &Path,
    message: &str,
    project_id: Option<&str>,
    code: Option<&str>,
) -> Result<(), VaneCliError> {
    let message = crate::log::redact_line(message);
    let mut body = json!({
        "at": crate::progress::unix_now(),
        "message": message,
    });
    if let Some(pid) = project_id {
        body["project_id"] = json!(pid);
    }
    if let Some(c) = code {
        if !c.is_empty() {
            body["code"] = json!(c);
        }
    }
    let path = last_error_path(home);
    let bytes = serde_json::to_vec_pretty(&body)
        .map_err(|e| VaneCliError::new(format!("serialize last_error.json: {e}")))?;
    atomic_write_last_error(&path, &bytes)?;
    if let Some(pid) = project_id {
        let sp = state_path(home, pid);
        let mut state = ProjectState::load(&sp)?;
        state.last_error = Some(message);
        state.save_atomic(&sp)?;
    }
    Ok(())
}

pub fn clear_project_last_error(home: &Path, project_id: &str) {
    let sp = state_path(home, project_id);
    if let Ok(mut state) = ProjectState::load(&sp) {
        if state.last_error.is_some() {
            state.last_error = None;
            let _ = state.save_atomic(&sp);
        }
    }
    if let Some(v) = read_last_error(home) {
        if v.get("project_id").and_then(|p| p.as_str()) == Some(project_id) {
            let _ = fs::remove_file(last_error_path(home));
        }
    }
}

fn atomic_write_last_error(path: &Path, bytes: &[u8]) -> Result<(), VaneCliError> {
    let dir = path.parent().ok_or_else(|| {
        VaneCliError::new(format!(
            "last_error.json path has no parent: {}",
            path.display()
        ))
    })?;
    fs::create_dir_all(dir).map_err(|e| {
        VaneCliError::new(format!(
            "create last_error.json parent {}: {e}",
            dir.display()
        ))
    })?;
    let tmp = dir.join("last_error.json.tmp");
    {
        let mut f = File::create(&tmp).map_err(|e| {
            VaneCliError::new(format!(
                "create last_error.json temp {}: {e}",
                tmp.display()
            ))
        })?;
        f.write_all(bytes).map_err(|e| {
            VaneCliError::new(format!("write last_error.json temp {}: {e}", tmp.display()))
        })?;
        f.sync_all().map_err(|e| {
            VaneCliError::new(format!("sync last_error.json temp {}: {e}", tmp.display()))
        })?;
    }
    fs::rename(&tmp, path).map_err(|e| {
        VaneCliError::new(format!(
            "rename {} -> {}: {e}",
            tmp.display(),
            path.display()
        ))
    })
}

pub fn dirty_queue_size(home: &Path) -> u64 {
    DirtyQueue::load(&dirty_path(home)).len() as u64
}

pub fn explain_empty_query(
    home: &Path,
    cwd: &Path,
    query: &str,
    user_all: bool,
    user_root: Option<&Path>,
) -> EmptyQueryWhy {
    let cfg_path = home.join("config").join("config.toml");
    if !cfg_path.is_file() {
        return why("not_initialized", "not initialized — run `vane init`");
    }
    let Ok(cfg) = load_config(home) else {
        return why("not_initialized", "not initialized — run `vane init`");
    };
    let registered: Vec<PathBuf> = cfg
        .projects
        .iter()
        .map(|p| {
            p.path
                .canonicalize()
                .unwrap_or_else(|_| crate::fsutil::expand_tilde(&p.path))
        })
        .collect();
    let in_root = find_current_root(cwd, &registered);
    if !user_all && user_root.is_none() && in_root.is_none() {
        return why(
            "not_registered",
            "cwd is not a registered root — run `vane add` or pass --root / --all",
        );
    }

    let selected = selected_roots(&cfg, user_all, user_root, in_root.as_deref());
    if let Some(msg) = still_indexing_reason(home, &selected) {
        return why("still_indexing", msg);
    }

    if embedder_recorded_down(home, &selected) {
        return why(
            "embedder",
            "embedder is down or degraded — check `vane doctor` and the embed provider",
        );
    }

    if likely_excluded(&cfg, &selected, query) {
        return why(
            "excluded",
            "query looks like an excluded or untyped path — adjust include/exclude or search another file",
        );
    }

    if !user_all && selected.len() == 1 && registered.len() > 1 {
        return why(
            "wrong_root",
            "no hits in this root — try `vane query --all` or pass --root",
        );
    }

    if selected.iter().all(|root| {
        let pid = project_id(root);
        LiveSet::load_for_project(home, &pid)
            .map(|live| live.files.is_empty())
            .unwrap_or(true)
    }) {
        return why(
            "empty_index",
            "index is empty — run `vane add` and wait for reconcile",
        );
    }

    why(
        "no_match",
        "no matching chunks — try different terms or `vane query --all`",
    )
}

fn why(id: &'static str, message: impl Into<String>) -> EmptyQueryWhy {
    EmptyQueryWhy {
        id,
        message: message.into(),
    }
}

fn check_config(home: &Path, checks: &mut Vec<DoctorCheck>) -> Option<Config> {
    let path = home.join("config").join("config.toml");
    if !path.is_file() {
        checks.push(DoctorCheck {
            id: "config".into(),
            level: CheckLevel::Red,
            message: format!("missing {}", path.display()),
            fix: "run `vane init`".into(),
        });
        return None;
    }
    match load_config(home) {
        Ok(cfg) => {
            checks.push(DoctorCheck {
                id: "config".into(),
                level: CheckLevel::Green,
                message: format!("{} exists", path.display()),
                fix: String::new(),
            });
            check_config_mode(&path, checks);
            Some(cfg)
        }
        Err(e) => {
            checks.push(DoctorCheck {
                id: "config".into(),
                level: CheckLevel::Red,
                message: e.message,
                fix: "fix config.toml or re-run `vane init`".into(),
            });
            if path.is_file() {
                check_config_mode(&path, checks);
            }
            None
        }
    }
}

fn check_config_mode(path: &Path, checks: &mut Vec<DoctorCheck>) {
    let Ok(meta) = fs::metadata(path) else {
        checks.push(DoctorCheck {
            id: "config_mode".into(),
            level: CheckLevel::Red,
            message: format!("cannot stat {}", path.display()),
            fix: format!("chmod 0600 {}", path.display()),
        });
        return;
    };
    let mode = meta.permissions().mode() & 0o777;
    if mode & 0o004 != 0 {
        checks.push(DoctorCheck {
            id: "config_mode".into(),
            level: CheckLevel::Red,
            message: format!("{} is world-readable ({mode:04o})", path.display()),
            fix: format!("chmod 0600 {}", path.display()),
        });
    } else if mode != 0o600 {
        checks.push(DoctorCheck {
            id: "config_mode".into(),
            level: CheckLevel::Yellow,
            message: format!("{} mode is {mode:04o}, expected 0600", path.display()),
            fix: format!("chmod 0600 {}", path.display()),
        });
    } else {
        checks.push(DoctorCheck {
            id: "config_mode".into(),
            level: CheckLevel::Green,
            message: format!("{} mode 0600", path.display()),
            fix: String::new(),
        });
    }
}

fn check_socket(home: &Path, checks: &mut Vec<DoctorCheck>) {
    let sock = crate::daemon::socket_path(home);
    match UnixStream::connect(&sock) {
        Ok(_) => checks.push(DoctorCheck {
            id: "socket".into(),
            level: CheckLevel::Green,
            message: format!("{} is connectable", sock.display()),
            fix: String::new(),
        }),
        Err(_) => checks.push(DoctorCheck {
            id: "socket".into(),
            level: CheckLevel::Red,
            message: format!("cannot connect to {}", sock.display()),
            fix: "run `vane start`".into(),
        }),
    }
}

fn check_daemon(home: &Path, checks: &mut Vec<DoctorCheck>) {
    if crate::daemon::is_running(home) {
        checks.push(DoctorCheck {
            id: "daemon".into(),
            level: CheckLevel::Green,
            message: "daemon is running".into(),
            fix: String::new(),
        });
    } else {
        checks.push(DoctorCheck {
            id: "daemon".into(),
            level: CheckLevel::Red,
            message: "daemon is not running".into(),
            fix: "run `vane start`".into(),
        });
    }
}

fn check_service(checks: &mut Vec<DoctorCheck>) {
    let paths = crate::service::service_paths_from_env();
    if paths.unit_path.is_file() {
        checks.push(DoctorCheck {
            id: "service".into(),
            level: CheckLevel::Green,
            message: format!("user service unit {}", paths.unit_path.display()),
            fix: String::new(),
        });
    } else {
        checks.push(DoctorCheck {
            id: "service".into(),
            level: CheckLevel::Yellow,
            message: format!("user service unit missing ({})", paths.unit_path.display()),
            fix: "run `vane init` and install the user service".into(),
        });
    }
}

fn check_embed(cfg: &Config, checks: &mut Vec<DoctorCheck>) {
    let embedder = embedder_from_config(&cfg.defaults.embed);
    match embedder.probe_dim() {
        Ok(dim) => checks.push(DoctorCheck {
            id: "embed".into(),
            level: CheckLevel::Green,
            message: format!(
                "embed probe ok ({} / {} dim {dim})",
                cfg.defaults.embed.provider, cfg.defaults.embed.model
            ),
            fix: String::new(),
        }),
        Err(e) => checks.push(DoctorCheck {
            id: "embed".into(),
            level: CheckLevel::Red,
            message: format!("embed probe failed: {}", redact_secrets(&e.message)),
            fix: format!(
                "check embedder at {} (ollama serve, or OPENAI_API_KEY / VANE_EMBED_API_KEY)",
                cfg.defaults.embed.base_url
            ),
        }),
    }
}

fn check_roots(cfg: &Config, checks: &mut Vec<DoctorCheck>) {
    if cfg.projects.is_empty() {
        checks.push(DoctorCheck {
            id: "root".into(),
            level: CheckLevel::Yellow,
            message: "no registered roots".into(),
            fix: "run `vane add <path>`".into(),
        });
        return;
    }
    for proj in &cfg.projects {
        let stored = crate::fsutil::expand_tilde(&proj.path);
        let path = stored.canonicalize().unwrap_or_else(|_| stored.clone());
        let pid = project_id(&path);
        let id = format!("root:{pid}");
        if !path.exists() {
            checks.push(DoctorCheck {
                id,
                level: CheckLevel::Red,
                message: format!("root missing: {}", proj.path.display()),
                fix: format!("restore the folder or `vane rm {}`", proj.path.display()),
            });
            continue;
        }
        if fs::read_dir(&path).is_err() {
            checks.push(DoctorCheck {
                id,
                level: CheckLevel::Red,
                message: format!("root not readable: {}", proj.path.display()),
                fix: format!("chmod the directory so vane can read {}", path.display()),
            });
            continue;
        }
        checks.push(DoctorCheck {
            id,
            level: CheckLevel::Green,
            message: format!("root ok: {}", proj.path.display()),
            fix: String::new(),
        });
    }
}

fn check_disk(home: &Path, checks: &mut Vec<DoctorCheck>) {
    let stats = disk_stats(home);
    if stats.home_bytes > DISK_YELLOW_BYTES {
        checks.push(DoctorCheck {
            id: "disk".into(),
            level: CheckLevel::Yellow,
            message: format!(
                "{} is {} bytes (cas {})",
                home.display(),
                stats.home_bytes,
                stats.cas_bytes
            ),
            fix: "run `vane gc --all` to compact unused RAG data".into(),
        });
    } else {
        checks.push(DoctorCheck {
            id: "disk".into(),
            level: CheckLevel::Green,
            message: format!(
                "{} is {} bytes (cas {})",
                home.display(),
                stats.home_bytes,
                stats.cas_bytes
            ),
            fix: String::new(),
        });
    }
}

fn root_status_object(home: &Path, stored: &Path, dirty: &DirtyQueue) -> Value {
    let for_id = stored
        .canonicalize()
        .unwrap_or_else(|_| crate::fsutil::expand_tilde(stored));
    let pid = project_id(&for_id);
    let state = ProjectState::load(&state_path(home, &pid)).unwrap_or_default();
    let live = LiveSet::load_for_project(home, &pid).unwrap_or_default();
    let mut obj = json!({
        "path": stored.display().to_string(),
        "project_id": pid,
        "model": state.embed_model_id,
        "dim": state.dim,
        "live_files": live.files.len(),
        "last_reconcile": state.last_reconcile,
        "tokenizer_fallback": state.tokenizer_fallback,
        "rebuilding": state.rebuild.is_some(),
        "rebuild": state.rebuild,
        "reindex_error": state.reindex_error,
        "dirty_queue_size": dirty.len_for(&pid) as u64,
    });
    if let Some(err) = state.last_error.as_deref() {
        obj["last_error"] = json!(redact_secrets(err));
    }
    if let Some(n) = crate::progress::skip_count(home, &pid) {
        obj["skip_count"] = json!(n);
    }
    obj
}

fn read_last_error(home: &Path) -> Option<Value> {
    let path = last_error_path(home);
    let bytes = fs::read(path).ok()?;
    let mut v: Value = serde_json::from_slice(&bytes).ok()?;
    if let Some(obj) = v.as_object_mut() {
        obj.remove("api_key");
        if let Some(msg) = obj.get("message").and_then(|m| m.as_str()) {
            let redacted = redact_secrets(msg);
            obj.insert("message".into(), json!(redacted));
        }
    }
    Some(v)
}

fn selected_roots(
    cfg: &Config,
    user_all: bool,
    user_root: Option<&Path>,
    cwd_root: Option<&Path>,
) -> Vec<PathBuf> {
    if user_all {
        return cfg
            .projects
            .iter()
            .map(|p| {
                p.path
                    .canonicalize()
                    .unwrap_or_else(|_| crate::fsutil::expand_tilde(&p.path))
            })
            .collect();
    }
    if let Some(r) = user_root {
        let expanded = crate::fsutil::expand_tilde(r);
        return vec![expanded.canonicalize().unwrap_or(expanded)];
    }
    if let Some(r) = cwd_root {
        return vec![r.to_path_buf()];
    }
    cfg.projects
        .iter()
        .map(|p| {
            p.path
                .canonicalize()
                .unwrap_or_else(|_| crate::fsutil::expand_tilde(&p.path))
        })
        .collect()
}

fn still_indexing_reason(home: &Path, selected: &[PathBuf]) -> Option<String> {
    if let Some(progress) = crate::progress::load_progress(home) {
        if progress.phase != crate::progress::ProgressPhase::Idle {
            let extra = format!(" {}/{}", progress.scanned, progress.total_estimate);
            return Some(format!(
                "still indexing (phase={}{extra}) — wait and retry",
                progress.phase.as_str()
            ));
        }
    }
    for root in selected {
        let pid = project_id(root);
        let live = LiveSet::load_for_project(home, &pid).unwrap_or_default();
        let state = ProjectState::load(&state_path(home, &pid)).unwrap_or_default();
        if live.files.is_empty() && state.last_reconcile.is_none() {
            return Some("still indexing — no live files yet; wait for reconcile".into());
        }
    }
    None
}

fn embedder_recorded_down(home: &Path, selected: &[PathBuf]) -> bool {
    if let Some(err) = read_last_error(home) {
        if err.get("message").and_then(|m| m.as_str()).is_some() {
            return true;
        }
    }
    for root in selected {
        let pid = project_id(root);
        if let Ok(state) = ProjectState::load(&state_path(home, &pid)) {
            if state.last_error.as_ref().is_some_and(|s| !s.is_empty()) {
                return true;
            }
        }
    }
    false
}

fn likely_excluded(cfg: &Config, selected: &[PathBuf], query: &str) -> bool {
    let Some(rel) = query_as_rel_path(query, selected) else {
        return false;
    };
    for root in selected {
        let pf = fs::read_to_string(root.join(".vane.toml"))
            .ok()
            .and_then(|t| ProjectFile::parse_toml(&t).ok());
        let Ok(policy) = resolve_policy(cfg, root, pf.as_ref()) else {
            continue;
        };
        match classify(&rel, &policy) {
            Err(SkipReason::Excluded | SkipReason::NoType | SkipReason::Disabled) => return true,
            Ok(_) => {}
        }
    }
    false
}

fn query_as_rel_path(query: &str, selected: &[PathBuf]) -> Option<String> {
    let q = query.trim().trim_matches('"');
    if q.is_empty() {
        return None;
    }
    let path = PathBuf::from(q);
    if path.is_absolute() {
        for root in selected {
            if let Ok(rel) = path.strip_prefix(root) {
                return Some(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let looks_like = q.contains('/')
        || q.contains('\\')
        || q.starts_with('.')
        || q.contains("node_modules")
        || (q.contains('.') && !q.contains(' '));
    if looks_like {
        Some(q.replace('\\', "/"))
    } else {
        None
    }
}

fn redact_secrets(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("sk-proj-") {
            let n = secret_token_len(after);
            out.push_str("sk-proj-***");
            rest = &after[n..];
        } else if let Some(after) = rest.strip_prefix("sk-") {
            let n = secret_token_len(after);
            out.push_str("sk-***");
            rest = &after[n..];
        } else {
            let ch = rest.chars().next().expect("rest non-empty");
            out.push(ch);
            rest = &rest[ch.len_utf8()..];
        }
    }
    out
}

fn secret_token_len(s: &str) -> usize {
    s.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
        .unwrap_or(s.len())
}
