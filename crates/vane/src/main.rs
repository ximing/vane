use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::Duration;

use clap::{Parser, Subcommand};
use serde_json::json;
use vane::config::{
    default_exclude, default_types, inspect_policy, load_config, resolve_policy, ProjectFile,
    TypeRule,
};
use vane::home::{default_fallback, disk_stats, resolve_home};
use vane::project::{find_current_root, project_id, resolve_query_scope, QueryScope};
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
    /// Write global config (re-run to edit; empty answers keep current values)
    Init,
    /// Register a project root and notify the daemon
    Add {
        path: PathBuf,
        /// Skip the project-config wizard and do not write `.vane.toml`
        #[arg(long, short = 'y')]
        yes: bool,
    },
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
    /// Diagnose sidecar home, daemon, embedder, and registered roots
    Doctor,
    /// List skipped files (too large, invalid UTF-8, embed / extractor errors)
    Issues {
        /// Target a registered root
        #[arg(long)]
        root: Option<PathBuf>,
        /// Every registered root
        #[arg(long)]
        all: bool,
    },
    /// Print recent daemon logs (redacted)
    Logs {
        /// Follow new lines (like tail -f)
        #[arg(long)]
        follow: bool,
        /// How many existing lines to print (default 50)
        #[arg(long, default_value_t = 50)]
        lines: usize,
    },
    /// Print resolved embed / chunk / exclude / types policy
    Inspect {
        /// Target a registered root
        #[arg(long)]
        root: Option<PathBuf>,
        /// Print global defaults only
        #[arg(long)]
        global: bool,
    },
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
        /// Force global scope (same as --all); ignore `.vane.toml` walk-up
        #[arg(long)]
        global: bool,
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
    /// JSON-RPC 2.0 MCP stdio bridge to a running daemon (no args), or `install`
    Mcp {
        #[command(subcommand)]
        cmd: Option<McpCmd>,
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
        /// Vector dimension (openai_compat sends this as `dimensions`)
        #[arg(long)]
        dim: Option<u32>,
    },
    /// User service (launchd / systemd --user)
    Service {
        #[command(subcommand)]
        action: ServiceCmd,
    },
    /// Show disk usage for $VANE_HOME, CAS, and per-project dbs
    Df,
    /// Compact the project index and drop unreferenced CAS
    Gc {
        /// Target a registered root (default: current project from cwd)
        #[arg(long)]
        root: Option<PathBuf>,
        /// All projects plus global orphan CAS
        #[arg(long)]
        all: bool,
        /// Count what would be deleted without removing anything
        #[arg(long)]
        dry_run: bool,
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

#[derive(Subcommand, Debug)]
enum McpCmd {
    /// Merge `mcpServers.vane` into Claude / Cursor / Codex configs under $HOME
    Install {
        /// Print what would be written without touching files
        #[arg(long)]
        dry_run: bool,
        /// Target one client (default: all known)
        #[arg(long, value_parser = ["claude", "cursor", "codex"])]
        client: Option<String>,
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
        Commands::Init => run_init(&home),
        Commands::Add { path, yes } => run_add(&home, &path, yes),
        Commands::Rm { path } => run_rm(&home, &path),
        Commands::Include { action } => run_glob_policy(&home, PolicyKind::Include, action),
        Commands::Exclude { action } => run_glob_policy(&home, PolicyKind::Exclude, action),
        Commands::Status => run_status(&home),
        Commands::Doctor => run_doctor(&home),
        Commands::Issues { root, all } => run_issues(&home, root, all),
        Commands::Logs { follow, lines } => run_logs(&home, follow, lines),
        Commands::Inspect { root, global } => run_inspect(&home, root, global),
        Commands::Daemon => match vane::daemon::serve_forever(home) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}");
                ExitCode::from(1)
            }
        },
        Commands::Start => run_start(&home),
        Commands::Stop => run_stop(&home),
        Commands::Mcp { cmd } => match cmd {
            None => run_mcp(&home),
            Some(McpCmd::Install { dry_run, client }) => run_mcp_install(dry_run, client),
        },
        Commands::Query {
            q,
            all,
            global,
            root,
            extractor,
            top_k,
        } => run_query(&home, q, all || global, root, extractor, top_k),
        Commands::Model {
            global,
            root,
            provider,
            model,
            base_url,
            dim,
        } => run_model(&home, global, root, provider, model, base_url, dim),
        Commands::Service {
            action: ServiceCmd::Uninstall,
        } => run_service_uninstall(&home),
        Commands::Df => run_df(&home),
        Commands::Gc { root, all, dry_run } => run_gc(&home, root, all, dry_run),
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
    let result = if vane::ui::interactive() {
        vane::wizard::run_init_tty(home)
    } else {
        vane::wizard::run_init(home, std::io::stdin(), std::io::stdout(), None)
    };
    match result {
        Ok(()) => {
            vane::ui::success("initialized");
            if vane::ui::stdout_tty() {
                vane::ui::print_next_steps(home);
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            vane::ui::error(&e.message);
            ExitCode::from(1)
        }
    }
}

