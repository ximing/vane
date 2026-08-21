//! Foreground-watch diff primitives: pure functions that classify live-set and
//! dirty-queue changes between two polling frames of `vane watch`.
//!
//! `diff_live` / `diff_queued` are pure iterators over in-memory snapshots, so
//! they are directly unit-testable without a daemon, a filesystem, or IPC.

use serde_json::json;

use crate::i18n::Lang;
use crate::live::LiveSet;

/// One observed change between two polling frames of a single root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEvent {
    Added(String),
    Updated(String),
    Removed(String),
    Queued(String),
}

/// Diff two `LiveSet`s for the same project. A path present in `next` but not
/// `prev` is `Added`; present in both but with a different `extract_key` is
/// `Updated`; present in `prev` but not `next` is `Removed`. `LiveSet.files` is
/// a `BTreeMap`, so iteration order is stable.
pub fn diff_live(prev: &LiveSet, next: &LiveSet) -> Vec<WatchEvent> {
    let mut events = Vec::new();
    for (path, nf) in &next.files {
        match prev.files.get(path) {
            None => events.push(WatchEvent::Added(path.clone())),
            Some(pf) if pf.extract_key != nf.extract_key => {
                events.push(WatchEvent::Updated(path.clone()));
            }
            _ => {}
        }
    }
    for path in prev.files.keys() {
        if !next.files.contains_key(path) {
            events.push(WatchEvent::Removed(path.clone()));
        }
    }
    events
}

/// Diff two dirty-queue path lists. Only paths newly present in `next` (and
/// absent from `prev`) are reported as `Queued`. Paths that disappear — because
/// the daemon dequeued or reconciled them — are silent by design: the live
/// set's `Updated` / `Removed` already expresses the materialised result, so
/// echoing a "dequeue" event would be noise.
pub fn diff_queued(prev: &[String], next: &[String]) -> Vec<WatchEvent> {
    next.iter()
        .filter(|p| !prev.contains(p))
        .map(|p| WatchEvent::Queued(p.clone()))
        .collect()
}

/// Human (TTY) line for one event, rendered in the detected language.
/// `added {path}` / `updated {path}` / `removed {path}` / `queued {path}`
/// (zh: 新增 / 更新 / 移除 / 排队).
pub fn event_line(ev: &WatchEvent, lang: Lang) -> String {
    let (key, path): (&'static str, &str) = match ev {
        WatchEvent::Added(p) => ("watch.added", p.as_str()),
        WatchEvent::Updated(p) => ("watch.updated", p.as_str()),
        WatchEvent::Removed(p) => ("watch.removed", p.as_str()),
        WatchEvent::Queued(p) => ("watch.queued", p.as_str()),
    };
    crate::i18n::tr(lang, key).replace("{path}", path)
}

/// Machine (non-TTY) JSON object for one event:
/// `{"event":"updated","path":"notes/a.md","root":"…","at":…}`.
pub fn event_json(ev: &WatchEvent, root: &str, at: u64) -> serde_json::Value {
    let (event, path): (&'static str, &str) = match ev {
        WatchEvent::Added(p) => ("added", p.as_str()),
        WatchEvent::Updated(p) => ("updated", p.as_str()),
        WatchEvent::Removed(p) => ("removed", p.as_str()),
        WatchEvent::Queued(p) => ("queued", p.as_str()),
    };
    json!({
        "event": event,
        "path": path,
        "root": root,
        "at": at,
    })
}

/// `--interval-ms` bounds (spec test 13): `100..=60000` inclusive.
/// `valid_interval(0)`, `valid_interval(99)`, `valid_interval(60_001)` → false;
/// `valid_interval(100)`, `valid_interval(60_000)` → true.
pub fn valid_interval(ms: u64) -> bool {
    (100..=60_000).contains(&ms)
}
