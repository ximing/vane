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

use crate::config::{load_config, resolve_policy, Config, ProjectFile, ResolvedPolicy};
use crate::error::VaneCliError;
use crate::index::{state_path, ProjectState};
use crate::ipc::{
    encode_response, parse_request, RpcRequest, RpcResponse, INTERNAL_ERROR, INVALID_PARAMS,
    INVALID_REQUEST, METHOD_NOT_FOUND, PARSE_ERROR,
};
use crate::live::LiveSet;
use crate::log::{DailyLogger, Level};
use crate::project::{project_id, reject_nested};
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
        "search" => Ok(json!([])),
        "read" => Ok(json!([])),
        "list_roots" => handle_list_roots(shared),
        "reload_config" => writer_call(shared, |resp| WriterCmd::Reload { resp }),
        "add_root" | "remove_root" => match param_path(&params) {
            Ok(path) if method == "add_root" => {
                writer_call(shared, |resp| WriterCmd::AddRoot { path, resp })
            }
            Ok(path) => writer_call(shared, |resp| WriterCmd::RemoveRoot { path, resp }),
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
