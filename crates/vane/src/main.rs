use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use serde_json::json;
use vane::config::{load_config, resolve_policy, ProjectFile};
use vane::home::{default_fallback, resolve_home};
use vane::project::{find_current_root, project_id};
use vane::sync::rebuild_for_new_model;

#[derive(Parser, Debug)]
#[command(name = "vane", version, about = "Vane local document sidecar")]
struct Cli {
    /// Sidecar home directory. Precedence: --home > VANE_HOME > ~/.vane
    #[arg(long, global = true)]
    home: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Print the resolved home directory and initialization status
    Status,
    /// Run the sidecar daemon in the foreground
    Daemon,
    /// Start the daemon (prints a note until a user service is installed)
    Start,
    /// Search the current project (or --all / --root)
    Query {
        /// Query text
        q: String,
        /// Fuse hits across every registered project (RRF)
        #[arg(long)]
        all: bool,
        /// Search a single registered root
        #[arg(long)]
        root: Option<PathBuf>,
        /// Filter by extractor name (`text` / `image`)
        #[arg(long = "type")]
        extractor: Option<String>,
        /// Max hits (default 8, cap 50)
        #[arg(long, default_value_t = 8)]
        top_k: u32,
    },
    /// Change the embedding model and rebuild the project index
    Model {
        /// Write `[defaults.embed]` in the global config instead of `.vane.toml`
        #[arg(long)]
        global: bool,
        /// Target a registered root (default: current project from cwd)
        #[arg(long)]
        root: Option<PathBuf>,
        /// Embedding provider (`ollama` | `openai_compat`)
        #[arg(long)]
        provider: Option<String>,
        /// Embedding model name
        #[arg(long)]
        model: Option<String>,
        /// Provider base URL
        #[arg(long = "base-url")]
        base_url: Option<String>,
    },
}

fn resolved_home(cli_home: Option<&std::path::Path>) -> PathBuf {
    let fallback = default_fallback();
    let env_home = std::env::var_os("VANE_HOME");
    resolve_home(cli_home, env_home.as_deref(), &fallback)
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let home = resolved_home(cli.home.as_deref());

    match cli.command {
        Commands::Status => {
            println!("{}", home.display());
            let config = home.join("config").join("config.toml");
            if !config.is_file() {
                eprintln!("not initialized");
                return ExitCode::from(1);
            }
            ExitCode::SUCCESS
        }
        Commands::Daemon => match vane::daemon::serve_forever(home) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}");
                ExitCode::from(1)
            }
        },
        Commands::Start => {
            println!(
                "user service not installed yet; run `vane daemon --home {}` in the foreground",
                home.display()
            );
            ExitCode::SUCCESS
        }
        Commands::Query {
            q,
            all,
            root,
            extractor,
            top_k,
        } => run_query(&home, q, all, root, extractor, top_k),
        Commands::Model {
            global,
            root,
            provider,
            model,
            base_url,
        } => run_model(&home, global, root, provider, model, base_url),
    }
}

fn require_init(home: &std::path::Path) -> Result<(), ExitCode> {
    let config = home.join("config").join("config.toml");
    if config.is_file() {
        Ok(())
    } else {
        eprintln!("not initialized");
        Err(ExitCode::from(1))
    }
}

