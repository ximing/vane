use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};

use crate::cas::Cas;
use crate::config::{
    load_config, resolve_policy, Config, EmbedConfig, ProjectFile, ResolvedPolicy,
};
use crate::embed::embedder_from_config;
use crate::error::VaneCliError;
use crate::index::{project_db_path, state_path, ProjectIndex, ProjectState};
use crate::ipc::{
    encode_response, parse_request, RpcRequest, RpcResponse, INTERNAL_ERROR, INVALID_PARAMS,
    INVALID_REQUEST, METHOD_NOT_FOUND, PARSE_ERROR,
};
use crate::live::LiveSet;
use crate::log::{DailyLogger, Level};
use crate::project::{project_id, reject_nested};
use crate::search::{read_by_id, read_by_path, search_all, search_project, ProjectSearch};
use crate::sync::rebuild_for_new_model;
use crate::watch::{watch_roots, WatchEvent, WatchGuard};

enum WriterCmd {
    Reload {
        resp: Sender<Result<Value, VaneCliError>>,
    },
    AddRoot {
        path: PathBuf,
        resp: Sender<Result<Value, VaneCliError>>,
    },
    RemoveRoot {
        path: PathBuf,
        resp: Sender<Result<Value, VaneCliError>>,
    },
    WatchBatch {
        events: Vec<WatchEvent>,
    },
    Rebuild {
        root: PathBuf,
        embed: EmbedConfig,
        resp: Sender<Result<Value, VaneCliError>>,
    },
    Shutdown,
}

struct Shared {
    home: PathBuf,
    config: Mutex<Config>,
    logger: Mutex<DailyLogger>,
    writer: Sender<WriterCmd>,
    watch_tx: Sender<Vec<WatchEvent>>,
    watch: Mutex<Option<WatchGuard>>,
}

pub fn socket_path(home: &Path) -> PathBuf {
    home.join("run").join("vane.sock")
}

pub fn pid_path(home: &Path) -> PathBuf {
    home.join("run").join("vane.pid")
}

pub fn acquire_pid_lock(home: &Path) -> Result<File, VaneCliError> {
    let run = home.join("run");
    fs::create_dir_all(&run)
        .map_err(|e| VaneCliError::new(format!("create run dir {}: {e}", run.display())))?;
    let path = pid_path(home);
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|e| VaneCliError::new(format!("open {}: {e}", path.display())))?;

    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc != 0 {
        return Err(already_running_error(&path));
    }
    write_pid(&mut file)?;
    Ok(file)
}

fn already_running_error(pid_file: &Path) -> VaneCliError {
    let existing = fs::read_to_string(pid_file).unwrap_or_default();
    let pid_str = existing.trim();
    if let Ok(pid) = pid_str.parse::<u32>() {
        if pid_alive(pid) {
            return VaneCliError::new(format!("already running (pid {pid})"));
        }
    }
    if pid_str.is_empty() {
        VaneCliError::new("already running")
    } else {
        VaneCliError::new(format!("already running (pid {pid_str})"))
    }
}

fn pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    let r = unsafe { libc::kill(pid as i32, 0) };
    r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

fn write_pid(file: &mut File) -> Result<(), VaneCliError> {
    file.set_len(0)
        .map_err(|e| VaneCliError::new(format!("truncate pid file: {e}")))?;
    file.seek(SeekFrom::Start(0))
        .map_err(|e| VaneCliError::new(format!("seek pid file: {e}")))?;
    write!(file, "{}", std::process::id())
        .map_err(|e| VaneCliError::new(format!("write pid file: {e}")))?;
    file.sync_all()
        .map_err(|e| VaneCliError::new(format!("sync pid file: {e}")))?;
    Ok(())
}

