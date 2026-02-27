use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::VaneCliError;
use crate::index::project_dir;
use crate::project::project_id;

pub const SKIPS_CAP: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressPhase {
    Scan,
    Extract,
    Embed,
    Flush,
    Idle,
}

impl ProgressPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Scan => "scan",
            Self::Extract => "extract",
            Self::Embed => "embed",
            Self::Flush => "flush",
            Self::Idle => "idle",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    pub project_id: String,
    pub root: String,
    pub phase: ProgressPhase,
    #[serde(default)]
    pub scanned: u64,
    #[serde(default)]
    pub total_estimate: u64,
    #[serde(default)]
    pub added: u64,
    #[serde(default)]
    pub embedded: u64,
    #[serde(default)]
    pub skipped: u64,
    #[serde(default)]
    pub updated_at: u64,
}

impl Progress {
    pub fn new(
        project_id: impl Into<String>,
        root: impl Into<String>,
        phase: ProgressPhase,
    ) -> Self {
        Self {
            project_id: project_id.into(),
            root: root.into(),
            phase,
            scanned: 0,
            total_estimate: 0,
            added: 0,
            embedded: 0,
            skipped: 0,
            updated_at: unix_now(),
        }
    }

    pub fn touch(&mut self) {
        self.updated_at = unix_now();
    }

    pub fn save(&self, home: &Path) -> Result<(), VaneCliError> {
        save_progress(home, self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipFileReason {
    TooLarge,
    InvalidUtf8,
    EmbedError,
    ExtractorUnsupported,
}

impl SkipFileReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TooLarge => "too_large",
            Self::InvalidUtf8 => "invalid_utf8",
            Self::EmbedError => "embed_error",
            Self::ExtractorUnsupported => "extractor_unsupported",
        }
    }
}

impl std::fmt::Display for SkipFileReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkipFile {
    pub path: String,
    pub reason: SkipFileReason,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub at: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkipLog {
    #[serde(default)]
    pub files: Vec<SkipFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IssuesReport {
    pub roots: Vec<RootIssues>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RootIssues {
    pub path: String,
    pub project_id: String,
    pub files: Vec<SkipFile>,
}

pub fn progress_path(home: &Path) -> PathBuf {
    home.join("run").join("progress.json")
}

pub fn skips_path(home: &Path, project_id: &str) -> PathBuf {
    project_dir(home, project_id).join("skips.json")
}

pub fn load_progress(home: &Path) -> Option<Progress> {
    let path = progress_path(home);
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub fn save_progress(home: &Path, progress: &Progress) -> Result<(), VaneCliError> {
    let path = progress_path(home);
    let payload = serde_json::to_vec_pretty(progress)
        .map_err(|e| VaneCliError::new(format!("serialize progress.json: {e}")))?;
    atomic_write(&path, &payload, "progress.json")
}

pub fn load_skips(home: &Path, project_id: &str) -> SkipLog {
    let path = skips_path(home, project_id);
    let Ok(bytes) = fs::read(&path) else {
        return SkipLog::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

pub fn skip_count(home: &Path, project_id: &str) -> Option<u64> {
    let path = skips_path(home, project_id);
    if !path.is_file() {
        return None;
    }
    Some(load_skips(home, project_id).files.len() as u64)
}

pub fn persist_skips(
    home: &Path,
    project_id: &str,
    new_files: Vec<SkipFile>,
    resolved_paths: &[String],
) -> Result<(), VaneCliError> {
    let path = skips_path(home, project_id);
    let mut log = load_skips(home, project_id);
    if new_files.is_empty() && log.files.is_empty() {
        return Ok(());
    }
    log.files
        .retain(|f| !resolved_paths.iter().any(|p| p == &f.path));
    for skip in new_files {
        log.files.retain(|f| f.path != skip.path);
        log.files.push(skip);
    }
    if log.files.len() > SKIPS_CAP {
        let drain = log.files.len() - SKIPS_CAP;
        log.files.drain(..drain);
    }
    if log.files.is_empty() {
        if path.is_file() {
            fs::remove_file(&path)
                .map_err(|e| VaneCliError::new(format!("remove {}: {e}", path.display())))?;
        }
        return Ok(());
    }
    let payload = serde_json::to_vec_pretty(&log)
        .map_err(|e| VaneCliError::new(format!("serialize skips.json: {e}")))?;
    atomic_write(&path, &payload, "skips.json")
}

pub fn skip_file(
    path: impl Into<String>,
    reason: SkipFileReason,
    detail: impl Into<String>,
) -> SkipFile {
    SkipFile {
        path: path.into(),
        reason,
        detail: sanitize_detail(&detail.into()),
        at: unix_now(),
    }
}

pub fn issues_report(home: &Path, roots: &[PathBuf]) -> IssuesReport {
    let mut out = Vec::with_capacity(roots.len());
    for root in roots {
        let stored = root.display().to_string();
        let for_id = root.canonicalize().unwrap_or_else(|_| root.clone());
        let pid = project_id(&for_id);
        out.push(RootIssues {
            path: stored,
            project_id: pid.clone(),
            files: load_skips(home, &pid).files,
        });
    }
    IssuesReport { roots: out }
}

pub fn spinner_message(progress: &Progress) -> String {
    format!(
        "{} {} {}/{} +{} skip {}",
        progress.phase.as_str(),
        progress.root,
        progress.scanned,
        progress.total_estimate,
        progress.added,
        progress.skipped
    )
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn sanitize_detail(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("sk-proj-") {
            let n = secret_token_len(after);
            out.push_str("sk-proj-***");
            rest = &after[n..];
        } else if let Some(after) = rest.strip_prefix("sk-") {
            let n = secret_token_len(after);
            out.push_str("sk-***");
            rest = &after[n..];
        } else if let Some(after) = rest.strip_prefix("api_key") {
            out.push_str("api_key");
            rest = after.trim_start_matches([' ', '=', ':', '"', '\'']);
            if rest.len() < after.len() {
                out.push_str("=***");
                let n = secret_token_len(rest);
                rest = &rest[n..];
            }
        } else {
            let ch = rest.chars().next().expect("rest non-empty");
            out.push(ch);
            rest = &rest[ch.len_utf8()..];
        }
    }
    out
}

fn secret_token_len(s: &str) -> usize {
    s.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
        .unwrap_or(s.len())
}

fn atomic_write(path: &Path, bytes: &[u8], label: &str) -> Result<(), VaneCliError> {
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
