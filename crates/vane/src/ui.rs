use std::io::IsTerminal;
use std::time::Duration;

use console::style;
use indicatif::{ProgressBar, ProgressStyle};

pub fn colors_enabled() -> bool {
    std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal()
}

pub fn interactive() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

pub fn success(msg: &str) {
    if colors_enabled() {
        println!("{} {msg}", style("✔").green().bold());
    } else {
        println!("ok {msg}");
    }
}

pub fn warn(msg: &str) {
    if colors_enabled() {
        eprintln!("{} {msg}", style("⚠").yellow().bold());
    } else {
        eprintln!("warning: {msg}");
    }
}

pub fn error(msg: &str) {
    if colors_enabled() {
        eprintln!("{} {msg}", style("✘").red().bold());
    } else {
        eprintln!("{msg}");
    }
}

pub fn dim(msg: &str) -> String {
    if colors_enabled() {
        style(msg).dim().to_string()
    } else {
        msg.to_string()
    }
}

pub fn accent(msg: &str) -> String {
    if colors_enabled() {
        style(msg).cyan().bold().to_string()
    } else {
        msg.to_string()
    }
}

pub fn path_display(p: &std::path::Path) -> String {
    accent(&p.display().to_string())
}

pub fn spinner(message: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    if let Ok(style) = ProgressStyle::with_template("{spinner:.cyan} {msg}") {
        pb.set_style(style.tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"));
    }
    pb.set_message(message.to_string());
    pb.enable_steady_tick(Duration::from_millis(80));
    pb
}

pub fn print_hits(hits: &[serde_json::Value]) {
    if hits.is_empty() {
        warn("no hits");
        return;
    }
    for (i, hit) in hits.iter().enumerate() {
        let score = hit.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let path = hit.get("path").and_then(|v| v.as_str()).unwrap_or("-");
        let root = hit.get("root").and_then(|v| v.as_str()).unwrap_or("");
        let snippet = hit.get("snippet").and_then(|v| v.as_str()).unwrap_or("");
        let degraded = hit
            .get("degraded")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let id = hit.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let n = i + 1;
        if colors_enabled() {
            print!("{} ", style(format!("{n:>2}.")).cyan().bold());
            print!("{} ", style(format!("{score:.3}")).magenta());
            println!("{}", style(path).green());
        } else {
            println!("{n:>2}. {score:.3}  {path}");
        }
        if !root.is_empty() {
            println!("    {}", dim(root));
        }
        if !snippet.is_empty() {
            println!("    {snippet}");
        }
        if degraded {
            warn("    degraded (BM25 only; embedder unreachable)");
        }
        if !id.is_empty() {
            println!("    {}", dim(&format!("id {id}")));
        }
    }
}
