use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::VaneCliError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveFile {
    pub content_sha256: String,
    pub extract_key: String,
    pub chunk_count: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveSet {
    #[serde(default)]
    pub files: BTreeMap<String, LiveFile>,
}

impl LiveSet {
    pub fn load(path: &Path) -> Result<Self, VaneCliError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let bytes = fs::read(path).map_err(|e| io_err("read live.json", path, e))?;
        serde_json::from_slice(&bytes)
            .map_err(|e| VaneCliError::new(format!("parse {}: {e}", path.display())))
    }

    pub fn save_atomic(&self, path: &Path) -> Result<(), VaneCliError> {
        let payload = serde_json::to_vec(self)
            .map_err(|e| VaneCliError::new(format!("serialize live.json: {e}")))?;
        atomic_write(path, &payload)
    }

    pub fn load_for_project(home: &Path, project_id: &str) -> Result<Self, VaneCliError> {
        Self::load(&live_path(home, project_id))
    }

    pub fn save_for_project(&self, home: &Path, project_id: &str) -> Result<(), VaneCliError> {
        self.save_atomic(&live_path(home, project_id))
    }
}

pub fn live_path(home: &Path, project_id: &str) -> PathBuf {
    home.join("rag")
        .join("projects")
        .join(project_id)
        .join("live.json")
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), VaneCliError> {
    let dir = path.parent().ok_or_else(|| {
        VaneCliError::new(format!("live.json path has no parent: {}", path.display()))
    })?;
    fs::create_dir_all(dir).map_err(|e| io_err("create live.json parent", dir, e))?;
    let tmp = dir.join(format!(
        "{}.tmp",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("live.json")
    ));
    {
        let mut f = File::create(&tmp).map_err(|e| io_err("create live.json temp", &tmp, e))?;
        f.write_all(bytes)
            .map_err(|e| io_err("write live.json temp", &tmp, e))?;
        f.sync_all()
            .map_err(|e| io_err("sync live.json temp", &tmp, e))?;
    }
    fs::rename(&tmp, path).map_err(|e| {
        VaneCliError::new(format!(
            "rename {} -> {}: {e}",
            tmp.display(),
            path.display()
        ))
    })
}

fn io_err(op: &str, path: &Path, err: std::io::Error) -> VaneCliError {
    VaneCliError::new(format!("{op} {}: {err}", path.display()))
}
