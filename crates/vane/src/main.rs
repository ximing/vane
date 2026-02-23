use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use clap::{Parser, Subcommand};
use serde_json::json;
use vane::config::{
    default_exclude, default_types, load_config, resolve_policy, ProjectFile, TypeRule,
};
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
    /// Write global config (interactive unless used from tests via assume)
    Init,
    /// Register a project root and notify the daemon
    Add { path: PathBuf },
    /// Unregister a project root (keeps CAS / project db until gc)
    Rm { path: PathBuf },
    /// Change the current project's include / types table
    Include {
        #[command(subcommand)]
        action: GlobAction,
    },
    /// Change the current project's extra excludes
    Exclude {
        #[command(subcommand)]
        action: GlobAction,
    },
    /// Print the resolved home directory and daemon status
    Status,
    /// Run the sidecar daemon in the foreground
    Daemon,
    /// Start the daemon (user service if installed, else background process)
    Start,
    /// Stop the daemon
    Stop,
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
    /// JSON-RPC 2.0 MCP stdio bridge to a running daemon
    Mcp,
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
    /// User service (launchd / systemd --user)
    Service {
        #[command(subcommand)]
        action: ServiceCmd,
    },
    /// Compact the project index and drop unreferenced CAS
    Gc {
        /// Target a registered root (default: current project from cwd)
        #[arg(long)]
        root: Option<PathBuf>,
        /// All projects plus global orphan CAS
        #[arg(long)]
        all: bool,
    },
}

#[derive(Subcommand, Debug)]
enum GlobAction {
    /// Append a glob
    Add {
        glob: String,
        /// Write the global config instead of the current project
        #[arg(long)]
        global: bool,
        /// Target a registered root
        #[arg(long)]
        root: Option<PathBuf>,
    },
    /// Restore defaults (global) or clear project overrides
    Reset {
        /// Write the global config instead of the current project
        #[arg(long)]
        global: bool,
        /// Target a registered root
        #[arg(long)]
        root: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
enum ServiceCmd {
    /// Remove the user service definition and stop the daemon (keeps data)
    Uninstall,
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
        Commands::Init => run_init(&home),
        Commands::Add { path } => run_add(&home, &path),
        Commands::Rm { path } => run_rm(&home, &path),
        Commands::Include { action } => run_glob_policy(&home, PolicyKind::Include, action),
        Commands::Exclude { action } => run_glob_policy(&home, PolicyKind::Exclude, action),
        Commands::Status => run_status(&home),
        Commands::Daemon => match vane::daemon::serve_forever(home) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}");
                ExitCode::from(1)
            }
        },
        Commands::Start => run_start(&home),
        Commands::Stop => run_stop(&home),
        Commands::Mcp => run_mcp(&home),
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
        Commands::Service {
            action: ServiceCmd::Uninstall,
        } => run_service_uninstall(&home),
        Commands::Gc { root, all } => run_gc(&home, root, all),
    }
}

fn require_init(home: &std::path::Path) -> Result<(), ExitCode> {
    let config = home.join("config").join("config.toml");
    if config.is_file() {
        Ok(())
    } else {
        eprintln!(
            "not initialized: missing {}; run `vane init`",
            config.display()
        );
        Err(ExitCode::from(1))
    }
}

fn run_init(home: &Path) -> ExitCode {
    match vane::wizard::run_init(home, std::io::stdin(), std::io::stdout(), None) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
    }
}

fn run_status(home: &Path) -> ExitCode {
    println!("{}", home.display());
    if require_init(home).is_err() {
        return ExitCode::from(1);
    }
    match vane::ipc::rpc_call(home, "status", json!({})) {
        Ok(v) => print_json(&v),
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
    }
}

fn run_start(home: &Path) -> ExitCode {
    if require_init(home).is_err() {
        return ExitCode::from(1);
    }
    if vane::daemon::is_running(home) {
        println!("already running");
        return ExitCode::SUCCESS;
    }
    match vane::service::start_installed_service() {
        Ok(true) => {
            println!("started user service");
            ExitCode::SUCCESS
        }
        Ok(false) => spawn_background_daemon(home),
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
    }
}

