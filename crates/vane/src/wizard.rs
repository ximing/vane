use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};

use crate::config::{
    default_chunk, default_embed, default_exclude, default_types, ChunkConfig, Config, EmbedConfig,
};
use crate::error::VaneCliError;
use crate::service::install_user_service;

#[derive(Debug, Clone)]
pub struct InitAnswers {
    pub provider: String,
    pub model: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub dim: Option<u32>,
    pub split: String,
    pub max_chars: u32,
    pub overlap_chars: u32,
    pub min_chars: u32,
    pub first_root: Option<PathBuf>,
    pub exclude: Vec<String>,
    pub images: bool,
    pub install_service: bool,
}

impl Default for InitAnswers {
    fn default() -> Self {
        let embed = default_embed();
        let chunk = default_chunk();
        Self {
            provider: embed.provider,
            model: embed.model,
            base_url: embed.base_url,
            api_key: None,
            dim: None,
            split: chunk.split,
            max_chars: chunk.max_chars,
            overlap_chars: chunk.overlap_chars,
            min_chars: chunk.min_chars,
            first_root: None,
            exclude: default_exclude(),
            images: false,
            install_service: true,
        }
    }
}

pub fn run_init<R, W>(
    home: &Path,
    stdin: R,
    mut stdout: W,
    assume: Option<InitAnswers>,
) -> Result<(), VaneCliError>
where
    R: Read,
    W: Write,
{
    let cfg_path = home.join("config").join("config.toml");
    let existing = crate::config::load_config(home).ok();
    let answers = match assume {
        Some(a) => a,
        None => {
            let mut reader = BufReader::new(stdin);
            prompt_answers(&mut reader, &mut stdout, existing.as_ref())?
        }
    };
    validate_provider(&answers.provider)?;
    write_config_from_answers(home, &answers)?;
    let _ = writeln!(stdout, "wrote {}", cfg_path.display());

    if answers.install_service {
        let bin = std::env::current_exe()
            .map_err(|e| VaneCliError::new(format!("cannot resolve vane binary: {e}")))?;
        install_user_service(home, &bin)?;
        let _ = writeln!(stdout, "installed user service");
    }
    Ok(())
}

fn validate_provider(provider: &str) -> Result<(), VaneCliError> {
    match provider {
        "ollama" | "openai_compat" => Ok(()),
        other => Err(VaneCliError::new(format!(
            "unknown embed provider {other:?}, expected ollama or openai_compat"
        ))),
    }
}