pub fn serve_forever(home: PathBuf) -> Result<(), VaneCliError> {
    let cfg = load_config(&home)?;
    let retain_days = cfg.log.retain_days;
    let _lock = acquire_pid_lock(&home)?;

    let mut logger = DailyLogger::open(&home.join("log"), retain_days)?;
    logger.write(Level::Info, "daemon started");

    let sock = socket_path(&home);
    if sock.exists() {
        let _ = fs::remove_file(&sock);
    }
    let listener = UnixListener::bind(&sock)
        .map_err(|e| VaneCliError::new(format!("bind {}: {e}", sock.display())))?;
    fs::set_permissions(&sock, fs::Permissions::from_mode(0o600))
        .map_err(|e| VaneCliError::new(format!("chmod socket: {e}")))?;

    let (tx, rx) = mpsc::channel();
    let (watch_tx, watch_rx) = mpsc::channel();
    let shared = Arc::new(Shared {
        home: home.clone(),
        config: Mutex::new(cfg),
        logger: Mutex::new(logger),
        writer: tx.clone(),
        watch_tx,
        watch: Mutex::new(None),
    });
    let writer_shared = Arc::clone(&shared);
    let writer_thread = thread::Builder::new()
        .name("vane-writer".into())
        .spawn(move || writer_loop(writer_shared, rx))
        .map_err(|e| VaneCliError::new(format!("spawn writer: {e}")))?;

    let fwd = tx.clone();
    let _watch_fwd = thread::Builder::new()
        .name("vane-watch-fwd".into())
        .spawn(move || {
            while let Ok(events) = watch_rx.recv() {
                if fwd.send(WriterCmd::WatchBatch { events }).is_err() {
                    break;
                }
            }
        })
        .map_err(|e| VaneCliError::new(format!("spawn watch forwarder: {e}")))?;

    restart_watch(&shared);

    let result = accept_loop(&listener, &shared);
    let _ = tx.send(WriterCmd::Shutdown);
    let _ = writer_thread.join();
    if let Ok(mut slot) = shared.watch.lock() {
        *slot = None;
    }
    let _ = fs::remove_file(&sock);
    result
}

fn accept_loop(listener: &UnixListener, shared: &Arc<Shared>) -> Result<(), VaneCliError> {
    loop {
        match listener.accept() {
            Ok((stream, _)) => handle_client(stream, shared),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(VaneCliError::new(format!("accept: {e}"))),
        }
    }
}

