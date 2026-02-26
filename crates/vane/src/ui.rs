use std::io::IsTerminal;
use std::time::Duration;

use console::style;
use indicatif::{ProgressBar, ProgressStyle};

use crate::doctor::{CheckLevel, DoctorReport};

pub fn colors_enabled() -> bool {
    std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal()
}

pub fn interactive() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

/// Structured command output: TTY is human; piped is JSON.
pub fn stdout_tty() -> bool {
    std::io::stdout().is_terminal()
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

pub fn print_why(reason: &str) {
    if colors_enabled() {
        println!(
            "{} {}",
            style("why").yellow().bold(),
            style(reason).yellow()
        );
    } else {
        println!("no hits — {reason}");
    }
}

pub fn print_doctor(report: &DoctorReport) {
    for check in &report.checks {
        print_doctor_check(check.level, &check.id, &check.message, &check.fix);
    }
}

fn print_doctor_check(level: CheckLevel, id: &str, message: &str, fix: &str) {
    let (mark, painted) = if colors_enabled() {
        match level {
            CheckLevel::Green => (
                style("✔").green().bold().to_string(),
                style(message).green().to_string(),
            ),
            CheckLevel::Yellow => (
                style("⚠").yellow().bold().to_string(),
                style(message).yellow().to_string(),
            ),
            CheckLevel::Red => (
                style("✘").red().bold().to_string(),
                style(message).red().to_string(),
            ),
        }
    } else {
        let tag = match level {
            CheckLevel::Green => "ok",
            CheckLevel::Yellow => "warn",
            CheckLevel::Red => "fail",
        };
        (tag.to_string(), message.to_string())
    };
    println!("{mark} {:<12} {painted}", id);
    if !fix.is_empty() && level != CheckLevel::Green {
        println!("  {}", dim(&format!("fix: {fix}")));
    }
}

pub fn print_status_dashboard(v: &serde_json::Value) {
    if let Some(home) = v.get("home").and_then(|x| x.as_str()) {
        println!("{} {}", dim("home"), accent(home));
    }
    let running = v.get("running").and_then(|x| x.as_bool()).unwrap_or(false);
    if running {
        success("daemon running");
    } else {
        warn("daemon not running — vane start");
    }
    if let Some(n) = v.get("dirty_queue_size").and_then(|x| x.as_u64()) {
        println!("{} {}", dim("dirty"), accent(&n.to_string()));
    }
    if let Some(disk) = v.get("disk") {
        let home_b = disk.get("home_bytes").and_then(|x| x.as_u64()).unwrap_or(0);
        let cas_b = disk.get("cas_bytes").and_then(|x| x.as_u64()).unwrap_or(0);
        println!(
            "{} {}  {} {}",
            dim("disk"),
            accent(&fmt_bytes(home_b)),
            dim("cas"),
            dim(&fmt_bytes(cas_b))
        );
    }
    if let Some(err) = last_error_text(v.get("last_error")) {
        println!("{} {err}", dim("last_error"));
    }
    let Some(roots) = v.get("roots").and_then(|p| p.as_array()) else {
        return;
    };
    if roots.is_empty() {
        println!("{}", dim("no registered roots"));
        return;
    }
    for root in roots {
        let path = root.get("path").and_then(|x| x.as_str()).unwrap_or("?");
        println!("{} {}", accent("root"), path);
        let live = root.get("live_files").and_then(|x| x.as_u64()).unwrap_or(0);
        println!("  {} {}", dim("live_files"), accent(&live.to_string()));
        match root.get("last_reconcile").and_then(|x| x.as_u64()) {
            Some(ts) => println!("  {} {ts}", dim("last_reconcile")),
            None => println!("  {} {}", dim("last_reconcile"), dim("never")),
        }
        let model = root.get("model").and_then(|x| x.as_str()).unwrap_or("-");
        let dim_v = root
            .get("dim")
            .and_then(|x| x.as_u64())
            .map(|d| d.to_string())
            .unwrap_or_else(|| "-".into());
        println!("  {} {}  {} {dim_v}", dim("model"), dim(model), dim("dim"));
        if let Some(n) = root.get("dirty_queue_size").and_then(|x| x.as_u64()) {
            println!("  {} {n}", dim("dirty"));
        }
        if let Some(err) = last_error_text(root.get("last_error")) {
            println!("  {} {err}", dim("last_error"));
        }
        if let Some(n) = root.get("skip_count").and_then(|x| x.as_u64()) {
            println!("  {} {n}", dim("skips"));
        }
    }
}

fn last_error_text(v: Option<&serde_json::Value>) -> Option<String> {
    let v = v?;
    if v.is_null() {
        return None;
    }
    if let Some(s) = v.as_str() {
        if s.is_empty() {
            return None;
        }
        return Some(s.to_string());
    }
    v.get("message")
        .and_then(|m| m.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn fmt_bytes(n: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * 1024;
    const GIB: u64 = 1024 * 1024 * 1024;
    if n >= GIB {
        format!("{:.1} GiB", n as f64 / GIB as f64)
    } else if n >= MIB {
        format!("{:.1} MiB", n as f64 / MIB as f64)
    } else if n >= KIB {
        format!("{:.1} KiB", n as f64 / KIB as f64)
    } else {
        format!("{n} B")
    }
}
