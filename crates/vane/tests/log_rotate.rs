use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use std::process::Command;

use vane::log::{recent_lines, redact_line, DailyLogger, Level, LogTail, NaiveDate};

struct TempHome {
    path: PathBuf,
}

fn tempfile_dir() -> TempHome {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "vane-log-rotate-test-{}-{}-{}",
        std::process::id(),
        nanos,
        n
    ));
    fs::create_dir_all(&path).unwrap();
    TempHome { path }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let tmp = std::env::temp_dir();
        if self.path.starts_with(&tmp) && self.path != tmp {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

impl std::ops::Deref for TempHome {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.path
    }
}

#[test]
fn prune_retain_days_keeps_window_and_drops_legacy() {
    let tmp = tempfile_dir();
    let log_dir = tmp.join("log");
    // open() prunes using the real local date; seed fixtures after that
    // so this assertion does not depend on the host clock.
    let mut logger = DailyLogger::open(&log_dir, 3).unwrap();

    fs::write(log_dir.join("daemon.2026-08-16.log"), "d16").unwrap();
    fs::write(log_dir.join("daemon.2026-08-17.log"), "d17").unwrap();
    fs::write(log_dir.join("daemon.log"), "legacy").unwrap();

    let today = NaiveDate::from_ymd(2026, 8, 19).expect("valid fixture date");
    logger.prune_with_today(today);

    assert!(
        !log_dir.join("daemon.2026-08-16.log").exists(),
        "16th is outside retain_days=3 window for 2026-08-19"
    );
    assert!(
        log_dir.join("daemon.2026-08-17.log").exists(),
        "17th is the oldest day that must be kept"
    );
    assert!(
        !log_dir.join("daemon.log").exists(),
        "undated legacy daemon.log is deleted on prune"
    );
    assert!(
        !log_dir.join("daemon.2026-08-19.log").exists(),
        "today's file is created on the next write, not by prune"
    );

    logger.write_on(today, Level::Info, "hello");
    let today_path = log_dir.join("daemon.2026-08-19.log");
    assert!(today_path.is_file(), "write_on creates today's dated file");
    let body = fs::read_to_string(&today_path).unwrap();
    assert!(
        body.contains("INFO hello"),
        "log line must include LEVEL and message, got {body:?}"
    );
}

fn write_min_config(home: &Path) {
    let cfg = home.join("config").join("config.toml");
    fs::create_dir_all(cfg.parent().unwrap()).unwrap();
    fs::write(
        &cfg,
        r#"
[defaults.embed]
provider = "ollama"
model = "nomic-embed-text"
base_url = "http://127.0.0.1:9"
"#,
    )
    .unwrap();
}

fn fake_user_home(home: &Path) -> PathBuf {
    let fake = home.join("uh");
    fs::create_dir_all(&fake).unwrap();
    fake
}

fn run_cli(home: &Path, args: &[&str]) -> (i32, String, String) {
    let bin = env!("CARGO_BIN_EXE_vane");
    let output = Command::new(bin)
        .args(["--home", home.to_str().expect("utf-8 home")])
        .args(args)
        .env("VANE_HOME", home)
        .env("HOME", fake_user_home(home))
        .env_remove("XDG_CONFIG_HOME")
        .output()
        .expect("run vane");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn redact_line_strips_sk_and_api_key() {
    let line = r#"ERROR embed 401 api_key=sk-secretvalue also sk-proj-tokendata"#;
    let red = redact_line(line);
    assert!(
        !red.contains("secretvalue") && !red.contains("tokendata"),
        "secrets must be redacted, got {red}"
    );
    assert!(
        red.contains("sk-proj-***") && red.contains("***"),
        "sk tokens become *** , got {red}"
    );
    assert!(
        !red.contains("api_key=sk-secret") && red.contains("api_key="),
        "api_key assignment value redacted, got {red}"
    );
    let quoted = redact_line(r#"warn "api_key": "hunter2""#);
    assert!(
        !quoted.contains("hunter2"),
        "quoted api_key value must go, got {quoted}"
    );
}

#[test]
fn recent_lines_reads_fake_daily_log_and_redacts() {
    let tmp = tempfile_dir();
    let log_dir = tmp.join("log");
    fs::create_dir_all(&log_dir).unwrap();
    fs::write(
        log_dir.join("daemon.2026-08-18.log"),
        "2026-08-18T00:00:00+0000 INFO older\n",
    )
    .unwrap();
    fs::write(
        log_dir.join("daemon.2026-08-19.log"),
        "2026-08-19T00:00:00+0000 INFO keep-me\n2026-08-19T00:00:01+0000 ERROR key sk-abc123leak\n",
    )
    .unwrap();
    let lines = recent_lines(&tmp, 2);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].contains("keep-me"), "{lines:?}");
    assert!(lines[1].contains("ERROR"), "{lines:?}");
    assert!(
        !lines.iter().any(|l| l.contains("abc123leak")),
        "recent_lines must redact, got {lines:?}"
    );
    assert!(lines[1].contains("sk-***"), "{lines:?}");
}

#[test]
fn log_tail_picks_up_appended_line() {
    let tmp = tempfile_dir();
    let log_dir = tmp.join("log");
    fs::create_dir_all(&log_dir).unwrap();
    let path = log_dir.join("daemon.2026-08-19.log");
    fs::write(&path, "2026-08-19T00:00:00+0000 INFO first\n").unwrap();
    let mut tail = LogTail::open_at_end(&tmp);
    assert!(tail.poll().is_empty(), "open_at_end should start at EOF");
    use std::io::Write;
    let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
    writeln!(f, "2026-08-19T00:00:01+0000 INFO second sk-xyz").unwrap();
    f.flush().unwrap();
    let got = tail.poll();
    assert_eq!(got.len(), 1, "{got:?}");
    assert!(got[0].contains("second"), "{got:?}");
    assert!(!got[0].contains("xyz"), "{got:?}");
}

#[test]
fn logs_cli_piped_json_from_temp_home() {
    let tmp = tempfile_dir();
    assert!(tmp.starts_with(std::env::temp_dir()));
    write_min_config(&tmp);
    let log_dir = tmp.join("log");
    fs::create_dir_all(&log_dir).unwrap();
    fs::write(
        log_dir.join("daemon.2026-08-19.log"),
        "2026-08-19T10:00:00+0000 INFO started\n\
         2026-08-19T10:00:01+0000 ERROR auth api_key=sk-proj-supersecret\n\
         2026-08-19T10:00:02+0000 WARN later\n",
    )
    .unwrap();

    let (code, stdout, stderr) = run_cli(&tmp, &["logs", "--lines", "2"]);
    assert_eq!(code, 0, "stderr={stderr}");
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("logs JSON");
    let lines = v["lines"].as_array().expect("lines array");
    assert_eq!(lines.len(), 2, "{v}");
    let joined = lines
        .iter()
        .map(|l| l.as_str().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("later"), "{v}");
    assert!(
        !joined.contains("supersecret") && !stdout.contains("supersecret"),
        "must never print secrets: {stdout}"
    );
    assert!(
        !stdout.contains("\"api_key\":") && !joined.contains("sk-proj-supersecret"),
        "{stdout}"
    );
}