fn handle_client(stream: UnixStream, shared: &Arc<Shared>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let mut reader = match stream.try_clone() {
        Ok(s) => BufReader::new(s),
        Err(_) => return,
    };
    let mut writer = BufWriter::new(stream);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                if line.trim().is_empty() {
                    continue;
                }
                let resp = dispatch_line(&line, shared);
                let encoded = match encode_response(&resp) {
                    Ok(s) => s,
                    Err(e) => {
                        log_msg(shared, Level::Warn, &format!("encode response: {e}"));
                        break;
                    }
                };
                if writeln!(writer, "{encoded}").is_err() || writer.flush().is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

fn dispatch_line(line: &str, shared: &Arc<Shared>) -> RpcResponse {
    match parse_request(line) {
        Ok(req) => dispatch(req, shared),
        Err(e) => {
            if let Ok(v) = serde_json::from_str::<Value>(line.trim()) {
                if v.get("id").is_none() || v.get("method").is_none() {
                    let id = v
                        .get("id")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string();
                    return RpcResponse::err(id, INVALID_REQUEST, e.message);
                }
            }
            let id = scrape_id(line).unwrap_or_default();
            RpcResponse::err(id, PARSE_ERROR, e.message)
        }
    }
}

fn scrape_id(line: &str) -> Option<String> {
    let v: Value = serde_json::from_str(line.trim()).ok()?;
    v.get("id")?.as_str().map(str::to_string)
}

fn dispatch(req: RpcRequest, shared: &Arc<Shared>) -> RpcResponse {
    let RpcRequest { id, method, params } = req;
    let result = match method.as_str() {
        "status" => handle_status(shared),
        "search" => handle_search(shared, &params),
        "read" => handle_read(shared, &params),
        "list_roots" => handle_list_roots(shared),
        "reload_config" => writer_call(shared, |resp| WriterCmd::Reload { resp }),
        "add_root" | "remove_root" => match param_path(&params) {
            Ok(path) if method == "add_root" => {
                writer_call(shared, |resp| WriterCmd::AddRoot { path, resp })
            }
            Ok(path) => writer_call(shared, |resp| WriterCmd::RemoveRoot { path, resp }),
            Err(e) => return RpcResponse::err(id, INVALID_PARAMS, e.message),
        },
        "rebuild" | "set_model" => match rebuild_params(&params) {
            Ok((path, embed)) => writer_call(shared, |resp| WriterCmd::Rebuild {
                root: path,
                embed,
                resp,
            }),
            Err(e) => return RpcResponse::err(id, INVALID_PARAMS, e.message),
        },
        other => {
            return RpcResponse::err(id, METHOD_NOT_FOUND, format!("method not found: {other}"));
        }
    };
    match result {
        Ok(v) => RpcResponse::ok(id, v),
        Err(e) => RpcResponse::err(id, INTERNAL_ERROR, e.message),
    }
}

fn handle_search(shared: &Arc<Shared>, params: &Value) -> Result<Value, VaneCliError> {
    let query = params
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| VaneCliError::new("missing params.query"))?;
    if query.is_empty() {
        return Ok(json!([]));
    }
    let top_k = params
        .get("top_k")
        .and_then(|v| v.as_u64())
        .unwrap_or(8)
        .min(50) as u32;
    let extractor = params.get("type").and_then(|v| v.as_str());
    let want_all = params.get("all").and_then(|v| v.as_bool()).unwrap_or(false);
    let root_filter = params.get("root").and_then(|v| v.as_str());

    let cfg = lock_config(shared)?.clone();
    let selected = select_projects(&cfg, root_filter, want_all)?;
    let cas = Cas::new(shared.home.join("rag").join("cas"));
    let opened = open_search_targets(&shared.home, &cfg, &selected, extractor)?;
    let scopes: Vec<ProjectSearch<'_>> = opened
        .iter()
        .map(|o| ProjectSearch {
            index: &o.index,
            embedder: o.embedder.as_ref(),
            cas: &cas,
            live: &o.live,
            root: &o.root,
            extractor: o.extractor,
        })
        .collect();
    let hits = if scopes.len() <= 1 {
        match scopes.first() {
            Some(p) => search_project(p, query, top_k)?,
            None => Vec::new(),
        }
    } else {
        search_all(&scopes, query, top_k)?
    };
    serde_json::to_value(hits).map_err(|e| VaneCliError::new(format!("encode hits: {e}")))
}

fn handle_read(shared: &Arc<Shared>, params: &Value) -> Result<Value, VaneCliError> {
    let cas = Cas::new(shared.home.join("rag").join("cas"));
    let cfg = lock_config(shared)?.clone();
    if let Some(id) = params.get("id").and_then(|v| v.as_str()) {
        if !id.is_empty() {
            let (pid, _, _) = crate::search::parse_doc_id(id)?;
            let (root, live) = live_for_project(&shared.home, &cfg, Some(&pid), None)?;
            let chunk = read_by_id(&cas, &live, &pid, &root, id)?;
            return serde_json::to_value(chunk)
                .map_err(|e| VaneCliError::new(format!("encode read: {e}")));
        }
    }
    let path = params
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| VaneCliError::new("missing params.id or params.path"))?;
    let root_hint = params.get("root").and_then(|v| v.as_str());
    let (root, pid, live) = resolve_read_target(&shared.home, &cfg, path, root_hint)?;
    let chunks = read_by_path(&cas, &live, &pid, &root, path)?;
    serde_json::to_value(chunks).map_err(|e| VaneCliError::new(format!("encode read: {e}")))
}

struct OpenedSearch<'a> {
    index: ProjectIndex,
    embedder: Box<dyn crate::embed::Embedder>,
    live: LiveSet,
    root: String,
    extractor: Option<&'a str>,
}

