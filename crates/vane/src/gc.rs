use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::cas::{embed_key, Cas};
use crate::config::load_config;
use crate::error::VaneCliError;
use crate::index::{
    open_or_create_at, project_db_path, project_db_prev_path, project_dir, state_path, ProjectState,
};
use crate::live::LiveSet;
use crate::project::project_id;

const SECS_PER_DAY: u64 = 86_400;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiveKeySet {
    extract: BTreeSet<String>,
    embed: BTreeSet<String>,
}

impl LiveKeySet {
    pub fn insert_extract(&mut self, key: impl Into<String>) {
        self.extract.insert(key.into());
    }

    pub fn insert_embed(&mut self, key: impl Into<String>) {
        self.embed.insert(key.into());
    }

    pub fn contains_extract(&self, key: &str) -> bool {
        self.extract.contains(key)
    }

    pub fn contains_embed(&self, key: &str) -> bool {
        self.embed.contains(key)
    }

    /// Pull recorded embed keys for every live extract so shared vectors stay pinned.
    pub fn expand_from_cas(&mut self, cas: &Cas) {
        let extracts: Vec<String> = self.extract.iter().cloned().collect();
        for ek in extracts {
            for vk in cas.stored_embed_keys(&ek) {
                self.embed.insert(vk);
            }
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct GcReport {
    pub extract_deleted: u64,
    pub embed_deleted: u64,
    pub db_prev_removed: u64,
    pub projects_removed: u64,
    pub compacted: u64,
    pub errors: u64,
}

impl GcReport {
    fn merge(&mut self, other: Self) {
        self.extract_deleted += other.extract_deleted;
        self.embed_deleted += other.embed_deleted;
        self.db_prev_removed += other.db_prev_removed;
        self.projects_removed += other.projects_removed;
        self.compacted += other.compacted;
        self.errors += other.errors;
    }
}

/// Delete CAS objects whose `last_seen` is older than `retain_days` and not in `live_keys`.
pub fn gc_ttl(cas: &Cas, live_keys: &LiveKeySet, now: u64, retain_days: u32) -> GcReport {
    let mut live = live_keys.clone();
    live.expand_from_cas(cas);
    let retain_secs = u64::from(retain_days.max(1)) * SECS_PER_DAY;
    sweep(cas, &live, Some((now, retain_secs)))
}

/// Delete extract/embed objects that are not referenced by any remaining live set.
pub fn gc_unreferenced(cas: &Cas, live_keys: &LiveKeySet) -> GcReport {
    let mut live = live_keys.clone();
    live.expand_from_cas(cas);
    sweep(cas, &live, None)
}

/// Compact the project db, drop `db.prev`, then delete unreferenced CAS using the union of lives.
///
/// Never deletes files under the project source root. If `root` is no longer registered but
/// `state.json` still records that `root_path`, the leftover `projects/<id>/` directory is removed.
pub fn gc_project(home: &Path, root: &Path, all_lives: &LiveKeySet, cas: &Cas) -> GcReport {
    let mut report = GcReport::default();
    let expanded = expand_root(root);
    let pid = project_id(&expanded);

    if project_is_registered(home, &expanded) {
        report.merge(compact_and_drop_prev(home, &pid));
    } else if state_root_matches(home, &pid, &expanded) {
        let dir = project_dir(home, &pid);
        if dir.exists() {
            match fs::remove_dir_all(&dir) {
                Ok(()) => report.projects_removed += 1,
                Err(_) => report.errors += 1,
            }
        }
    }

    report.merge(gc_unreferenced(cas, all_lives));
    report
}

/// Compact every registered project, drop leftover `projects/<id>/` after `rm`, then orphan CAS.
pub fn gc_all(home: &Path, cas: &Cas) -> GcReport {
    let mut report = GcReport::default();
    let live = collect_live_keys(home, cas).unwrap_or_default();
    let registered = registered_project_ids(home);

    for pid in &registered {
        report.merge(compact_and_drop_prev(home, pid));
    }
    for pid in list_on_disk_project_ids(home) {
        if registered.contains(&pid) {
            continue;
        }
        let dir = project_dir(home, &pid);
        if dir.exists() {
            match fs::remove_dir_all(&dir) {
                Ok(()) => report.projects_removed += 1,
                Err(_) => report.errors += 1,
            }
        }
    }
    report.merge(gc_unreferenced(cas, &live));
    report
}

/// Union of extract/embed keys still referenced by **registered** projects.
pub fn collect_live_keys(home: &Path, cas: &Cas) -> Result<LiveKeySet, VaneCliError> {
    let cfg = load_config(home)?;
    let mut keys = LiveKeySet::default();
    for proj in &cfg.projects {
        let root = expand_root(&proj.path);
        let pid = project_id(&root);
        let live = LiveSet::load_for_project(home, &pid)?;
        let state = ProjectState::load(&state_path(home, &pid)).unwrap_or_default();
        let model_id = state.embed_model_id.as_deref();
        for file in live.files.values() {
            keys.insert_extract(file.extract_key.clone());
            if let Some(docs) = cas.get_extract(&file.extract_key) {
                if let Some(model) = model_id {
                    for doc in docs {
                        keys.insert_embed(embed_key(&doc.text, model));
                    }
                }
            }
        }
    }
    keys.expand_from_cas(cas);
    Ok(keys)
}

fn sweep(cas: &Cas, live: &LiveKeySet, ttl: Option<(u64, u64)>) -> GcReport {
    let mut report = GcReport::default();
    let mut cascade = BTreeSet::new();

    for key in cas.list_extract_keys() {
        if live.contains_extract(&key) {
            continue;
        }
        if let Some((now, retain_secs)) = ttl {
            if !age_exceeded(cas.last_seen(&key), now, retain_secs) {
                continue;
            }
        }
        for vk in cas.stored_embed_keys(&key) {
            cascade.insert(vk);
        }
        match cas.delete_extract(&key) {
            Ok(()) => report.extract_deleted += 1,
            Err(_) => report.errors += 1,
        }
    }

    for key in cas.list_embed_keys() {
        if live.contains_embed(&key) {
            continue;
        }
        if let Some((now, retain_secs)) = ttl {
            if !cascade.contains(&key) && !age_exceeded(cas.last_seen(&key), now, retain_secs) {
                continue;
            }
        }
        match cas.delete_embed(&key) {
            Ok(()) => report.embed_deleted += 1,
            Err(_) => report.errors += 1,
        }
    }
    report
}

fn age_exceeded(last_seen: Option<u64>, now: u64, retain_secs: u64) -> bool {
    // No last_seen means we cannot prove the object is older than retain_days.
    match last_seen {
        Some(seen) => now.saturating_sub(seen) > retain_secs,
        None => false,
    }
}

fn compact_and_drop_prev(home: &Path, project_id: &str) -> GcReport {
    let mut report = GcReport::default();
    let prev = project_db_prev_path(home, project_id);
    if prev.exists() {
        match fs::remove_dir_all(&prev) {
            Ok(()) => report.db_prev_removed += 1,
            Err(_) => report.errors += 1,
        }
    }

    let db = project_db_path(home, project_id);
    if !db.exists() {
        return report;
    }
    let state = match ProjectState::load(&state_path(home, project_id)) {
        Ok(s) => s,
        Err(_) => return report,
    };
    let (Some(dim), Some(model_id)) = (state.dim, state.embed_model_id.as_deref()) else {
        return report;
    };
    let prefer_cjk = state.tokenizer_fallback.as_deref() == Some("cjk_bigram");
    match open_or_create_at(&db, dim, model_id, prefer_cjk) {
        Ok(idx) => {
            match idx.compact() {
                Ok(()) => report.compacted += 1,
                Err(_) => report.errors += 1,
            }
            let _ = idx.close();
        }
        Err(_) => report.errors += 1,
    }
    report
}

fn project_is_registered(home: &Path, root: &Path) -> bool {
    let Ok(cfg) = load_config(home) else {
        return false;
    };
    let want = expand_root(root);
    cfg.projects.iter().any(|p| expand_root(&p.path) == want)
}

fn registered_project_ids(home: &Path) -> BTreeSet<String> {
    let Ok(cfg) = load_config(home) else {
        return BTreeSet::new();
    };
    cfg.projects
        .iter()
        .map(|p| project_id(&expand_root(&p.path)))
        .collect()
}

fn list_on_disk_project_ids(home: &Path) -> Vec<String> {
    let dir = home.join("rag").join("projects");
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        if let Some(name) = entry.file_name().to_str() {
            if !name.starts_with('.') {
                out.push(name.to_string());
            }
        }
    }
    out
}

fn state_root_matches(home: &Path, project_id: &str, root: &Path) -> bool {
    let Ok(state) = ProjectState::load(&state_path(home, project_id)) else {
        return false;
    };
    let Some(stored) = state.root_path.as_deref() else {
        return false;
    };
    expand_root(Path::new(stored)) == expand_root(root)
}

fn expand_root(path: &Path) -> PathBuf {
    let expanded = expand_tilde(path);
    expanded.canonicalize().unwrap_or(expanded)
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
