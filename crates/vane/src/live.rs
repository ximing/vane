use std::collections::BTreeMap;
use std::fs;
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
        crate::fsutil::atomic_write(path, &payload, "live.json")
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

fn io_err(op: &str, path: &Path, err: std::io::Error) -> VaneCliError {
    VaneCliError::new(format!("{op} {}: {err}", path.display()))
}
