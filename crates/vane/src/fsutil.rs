use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::VaneCliError;

/// Atomic write: create parents, write `<name>.tmp` beside `path`, fsync, rename.
/// `label` only appears in error messages.
pub fn atomic_write(path: &Path, bytes: &[u8], label: &str) -> Result<(), VaneCliError> {
    let dir = path.parent().ok_or_else(|| {
        VaneCliError::new(format!("{label} path has no parent: {}", path.display()))
    })?;
    fs::create_dir_all(dir).map_err(|e| {
        VaneCliError::new(format!("create {} parent {}: {e}", label, dir.display()))
    })?;
    let tmp = dir.join(format!(
        "{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or(label)
    ));
    {
        let mut f = File::create(&tmp).map_err(|e| {
            VaneCliError::new(format!("create {} temp {}: {e}", label, tmp.display()))
        })?;
        f.write_all(bytes).map_err(|e| {
            VaneCliError::new(format!("write {} temp {}: {e}", label, tmp.display()))
        })?;
        f.sync_all().map_err(|e| {
            VaneCliError::new(format!("sync {} temp {}: {e}", label, tmp.display()))
        })?;
    }
    fs::rename(&tmp, path).map_err(|e| {
        VaneCliError::new(format!(
            "rename {} -> {}: {e}",
            tmp.display(),
            path.display()
        ))
    })
}

/// `~` / `~/…` expand to `$HOME`; everything else passes through unchanged.
pub fn expand_tilde(path: &Path) -> PathBuf {
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