fn run_query(
    home: &std::path::Path,
    q: String,
    all: bool,
    root: Option<PathBuf>,
    extractor: Option<String>,
    top_k: u32,
) -> ExitCode {
    if require_init(home).is_err() {
        return ExitCode::from(1);
    }
    let cfg = match load_config(home) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(1);
        }
    };
    let mut params = json!({ "query": q, "top_k": top_k });
    if let Some(t) = extractor {
        params["type"] = json!(t);
    }
    if all {
        params["all"] = json!(true);
    } else if let Some(r) = root {
        params["root"] = json!(r.display().to_string());
    } else {
        let cwd = match std::env::current_dir() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("cannot read cwd: {e}");
                return ExitCode::from(1);
            }
        };
        let roots: Vec<PathBuf> = cfg
            .projects
            .iter()
            .map(|p| p.path.canonicalize().unwrap_or_else(|_| p.path.clone()))
            .collect();
        match find_current_root(&cwd, &roots) {
            Some(r) => params["root"] = json!(r.display().to_string()),
            None => {
                eprintln!(
                    "cwd is not inside a registered root; run `vane add` or pass --all / --root"
                );
                return ExitCode::from(1);
            }
        }
    }
    match vane::ipc::rpc_call(home, "search", params) {
        Ok(v) => {
            match serde_json::to_string_pretty(&v) {
                Ok(s) => println!("{s}"),
                Err(_) => println!("{v}"),
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
    }
}

fn run_model(
    home: &Path,
    global: bool,
    root: Option<PathBuf>,
    provider: Option<String>,
    model: Option<String>,
    base_url: Option<String>,
) -> ExitCode {
    if require_init(home).is_err() {
        return ExitCode::from(1);
    }
    let cfg = match load_config(home) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(1);
        }
    };
    if provider.is_none() && model.is_none() && base_url.is_none() {
        eprintln!("vane model: pass --provider, --model, and/or --base-url");
        return ExitCode::from(1);
    }

    let roots: Vec<PathBuf> = cfg
        .projects
        .iter()
        .map(|p| p.path.canonicalize().unwrap_or_else(|_| p.path.clone()))
        .collect();

    let targets: Vec<PathBuf> = if global {
        roots
    } else if let Some(r) = root {
        let expanded = expand_tilde(&r);
        vec![expanded.canonicalize().unwrap_or(expanded)]
    } else {
        let cwd = match std::env::current_dir() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("cannot read cwd: {e}");
                return ExitCode::from(1);
            }
        };
        match find_current_root(&cwd, &roots) {
            Some(r) => vec![r],
            None => {
                eprintln!(
                    "cwd is not inside a registered root; run `vane add` or pass --root / --global"
                );
                return ExitCode::from(1);
            }
        }
    };

    if let Err(e) = write_embed_overlay(home, global, &targets, &provider, &model, &base_url) {
        eprintln!("{e}");
        return ExitCode::from(1);
    }

    let mut failed = false;
    for target in &targets {
        let mut params = json!({ "root": target.display().to_string() });
        if let Some(p) = &provider {
            params["provider"] = json!(p);
        }
        if let Some(m) = &model {
            params["model"] = json!(m);
        }
        if let Some(u) = &base_url {
            params["base_url"] = json!(u);
        }
        match vane::ipc::rpc_call(home, "rebuild", params) {
            Ok(v) => match serde_json::to_string_pretty(&v) {
                Ok(s) => println!("{s}"),
                Err(_) => println!("{v}"),
            },
            Err(e) if e.message.contains("not running") => {
                if let Err(err) = rebuild_local(home, target) {
                    eprintln!("{err}");
                    failed = true;
                } else {
                    println!("rebuilt {}", target.display());
                }
            }
            Err(e) => {
                eprintln!("{e}");
                failed = true;
            }
        }
    }
    if failed {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn rebuild_local(home: &Path, root: &Path) -> Result<(), vane::error::VaneCliError> {
    let cfg = load_config(home)?;
    let pf = std::fs::read_to_string(root.join(".vane.toml"))
        .ok()
        .and_then(|t| ProjectFile::parse_toml(&t).ok());
    let policy = resolve_policy(&cfg, root, pf.as_ref())?;
    let pid = project_id(root);
    rebuild_for_new_model(home, &pid, &policy.embed)
}

fn write_embed_overlay(
    home: &Path,
    global: bool,
    targets: &[PathBuf],
    provider: &Option<String>,
    model: &Option<String>,
    base_url: &Option<String>,
) -> Result<(), vane::error::VaneCliError> {
    if global {
        let path = home.join("config").join("config.toml");
        patch_embed_toml(&path, &["defaults", "embed"], provider, model, base_url)
    } else {
        for root in targets {
            let path = root.join(".vane.toml");
            patch_embed_toml(&path, &["embed"], provider, model, base_url)?;
        }
        Ok(())
    }
}

fn patch_embed_toml(
    path: &Path,
    table_path: &[&str],
    provider: &Option<String>,
    model: &Option<String>,
    base_url: &Option<String>,
) -> Result<(), vane::error::VaneCliError> {
    let text = if path.is_file() {
        std::fs::read_to_string(path)
            .map_err(|e| vane::error::VaneCliError::new(format!("read {}: {e}", path.display())))?
    } else {
        String::new()
    };
    let mut value: toml::Value = if text.trim().is_empty() {
        toml::Value::Table(toml::map::Map::new())
    } else {
        toml::from_str(&text)
            .map_err(|e| vane::error::VaneCliError::new(format!("parse {}: {e}", path.display())))?
    };
    let mut cur = value.as_table_mut().ok_or_else(|| {
        vane::error::VaneCliError::new(format!("{} is not a table", path.display()))
    })?;
    for key in table_path {
        let entry = cur
            .entry(key.to_string())
            .or_insert(toml::Value::Table(toml::map::Map::new()));
        cur = entry.as_table_mut().ok_or_else(|| {
            vane::error::VaneCliError::new(format!("{key} is not a table in {}", path.display()))
        })?;
    }
    if let Some(p) = provider {
        cur.insert("provider".into(), toml::Value::String(p.clone()));
    }
    if let Some(m) = model {
        cur.insert("model".into(), toml::Value::String(m.clone()));
    }
    if let Some(u) = base_url {
        cur.insert("base_url".into(), toml::Value::String(u.clone()));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            vane::error::VaneCliError::new(format!("create {}: {e}", parent.display()))
        })?;
    }
    let out = toml::to_string_pretty(&value).map_err(|e| {
        vane::error::VaneCliError::new(format!("serialize {}: {e}", path.display()))
    })?;
    std::fs::write(path, out)
        .map_err(|e| vane::error::VaneCliError::new(format!("write {}: {e}", path.display())))
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
