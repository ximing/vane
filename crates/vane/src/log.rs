use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::VaneCliError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

impl Level {
    fn as_str(self) -> &'static str {
        match self {
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
        }
    }
}

/// Local calendar date (year/month/day). Injected in tests so prune is clock-free.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct NaiveDate {
    year: i32,
    month: u8,
    day: u8,
}

impl NaiveDate {
    pub fn from_ymd(year: i32, month: u8, day: u8) -> Option<Self> {
        if !(1..=12).contains(&month) {
            return None;
        }
        if day < 1 || day > days_in_month(year, month) {
            return None;
        }
        Some(Self { year, month, day })
    }

    pub fn today_local() -> Self {
        local_civil_date().unwrap_or(Self {
            year: 1970,
            month: 1,
            day: 1,
        })
    }

    fn ymd_string(self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    fn rata_die(self) -> i32 {
        days_from_civil(self.year, u32::from(self.month), u32::from(self.day))
    }
}

pub struct DailyLogger {
    dir: PathBuf,
    retain_days: u32,
    current_date: Option<NaiveDate>,
    file: Option<File>,
}

impl DailyLogger {
    pub fn open(dir: &Path, retain_days: u32) -> Result<Self, VaneCliError> {
        if retain_days < 1 {
            return Err(VaneCliError::new(format!(
                "log.retain_days must be >= 1, got {retain_days}"
            )));
        }
        fs::create_dir_all(dir)
            .map_err(|e| VaneCliError::new(format!("create log dir {}: {e}", dir.display())))?;
        let logger = Self {
            dir: dir.to_path_buf(),
            retain_days,
            current_date: None,
            file: None,
        };
        logger.prune_with_today(NaiveDate::today_local());
        Ok(logger)
    }

    pub fn write(&mut self, level: Level, msg: &str) {
        self.write_on(NaiveDate::today_local(), level, msg);
    }

    /// Write using an injected local calendar date (tests + rotation).
    pub fn write_on(&mut self, today: NaiveDate, level: Level, msg: &str) {
        if self.current_date != Some(today) {
            if let Err(e) = self.reopen(today) {
                eprintln!("vane log write failed: {e}");
                return;
            }
            self.prune_with_today(today);
        }
        let line = format!(
            "{} {} {}\n",
            format_local_timestamp(),
            level.as_str(),
            sanitize(msg)
        );
        if let Err(e) = self.write_line(&line) {
            eprintln!("vane log write failed: {e}");
        }
    }

    pub fn prune_with_today(&self, today: NaiveDate) {
        let entries = match fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        let today_rd = today.rata_die();
        let oldest = today_rd - (self.retain_days as i32 - 1);
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if name == "daemon.log" {
                let _ = fs::remove_file(entry.path());
                continue;
            }
            let Some(date) = parse_daemon_log_name(name) else {
                continue;
            };
            let rd = date.rata_die();
            if rd < oldest || rd > today_rd {
                let _ = fs::remove_file(entry.path());
            }
        }
    }

    fn reopen(&mut self, today: NaiveDate) -> Result<(), VaneCliError> {
        self.file = None;
        let path = self.dir.join(format!("daemon.{}.log", today.ymd_string()));
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| VaneCliError::new(format!("open log {}: {e}", path.display())))?;
        self.file = Some(file);
        self.current_date = Some(today);
        Ok(())
    }

    fn write_line(&mut self, line: &str) -> Result<(), VaneCliError> {
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| VaneCliError::new("log file not open"))?;
        file.write_all(line.as_bytes())
            .map_err(|e| VaneCliError::new(format!("write log: {e}")))?;
        let _ = file.flush();
        Ok(())
    }
}

fn sanitize(msg: &str) -> String {
    msg.replace(['\n', '\r'], " ")
}

fn parse_daemon_log_name(name: &str) -> Option<NaiveDate> {
    let rest = name.strip_prefix("daemon.")?;
    let rest = rest.strip_suffix(".log")?;
    if rest.len() != 10 {
        return None;
    }
    let bytes = rest.as_bytes();
    if bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    let year: i32 = rest[0..4].parse().ok()?;
    let month: u8 = rest[5..7].parse().ok()?;
    let day: u8 = rest[8..10].parse().ok()?;
    NaiveDate::from_ymd(year, month, day)
}

fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

/// Civil days since Unix epoch (Howard Hinnant).
fn days_from_civil(y: i32, m: u32, d: u32) -> i32 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = (y - era * 400) as u32;
    let mprime = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mprime + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe as i32 - 719468
}

fn local_civil_date() -> Option<NaiveDate> {
    let (year, month, day) = local_ymd()?;
    NaiveDate::from_ymd(year, month, day)
}

fn local_ymd() -> Option<(i32, u8, u8)> {
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        if t == -1 {
            return None;
        }
        let mut tm = std::mem::zeroed::<libc::tm>();
        if libc::localtime_r(&t, &mut tm).is_null() {
            return None;
        }
        let year = tm.tm_year + 1900;
        let month = u8::try_from(tm.tm_mon + 1).ok()?;
        let day = u8::try_from(tm.tm_mday).ok()?;
        Some((year, month, day))
    }
}

fn format_local_timestamp() -> String {
    let mut buf = [0u8; 64];
    let n = unsafe {
        let t = libc::time(std::ptr::null_mut());
        if t == -1 {
            return "1970-01-01T00:00:00+0000".into();
        }
        let mut tm = std::mem::zeroed::<libc::tm>();
        if libc::localtime_r(&t, &mut tm).is_null() {
            return "1970-01-01T00:00:00+0000".into();
        }
        libc::strftime(
            buf.as_mut_ptr().cast::<libc::c_char>(),
            buf.len(),
            c"%Y-%m-%dT%H:%M:%S%z".as_ptr(),
            &tm,
        )
    };
    if n == 0 {
        return "1970-01-01T00:00:00+0000".into();
    }
    String::from_utf8_lossy(&buf[..n as usize]).into_owned()
}
