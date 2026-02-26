use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct DiskStats {
    pub home_bytes: u64,
    pub cas_bytes: u64,
    pub projects: Vec<ProjectDisk>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct ProjectDisk {
    pub project_id: String,
    pub db_bytes: u64,
}

pub fn resolve_home(
    cli_home: Option<&Path>,
    env_vane_home: Option<&OsStr>,
    fallback_home: &Path,
) -> PathBuf {
    if let Some(p) = cli_home {
        return p.to_path_buf();
    }
    if let Some(p) = env_vane_home {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    fallback_home.to_path_buf()
}

pub fn default_fallback() -> PathBuf {
    dirs_next_or_home().join(".vane")
}

fn dirs_next_or_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// On-read sizes under `$VANE_HOME`. Does not write a cache file.
pub fn disk_stats(home: &Path) -> DiskStats {
    let home_bytes = dir_bytes(home);
    let cas_bytes = dir_bytes(&home.join("rag").join("cas"));
    let mut projects = Vec::new();
    let proj_root = home.join("rag").join("projects");
    if let Ok(entries) = fs::read_dir(&proj_root) {
        for ent in entries.flatten() {
            let Ok(ft) = ent.file_type() else {
                continue;
            };
            if !ft.is_dir() {
                continue;
            }
            let name = ent.file_name();
            let Some(id) = name.to_str() else {
                continue;
            };
            if id.is_empty() || id.starts_with('.') {
                continue;
            }
            projects.push(ProjectDisk {
                project_id: id.to_string(),
                db_bytes: dir_bytes(&ent.path().join("db")),
            });
        }
    }
    projects.sort_by(|a, b| a.project_id.cmp(&b.project_id));
    DiskStats {
        home_bytes,
        cas_bytes,
        projects,
    }
}

fn dir_bytes(path: &Path) -> u64 {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return 0;
    };
    if meta.file_type().is_symlink() {
        return 0;
    }
    if meta.is_file() {
        return meta.len();
    }
    if !meta.is_dir() {
        return 0;
    }
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    let mut total = 0u64;
    for ent in entries.flatten() {
        total = total.saturating_add(dir_bytes(&ent.path()));
    }
    total
}