fn run_status(home: &Path) -> ExitCode {
    if require_init(home).is_err() {
        return ExitCode::from(1);
    }
    let running = vane::daemon::is_running(home);
    let v = if running {
        match vane::ipc::rpc_call(home, "status", json!({})) {
            Ok(v) => v,
            Err(_) => vane::doctor::status_from_disk(home, vane::daemon::is_running(home)),
        }
    } else {
        vane::doctor::status_from_disk(home, false)
    };
    if vane::ui::stdout_tty() {
        vane::ui::print_status_dashboard(&v);
        ExitCode::SUCCESS
    } else {
        print_json(&v)
    }
}

fn run_doctor(home: &Path) -> ExitCode {
    let report = vane::doctor::run(home);
    if vane::ui::stdout_tty() {
        vane::ui::print_doctor(&report);
    } else {
        match serde_json::to_value(&report) {
            Ok(v) => {
                print_json(&v);
            }
            Err(e) => {
                vane::ui::error(&format!("encode doctor report: {e}"));
                return ExitCode::from(1);
            }
        }
    }
    if report.ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
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

fn run_mcp_install(dry_run: bool, client: Option<String>) -> ExitCode {
    let client = match parse_mcp_client(client.as_deref()) {
        Ok(c) => c,
        Err(e) => {
            vane::ui::error(&e);
            return ExitCode::from(1);
        }
    };
    let user_home = match std::env::var_os("HOME") {
        Some(h) if !h.is_empty() => PathBuf::from(h),
        _ => {
            vane::ui::error("HOME is not set");
            return ExitCode::from(1);
        }
    };
    match vane::mcp::install_mcp(&user_home, dry_run, client) {
        Ok(report) => {
            if vane::ui::stdout_tty() {
                vane::ui::print_mcp_install(&report);
                ExitCode::SUCCESS
            } else {
                match serde_json::to_value(&report) {
                    Ok(v) => print_json(&v),
                    Err(e) => {
                        vane::ui::error(&format!("encode mcp install report: {e}"));
                        ExitCode::from(1)
                    }
                }
            }
        }
        Err(e) => {
            vane::ui::error(&e.message);
            ExitCode::from(1)
        }
    }
}

fn parse_mcp_client(raw: Option<&str>) -> Result<Option<vane::mcp::McpClient>, String> {
    match raw {
        None => Ok(None),
        Some("claude") => Ok(Some(vane::mcp::McpClient::Claude)),
        Some("cursor") => Ok(Some(vane::mcp::McpClient::Cursor)),
        Some("codex") => Ok(Some(vane::mcp::McpClient::Codex)),
        Some(other) => Err(format!(
            "unknown MCP client {other:?}, expected claude, cursor, or codex"
        )),
    }
}

fn run_add(home: &Path, path: &Path, yes: bool) -> ExitCode {
    if require_init(home).is_err() {
        return ExitCode::from(1);
    }
    let resolved = match resolve_root_arg(path) {
        Ok(p) => p,
        Err(e) => {
            vane::ui::error(&e.message);
            return ExitCode::from(1);
        }
    };
    if !yes && vane::ui::interactive() {
        let cfg = match load_config(home) {
            Ok(c) => c,
            Err(e) => {
                vane::ui::error(&e.message);
                return ExitCode::from(1);
            }
        };
        let setup = if vane::ui::interactive() {
            vane::wizard::prompt_project_setup_tty(&cfg.defaults.chunk)
        } else {
            vane::wizard::prompt_project_setup(
                &mut std::io::stdin().lock(),
                &mut std::io::stdout(),
                &cfg.defaults.chunk,
            )
        };
        match setup {
            Ok(s) if s.write_file => {
                match vane::wizard::write_project_toml(&resolved, &s.chunk, s.images) {
                    Ok(p) => vane::ui::success(&format!("wrote {}", p.display())),
                    Err(e) => {
                        vane::ui::error(&e.message);
                        return ExitCode::from(1);
                    }
                }
            }
            Ok(_) => {}
            Err(e) => {
                vane::ui::error(&e.message);
                return ExitCode::from(1);
            }
        }
    }
    let spin = vane::ui::spinner(&format!("indexing {}", resolved.display()));
    let result = if vane::ui::stdout_tty() {
        add_root_poll_progress(home, &resolved, &spin)
    } else {
        vane::ipc::rpc_call(
            home,
            "add_root",
            json!({ "path": resolved.display().to_string() }),
        )
    };
    spin.finish_and_clear();
    match result {
        Ok(v) => {
            print_add_report(&resolved, &v);
            if vane::ui::stdout_tty() {
                vane::ui::print_next_steps(home);
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            vane::ui::error(&e.message);
            ExitCode::from(1)
        }
    }
}

fn add_root_poll_progress(
    home: &Path,
    resolved: &Path,
    spin: &indicatif::ProgressBar,
) -> Result<serde_json::Value, vane::error::VaneCliError> {
    let home_rpc = home.to_path_buf();
    let path = resolved.display().to_string();
    let handle = std::thread::spawn(move || {
        vane::ipc::rpc_call(&home_rpc, "add_root", json!({ "path": path }))
    });
    while !handle.is_finished() {
        if let Some(progress) = vane::progress::load_progress(home) {
            spin.set_message(vane::progress::spinner_message(&progress));
        }
        std::thread::sleep(Duration::from_millis(80));
    }
    handle
        .join()
        .unwrap_or_else(|_| Err(vane::error::VaneCliError::new("add_root worker panicked")))
}

fn print_add_report(root: &Path, v: &serde_json::Value) {
    let scanned = v.get("scanned").and_then(|x| x.as_u64()).unwrap_or(0);
    let added = v.get("added").and_then(|x| x.as_u64()).unwrap_or(0);
    let embedded = v.get("embedded").and_then(|x| x.as_u64()).unwrap_or(0);
    let unchanged = v.get("unchanged").and_then(|x| x.as_u64()).unwrap_or(0);
    let skipped = v.get("skipped").and_then(|x| x.as_u64()).unwrap_or(0);
    vane::ui::success(&format!(
        "added {}  scanned {scanned}  new {added}  embedded {embedded}  unchanged {unchanged}  skipped {skipped}",
        root.display()
    ));
    if !vane::ui::interactive() {
        print_json(v);
    }
}

fn run_issues(home: &Path, root: Option<PathBuf>, all: bool) -> ExitCode {
    if require_init(home).is_err() {
        return ExitCode::from(1);
    }
    let cfg = match load_config(home) {
        Ok(c) => c,
        Err(e) => {
            vane::ui::error(&e.message);
            return ExitCode::from(1);
        }
    };
    let selected = if all {
        cfg.projects
            .iter()
            .map(|p| p.path.canonicalize().unwrap_or_else(|_| p.path.clone()))
            .collect()
    } else if let Some(r) = root {
        match resolve_root_arg(&r) {
            Ok(p) => vec![p],
            Err(e) => {
                vane::ui::error(&e.message);
                return ExitCode::from(1);
            }
        }
    } else {
        match current_issues_root(home) {
            Ok(p) => vec![p],
            Err(e) => {
                vane::ui::error(&e.message);
                return ExitCode::from(1);
            }
        }
    };
    let report = vane::progress::issues_report(home, &selected);
    if vane::ui::stdout_tty() {
        vane::ui::print_issues(&report);
        ExitCode::SUCCESS
    } else {
        match serde_json::to_value(&report) {
            Ok(v) => print_json(&v),
            Err(e) => {
                vane::ui::error(&format!("encode issues: {e}"));
                ExitCode::from(1)
            }
        }
    }
}

fn current_issues_root(home: &Path) -> Result<PathBuf, vane::error::VaneCliError> {
    let cfg = load_config(home)?;
    let cwd = std::env::current_dir()
        .map_err(|e| vane::error::VaneCliError::new(format!("cannot read cwd: {e}")))?;
    let roots: Vec<PathBuf> = cfg
        .projects
        .iter()
        .map(|p| p.path.canonicalize().unwrap_or_else(|_| p.path.clone()))
        .collect();
    find_current_root(&cwd, &roots).ok_or_else(|| {
        vane::error::VaneCliError::new("cwd is not inside a registered root; pass --root or --all")
    })
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

fn run_logs(home: &Path, follow: bool, lines: usize) -> ExitCode {
    if require_init(home).is_err() {
        return ExitCode::from(1);
    }
    let recent = vane::log::recent_lines(home, lines);
    if follow {
        if vane::ui::stdout_tty() {
            for line in &recent {
                vane::ui::print_log_line(line);
            }
        } else {
            for line in &recent {
                println!("{}", json!({ "line": line }));
            }
        }
        let _ = std::io::Write::flush(&mut std::io::stdout());
        let mut tail = vane::log::LogTail::open_at_end(home);
        loop {
            for line in tail.poll() {
                if vane::ui::stdout_tty() {
                    vane::ui::print_log_line(&line);
                } else {
                    println!("{}", json!({ "line": line }));
                }
            }
            let _ = std::io::Write::flush(&mut std::io::stdout());
            std::thread::sleep(Duration::from_millis(250));
        }
    } else if vane::ui::stdout_tty() {
        for line in &recent {
            vane::ui::print_log_line(line);
        }
        ExitCode::SUCCESS
    } else {
        print_json(&json!({ "lines": recent }))
    }
}

fn run_inspect(home: &Path, root: Option<PathBuf>, global: bool) -> ExitCode {
    if require_init(home).is_err() {
        return ExitCode::from(1);
    }
    let cfg = match load_config(home) {
        Ok(c) => c,
        Err(e) => {
            vane::ui::error(&e.message);
            return ExitCode::from(1);
        }
    };
    let root = if global {
        None
    } else if let Some(r) = root {
        match resolve_root_arg(&r) {
            Ok(p) => Some(p),
            Err(e) => {
                vane::ui::error(&e.message);
                return ExitCode::from(1);
            }
        }
    } else {
        match current_root(home) {
            Ok(p) => Some(p),
            Err(e) => {
                vane::ui::error(&e.message);
                return ExitCode::from(1);
            }
        }
    };
    let report = match inspect_policy(&cfg, root.as_deref(), global) {
        Ok(r) => r,
        Err(e) => {
            vane::ui::error(&e.message);
            return ExitCode::from(1);
        }
    };
    if vane::ui::stdout_tty() {
        vane::ui::print_inspect(&report);
        ExitCode::SUCCESS
    } else {
        match serde_json::to_value(&report) {
            Ok(v) => {
                let dumped = v.to_string();
                if dumped.contains("\"api_key\"") {
                    vane::ui::error("inspect refused to print api_key");
                    return ExitCode::from(1);
                }
                print_json(&v)
            }
            Err(e) => {
                vane::ui::error(&format!("encode inspect: {e}"));
                ExitCode::from(1)
            }
        }
    }
}

fn run_df(home: &Path) -> ExitCode {
    if require_init(home).is_err() {
        return ExitCode::from(1);
    }
    let stats = disk_stats(home);
    if vane::ui::stdout_tty() {
        vane::ui::print_df(home, &stats);
        ExitCode::SUCCESS
    } else {
        print_json(&json!({
            "home": home.display().to_string(),
            "home_bytes": stats.home_bytes,
            "cas_bytes": stats.cas_bytes,
            "projects": stats.projects,
            "large": stats.home_bytes > (1u64 << 30),
        }))
    }
}

fn run_gc(home: &Path, root: Option<PathBuf>, all: bool, dry_run: bool) -> ExitCode {
    if require_init(home).is_err() {
        return ExitCode::from(1);
    }
    let mut params = if all {
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
    if dry_run {
        params["dry_run"] = json!(true);
    }
    match vane::ipc::rpc_call(home, "gc", params) {
        Ok(v) => {
            if vane::ui::stdout_tty() {
                match serde_json::from_value::<vane::gc::GcReport>(v) {
                    Ok(report) => {
                        vane::ui::print_gc(&report);
                        ExitCode::SUCCESS
                    }
                    Err(e) => {
                        vane::ui::error(&format!("decode gc report: {e}"));
                        ExitCode::from(1)
                    }
                }
            } else {
                print_json(&v)
            }
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
    }
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
            vane::ui::error(&e.message);
            return ExitCode::from(1);
        }
    };
    let mut params = json!({ "query": q, "top_k": top_k });
    if let Some(t) = extractor {
        params["type"] = json!(t);
    }
    if all {
        params["all"] = json!(true);
    } else if let Some(r) = root.as_ref() {
        params["root"] = json!(r.display().to_string());
    } else {
        let cwd = match std::env::current_dir() {
            Ok(c) => c,
            Err(e) => {
                vane::ui::error(&format!("cannot read cwd: {e}"));
                return ExitCode::from(1);
            }
        };
        let roots: Vec<PathBuf> = cfg
            .projects
            .iter()
            .map(|p| p.path.canonicalize().unwrap_or_else(|_| p.path.clone()))
            .collect();
        match resolve_query_scope(&cwd, &roots, false) {
            QueryScope::All => params["all"] = json!(true),
            QueryScope::Root(r) => params["root"] = json!(r.display().to_string()),
        }
    }
    match vane::ipc::rpc_call(home, "search", params) {
        Ok(v) => print_search_result(home, &v, &q, all, root.as_deref()),
        Err(e) => {
            vane::ui::error(&e.message);
            ExitCode::from(1)
        }
    }
}

fn print_search_result(
    home: &Path,
    v: &serde_json::Value,
    q: &str,
    all: bool,
    root: Option<&Path>,
) -> ExitCode {
    let hits = v.as_array().cloned().unwrap_or_else(|| {
        v.get("hits")
            .and_then(|h| h.as_array())
            .cloned()
            .unwrap_or_default()
    });
    if hits.is_empty() {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let why = vane::doctor::explain_empty_query(home, &cwd, q, all, root);
        if vane::ui::stdout_tty() {
            vane::ui::print_why(&why.message);
            return ExitCode::SUCCESS;
        }
        print_json(v);
        eprintln!("{}", why.message);
        return ExitCode::SUCCESS;
    }
    if vane::ui::stdout_tty() {
        vane::ui::print_hits(&hits);
        ExitCode::SUCCESS
    } else {
        print_json(v)
    }
}

fn run_model(
    home: &Path,
    global: bool,
    root: Option<PathBuf>,
    provider: Option<String>,
    model: Option<String>,
    base_url: Option<String>,
    dim: Option<u32>,
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
    if provider.is_none() && model.is_none() && base_url.is_none() && dim.is_none() {
        eprintln!("vane model: pass --provider, --model, --base-url, and/or --dim");
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

    if let Err(e) = write_embed_overlay(home, global, &targets, &provider, &model, &base_url, dim) {
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
        if let Some(d) = dim {
            params["dim"] = json!(d);
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
    dim: Option<u32>,
) -> Result<(), vane::error::VaneCliError> {
    if global {
        let path = home.join("config").join("config.toml");
        patch_embed_toml(
            &path,
            &["defaults", "embed"],
            provider,
            model,
            base_url,
            dim,
        )
    } else {
        for root in targets {
            let path = root.join(".vane.toml");
            patch_embed_toml(&path, &["embed"], provider, model, base_url, dim)?;
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
    dim: Option<u32>,
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
    if let Some(d) = dim {
        cur.insert("dim".into(), toml::Value::Integer(i64::from(d)));
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