fn prompt_answers<R, W>(
    stdin: &mut R,
    stdout: &mut W,
    existing: Option<&Config>,
) -> Result<InitAnswers, VaneCliError>
where
    R: BufRead,
    W: Write,
{
    if existing.is_some() {
        let _ = writeln!(
            stdout,
            "already initialized — empty answers keep the current value"
        );
    }
    let def = existing
        .map(|c| c.defaults.embed.clone())
        .unwrap_or_else(default_embed);
    let chunk_def = existing
        .map(|c| c.defaults.chunk.clone())
        .unwrap_or_else(default_chunk);
    let provider = prompt(
        stdin,
        stdout,
        "Embedding provider (ollama / openai_compat)",
        &def.provider,
    )?;
    validate_provider(&provider)?;
    let (default_model, default_url) = if provider == def.provider {
        (def.model.clone(), def.base_url.clone())
    } else if provider == "openai_compat" {
        (
            "text-embedding-3-small".to_string(),
            "https://api.openai.com/v1".to_string(),
        )
    } else {
        let fallback = default_embed();
        (fallback.model, fallback.base_url)
    };
    let model = prompt(stdin, stdout, "Model", &default_model)?;
    let base_url = prompt(stdin, stdout, "Base URL", &default_url)?;
    let api_key = if provider == "openai_compat" {
        prompt_api_key(stdin, stdout)?
    } else {
        None
    };
    let dim = prompt_dim(stdin, stdout, def.dim)?;
    let split = prompt(
        stdin,
        stdout,
        "Chunk split (markdown / plain)",
        &chunk_def.split,
    )?;
    if split != "markdown" && split != "plain" {
        return Err(VaneCliError::new(format!(
            "invalid chunk split {split:?}, expected markdown or plain"
        )));
    }
    let max_chars = prompt_u32(stdin, stdout, "Chunk max_chars", chunk_def.max_chars)?;
    let overlap_chars = prompt_u32(
        stdin,
        stdout,
        "Chunk overlap_chars",
        chunk_def.overlap_chars,
    )?;
    let min_chars = prompt_u32(stdin, stdout, "Chunk min_chars", chunk_def.min_chars)?;

    let embed = EmbedConfig {
        provider: provider.clone(),
        model: model.clone(),
        base_url: base_url.clone(),
        api_key: api_key.clone(),
        dim,
    };
    if provider == "openai_compat" && embed.api_key.is_none() && !env_embed_api_key_set() {
        let _ = writeln!(
            stdout,
            "warning: no API key; probe will likely 401. Enter a key, or export OPENAI_API_KEY / VANE_EMBED_API_KEY"
        );
    }
    match crate::embed::embedder_from_config(&embed).probe_dim() {
        Ok(dim) => {
            let _ = writeln!(stdout, "probe ok, dim={dim}");
        }
        Err(e) => {
            let _ = writeln!(stdout, "warning: embed probe failed ({e}); continuing");
        }
    }

    let root_s = prompt(stdin, stdout, "First project root (empty to skip)", "")?;
    let first_root = if root_s.is_empty() {
        None
    } else {
        Some(PathBuf::from(root_s))
    };

    let defaults = existing
        .map(|c| c.exclude.clone())
        .unwrap_or_else(default_exclude);
    let _ = writeln!(stdout, "Default excludes:");
    for (i, e) in defaults.iter().enumerate() {
        let _ = writeln!(stdout, "  [{}] {e}", i + 1);
    }
    let drop_s = prompt(
        stdin,
        stdout,
        "Numbers to uncheck (comma-separated, empty to keep all)",
        "",
    )?;
    let mut exclude = defaults;
    if !drop_s.is_empty() {
        let drop_idx: Vec<usize> = drop_s
            .split(',')
            .filter_map(|s| s.trim().parse::<usize>().ok())
            .filter(|n| *n >= 1)
            .map(|n| n - 1)
            .collect();
        exclude = exclude
            .into_iter()
            .enumerate()
            .filter(|(i, _)| !drop_idx.contains(i))
            .map(|(_, e)| e)
            .collect();
    }
    let extra = prompt(
        stdin,
        stdout,
        "Additional exclude glob or folder (empty to skip)",
        "",
    )?;
    if !extra.is_empty() {
        let extra_glob = folder_to_exclude_glob(&extra);
        if !exclude.iter().any(|e| e == &extra_glob) {
            exclude.push(extra_glob);
        }
    }

    let images_default = existing
        .map(|c| c.types.iter().any(|t| t.extractor == "image" && t.enabled))
        .unwrap_or(false);
    let images = prompt_yes_no(stdin, stdout, "Enable image types?", images_default)?;
    let install_service =
        prompt_yes_no(stdin, stdout, "Install user service?", existing.is_none())?;

    Ok(InitAnswers {
        provider,
        model,
        base_url,
        api_key: api_key.or_else(|| def.api_key.clone()),
        dim,
        split,
        max_chars,
        overlap_chars,
        min_chars,
        first_root,
        exclude,
        images,
        install_service,
    })
}

fn env_embed_api_key_set() -> bool {
    env_nonempty("OPENAI_API_KEY") || env_nonempty("VANE_EMBED_API_KEY")
}

fn env_nonempty(name: &str) -> bool {
    std::env::var(name).map(|v| !v.is_empty()).unwrap_or(false)
}

fn prompt_dim<R, W>(
    stdin: &mut R,
    stdout: &mut W,
    current: Option<u32>,
) -> Result<Option<u32>, VaneCliError>
where
    R: BufRead,
    W: Write,
{
    let default = current.map(|d| d.to_string()).unwrap_or_default();
    let raw = prompt(
        stdin,
        stdout,
        "Vector dimension (empty to probe from the API)",
        &default,
    )?;
    parse_dim(&raw)
}

fn prompt_u32<R, W>(
    stdin: &mut R,
    stdout: &mut W,
    label: &str,
    default: u32,
) -> Result<u32, VaneCliError>
where
    R: BufRead,
    W: Write,
{
    let raw = prompt(stdin, stdout, label, &default.to_string())?;
    raw.parse::<u32>()
        .map_err(|_| VaneCliError::new(format!("invalid {label}: {raw:?}")))
}

pub(crate) fn parse_dim(raw: &str) -> Result<Option<u32>, VaneCliError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let n: u32 = trimmed
        .parse()
        .map_err(|_| VaneCliError::new(format!("invalid vector dimension {trimmed:?}")))?;
    if n == 0 || n > 16_384 {
        return Err(VaneCliError::new(format!(
            "vector dimension must be 1..=16384, got {n}"
        )));
    }
    Ok(Some(n))
}