fn spawn_background_daemon(home: &Path) -> ExitCode {
    let bin = match std::env::current_exe() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("cannot resolve vane binary: {e}");
            return ExitCode::from(1);
        }
    };
    let home_s = home.display().to_string();
    let mut cmd = Command::new(&bin);
    cmd.args(["daemon", "--home", &home_s])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    match cmd.spawn() {
        Ok(_) => {
            println!("started");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("failed to start daemon: {e}");
            ExitCode::from(1)
        }
    }
}

fn run_stop(home: &Path) -> ExitCode {
    if require_init(home).is_err() {
        return ExitCode::from(1);
    }
    let _ = vane::service::stop_installed_service();
    match vane::daemon::stop_daemon(home) {
        Ok(()) => {
            println!("stopped");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
    }
}

fn run_service_uninstall(home: &Path) -> ExitCode {
    let _ = vane::service::stop_installed_service();
    if let Err(e) = vane::daemon::stop_daemon(home) {
        eprintln!("{e}");
        return ExitCode::from(1);
    }
    match vane::service::uninstall_user_service() {
        Ok(()) => {
            println!("user service uninstalled (data kept)");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
    }
}

fn run_mcp(home: &std::path::Path) -> ExitCode {
    if require_init(home).is_err() {
        return ExitCode::from(1);
    }
    match vane::mcp::serve_stdio(home.to_path_buf()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
    }
}

fn run_add(home: &Path, path: &Path) -> ExitCode {
    if require_init(home).is_err() {
        return ExitCode::from(1);
    }
    let resolved = match resolve_root_arg(path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(1);
        }
    };
    rpc_print(
        home,
        "add_root",
        json!({ "path": resolved.display().to_string() }),
    )
}

fn run_rm(home: &Path, path: &Path) -> ExitCode {
    if require_init(home).is_err() {
        return ExitCode::from(1);
    }
    let resolved = match resolve_root_arg(path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(1);
        }
    };
    rpc_print(
        home,
        "remove_root",
        json!({ "path": resolved.display().to_string() }),
    )
}

fn run_gc(home: &Path, root: Option<PathBuf>, all: bool) -> ExitCode {
    if require_init(home).is_err() {
        return ExitCode::from(1);
    }
    let params = if all {
        json!({ "all": true })
    } else if let Some(r) = root {
        match resolve_root_arg(&r) {
            Ok(p) => json!({ "root": p.display().to_string() }),
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::from(1);
            }
        }
    } else {
        match current_root(home) {
            Ok(p) => json!({ "root": p.display().to_string() }),
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::from(1);
            }
        }
    };
    rpc_print(home, "gc", params)
}

#[derive(Clone, Copy)]
enum PolicyKind {
    Include,
    Exclude,
}

fn run_glob_policy(home: &Path, kind: PolicyKind, action: GlobAction) -> ExitCode {
    if require_init(home).is_err() {
        return ExitCode::from(1);
    }
    let result = match action {
        GlobAction::Add { glob, global, root } => apply_glob_add(home, kind, global, root, &glob),
        GlobAction::Reset { global, root } => apply_glob_reset(home, kind, global, root),
    };
    if let Err(e) = result {
        eprintln!("{e}");
        return ExitCode::from(1);
    }
    match vane::ipc::rpc_call(home, "reload_config", json!({})) {
        Ok(v) => print_json(&v),
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
    }
}

