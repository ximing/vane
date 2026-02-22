use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::classify::{classify, should_watch_dir, SkipReason};
use crate::config::ResolvedPolicy;
use crate::error::VaneCliError;

const QUIET: Duration = Duration::from_millis(500);
const MAX_WAIT: Duration = Duration::from_secs(2);
const STOP_POLL: Duration = Duration::from_millis(200);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchKind {
    Create,
    Modify,
    Remove,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchEvent {
    pub root: PathBuf,
    pub rel: String,
    pub kind: WatchKind,
}

pub struct WatchGuard {
    watched: Arc<Mutex<Vec<PathBuf>>>,
    stop: Option<Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl WatchGuard {
    pub fn watched_paths_for_test(&self) -> Vec<PathBuf> {
        self.watched.lock().map(|g| g.clone()).unwrap_or_default()
    }
}

impl Drop for WatchGuard {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Watch each allowed directory under `roots`. Excluded trees are never registered.
pub fn watch_roots(
    roots: Vec<(PathBuf, ResolvedPolicy)>,
    tx: Sender<Vec<WatchEvent>>,
) -> Result<WatchGuard, VaneCliError> {
    let mut prepared = Vec::with_capacity(roots.len());
    for (root, policy) in roots {
        let canon = root
            .canonicalize()
            .map_err(|e| VaneCliError::new(format!("canonicalize {}: {e}", root.display())))?;
        if !canon.is_dir() {
            return Err(VaneCliError::new(format!(
                "watch root is not a directory: {}",
                canon.display()
            )));
        }
        prepared.push((canon, policy));
    }

    let watched = Arc::new(Mutex::new(Vec::new()));
    let (raw_tx, raw_rx) = mpsc::channel();
    let (stop_tx, stop_rx) = mpsc::channel();

    let mut watcher = RecommendedWatcher::new(
        move |res| {
            let _ = raw_tx.send(res);
        },
        notify::Config::default(),
    )
    .map_err(|e| VaneCliError::new(format!("create watcher: {e}")))?;

    for (root, policy) in &prepared {
        register_tree(&mut watcher, root, "", policy, &watched)?;
    }

    let watched_thread = Arc::clone(&watched);
    let thread = thread::Builder::new()
        .name("vane-watch".into())
        .spawn(move || {
            run_loop(watcher, raw_rx, stop_rx, tx, prepared, watched_thread);
        })
        .map_err(|e| VaneCliError::new(format!("spawn watch: {e}")))?;

    Ok(WatchGuard {
        watched,
        stop: Some(stop_tx),
        thread: Some(thread),
    })
}

fn run_loop(
    mut watcher: RecommendedWatcher,
    raw_rx: Receiver<notify::Result<Event>>,
    stop_rx: Receiver<()>,
    out_tx: Sender<Vec<WatchEvent>>,
    roots: Vec<(PathBuf, ResolvedPolicy)>,
    watched: Arc<Mutex<Vec<PathBuf>>>,
) {
    let mut pending: Vec<WatchEvent> = Vec::new();
    let mut first: Option<Instant> = None;
    let mut last: Option<Instant> = None;

    loop {
        if stop_rx.try_recv().is_ok() {
            flush(&out_tx, &mut pending, &mut first, &mut last);
            break;
        }

        let timeout = if pending.is_empty() {
            STOP_POLL
        } else {
            let now = Instant::now();
            let quiet_left = QUIET.saturating_sub(now.saturating_duration_since(last.unwrap()));
            let max_left = MAX_WAIT.saturating_sub(now.saturating_duration_since(first.unwrap()));
            let left = quiet_left.min(max_left);
            if left.is_zero() {
                flush(&out_tx, &mut pending, &mut first, &mut last);
                continue;
            }
            left
        };

        match raw_rx.recv_timeout(timeout) {
            Ok(Ok(event)) => {
                let events = ingest(&mut watcher, event, &roots, &watched);
                if events.is_empty() {
                    continue;
                }
                let now = Instant::now();
                if first.is_none() {
                    first = Some(now);
                }
                last = Some(now);
                pending.extend(events);
                if should_flush(first, last, now) {
                    flush(&out_tx, &mut pending, &mut first, &mut last);
                }
            }
            Ok(Err(_)) => {}
            Err(RecvTimeoutError::Timeout) => {
                if !pending.is_empty() && should_flush(first, last, Instant::now()) {
                    flush(&out_tx, &mut pending, &mut first, &mut last);
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                flush(&out_tx, &mut pending, &mut first, &mut last);
                break;
            }
        }
    }
}

fn should_flush(first: Option<Instant>, last: Option<Instant>, now: Instant) -> bool {
    let Some(first) = first else {
        return false;
    };
    let Some(last) = last else {
        return false;
    };
    now.duration_since(last) >= QUIET || now.duration_since(first) >= MAX_WAIT
}

fn flush(
    out_tx: &Sender<Vec<WatchEvent>>,
    pending: &mut Vec<WatchEvent>,
    first: &mut Option<Instant>,
    last: &mut Option<Instant>,
) {
    if pending.is_empty() {
        return;
    }
    let batch = std::mem::take(pending);
    let _ = out_tx.send(batch);
    *first = None;
    *last = None;
}

fn ingest(
    watcher: &mut RecommendedWatcher,
    event: Event,
    roots: &[(PathBuf, ResolvedPolicy)],
    watched: &Arc<Mutex<Vec<PathBuf>>>,
) -> Vec<WatchEvent> {
    let Some(kind) = map_kind(event.kind) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for path in event.paths {
        let Some((root, policy, rel)) = match_root(&path, roots) else {
            continue;
        };
        if under_excluded_dir(&rel, policy) {
            continue;
        }
        if classify(&rel, policy) == Err(SkipReason::Excluded) {
            continue;
        }
        if path.is_dir() && should_watch_dir(&rel, policy) && !already_watched(watched, &path) {
            let _ = register_tree(watcher, root, &rel, policy, watched);
        }
        out.push(WatchEvent {
            root: root.clone(),
            rel,
            kind,
        });
    }
    out
}

fn map_kind(kind: EventKind) -> Option<WatchKind> {
    match kind {
        EventKind::Create(_) => Some(WatchKind::Create),
        EventKind::Remove(_) => Some(WatchKind::Remove),
        EventKind::Modify(_) | EventKind::Any => Some(WatchKind::Modify),
        EventKind::Access(_) | EventKind::Other => None,
    }
}

fn match_root<'a>(
    path: &Path,
    roots: &'a [(PathBuf, ResolvedPolicy)],
) -> Option<(&'a PathBuf, &'a ResolvedPolicy, String)> {
    let mut best: Option<(&'a PathBuf, &'a ResolvedPolicy, String)> = None;
    for (root, policy) in roots {
        let Some(rel) = strip_root(root, path) else {
            continue;
        };
        let take = match &best {
            None => true,
            Some((prev, _, _)) => root.as_os_str().len() > prev.as_os_str().len(),
        };
        if take {
            best = Some((root, policy, rel));
        }
    }
    best
}

fn strip_root(root: &Path, path: &Path) -> Option<String> {
    if let Ok(rel) = path.strip_prefix(root) {
        return Some(posix(rel));
    }
    if let Ok(canon) = path.canonicalize() {
        if let Ok(rel) = canon.strip_prefix(root) {
            return Some(posix(rel));
        }
    }
    None
}

fn posix(path: &Path) -> String {
    path.components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn under_excluded_dir(rel: &str, policy: &ResolvedPolicy) -> bool {
    if rel.is_empty() {
        return !should_watch_dir("", policy);
    }
    let mut acc = String::new();
    for part in rel.split('/') {
        if part.is_empty() {
            continue;
        }
        if !acc.is_empty() {
            acc.push('/');
        }
        acc.push_str(part);
        if !should_watch_dir(&acc, policy) {
            return true;
        }
    }
    false
}

fn already_watched(watched: &Mutex<Vec<PathBuf>>, path: &Path) -> bool {
    let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    watched
        .lock()
        .map(|g| g.iter().any(|p| p == &canon || p == path))
        .unwrap_or(false)
}

fn register_tree(
    watcher: &mut RecommendedWatcher,
    root: &Path,
    rel: &str,
    policy: &ResolvedPolicy,
    watched: &Mutex<Vec<PathBuf>>,
) -> Result<(), VaneCliError> {
    if !should_watch_dir(rel, policy) {
        return Ok(());
    }

    let start = if rel.is_empty() {
        root.to_path_buf()
    } else {
        root.join(rel)
    };
    let start_canon = start
        .canonicalize()
        .map_err(|e| VaneCliError::new(format!("canonicalize {}: {e}", start.display())))?;
    if !is_path_prefix(root, &start_canon) {
        return Ok(());
    }

    let mut stack = vec![WalkFrame {
        dir: start,
        rel: rel.to_string(),
        chain: vec![start_canon],
    }];

    while let Some(frame) = stack.pop() {
        if !should_watch_dir(&frame.rel, policy) {
            continue;
        }
        let watch_path = frame
            .dir
            .canonicalize()
            .unwrap_or_else(|_| frame.dir.clone());
        if !already_watched(watched, &watch_path) {
            if let Err(e) = watcher.watch(&watch_path, RecursiveMode::NonRecursive) {
                if frame.rel.is_empty() {
                    return Err(VaneCliError::new(format!(
                        "watch {}: {e}",
                        watch_path.display()
                    )));
                }
                continue;
            }
            if let Ok(mut g) = watched.lock() {
                g.push(watch_path.clone());
            }
        }

        let entries = match fs::read_dir(&frame.dir) {
            Ok(e) => e,
            Err(e) if frame.rel.is_empty() => {
                return Err(VaneCliError::new(format!(
                    "read root {}: {e}",
                    frame.dir.display()
                )));
            }
            Err(_) => continue,
        };

        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let name = match entry.file_name().into_string() {
                Ok(n) if n != "." && n != ".." => n,
                _ => continue,
            };
            let child_rel = join_rel(&frame.rel, &name);
            if !should_watch_dir(&child_rel, policy) {
                continue;
            }
            let child_path = entry.path();
            let ft = match entry.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };

            let (is_dir, canon) = if ft.is_symlink() || ft.is_dir() {
                let canon = match fs::canonicalize(&child_path) {
                    Ok(t) => t,
                    Err(_) => continue,
                };
                if !is_path_prefix(root, &canon) {
                    continue;
                }
                (canon.is_dir(), canon)
            } else {
                continue;
            };
            if !is_dir {
                continue;
            }
            if frame.chain.iter().any(|p| p == &canon) {
                continue;
            }
            let mut next = frame.chain.clone();
            next.push(canon);
            stack.push(WalkFrame {
                dir: child_path,
                rel: child_rel,
                chain: next,
            });
        }
    }
    Ok(())
}

struct WalkFrame {
    dir: PathBuf,
    rel: String,
    chain: Vec<PathBuf>,
}

fn join_rel(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.replace('\\', "/")
    } else {
        format!("{parent}/{name}")
    }
}

fn is_path_prefix(prefix: &Path, path: &Path) -> bool {
    let mut path_c = path.components();
    for c in prefix.components() {
        match path_c.next() {
            Some(pc) if pc == c => {}
            _ => return false,
        }
    }
    true
}
