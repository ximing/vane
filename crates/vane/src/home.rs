use std::ffi::OsStr;
use std::path::{Path, PathBuf};

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