fn apply_glob_add(
    home: &Path,
    kind: PolicyKind,
    global: bool,
    root: Option<PathBuf>,
    glob: &str,
) -> Result<(), vane::error::VaneCliError> {
    if global {
        let path = home.join("config").join("config.toml");
        match kind {
            PolicyKind::Exclude => patch_string_list(&path, "exclude", ListOp::Add(glob))?,
            PolicyKind::Include => patch_global_type(&path, glob)?,
        }
        return Ok(());
    }
    let root = resolve_policy_root(home, root)?;
    let path = root.join(".vane.toml");
    match kind {
        PolicyKind::Exclude => patch_string_list(&path, "exclude", ListOp::Add(glob)),
        PolicyKind::Include => patch_string_list(&path, "include", ListOp::Add(glob)),
    }
}

fn apply_glob_reset(
    home: &Path,
    kind: PolicyKind,
    global: bool,
    root: Option<PathBuf>,
) -> Result<(), vane::error::VaneCliError> {
    if global {
        let path = home.join("config").join("config.toml");
        match kind {
            PolicyKind::Exclude => {
                patch_string_list(&path, "exclude", ListOp::Replace(default_exclude()))
            }
            PolicyKind::Include => patch_global_types_reset(&path),
        }
    } else {
        let root = resolve_policy_root(home, root)?;
        let path = root.join(".vane.toml");
        match kind {
            PolicyKind::Exclude => patch_string_list(&path, "exclude", ListOp::Clear),
            PolicyKind::Include => {
                patch_remove_keys(&path, &["include", "types"])?;
                Ok(())
            }
        }
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
    rpc_print(home, "search", params)
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
    let mut value = load_toml(path)?;
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
    save_toml(path, &value)
}

enum ListOp<'a> {
    Add(&'a str),
    Clear,
    Replace(Vec<String>),
}

fn load_toml(path: &Path) -> Result<toml::Value, vane::error::VaneCliError> {
    if !path.is_file() {
        return Ok(toml::Value::Table(toml::map::Map::new()));
    }
    let text = std::fs::read_to_string(path)
        .map_err(|e| vane::error::VaneCliError::new(format!("read {}: {e}", path.display())))?;
    if text.trim().is_empty() {
        return Ok(toml::Value::Table(toml::map::Map::new()));
    }
    toml::from_str(&text)
        .map_err(|e| vane::error::VaneCliError::new(format!("parse {}: {e}", path.display())))
}

fn save_toml(path: &Path, value: &toml::Value) -> Result<(), vane::error::VaneCliError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            vane::error::VaneCliError::new(format!("create {}: {e}", parent.display()))
        })?;
    }
    let out = toml::to_string_pretty(value).map_err(|e| {
        vane::error::VaneCliError::new(format!("serialize {}: {e}", path.display()))
    })?;
    std::fs::write(path, out)
        .map_err(|e| vane::error::VaneCliError::new(format!("write {}: {e}", path.display())))
}

fn patch_string_list(
    path: &Path,
    key: &str,
    op: ListOp<'_>,
) -> Result<(), vane::error::VaneCliError> {
    let mut value = load_toml(path)?;
    let table = value.as_table_mut().ok_or_else(|| {
        vane::error::VaneCliError::new(format!("{} is not a table", path.display()))
    })?;
    match op {
        ListOp::Add(item) => {
            let arr = table
                .entry(key.to_string())
                .or_insert(toml::Value::Array(Vec::new()));
            let list = arr.as_array_mut().ok_or_else(|| {
                vane::error::VaneCliError::new(format!(
                    "{key} is not an array in {}",
                    path.display()
                ))
            })?;
            if !list.iter().any(|v| v.as_str() == Some(item)) {
                list.push(toml::Value::String(item.to_string()));
            }
        }
        ListOp::Clear => {
            table.insert(key.to_string(), toml::Value::Array(Vec::new()));
        }
        ListOp::Replace(items) => {
            table.insert(
                key.to_string(),
                toml::Value::Array(items.into_iter().map(toml::Value::String).collect()),
            );
        }
    }
    save_toml(path, &value)
}

fn patch_remove_keys(path: &Path, keys: &[&str]) -> Result<(), vane::error::VaneCliError> {
    if !path.is_file() {
        return Ok(());
    }
    let mut value = load_toml(path)?;
    if let Some(table) = value.as_table_mut() {
        for key in keys {
            table.remove(*key);
        }
    }
    save_toml(path, &value)
}

