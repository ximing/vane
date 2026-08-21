use std::io::IsTerminal;
use std::time::Duration;

use console::style;
use indicatif::{ProgressBar, ProgressStyle};

use crate::config::InspectReport;
use crate::doctor::{CheckLevel, DoctorReport};
use crate::gc::GcReport;
use crate::home::DiskStats;
use crate::i18n::Lang;
use crate::mcp::McpInstallReport;
use crate::progress::{IssuesReport, ProgressPhase};

pub fn colors_enabled() -> bool {
    std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal()
}

pub fn interactive() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

pub fn confirm(prompt: &str, initial: bool) -> Result<bool, crate::error::VaneCliError> {
    cliclack::confirm(prompt)
        .initial_value(initial)
        .interact()
        .map_err(|e| crate::error::VaneCliError::new(format!("{e}")))
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

/// Fold a leading $HOME into `~` for display.
pub fn collapse_home(path: &str) -> String {
    if let Some(home) = std::env::var_os("HOME") {
        let home = home.to_string_lossy();
        if path == home.as_ref() {
            return "~".to_string();
        }
        if let Some(rest) = path.strip_prefix(format!("{home}/").as_str()) {
            return format!("~/{rest}");
        }
    }
    path.to_string()
}

/// The one-line scope header printed before TTY query hits (spec §2.1).
pub fn format_scope_header(
    root: Option<&str>,
    roots: usize,
    live: u64,
    degraded: bool,
    lang: Lang,
    colors: bool,
) -> String {
    let mode = if degraded {
        crate::i18n::tr(lang, "header.degraded").to_string()
    } else {
        crate::i18n::tr(lang, "header.hybrid").to_string()
    };
    let text = match root {
        Some(r) => crate::i18n::tr(lang, "header.searching_one")
            .replace("{root}", r)
            .replace("{n}", &live.to_string())
            .replace("{mode}", &mode),
        None => crate::i18n::tr(lang, "header.searching_all")
            .replace("{k}", &roots.to_string())
            .replace("{n}", &live.to_string())
            .replace("{mode}", &mode),
    };
    if colors && degraded {
        // Approved simplification: only the degraded mode substring is colored.
        text.replace(&mode, &style(&mode).yellow().to_string())
    } else {
        text
    }
}

/// One-line human summary after `vane add` (TTY only; the machine line stays).
/// `added + unchanged` is the indexed work-set total.
pub fn format_add_summary(added: u64, unchanged: u64, skipped: u64, lang: Lang) -> String {
    let n = added + unchanged;
    let key = if skipped == 0 {
        "add.summary"
    } else {
        "add.summary_skipped"
    };
    crate::i18n::tr(lang, key)
        .replace("{n}", &n.to_string())
        .replace("{skipped}", &skipped.to_string())
}

pub struct HitLineOpts {
    pub all: bool,
    pub verbose: bool,
    pub header_degraded: bool,
}

/// Pure per-hit line assembly; `print_hits` is a thin shell over this.
pub fn hit_lines(
    hit: &serde_json::Value,
    index: usize,
    opts: &HitLineOpts,
    colors: bool,
) -> Vec<String> {
    let score = hit.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let path = hit.get("path").and_then(|v| v.as_str()).unwrap_or("-");
    let root = hit.get("root").and_then(|v| v.as_str()).unwrap_or("");
    let snippet = hit.get("snippet").and_then(|v| v.as_str()).unwrap_or("");
    let degraded = hit
        .get("degraded")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let id = hit.get("id").and_then(|v| v.as_str()).unwrap_or("");
    let dim_if = |msg: &str| {
        if colors {
            style(msg).dim().to_string()
        } else {
            msg.to_string()
        }
    };
    let n = index + 1;
    let mut lines = Vec::new();
    if colors {
        lines.push(format!(
            "{} {} {}",
            style(format!("{n:>2}.")).cyan().bold(),
            style(format!("{score:.3}")).magenta(),
            style(path).green()
        ));
    } else {
        lines.push(format!("{n:>2}. {score:.3}  {path}"));
    }
    if opts.all && !root.is_empty() {
        lines.push(format!("    {}", dim_if(root)));
    }
    if !snippet.is_empty() {
        lines.push(format!("    {snippet}"));
    }
    if degraded && !opts.header_degraded {
        if colors {
            lines.push(format!(
                "    {} degraded (BM25 only; embedder unreachable)",
                style("⚠").yellow().bold()
            ));
        } else {
            lines.push("    warning: degraded (BM25 only; embedder unreachable)".to_string());
        }
    }
    if opts.verbose && !id.is_empty() {
        lines.push(format!("    {}", dim_if(&format!("id {id}"))));
    }
    lines
}

pub fn print_hits(hits: &[serde_json::Value], all: bool, verbose: bool, header_degraded: bool) {
    if hits.is_empty() {
        warn("no hits");
        return;
    }
    let opts = HitLineOpts {
        all,
        verbose,
        header_degraded,
    };
    let colors = colors_enabled();
    for (i, hit) in hits.iter().enumerate() {
        for line in hit_lines(hit, i, &opts, colors) {
            println!("{line}");
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
    println!(
        "{}",
        crate::i18n::tr(crate::i18n::Lang::detect(), "mcp.done_new_session")
    );
}

/// TTY-only: render the empty-query reason in the detected language, falling
/// back to the (dynamic, English) message when the key is unknown.
pub fn print_why(id: &str, fallback_en: &str) {
    let lang = Lang::detect();
    let key = format!("why.{id}");
    let text = crate::i18n::tr(lang, &key);
    let reason = if text == "missing-i18n-key" {
        fallback_en
    } else {
        text
    };
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
    let lang = Lang::detect();
    for check in &report.checks {
        let (message, fix) = if lang == Lang::Zh && !check.message_zh.is_empty() {
            (check.message_zh.as_str(), check.fix_zh.as_str())
        } else {
            (check.message.as_str(), check.fix.as_str())
        };
        print_doctor_check(check.level, &check.id, message, fix);
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

pub struct RootStatusView {
    pub path: String,
    pub live: u64,
    pub last_reconcile: Option<u64>,
    pub model: String,
    pub dim: Option<u64>,
    pub dirty: u64,
    pub skips: u64,
    pub last_error: Option<String>,
}

pub struct StatusView {
    pub home: String,
    pub running: bool,
    pub indexing: Option<(u64, u64)>, // (scanned, total) when progress phase != Idle
    pub dirty_total: u64,
    pub disk_home: u64,
    pub disk_cas: u64,
    pub last_error: Option<String>,
    pub roots: Vec<RootStatusView>,
}

/// Defensive JSON → view extraction (same read paths as the old dashboard).
pub fn status_view(v: &serde_json::Value, indexing: Option<(u64, u64)>) -> StatusView {
    let home = v
        .get("home")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let running = v.get("running").and_then(|x| x.as_bool()).unwrap_or(false);
    let dirty_total = v
        .get("dirty_queue_size")
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    let (disk_home, disk_cas) = v
        .get("disk")
        .map(|d| {
            (
                d.get("home_bytes").and_then(|x| x.as_u64()).unwrap_or(0),
                d.get("cas_bytes").and_then(|x| x.as_u64()).unwrap_or(0),
            )
        })
        .unwrap_or((0, 0));
    let last_error = last_error_text(v.get("last_error"));
    let roots = v
        .get("roots")
        .and_then(|r| r.as_array())
        .map(|arr| {
            arr.iter()
                .map(|root| RootStatusView {
                    path: root
                        .get("path")
                        .and_then(|x| x.as_str())
                        .unwrap_or("?")
                        .to_string(),
                    live: root.get("live_files").and_then(|x| x.as_u64()).unwrap_or(0),
                    last_reconcile: root.get("last_reconcile").and_then(|x| x.as_u64()),
                    model: root
                        .get("model")
                        .and_then(|x| x.as_str())
                        .unwrap_or("-")
                        .to_string(),
                    dim: root.get("dim").and_then(|x| x.as_u64()),
                    dirty: root
                        .get("dirty_queue_size")
                        .and_then(|x| x.as_u64())
                        .unwrap_or(0),
                    skips: root.get("skip_count").and_then(|x| x.as_u64()).unwrap_or(0),
                    last_error: last_error_text(root.get("last_error")),
                })
                .collect()
        })
        .unwrap_or_default();
    StatusView {
        home,
        running,
        indexing,
        dirty_total,
        disk_home,
        disk_cas,
        last_error,
        roots,
    }
}

/// Plain-text (no color) status lines; the unit-testable data layer.
pub fn format_status_lines(view: &StatusView, lang: Lang, now: u64) -> Vec<String> {
    use crate::i18n::tr;
    let mut lines = Vec::new();
    if !view.home.is_empty() {
        lines.push(format!("home {}", view.home));
    }
    let daemon = if !view.running {
        // Fixed English in both languages (not in the i18n tables).
        "daemon not running — vane start".to_string()
    } else if let Some((s, t)) = view.indexing {
        format!(
            "daemon {}",
            tr(lang, "status.indexing")
                .replace("{scanned}", &s.to_string())
                .replace("{total}", &t.to_string())
        )
    } else {
        format!("daemon {}", tr(lang, "status.watching"))
    };
    lines.push(daemon);
    lines.push(format!("dirty {}", view.dirty_total));
    lines.push(format!(
        "disk {}  cas {}",
        fmt_bytes(view.disk_home),
        fmt_bytes(view.disk_cas)
    ));
    if let Some(err) = &view.last_error {
        lines.push(format!("last_error {err}"));
    }
    if view.roots.is_empty() {
        lines.push("no registered roots".to_string());
        return lines;
    }
    for r in &view.roots {
        lines.push(format!("root {}", r.path));
        lines.push(format!("  live_files {}", r.live));
        let reconciled = match r.last_reconcile {
            Some(ts) if ts > 0 => tr(lang, "status.indexed_ago")
                .replace("{ago}", &crate::humanize::rel_time(ts, now, lang)),
            _ => tr(lang, "status.never_indexed").to_string(),
        };
        lines.push(format!("  {reconciled}"));
        let dim_s = r.dim.map(|d| d.to_string()).unwrap_or_else(|| "-".into());
        lines.push(format!("  model {}  dim {dim_s}", r.model));
        if r.dirty > 0 {
            lines.push(format!(
                "  {}",
                tr(lang, "status.pending_changes").replace("{n}", &r.dirty.to_string())
            ));
        }
        if let Some(err) = &r.last_error {
            lines.push(format!("  last_error {err}"));
        }
        if r.skips > 0 {
            lines.push(format!(
                "  {}",
                tr(lang, "status.skipped_hint").replace("{n}", &r.skips.to_string())
            ));
        }
    }
    lines
}

/// Thin shell: load progress, detect lang, print with minimal coloring.
pub fn print_status_dashboard(home: &std::path::Path, v: &serde_json::Value) {
    let indexing = crate::progress::load_progress(home)
        .filter(|p| p.phase != ProgressPhase::Idle)
        .map(|p| (p.scanned, p.total_estimate));
    let view = status_view(v, indexing);
    let lang = Lang::detect();
    let now = crate::progress::unix_now();
    let colors = colors_enabled();
    for line in format_status_lines(&view, lang, now) {
        println!("{}", color_status_line(&line, colors));
    }
}

/// Minimal coloring: root/home paths and numbers only; everything else plain.
fn color_status_line(line: &str, colors: bool) -> String {
    if !colors {
        return line.to_string();
    }
    if let Some(rest) = line.strip_prefix("home ") {
        return format!("{} {}", style("home").dim(), style(rest).cyan().bold());
    }
    if let Some(rest) = line.strip_prefix("root ") {
        return format!("{} {}", style("root").cyan().bold(), style(rest).green());
    }
    if line == "daemon not running — vane start" {
        return format!("{} {line}", style("⚠").yellow().bold());
    }
    line.to_string()
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
