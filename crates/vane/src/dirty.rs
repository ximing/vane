use std::collections::BTreeMap;

pub struct DirtyItem {
    pub project_id: String,
    pub path: String,
}

struct DirtyEntry {
    next_due: Option<u64>,
    delay: u64,
}

/// In-memory retry queue. First retry is 1s after the first `pop_due`, then
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
}

impl Default for DirtyQueue {
    fn default() -> Self {
        Self::new()
    }
}
