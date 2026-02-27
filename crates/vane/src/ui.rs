use std::io::IsTerminal;
use std::time::Duration;

use console::style;
use indicatif::{ProgressBar, ProgressStyle};

use crate::config::InspectReport;
use crate::doctor::{CheckLevel, DoctorReport};
use crate::gc::GcReport;
use crate::home::DiskStats;
use crate::mcp::McpInstallReport;
use crate::progress::IssuesReport;

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

pub fn print_issues(report: &IssuesReport) {
    if report.roots.is_empty() {
        println!("{}", dim("no registered roots"));
        return;
    }
    if report.roots.iter().all(|r| r.files.is_empty()) {
        success("no skipped files");
        return;
    }
    for root in &report.roots {
        println!("{} {}", accent("root"), root.path);
        if root.files.is_empty() {
            println!("  {}", dim("no skipped files"));
            continue;
        }
        for file in &root.files {
            println!(
                "  {}  {}  {}",
                accent(&file.path),
                dim(file.reason.as_str()),
                file.detail
            );
        }
    }
}

pub fn print_next_steps(home: &std::path::Path) {
    let running = crate::daemon::is_running(home);
    let card = crate::wizard::next_steps_card(running);
    println!();
    for (i, line) in card.lines().enumerate() {
        if i == 0 {
            println!("{}", accent(line));
        } else if colors_enabled() {
            println!("  {}", dim(line));
        } else {
            println!("  {line}");
        }
    }
}

pub fn print_mcp_install(report: &McpInstallReport) {
    let rows = if report.dry_run {
        report.would_write.as_slice()
    } else {
        report.written.as_slice()
    };
    if rows.is_empty() && report.skipped.is_empty() {
        println!("{}", dim("no MCP client configs to write"));
        return;
    }
    for row in rows {
        let verb = if report.dry_run {
            "would write"
        } else {
            "wrote"
        };
        success(&format!("{verb} {} ({})", row.path, row.action));
    }
    for skip in &report.skipped {
        println!(
            "  {} {}",
            dim("skip"),
            dim(&format!("{} ({})", skip.path, skip.reason))
        );
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

pub fn print_log_line(line: &str) {
    if colors_enabled() {
        let level = line.split_whitespace().nth(1).unwrap_or("");
        match level {
            "ERROR" => println!("{}", style(line).red()),
            "WARN" => println!("{}", style(line).yellow()),
            _ => println!("{line}"),
        }
    } else {
        println!("{line}");
    }
}

pub fn print_inspect(report: &InspectReport) {
    match report.root.as_deref() {
        Some(root) => {
            let pid = report.project_id.as_deref().unwrap_or("-");
            println!("{} {}  {} {pid}", accent("root"), root, dim("id"));
        }
        None => println!("{}", accent("global defaults")),
    }
    println!(
        "{} {} / {}  {} {}  {} {}",
        dim("embed"),
        report.embed.provider,
        report.embed.model,
        dim("dim"),
        report
            .embed
            .dim
            .map(|d| d.to_string())
            .unwrap_or_else(|| "-".into()),
        dim("source"),
        accent(&report.source.embed)
    );
    if !report.embed.base_url.is_empty() {
        println!("  {} {}", dim("base_url"), report.embed.base_url);
    }
    println!(
        "{} {}  max={} overlap={} min={}  {} {}",
        dim("chunk"),
        report.chunk.split,
        report.chunk.max_chars,
        report.chunk.overlap_chars,
        report.chunk.min_chars,
        dim("source"),
        accent(&report.source.chunk)
    );
    println!(
        "{} {} {}",
        dim("exclude"),
        dim("source"),
        accent(&report.source.exclude)
    );
    print_string_layer("global", &report.exclude.global);
    print_string_layer("project", &report.exclude.project);
    print_string_layer("effective", &report.exclude.effective);
    println!(
        "{} {} {}",
        dim("types"),
        dim("source"),
        accent(&report.source.types)
    );
    print_types_layer("global", &report.types.global);
    print_types_layer("project", &report.types.project);
    print_types_layer("effective", &report.types.effective);
}

fn print_string_layer(label: &str, items: &[String]) {
    if items.is_empty() {
        println!("  {} {}", dim(label), dim("(none)"));
        return;
    }
    for (i, item) in items.iter().enumerate() {
        if i == 0 {
            println!("  {} {item}", dim(label));
        } else {
            println!("  {} {item}", dim(""));
        }
    }
}

fn print_types_layer(label: &str, items: &[crate::config::TypeRule]) {
    if items.is_empty() {
        println!("  {} {}", dim(label), dim("(none)"));
        return;
    }
    for (i, t) in items.iter().enumerate() {
        let en = if t.enabled { "on" } else { "off" };
        let line = format!("{}  {}  {en}", t.glob, t.extractor);
        if i == 0 {
            println!("  {} {line}", dim(label));
        } else {
            println!("  {} {line}", dim(""));
        }
    }
}

pub fn print_df(home: &std::path::Path, stats: &DiskStats) {
    println!("{} {}", dim("home"), accent(&home.display().to_string()));
    println!(
        "{} {}  {} {}",
        dim("disk"),
        accent(&fmt_bytes(stats.home_bytes)),
        dim("cas"),
        dim(&fmt_bytes(stats.cas_bytes))
    );
    if stats.projects.is_empty() {
        println!("{}", dim("no project dbs"));
    } else {
        println!("{}", dim("projects"));
        for p in &stats.projects {
            println!(
                "  {} {}",
                accent(&p.project_id),
                dim(&fmt_bytes(p.db_bytes))
            );
        }
    }
    if stats.home_bytes > (1 << 30) {
        warn("home is larger than 1 GiB — run `vane gc --all`");
    }
}

pub fn print_gc(report: &GcReport) {
    let body = format!(
        "extract={}  embed={}  db_prev={}  projects={}  compacted={}  errors={}",
        report.extract_deleted,
        report.embed_deleted,
        report.db_prev_removed,
        report.projects_removed,
        report.compacted,
        report.errors
    );
    if report.dry_run {
        println!("{} {body}", dim("dry-run"));
    } else {
        success(&body);
    }
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
