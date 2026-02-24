use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::VaneCliError;

pub fn dirty_path(home: &Path) -> PathBuf {
    home.join("rag").join("dirty.json")
}

pub struct DirtyItem {
    pub project_id: String,
    pub path: String,
}

struct DirtyEntry {
    next_due: Option<u64>,
    delay: u64,
}

#[derive(Serialize, Deserialize)]
struct DirtyFile {
    items: Vec<DirtyFileItem>,
}

#[derive(Serialize, Deserialize)]
struct DirtyFileItem {
    project_id: String,
    path: String,
    next_due: Option<u64>,
    delay: u64,
}

/// Retry queue. First retry is 1s after the first `pop_due`, then
/// the wait doubles up to 60s. Items stay queued until `clear`.
pub struct DirtyQueue {
    items: BTreeMap<(String, String), DirtyEntry>,
}

impl DirtyQueue {
    pub fn new() -> Self {
        Self {
            items: BTreeMap::new(),
        }
    }

    pub fn push(&mut self, project_id: impl Into<String>, path: impl Into<String>) {
        let key = (project_id.into(), path.into());
        self.items.entry(key).or_insert(DirtyEntry {
            next_due: None,
            delay: 1,
        });
    }

    pub fn pop_due(&mut self, now: u64) -> Vec<DirtyItem> {
        let mut due = Vec::new();
        for ((project_id, path), entry) in self.items.iter_mut() {
            match entry.next_due {
                None => {
                    entry.next_due = Some(now.saturating_add(entry.delay));
                }
                Some(t) if t <= now => {
                    due.push(DirtyItem {
                        project_id: project_id.clone(),
                        path: path.clone(),
                    });
                    entry.delay = entry.delay.saturating_mul(2).min(60);
                    entry.next_due = Some(now.saturating_add(entry.delay));
                }
                Some(_) => {}
            }
        }
        due
    }

    pub fn clear(&mut self, project_id: &str, path: &str) {
        self.items
            .remove(&(project_id.to_string(), path.to_string()));
    }

    pub fn load(path: &Path) -> Self {
        let Ok(bytes) = fs::read(path) else {
            return Self::new();
        };
        let Ok(file) = serde_json::from_slice::<DirtyFile>(&bytes) else {
            return Self::new();
        };
        let mut items = BTreeMap::new();
        for it in file.items {
            items.insert(
                (it.project_id, it.path),
                DirtyEntry {
                    next_due: it.next_due,
                    delay: it.delay.clamp(1, 60),
                },
            );
        }
        Self { items }
    }

    pub fn save(&self, path: &Path) -> Result<(), VaneCliError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| VaneCliError::new(format!("create {}: {e}", parent.display())))?;
        }
        let file = DirtyFile {
            items: self
                .items
                .iter()
                .map(|((project_id, p), entry)| DirtyFileItem {
                    project_id: project_id.clone(),
                    path: p.clone(),
                    next_due: entry.next_due,
                    delay: entry.delay,
                })
                .collect(),
        };
        let payload = serde_json::to_vec_pretty(&file)
            .map_err(|e| VaneCliError::new(format!("serialize dirty.json: {e}")))?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, payload)
            .map_err(|e| VaneCliError::new(format!("write {}: {e}", tmp.display())))?;
        fs::rename(&tmp, path)
            .map_err(|e| VaneCliError::new(format!("rename {}: {e}", tmp.display())))?;
        Ok(())
    }
}

impl Default for DirtyQueue {
    fn default() -> Self {
        Self::new()
    }
}
