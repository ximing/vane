use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};

use crate::config::{default_embed, default_exclude, default_types, EmbedConfig};
use crate::error::VaneCliError;
use crate::service::install_user_service;

#[derive(Debug, Clone)]
pub struct InitAnswers {
    pub provider: String,
    pub model: String,
    pub base_url: String,
    pub first_root: Option<PathBuf>,
    pub exclude: Vec<String>,
    pub images: bool,
    pub install_service: bool,
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
    if cfg_path.is_file() {
        return Err(VaneCliError::new(format!(
            "already initialized: {}",
            cfg_path.display()
        )));
    }

    let answers = match assume {
        Some(a) => a,
        None => {
            let mut reader = BufReader::new(stdin);
            prompt_answers(&mut reader, &mut stdout)?
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

fn prompt_answers<R, W>(stdin: &mut R, stdout: &mut W) -> Result<InitAnswers, VaneCliError>
where
    R: BufRead,
    W: Write,
{
    let def = default_embed();
    let provider = prompt(
        stdin,
        stdout,
        "Embedding provider (ollama / openai_compat)",
        &def.provider,
    )?;
    validate_provider(&provider)?;
    let (default_model, default_url) = if provider == "openai_compat" {
        (
            "text-embedding-3-small".to_string(),
            "https://api.openai.com/v1".to_string(),
        )
    } else {
        (def.model.clone(), def.base_url.clone())
    };
    let model = prompt(stdin, stdout, "Model", &default_model)?;
    let base_url = prompt(stdin, stdout, "Base URL", &default_url)?;

    let embed = EmbedConfig {
        provider: provider.clone(),
        model: model.clone(),
        base_url: base_url.clone(),
        api_key: None,
    };
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

    let defaults = default_exclude();
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

    let images = prompt_yes_no(stdin, stdout, "Enable image types?", false)?;
    let install_service = prompt_yes_no(stdin, stdout, "Install user service?", true)?;

    Ok(InitAnswers {
        provider,
        model,
        base_url,
        first_root,
        exclude,
        images,
        install_service,
    })
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

    let mut rerank = toml::map::Map::new();
    rerank.insert("provider".into(), toml::Value::String("none".into()));

    let mut chunk = toml::map::Map::new();
    chunk.insert("split".into(), toml::Value::String("markdown".into()));
    chunk.insert("max_chars".into(), toml::Value::Integer(1200));
    chunk.insert("overlap_chars".into(), toml::Value::Integer(200));
    chunk.insert("min_chars".into(), toml::Value::Integer(50));

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

    if let Some(first) = &answers.first_root {
        let canon = normalize_root(first)?;
        let mut entry = toml::map::Map::new();
        entry.insert(
            "path".into(),
            toml::Value::String(canon.display().to_string()),
        );
        root.insert(
            "projects".into(),
            toml::Value::Array(vec![toml::Value::Table(entry)]),
        );
    }

    let path = home.join("config").join("config.toml");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| VaneCliError::new(format!("create {}: {e}", parent.display())))?;
    }
    let body = toml::to_string_pretty(&toml::Value::Table(root))
        .map_err(|e| VaneCliError::new(format!("serialize config: {e}")))?;
    std::fs::write(&path, body)
        .map_err(|e| VaneCliError::new(format!("write {}: {e}", path.display())))
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

fn io_err(e: std::io::Error) -> VaneCliError {
    VaneCliError::new(format!("init io: {e}"))
}
