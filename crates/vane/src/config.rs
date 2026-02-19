use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::VaneCliError;

const DEFAULT_EMBED_PROVIDER: &str = "ollama";
const DEFAULT_EMBED_MODEL: &str = "nomic-embed-text";
const DEFAULT_EMBED_BASE_URL: &str = "http://127.0.0.1:11434";
const DEFAULT_CHUNK_SPLIT: &str = "markdown";
const DEFAULT_MAX_CHARS: u32 = 1200;
const DEFAULT_OVERLAP_CHARS: u32 = 200;
const DEFAULT_MIN_CHARS: u32 = 50;
const DEFAULT_RERANK_PROVIDER: &str = "none";
const DEFAULT_LOG_RETAIN_DAYS: u32 = 3;
const DEFAULT_CAS_RETAIN_DAYS: u32 = 365;

const DEFAULT_EXCLUDE: &[&str] = &[
    "**/.git/**",
    "**/node_modules/**",
    "**/target/**",
    "**/dist/**",
    "**/.venv/**",
    "**/*.log",
    "**/*.lock",
    "**/package-lock.json",
    "**/pnpm-lock.yaml",
    "**/*.min.js",
    "**/*.map",
    "**/.DS_Store",
    "**/.env",
    "**/.env.*",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub defaults: Defaults,
    pub exclude: Vec<String>,
    pub types: Vec<TypeRule>,
    pub projects: Vec<ProjectEntry>,
    pub log: LogConfig,
    pub gc: GcConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Defaults {
    pub embed: EmbedConfig,
    pub chunk: ChunkConfig,
    pub rerank: RerankConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbedConfig {
    pub provider: String,
    pub model: String,
    pub base_url: String,
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkConfig {
    pub split: String,
    pub max_chars: u32,
    pub overlap_chars: u32,
    pub min_chars: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RerankConfig {
    pub provider: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogConfig {
    pub retain_days: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcConfig {
    pub cas_retain_days: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TypeRule {
    pub glob: String,
    pub extractor: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct EmbedOverlay {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct ChunkOverlay {
    pub split: Option<String>,
    pub max_chars: Option<u32>,
    pub overlap_chars: Option<u32>,
    pub min_chars: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectEntry {
    pub path: PathBuf,
    pub exclude: Vec<String>,
    pub include: Option<Vec<String>>,
    pub types: Option<Vec<TypeRule>>,
    pub embed: Option<EmbedOverlay>,
    pub chunk: Option<ChunkOverlay>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectFile {
    pub exclude: Vec<String>,
    pub include: Option<Vec<String>>,
    pub types: Option<Vec<TypeRule>>,
    pub embed: Option<EmbedOverlay>,
    pub chunk: Option<ChunkOverlay>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPolicy {
    pub embed: EmbedConfig,
    pub chunk: ChunkConfig,
    pub exclude: Vec<String>,
    pub types: Vec<TypeRule>,
}

#[derive(Deserialize, Default)]
struct RawConfig {
    #[serde(default)]
    defaults: RawDefaults,
    #[serde(default)]
    exclude: Option<Vec<String>>,
    #[serde(default)]
    include: Option<Vec<String>>,
    #[serde(default)]
    types: Option<Vec<TypeRule>>,
    #[serde(default)]
    projects: Vec<RawProjectEntry>,
    #[serde(default)]
    log: RawLog,
    #[serde(default)]
    gc: RawGc,
}

#[derive(Deserialize, Default)]
struct RawDefaults {
    #[serde(default)]
    embed: EmbedOverlay,
    #[serde(default)]
    chunk: ChunkOverlay,
    #[serde(default)]
    rerank: RawRerank,
}

#[derive(Deserialize, Default)]
struct RawRerank {
    provider: Option<String>,
}

#[derive(Deserialize, Default)]
struct RawLog {
    retain_days: Option<u32>,
}

#[derive(Deserialize, Default)]
struct RawGc {
    cas_retain_days: Option<u32>,
}

#[derive(Deserialize, Default)]
struct RawProjectEntry {
    path: String,
    #[serde(default)]
    exclude: Vec<String>,
    #[serde(default)]
    include: Option<Vec<String>>,
    #[serde(default)]
    types: Option<Vec<TypeRule>>,
    #[serde(default)]
    embed: Option<EmbedOverlay>,
    #[serde(default)]
    chunk: Option<ChunkOverlay>,
}

#[derive(Deserialize, Default)]
struct RawProjectFile {
    #[serde(default)]
    exclude: Vec<String>,
    include: Option<Vec<String>>,
    types: Option<Vec<TypeRule>>,
    embed: Option<EmbedOverlay>,
    chunk: Option<ChunkOverlay>,
}

fn default_exclude() -> Vec<String> {
    DEFAULT_EXCLUDE.iter().map(|s| (*s).to_string()).collect()
}

fn default_types() -> Vec<TypeRule> {
    vec![
        TypeRule {
            glob: "**/*.{md,mdx,txt,rst,org,html}".into(),
            extractor: "text".into(),
            enabled: true,
        },
        TypeRule {
            glob: "**/*.{png,jpg,jpeg,webp,gif}".into(),
            extractor: "image".into(),
            enabled: false,
        },
    ]
}

fn default_embed() -> EmbedConfig {
    EmbedConfig {
        provider: DEFAULT_EMBED_PROVIDER.into(),
        model: DEFAULT_EMBED_MODEL.into(),
        base_url: DEFAULT_EMBED_BASE_URL.into(),
        api_key: None,
    }
}

fn default_chunk() -> ChunkConfig {
    ChunkConfig {
        split: DEFAULT_CHUNK_SPLIT.into(),
        max_chars: DEFAULT_MAX_CHARS,
        overlap_chars: DEFAULT_OVERLAP_CHARS,
        min_chars: DEFAULT_MIN_CHARS,
    }
}

pub fn load_config(home: &Path) -> Result<Config, VaneCliError> {
    let path = home.join("config").join("config.toml");
    if !path.is_file() {
        return Err(VaneCliError::new(format!(
            "not initialized: missing {}",
            path.display()
        )));
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|e| VaneCliError::new(format!("failed to read {}: {e}", path.display())))?;
    let raw: RawConfig = toml::from_str(&text)
        .map_err(|e| VaneCliError::new(format!("invalid config {}: {e}", path.display())))?;

    let embed = overlay_embed(&default_embed(), &raw.defaults.embed);
    let chunk = overlay_chunk(&default_chunk(), &raw.defaults.chunk);
    validate_chunk(&chunk)?;

    let retain_days = raw.log.retain_days.unwrap_or(DEFAULT_LOG_RETAIN_DAYS);
    if retain_days < 1 {
        return Err(VaneCliError::new(format!(
            "log.retain_days must be >= 1, got {retain_days}"
        )));
    }
    let cas_retain_days = raw.gc.cas_retain_days.unwrap_or(DEFAULT_CAS_RETAIN_DAYS);
    if cas_retain_days < 1 {
        return Err(VaneCliError::new(format!(
            "gc.cas_retain_days must be >= 1, got {cas_retain_days}"
        )));
    }

    let exclude = raw.exclude.unwrap_or_else(default_exclude);
    let types = resolve_types(raw.types, raw.include.as_deref(), default_types());

    let projects = raw
        .projects
        .into_iter()
        .map(|p| {
            let entry = ProjectEntry {
                path: PathBuf::from(p.path),
                exclude: p.exclude,
                include: p.include,
                types: p.types,
                embed: p.embed,
                chunk: p.chunk,
            };
            let merged_chunk = match &entry.chunk {
                Some(over) => overlay_chunk(&chunk, over),
                None => chunk.clone(),
            };
            validate_chunk(&merged_chunk)?;
            Ok(entry)
        })
        .collect::<Result<Vec<_>, VaneCliError>>()?;

    Ok(Config {
        defaults: Defaults {
            embed,
            chunk,
            rerank: RerankConfig {
                provider: raw
                    .defaults
                    .rerank
                    .provider
                    .unwrap_or_else(|| DEFAULT_RERANK_PROVIDER.into()),
            },
        },
        exclude,
        types,
        projects,
        log: LogConfig { retain_days },
        gc: GcConfig { cas_retain_days },
    })
}

pub fn resolve_policy(
    cfg: &Config,
    root: &Path,
    project_file: Option<&ProjectFile>,
) -> Result<ResolvedPolicy, VaneCliError> {
    let mut embed = cfg.defaults.embed.clone();
    let mut chunk = cfg.defaults.chunk.clone();
    let mut exclude = cfg.exclude.clone();
    let mut types = cfg.types.clone();

    if let Some(entry) = matching_project(cfg, root) {
        exclude = union_exclude(&exclude, &entry.exclude);
        types = resolve_types(entry.types.clone(), entry.include.as_deref(), types);
        if let Some(over) = &entry.embed {
            embed = overlay_embed(&embed, over);
        }
        if let Some(over) = &entry.chunk {
            chunk = overlay_chunk(&chunk, over);
        }
    }

    if let Some(pf) = project_file {
        exclude = union_exclude(&exclude, &pf.exclude);
        types = resolve_types(pf.types.clone(), pf.include.as_deref(), types);
        if let Some(over) = &pf.embed {
            embed = overlay_embed(&embed, over);
        }
        if let Some(over) = &pf.chunk {
            chunk = overlay_chunk(&chunk, over);
        }
    }

    validate_chunk(&chunk)?;
    Ok(ResolvedPolicy {
        embed,
        chunk,
        exclude,
        types,
    })
}

impl ProjectFile {
    pub fn parse_toml(s: &str) -> Result<Self, VaneCliError> {
        let value: toml::Value = toml::from_str(s)
            .map_err(|e| VaneCliError::new(format!("invalid project file: {e}")))?;
        if contains_key_named(&value, "api_key") {
            return Err(VaneCliError::new(
                "project file must not contain api_key (use global config or OPENAI_API_KEY / VANE_EMBED_API_KEY)",
            ));
        }
        let raw: RawProjectFile = value
            .try_into()
            .map_err(|e| VaneCliError::new(format!("invalid project file: {e}")))?;
        Ok(ProjectFile {
            exclude: raw.exclude,
            include: raw.include,
            types: raw.types,
            embed: raw.embed,
            chunk: raw.chunk,
        })
    }
}

fn matching_project<'a>(cfg: &'a Config, root: &Path) -> Option<&'a ProjectEntry> {
    cfg.projects.iter().find(|p| paths_match(&p.path, root))
}

fn paths_match(a: &Path, b: &Path) -> bool {
    expand_tilde(a)
        .components()
        .eq(expand_tilde(b).components())
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

fn resolve_types(
    types: Option<Vec<TypeRule>>,
    include: Option<&[String]>,
    inherited: Vec<TypeRule>,
) -> Vec<TypeRule> {
    if let Some(types) = types {
        types
    } else if let Some(include) = include {
        types_from_include(include)
    } else {
        inherited
    }
}

fn types_from_include(include: &[String]) -> Vec<TypeRule> {
    include
        .iter()
        .map(|glob| TypeRule {
            glob: glob.clone(),
            extractor: "text".into(),
            enabled: true,
        })
        .collect()
}

fn union_exclude(base: &[String], extra: &[String]) -> Vec<String> {
    let mut out = base.to_vec();
    for e in extra {
        if !out.iter().any(|x| x == e) {
            out.push(e.clone());
        }
    }
    out
}

fn overlay_embed(base: &EmbedConfig, over: &EmbedOverlay) -> EmbedConfig {
    EmbedConfig {
        provider: over
            .provider
            .clone()
            .unwrap_or_else(|| base.provider.clone()),
        model: over.model.clone().unwrap_or_else(|| base.model.clone()),
        base_url: over
            .base_url
            .clone()
            .unwrap_or_else(|| base.base_url.clone()),
        api_key: over.api_key.clone().or_else(|| base.api_key.clone()),
    }
}

fn overlay_chunk(base: &ChunkConfig, over: &ChunkOverlay) -> ChunkConfig {
    ChunkConfig {
        split: over.split.clone().unwrap_or_else(|| base.split.clone()),
        max_chars: over.max_chars.unwrap_or(base.max_chars),
        overlap_chars: over.overlap_chars.unwrap_or(base.overlap_chars),
        min_chars: over.min_chars.unwrap_or(base.min_chars),
    }
}

fn validate_chunk(chunk: &ChunkConfig) -> Result<(), VaneCliError> {
    if chunk.overlap_chars >= chunk.max_chars {
        return Err(VaneCliError::new(format!(
            "invalid chunk: overlap_chars ({}) must be < max_chars ({})",
            chunk.overlap_chars, chunk.max_chars
        )));
    }
    if chunk.min_chars > chunk.max_chars {
        return Err(VaneCliError::new(format!(
            "invalid chunk: min_chars ({}) must be <= max_chars ({})",
            chunk.min_chars, chunk.max_chars
        )));
    }
    match chunk.split.as_str() {
        "markdown" | "plain" => Ok(()),
        other => Err(VaneCliError::new(format!(
            "invalid chunk split {other:?}, expected markdown or plain"
        ))),
    }
}

fn contains_key_named(v: &toml::Value, name: &str) -> bool {
    match v {
        toml::Value::Table(t) => {
            t.contains_key(name) || t.values().any(|child| contains_key_named(child, name))
        }
        toml::Value::Array(a) => a.iter().any(|child| contains_key_named(child, name)),
        _ => false,
    }
}