fn prompt_api_key<R, W>(stdin: &mut R, stdout: &mut W) -> Result<Option<String>, VaneCliError>
where
    R: BufRead,
    W: Write,
{
    let hint = if env_nonempty("OPENAI_API_KEY") {
        "empty keeps OPENAI_API_KEY"
    } else if env_nonempty("VANE_EMBED_API_KEY") {
        "empty keeps VANE_EMBED_API_KEY"
    } else {
        "empty uses OPENAI_API_KEY / VANE_EMBED_API_KEY"
    };
    let raw = prompt(stdin, stdout, &format!("API key ({hint})"), "")?;
    if raw.is_empty() {
        Ok(None)
    } else {
        Ok(Some(raw))
    }
}

fn folder_to_exclude_glob(s: &str) -> String {
    if s.contains('*') || s.contains('?') || s.contains('{') {
        return s.to_string();
    }
    let trimmed = s.trim().trim_matches('/');
    if trimmed.is_empty() {
        return s.to_string();
    }
    format!("**/{trimmed}/**")
}

fn prompt<R, W>(
    stdin: &mut R,
    stdout: &mut W,
    label: &str,
    default: &str,
) -> Result<String, VaneCliError>
where
    R: BufRead,
    W: Write,
{
    if default.is_empty() {
        write!(stdout, "{label}: ").map_err(io_err)?;
    } else {
        write!(stdout, "{label} [{default}]: ").map_err(io_err)?;
    }
    stdout.flush().map_err(io_err)?;
    let mut line = String::new();
    stdin.read_line(&mut line).map_err(io_err)?;
    let trimmed = line.trim();
    if trimmed.is_empty() {
        Ok(default.to_string())
    } else {
        Ok(trimmed.to_string())
    }
}

fn prompt_yes_no<R, W>(
    stdin: &mut R,
    stdout: &mut W,
    label: &str,
    default_yes: bool,
) -> Result<bool, VaneCliError>
where
    R: BufRead,
    W: Write,
{
    let hint = if default_yes { "Y/n" } else { "y/N" };
    let raw = prompt(stdin, stdout, &format!("{label} [{hint}]"), "")?;
    if raw.is_empty() {
        return Ok(default_yes);
    }
    match raw.to_ascii_lowercase().as_str() {
        "y" | "yes" => Ok(true),
        "n" | "no" => Ok(false),
        other => Err(VaneCliError::new(format!("expected y or n, got {other:?}"))),
    }
}

