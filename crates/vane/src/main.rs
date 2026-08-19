use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use vane::home::{default_fallback, resolve_home};

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
    }
}