fn patch_global_type(path: &Path, glob: &str) -> Result<(), vane::error::VaneCliError> {
    let mut value = load_toml(path)?;
    let table = value.as_table_mut().ok_or_else(|| {
        vane::error::VaneCliError::new(format!("{} is not a table", path.display()))
    })?;
    let arr = table
        .entry("types".to_string())
        .or_insert(toml::Value::Array(Vec::new()));
    let list = arr.as_array_mut().ok_or_else(|| {
        vane::error::VaneCliError::new(format!("types is not an array in {}", path.display()))
    })?;
    let exists = list.iter().any(|v| {
        v.get("glob")
            .and_then(|g| g.as_str())
            .is_some_and(|g| g == glob)
    });
    if !exists {
        let mut rule = toml::map::Map::new();
        rule.insert("glob".into(), toml::Value::String(glob.to_string()));
        rule.insert("extractor".into(), toml::Value::String("text".into()));
        rule.insert("enabled".into(), toml::Value::Boolean(true));
        list.push(toml::Value::Table(rule));
    }
    save_toml(path, &value)
}

fn patch_global_types_reset(path: &Path) -> Result<(), vane::error::VaneCliError> {
    let mut value = load_toml(path)?;
    let table = value.as_table_mut().ok_or_else(|| {
        vane::error::VaneCliError::new(format!("{} is not a table", path.display()))
    })?;
    table.insert(
        "types".into(),
        toml::Value::Array(default_types().into_iter().map(type_rule_toml).collect()),
    );
    save_toml(path, &value)
}

fn type_rule_toml(t: TypeRule) -> toml::Value {
    let mut m = toml::map::Map::new();
    m.insert("glob".into(), toml::Value::String(t.glob));
    m.insert("extractor".into(), toml::Value::String(t.extractor));
    m.insert("enabled".into(), toml::Value::Boolean(t.enabled));
    toml::Value::Table(m)
}

fn current_root(home: &Path) -> Result<PathBuf, vane::error::VaneCliError> {
    let cfg = load_config(home)?;
    let cwd = std::env::current_dir()
        .map_err(|e| vane::error::VaneCliError::new(format!("cannot read cwd: {e}")))?;
    let roots: Vec<PathBuf> = cfg
        .projects
        .iter()
        .map(|p| p.path.canonicalize().unwrap_or_else(|_| p.path.clone()))
        .collect();
    find_current_root(&cwd, &roots).ok_or_else(|| {
        vane::error::VaneCliError::new(
            "cwd is not inside a registered root; run `vane add` or pass --root / --global",
        )
    })
}

fn resolve_policy_root(
    home: &Path,
    root: Option<PathBuf>,
) -> Result<PathBuf, vane::error::VaneCliError> {
    if let Some(r) = root {
        return resolve_root_arg(&r);
    }
    current_root(home)
}

fn resolve_root_arg(path: &Path) -> Result<PathBuf, vane::error::VaneCliError> {
    let expanded = expand_tilde(path);
    let abs = if expanded.is_absolute() {
        expanded
    } else {
        let cwd = std::env::current_dir()
            .map_err(|e| vane::error::VaneCliError::new(format!("cannot read cwd: {e}")))?;
        cwd.join(expanded)
    };
    Ok(abs.canonicalize().unwrap_or(abs))
}

fn rpc_print(home: &Path, method: &str, params: serde_json::Value) -> ExitCode {
    match vane::ipc::rpc_call(home, method, params) {
        Ok(v) => print_json(&v),
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
    }
}

fn print_json(v: &serde_json::Value) -> ExitCode {
    match serde_json::to_string_pretty(v) {
        Ok(s) => println!("{s}"),
        Err(_) => println!("{v}"),
    }
    ExitCode::SUCCESS
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