fn write_config_from_answers(home: &Path, answers: &InitAnswers) -> Result<(), VaneCliError> {
    let mut root = toml::map::Map::new();

    let mut embed = toml::map::Map::new();
    embed.insert(
        "provider".into(),
        toml::Value::String(answers.provider.clone()),
    );
    embed.insert("model".into(), toml::Value::String(answers.model.clone()));
    embed.insert(
        "base_url".into(),
        toml::Value::String(answers.base_url.clone()),
    );
    let keep_key = crate::config::load_config(home)
        .ok()
        .and_then(|c| c.defaults.embed.api_key);
    if let Some(key) = answers
        .api_key
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or(keep_key)
    {
        embed.insert("api_key".into(), toml::Value::String(key));
    }
    if let Some(dim) = answers.dim {
        embed.insert("dim".into(), toml::Value::Integer(i64::from(dim)));
    }

    let mut rerank = toml::map::Map::new();
    rerank.insert("provider".into(), toml::Value::String("none".into()));

    let mut chunk = toml::map::Map::new();
    chunk.insert("split".into(), toml::Value::String(answers.split.clone()));
    chunk.insert(
        "max_chars".into(),
        toml::Value::Integer(i64::from(answers.max_chars)),
    );
    chunk.insert(
        "overlap_chars".into(),
        toml::Value::Integer(i64::from(answers.overlap_chars)),
    );
    chunk.insert(
        "min_chars".into(),
        toml::Value::Integer(i64::from(answers.min_chars)),
    );

    let mut defaults = toml::map::Map::new();
    defaults.insert("embed".into(), toml::Value::Table(embed));
    defaults.insert("rerank".into(), toml::Value::Table(rerank));
    defaults.insert("chunk".into(), toml::Value::Table(chunk));
    root.insert("defaults".into(), toml::Value::Table(defaults));

    let mut log = toml::map::Map::new();
    log.insert("retain_days".into(), toml::Value::Integer(3));
    root.insert("log".into(), toml::Value::Table(log));

    let mut gc = toml::map::Map::new();
    gc.insert("cas_retain_days".into(), toml::Value::Integer(365));
    root.insert("gc".into(), toml::Value::Table(gc));

    root.insert(
        "exclude".into(),
        toml::Value::Array(
            answers
                .exclude
                .iter()
                .map(|e| toml::Value::String(e.clone()))
                .collect(),
        ),
    );

    let mut types = default_types();
    for t in &mut types {
        if t.extractor == "image" {
            t.enabled = answers.images;
        }
    }
    root.insert(
        "types".into(),
        toml::Value::Array(types.into_iter().map(type_rule_value).collect()),
    );

    let mut projects = load_existing_projects(home);
    if let Some(first) = &answers.first_root {
        let canon = normalize_root(first)?;
        let canon_s = canon.display().to_string();
        let already = projects.iter().any(|p| {
            p.get("path")
                .and_then(|v| v.as_str())
                .is_some_and(|s| s == canon_s)
        });
        if !already {
            let mut entry = toml::map::Map::new();
            entry.insert("path".into(), toml::Value::String(canon_s));
            projects.push(toml::Value::Table(entry));
        }
    }
    if !projects.is_empty() {
        root.insert("projects".into(), toml::Value::Array(projects));
    }

    let path = home.join("config").join("config.toml");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| VaneCliError::new(format!("create {}: {e}", parent.display())))?;
    }
    let body = toml::to_string_pretty(&toml::Value::Table(root))
        .map_err(|e| VaneCliError::new(format!("serialize config: {e}")))?;
    std::fs::write(&path, body)
        .map_err(|e| VaneCliError::new(format!("write {}: {e}", path.display())))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&path)
            .map_err(|e| VaneCliError::new(format!("stat {}: {e}", path.display())))?
            .permissions();
        perms.set_mode(0o600);
        std::fs::set_permissions(&path, perms)
            .map_err(|e| VaneCliError::new(format!("chmod {}: {e}", path.display())))?;
    }
    Ok(())
}

fn type_rule_value(t: crate::config::TypeRule) -> toml::Value {
    let mut m = toml::map::Map::new();
    m.insert("glob".into(), toml::Value::String(t.glob));
    m.insert("extractor".into(), toml::Value::String(t.extractor));
    m.insert("enabled".into(), toml::Value::Boolean(t.enabled));
    toml::Value::Table(m)
}