fn select_projects<'a>(
    cfg: &'a Config,
    root: Option<&str>,
    all: bool,
) -> Result<Vec<&'a crate::config::ProjectEntry>, VaneCliError> {
    if !all {
        if let Some(r) = root {
            let target = canonical_or_as_is(&PathBuf::from(r));
            let found: Vec<_> = cfg
                .projects
                .iter()
                .filter(|p| canonical_or_as_is(&p.path) == target)
                .collect();
            if found.is_empty() {
                return Err(VaneCliError::new(format!("root not registered: {r}")));
            }
            return Ok(found);
        }
    }
    Ok(cfg.projects.iter().collect())
}

fn open_search_targets<'a>(
    home: &Path,
    cfg: &Config,
    entries: &[&crate::config::ProjectEntry],
    extractor: Option<&'a str>,
) -> Result<Vec<OpenedSearch<'a>>, VaneCliError> {
    let mut out = Vec::new();
    for entry in entries {
        let root = canonical_or_as_is(&entry.path);
        if !root.is_dir() {
            continue;
        }
        let pid = project_id(&root);
        let state = ProjectState::load(&state_path(home, &pid))?;
        let (Some(dim), Some(model_id)) = (state.dim, state.embed_model_id.clone()) else {
            continue;
        };
        if !project_db_path(home, &pid).exists() {
            continue;
        }
        let index = match crate::index::open_or_create(home, &pid, dim, &model_id) {
            Ok(i) => i,
            Err(_) => continue,
        };
        let pf = read_project_file(&root);
        let policy = match resolve_policy(cfg, &root, pf.as_ref()) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let live = LiveSet::load_for_project(home, &pid)?;
        out.push(OpenedSearch {
            index,
            embedder: embedder_from_config(&policy.embed),
            live,
            root: root.to_string_lossy().into_owned(),
            extractor,
        });
    }
    Ok(out)
}

fn live_for_project(
    home: &Path,
    cfg: &Config,
    pid: Option<&str>,
    root_hint: Option<&str>,
) -> Result<(PathBuf, LiveSet), VaneCliError> {
    if let Some(r) = root_hint {
        let root = canonical_or_as_is(&PathBuf::from(r));
        let pid = project_id(&root);
        let live = LiveSet::load_for_project(home, &pid)?;
        return Ok((root, live));
    }
    if let Some(pid) = pid {
        for p in &cfg.projects {
            let root = canonical_or_as_is(&p.path);
            if project_id(&root) == pid {
                let live = LiveSet::load_for_project(home, pid)?;
                return Ok((root, live));
            }
        }
        return Err(VaneCliError::new(format!("unknown project {pid}")));
    }
    Err(VaneCliError::new("missing root"))
}

fn resolve_read_target(
    home: &Path,
    cfg: &Config,
    rel_path: &str,
    root_hint: Option<&str>,
) -> Result<(PathBuf, String, LiveSet), VaneCliError> {
    if let Some(r) = root_hint {
        let root = canonical_or_as_is(&PathBuf::from(r));
        let pid = project_id(&root);
        let live = LiveSet::load_for_project(home, &pid)?;
        return Ok((root, pid, live));
    }
    let mut matches = Vec::new();
    for p in &cfg.projects {
        let root = canonical_or_as_is(&p.path);
        let pid = project_id(&root);
        let live = LiveSet::load_for_project(home, &pid)?;
        if live.files.contains_key(rel_path) {
            matches.push((root, pid, live));
        }
    }
    match matches.len() {
        0 => Err(VaneCliError::new(format!(
            "path not in working set: {rel_path}"
        ))),
        1 => Ok(matches.pop().expect("len == 1")),
        _ => Err(VaneCliError::new(format!(
            "path {rel_path} is in multiple roots; pass params.root"
        ))),
    }
}

fn canonical_or_as_is(path: &Path) -> PathBuf {
    let expanded = expand_tilde(path);
    expanded.canonicalize().unwrap_or(expanded)
}

