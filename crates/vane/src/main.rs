use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use serde_json::json;
use vane::config::load_config;
use vane::home::{default_fallback, resolve_home};
use vane::project::find_current_root;

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