fn normalize_root(path: &Path) -> Result<PathBuf, VaneCliError> {
    let expanded = expand_tilde(path);
    expanded
        .canonicalize()
        .map_err(|e| VaneCliError::new(format!("canonicalize {}: {e}", expanded.display())))
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

fn load_existing_projects(home: &Path) -> Vec<toml::Value> {
    let path = home.join("config").join("config.toml");
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(value) = text.parse::<toml::Value>() else {
        return Vec::new();
    };
    value
        .get("projects")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
}

/// Write `<root>/.vane.toml` with chunk policy. Never writes `api_key`.
pub fn write_project_toml(
    root: &Path,
    chunk: &ChunkConfig,
    images: bool,
) -> Result<PathBuf, VaneCliError> {
    let mut chunk_t = toml::map::Map::new();
    chunk_t.insert("split".into(), toml::Value::String(chunk.split.clone()));
    chunk_t.insert(
        "max_chars".into(),
        toml::Value::Integer(i64::from(chunk.max_chars)),
    );
    chunk_t.insert(
        "overlap_chars".into(),
        toml::Value::Integer(i64::from(chunk.overlap_chars)),
    );
    chunk_t.insert(
        "min_chars".into(),
        toml::Value::Integer(i64::from(chunk.min_chars)),
    );
    let mut root_t = toml::map::Map::new();
    root_t.insert("chunk".into(), toml::Value::Table(chunk_t));
    let mut types = default_types();
    for t in &mut types {
        if t.extractor == "image" {
            t.enabled = images;
        }
    }
    root_t.insert(
        "types".into(),
        toml::Value::Array(types.into_iter().map(type_rule_value).collect()),
    );
    let path = root.join(".vane.toml");
    let body = toml::to_string_pretty(&toml::Value::Table(root_t))
        .map_err(|e| VaneCliError::new(format!("serialize {}: {e}", path.display())))?;
    std::fs::write(&path, body)
        .map_err(|e| VaneCliError::new(format!("write {}: {e}", path.display())))?;
    Ok(path)
}

pub struct ProjectSetup {
    pub chunk: ChunkConfig,
    pub images: bool,
    pub write_file: bool,
}

/// Interactive project policy for `vane add`. Falls back to line prompts when not a TTY.
pub fn prompt_project_setup<R, W>(
    stdin: &mut R,
    stdout: &mut W,
    global: &ChunkConfig,
) -> Result<ProjectSetup, VaneCliError>
where
    R: BufRead,
    W: Write,
{
    let write_file = prompt_yes_no(
        stdin,
        stdout,
        "Write .vane.toml in this repo (chunk / types)?",
        true,
    )?;
    if !write_file {
        return Ok(ProjectSetup {
            chunk: global.clone(),
            images: false,
            write_file: false,
        });
    }
    let split = prompt(
        stdin,
        stdout,
        "Chunk split (markdown / plain)",
        &global.split,
    )?;
    let max_chars = prompt_u32(stdin, stdout, "Chunk max_chars", global.max_chars)?;
    let overlap_chars = prompt_u32(stdin, stdout, "Chunk overlap_chars", global.overlap_chars)?;
    let min_chars = prompt_u32(stdin, stdout, "Chunk min_chars", global.min_chars)?;
    let images = prompt_yes_no(stdin, stdout, "Enable image types?", false)?;
    Ok(ProjectSetup {
        chunk: ChunkConfig {
            split,
            max_chars,
            overlap_chars,
            min_chars,
        },
        images,
        write_file: true,
    })
}

/// TTY wizard using cliclack. Not used by tests (they inject stdin).
pub fn run_init_tty(home: &Path) -> Result<(), VaneCliError> {
    let existing = crate::config::load_config(home).ok();
    let answers = prompt_answers_tty(existing.as_ref())?;
    validate_provider(&answers.provider)?;
    write_config_from_answers(home, &answers)?;
    cliclack::outro(format!(
        "wrote {}",
        home.join("config").join("config.toml").display()
    ))
    .map_err(clack_err)?;
    if answers.install_service {
        let bin = std::env::current_exe()
            .map_err(|e| VaneCliError::new(format!("cannot resolve vane binary: {e}")))?;
        install_user_service(home, &bin)?;
    }
    Ok(())
}

fn prompt_answers_tty(existing: Option<&Config>) -> Result<InitAnswers, VaneCliError> {
    let def = existing
        .map(|c| c.defaults.embed.clone())
        .unwrap_or_else(default_embed);
    let chunk_def = existing
        .map(|c| c.defaults.chunk.clone())
        .unwrap_or_else(default_chunk);
    cliclack::intro("Vane sidecar").map_err(clack_err)?;
    if existing.is_some() {
        cliclack::log::info("Already initialized — empty answers keep the current value")
            .map_err(clack_err)?;
    }
    let provider = cliclack::select("Embedding provider")
        .initial_value(if def.provider == "openai_compat" {
            "openai_compat"
        } else {
            "ollama"
        })
        .item("ollama", "ollama", "local Ollama")
        .item("openai_compat", "openai_compat", "OpenAI-compatible HTTP")
        .interact()
        .map_err(clack_err)?
        .to_string();
    let (fallback_model, fallback_url) = if provider == "openai_compat" {
        (
            if def.provider == "openai_compat" {
                def.model.clone()
            } else {
                "text-embedding-3-small".into()
            },
            if def.provider == "openai_compat" {
                def.base_url.clone()
            } else {
                "https://api.openai.com/v1".into()
            },
        )
    } else {
        (def.model.clone(), def.base_url.clone())
    };
    let model: String = cliclack::input("Model")
        .default_input(&fallback_model)
        .interact()
        .map_err(clack_err)?;
    let base_url: String = cliclack::input("Base URL")
        .default_input(&fallback_url)
        .interact()
        .map_err(clack_err)?;
    let api_key = if provider == "openai_compat" {
        let typed: String = cliclack::input("API key (empty keeps env / stored key)")
            .required(false)
            .interact()
            .map_err(clack_err)?;
        if typed.trim().is_empty() {
            def.api_key.clone()
        } else {
            Some(typed)
        }
    } else {
        None
    };
    let dim_default = def.dim.map(|d| d.to_string()).unwrap_or_default();
    let dim_raw: String = if dim_default.is_empty() {
        cliclack::input("Vector dimension (empty to probe)")
            .required(false)
            .interact()
            .map_err(clack_err)?
    } else {
        cliclack::input("Vector dimension (empty to probe)")
            .default_input(&dim_default)
            .required(false)
            .interact()
            .map_err(clack_err)?
    };
    let dim = parse_dim(&dim_raw)?;
    let split = cliclack::select("Chunk split")
        .initial_value(if chunk_def.split == "plain" {
            "plain"
        } else {
            "markdown"
        })
        .item("markdown", "markdown", "split on ATX/Setext headings")
        .item("plain", "plain", "ignore headings")
        .interact()
        .map_err(clack_err)?
        .to_string();
    let max_chars = clack_u32("Chunk max_chars", chunk_def.max_chars)?;
    let overlap_chars = clack_u32("Chunk overlap_chars", chunk_def.overlap_chars)?;
    let min_chars = clack_u32("Chunk min_chars", chunk_def.min_chars)?;

    let embed = EmbedConfig {
        provider: provider.clone(),
        model: model.clone(),
        base_url: base_url.clone(),
        api_key: api_key.clone(),
        dim,
    };
    match crate::embed::embedder_from_config(&embed).probe_dim() {
        Ok(d) => {
            cliclack::log::success(format!("probe ok, dim={d}")).map_err(clack_err)?;
        }
        Err(e) => {
            cliclack::log::warning(format!("embed probe failed ({e}); continuing"))
                .map_err(clack_err)?;
        }
    }

    let root_s: String = cliclack::input("First project root (empty to skip)")
        .required(false)
        .interact()
        .map_err(clack_err)?;
    let first_root = if root_s.trim().is_empty() {
        None
    } else {
        Some(PathBuf::from(root_s.trim()))
    };
    let images_default = existing
        .map(|c| c.types.iter().any(|t| t.extractor == "image" && t.enabled))
        .unwrap_or(false);
    let images = cliclack::confirm("Enable image types?")
        .initial_value(images_default)
        .interact()
        .map_err(clack_err)?;
    let install_service = cliclack::confirm("Install user service?")
        .initial_value(existing.is_none())
        .interact()
        .map_err(clack_err)?;
    Ok(InitAnswers {
        provider,
        model,
        base_url,
        api_key,
        dim,
        split,
        max_chars,
        overlap_chars,
        min_chars,
        first_root,
        exclude: existing
            .map(|c| c.exclude.clone())
            .unwrap_or_else(default_exclude),
        images,
        install_service,
    })
}

pub fn prompt_project_setup_tty(global: &ChunkConfig) -> Result<ProjectSetup, VaneCliError> {
    cliclack::intro("Add project").map_err(clack_err)?;
    let write_file = cliclack::confirm("Write .vane.toml in this repo?")
        .initial_value(true)
        .interact()
        .map_err(clack_err)?;
    if !write_file {
        cliclack::outro("using global chunk defaults").map_err(clack_err)?;
        return Ok(ProjectSetup {
            chunk: global.clone(),
            images: false,
            write_file: false,
        });
    }
    let split = cliclack::select("Chunk split")
        .initial_value(if global.split == "plain" {
            "plain"
        } else {
            "markdown"
        })
        .item("markdown", "markdown", "")
        .item("plain", "plain", "")
        .interact()
        .map_err(clack_err)?
        .to_string();
    let max_chars = clack_u32("Chunk max_chars", global.max_chars)?;
    let overlap_chars = clack_u32("Chunk overlap_chars", global.overlap_chars)?;
    let min_chars = clack_u32("Chunk min_chars", global.min_chars)?;
    let images = cliclack::confirm("Enable image types?")
        .initial_value(false)
        .interact()
        .map_err(clack_err)?;
    cliclack::outro("project policy ready").map_err(clack_err)?;
    Ok(ProjectSetup {
        chunk: ChunkConfig {
            split,
            max_chars,
            overlap_chars,
            min_chars,
        },
        images,
        write_file: true,
    })
}

fn clack_u32(label: &str, default: u32) -> Result<u32, VaneCliError> {
    let raw: String = cliclack::input(label)
        .default_input(&default.to_string())
        .interact()
        .map_err(clack_err)?;
    raw.parse::<u32>()
        .map_err(|_| VaneCliError::new(format!("invalid {label}: {raw:?}")))
}

fn clack_err(e: impl std::fmt::Display) -> VaneCliError {
    VaneCliError::new(format!("{e}"))
}

fn io_err(e: std::io::Error) -> VaneCliError {
    VaneCliError::new(format!("init io: {e}"))
}