fn handle_status(shared: &Arc<Shared>) -> Result<Value, VaneCliError> {
    let listed = handle_list_roots(shared)?;
    Ok(json!({
        "home": shared.home.display().to_string(),
        "roots": listed.get("roots").cloned().unwrap_or_else(|| json!([])),
    }))
}

fn handle_list_roots(shared: &Arc<Shared>) -> Result<Value, VaneCliError> {
    let projects = {
        let cfg = lock_config(shared)?;
        cfg.projects.clone()
    };
    let mut roots = Vec::new();
    for proj in projects {
        let stored = proj.path.clone();
        let for_id = stored
            .canonicalize()
            .unwrap_or_else(|_| expand_tilde(&stored));
        let pid = project_id(&for_id);
        let state = ProjectState::load(&state_path(&shared.home, &pid))?;
        let live = LiveSet::load_for_project(&shared.home, &pid)?;
        roots.push(json!({
            "path": stored.display().to_string(),
            "project_id": pid,
            "model": state.embed_model_id,
            "dim": state.dim,
            "live_files": live.files.len(),
            "last_reconcile": Value::Null,
            "rebuilding": state.rebuild.is_some(),
            "rebuild": state.rebuild,
            "reindex_error": state.reindex_error,
        }));
    }
    Ok(json!({ "roots": roots }))
}

fn param_path(params: &Value) -> Result<PathBuf, VaneCliError> {
    let s = params
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| VaneCliError::new("missing params.path"))?;
    if s.is_empty() {
        return Err(VaneCliError::new("missing params.path"));
    }
    Ok(PathBuf::from(s))
}

fn rebuild_params(params: &Value) -> Result<(PathBuf, EmbedConfig), VaneCliError> {
    let root = params
        .get("root")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| param_path(params).ok())
        .ok_or_else(|| VaneCliError::new("missing params.root"))?;
    let cfg = lock_config_from_params_embed(params, &root)?;
    Ok((root, cfg))
}

