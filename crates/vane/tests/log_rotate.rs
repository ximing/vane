use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use vane::log::{DailyLogger, Level, NaiveDate};

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