fn lock_config_from_params_embed(
    params: &Value,
    _root: &Path,
) -> Result<EmbedConfig, VaneCliError> {
    Ok(EmbedConfig {
        provider: params
            .get("provider")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        model: params
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        base_url: params
            .get("base_url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        api_key: params
            .get("api_key")
            .and_then(|v| v.as_str())
            .map(str::to_string),
    })
}

fn do_rebuild(
    shared: &Shared,
    root: &Path,
    embed_over: &EmbedConfig,
) -> Result<Value, VaneCliError> {
    let cfg = load_config(&shared.home)?;
    {
        let mut slot = lock_config(shared)?;
        *slot = cfg.clone();
    }
    let expanded = expand_tilde(root);
    let canon = expanded.canonicalize().unwrap_or_else(|_| expanded.clone());
    let pid = project_id(&canon);
    let pf = read_project_file(&canon);
    let mut policy = resolve_policy(&cfg, &canon, pf.as_ref())?;
    if !embed_over.provider.is_empty() {
        policy.embed.provider = embed_over.provider.clone();
    }
    if !embed_over.model.is_empty() {
        policy.embed.model = embed_over.model.clone();
    }
    if !embed_over.base_url.is_empty() {
        policy.embed.base_url = embed_over.base_url.clone();
    }
    if embed_over.api_key.is_some() {
        policy.embed.api_key = embed_over.api_key.clone();
    }
    rebuild_for_new_model(&shared.home, &pid, &policy.embed)?;
    log_msg(
        shared,
        Level::Info,
        &format!("rebuilt {} with {}", pid, policy.embed.model),
    );
    Ok(json!({
        "ok": true,
        "project_id": pid,
        "provider": policy.embed.provider,
        "model": policy.embed.model,
    }))
}

fn writer_call<F>(shared: &Arc<Shared>, make: F) -> Result<Value, VaneCliError>
where
    F: FnOnce(Sender<Result<Value, VaneCliError>>) -> WriterCmd,
{
    let (tx, rx) = mpsc::channel();
    shared
        .writer
        .send(make(tx))
        .map_err(|_| VaneCliError::new("writer thread stopped"))?;
    rx.recv()
        .map_err(|_| VaneCliError::new("writer thread stopped"))?
}

fn writer_loop(shared: Arc<Shared>, rx: Receiver<WriterCmd>) {
    while let Ok(cmd) = rx.recv() {
        match cmd {
            WriterCmd::Shutdown => break,
            WriterCmd::Reload { resp } => {
                let _ = resp.send(do_reload(&shared));
            }
            WriterCmd::AddRoot { path, resp } => {
                let _ = resp.send(do_add_root(&shared, &path));
            }
            WriterCmd::RemoveRoot { path, resp } => {
                let _ = resp.send(do_remove_root(&shared, &path));
            }
            WriterCmd::WatchBatch { events } => {
                handle_watch_batch(&shared, events);
            }
            WriterCmd::Rebuild { root, embed, resp } => {
                let _ = resp.send(do_rebuild(&shared, &root, &embed));
            }
        }
    }
}

fn handle_watch_batch(shared: &Shared, events: Vec<WatchEvent>) {
    if events.is_empty() {
        return;
    }
    let reload = events
        .iter()
        .any(|e| e.rel == ".vane.toml" || e.rel.ends_with("/.vane.toml"));
    log_msg(
        shared,
        Level::Info,
        &format!("watch batch {} path(s)", events.len()),
    );
    if reload {
        let _ = do_reload(shared);
    }
}

fn do_reload(shared: &Shared) -> Result<Value, VaneCliError> {
    let cfg = load_config(&shared.home)?;
    let retain = cfg.log.retain_days;
    {
        let mut slot = lock_config(shared)?;
        *slot = cfg;
    }
    reopen_logger(shared, retain);
    restart_watch(shared);
    log_msg(shared, Level::Info, "config reloaded");
    Ok(json!({ "ok": true }))
}

fn do_add_root(shared: &Shared, path: &Path) -> Result<Value, VaneCliError> {
    let expanded = expand_tilde(path);
    let canon = expanded
        .canonicalize()
        .map_err(|e| VaneCliError::new(format!("canonicalize {}: {e}", expanded.display())))?;
    mutate_projects(shared, |arr| {
        let existing = project_paths(arr);
        reject_nested(&existing, &canon)?;
        let mut entry = toml::map::Map::new();
        entry.insert(
            "path".into(),
            toml::Value::String(canon.display().to_string()),
        );
        arr.push(toml::Value::Table(entry));
        Ok(())
    })?;
    restart_watch(shared);
    log_msg(
        shared,
        Level::Info,
        &format!("added root {}", canon.display()),
    );
    Ok(json!({ "ok": true, "path": canon.display().to_string() }))
}

fn do_remove_root(shared: &Shared, path: &Path) -> Result<Value, VaneCliError> {
    let expanded = expand_tilde(path);
    let target = expanded.canonicalize().unwrap_or_else(|_| expanded.clone());
    mutate_projects(shared, |arr| {
        let before = arr.len();
        arr.retain(|p| {
            let Some(s) = p.get("path").and_then(|v| v.as_str()) else {
                return true;
            };
            let stored = expand_tilde(&PathBuf::from(s));
            let canon = stored.canonicalize().unwrap_or(stored);
            canon != target
        });
        if arr.len() == before {
            return Err(VaneCliError::new(format!(
                "root not registered: {}",
                path.display()
            )));
        }
        Ok(())
    })?;
    restart_watch(shared);
    log_msg(
        shared,
        Level::Info,
        &format!("removed root {}", target.display()),
    );
    Ok(json!({ "ok": true, "path": target.display().to_string() }))
}

fn mutate_projects<F>(shared: &Shared, f: F) -> Result<(), VaneCliError>
where
    F: FnOnce(&mut Vec<toml::Value>) -> Result<(), VaneCliError>,
{
    let cfg_path = shared.home.join("config").join("config.toml");
    let text = fs::read_to_string(&cfg_path)
        .map_err(|e| VaneCliError::new(format!("read {}: {e}", cfg_path.display())))?;
    let mut value: toml::Value =
        toml::from_str(&text).map_err(|e| VaneCliError::new(format!("parse config: {e}")))?;
    let table = value
        .as_table_mut()
        .ok_or_else(|| VaneCliError::new("config is not a table"))?;
    let projects = table
        .entry("projects")
        .or_insert(toml::Value::Array(Vec::new()));
    let arr = projects
        .as_array_mut()
        .ok_or_else(|| VaneCliError::new("projects is not an array"))?;
    f(arr)?;
    let out = toml::to_string_pretty(&value)
        .map_err(|e| VaneCliError::new(format!("serialize config: {e}")))?;
    fs::write(&cfg_path, out)
        .map_err(|e| VaneCliError::new(format!("write {}: {e}", cfg_path.display())))?;
    let cfg = load_config(&shared.home)?;
    *lock_config(shared)? = cfg;
    Ok(())
}

fn project_paths(arr: &[toml::Value]) -> Vec<PathBuf> {
    arr.iter()
        .filter_map(|p| p.get("path")?.as_str().map(PathBuf::from))
        .map(|p| {
            let expanded = expand_tilde(&p);
            expanded.canonicalize().unwrap_or(expanded)
        })
        .collect()
}

fn restart_watch(shared: &Shared) {
    let cfg = match lock_config(shared) {
        Ok(g) => g.clone(),
        Err(e) => {
            log_msg(shared, Level::Warn, &e.message);
            return;
        }
    };
    let targets = collect_watch_targets(&cfg);
    match watch_roots(targets, shared.watch_tx.clone()) {
        Ok(guard) => match shared.watch.lock() {
            Ok(mut slot) => *slot = Some(guard),
            Err(_) => log_msg(shared, Level::Warn, "watch lock poisoned"),
        },
        Err(e) => log_msg(shared, Level::Warn, &format!("watch: {e}")),
    }
}

fn collect_watch_targets(cfg: &Config) -> Vec<(PathBuf, ResolvedPolicy)> {
    let mut out = Vec::new();
    for proj in &cfg.projects {
        let expanded = expand_tilde(&proj.path);
        let Ok(root) = expanded.canonicalize() else {
            continue;
        };
        if !root.is_dir() {
            continue;
        }
        let pf = read_project_file(&root);
        match resolve_policy(cfg, &root, pf.as_ref()) {
            Ok(pol) => out.push((root, pol)),
            Err(_) => continue,
        }
    }
    out
}

fn read_project_file(root: &Path) -> Option<ProjectFile> {
    let path = root.join(".vane.toml");
    let text = fs::read_to_string(path).ok()?;
    ProjectFile::parse_toml(&text).ok()
}

fn reopen_logger(shared: &Shared, retain_days: u32) {
    match DailyLogger::open(&shared.home.join("log"), retain_days) {
        Ok(new_log) => {
            if let Ok(mut slot) = shared.logger.lock() {
                *slot = new_log;
            }
        }
        Err(e) => eprintln!("vane log reopen failed: {e}"),
    }
}

fn lock_config(shared: &Shared) -> Result<std::sync::MutexGuard<'_, Config>, VaneCliError> {
    shared
        .config
        .lock()
        .map_err(|_| VaneCliError::new("config lock poisoned"))
}

fn log_msg(shared: &Shared, level: Level, msg: &str) {
    match shared.logger.lock() {
        Ok(mut log) => log.write(level, msg),
        Err(_) => eprintln!("vane log lock poisoned: {msg}"),
    }
}

fn expand_tilde(path: &Path) -> PathBuf {
    let Some(s) = path.to_str() else {
        return path.to_path_buf();
    };
    if s == "~" {
        return std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| path.to_path_buf());
    }
    if let Some(rest) = s.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    path.to_path_buf()
}
