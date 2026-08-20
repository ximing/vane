use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
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

pub fn log_dir(home: &Path) -> PathBuf {
    home.join("log")
}

/// Last `n` lines from dated `daemon.YYYY-MM-DD.log` files, oldest-first.
/// Secrets (`api_key` / `sk-` / `sk-proj-`) are redacted.
pub fn recent_lines(home: &Path, n: usize) -> Vec<String> {
    if n == 0 {
        return Vec::new();
    }
    let files = dated_log_paths(&log_dir(home));
    let mut all = Vec::new();
    for path in files {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        for line in text.lines() {
            if line.is_empty() {
                continue;
            }
            all.push(redact_line(line));
        }
    }
    let start = all.len().saturating_sub(n);
    all[start..].to_vec()
}

/// Follow new complete lines after the current end of the newest dated log.
pub struct LogTail {
    dir: PathBuf,
    path: Option<PathBuf>,
    offset: u64,
    pending: String,
}

impl LogTail {
    pub fn open_at_end(home: &Path) -> Self {
        let dir = log_dir(home);
        let path = dated_log_paths(&dir).into_iter().next_back();
        let offset = path
            .as_ref()
            .and_then(|p| fs::metadata(p).ok())
            .map(|m| m.len())
            .unwrap_or(0);
        Self {
            dir,
            path,
            offset,
            pending: String::new(),
        }
    }

    /// Newly completed lines since the last poll, already redacted.
    pub fn poll(&mut self) -> Vec<String> {
        self.switch_if_newer();
        let Some(path) = self.path.clone() else {
            return Vec::new();
        };
        let Ok(mut file) = File::open(&path) else {
            return Vec::new();
        };
        let Ok(meta) = file.metadata() else {
            return Vec::new();
        };
        if meta.len() < self.offset {
            self.offset = 0;
            self.pending.clear();
        }
        if file.seek(SeekFrom::Start(self.offset)).is_err() {
            return Vec::new();
        }
        let mut buf = String::new();
        if file.read_to_string(&mut buf).is_err() {
            return Vec::new();
        }
        self.offset = self.offset.saturating_add(buf.len() as u64);
        self.pending.push_str(&buf);
        let mut out = Vec::new();
        while let Some(pos) = self.pending.find('\n') {
            let mut line = self.pending.drain(..=pos).collect::<String>();
            if line.ends_with('\n') {
                line.pop();
            }
            if line.ends_with('\r') {
                line.pop();
            }
            if !line.is_empty() {
                out.push(redact_line(&line));
            }
        }
        out
    }

    fn switch_if_newer(&mut self) {
        let newest = dated_log_paths(&self.dir).into_iter().next_back();
        match (&self.path, newest) {
            (_, None) => {}
            (Some(cur), Some(new)) if cur == &new => {}
            (_, Some(new)) => {
                self.path = Some(new);
                self.offset = 0;
                self.pending.clear();
            }
        }
    }
}

/// Redact `api_key` assignments and `sk-` / `sk-proj-` tokens.
pub fn redact_line(line: &str) -> String {
    redact_api_key_assignments(&redact_sk_tokens(line))
}

fn dated_log_paths(dir: &Path) -> Vec<PathBuf> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    let mut dated = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(date) = parse_daemon_log_name(name) else {
            continue;
        };
        dated.push((date, entry.path()));
    }
    dated.sort_by_key(|a| a.0);
    dated.into_iter().map(|(_, p)| p).collect()
}

fn redact_sk_tokens(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("sk-proj-") {
            let n = secret_token_len(after);
            out.push_str("sk-proj-***");
            rest = &after[n..];
        } else if let Some(after) = rest.strip_prefix("sk-") {
            let n = secret_token_len(after);
            out.push_str("sk-***");
            rest = &after[n..];
        } else {
            let ch = rest.chars().next().expect("rest non-empty");
            out.push(ch);
            rest = &rest[ch.len_utf8()..];
        }
    }
    out
}

fn secret_token_len(s: &str) -> usize {
    s.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
        .unwrap_or(s.len())
}

fn redact_api_key_assignments(input: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        let Some(rel) = lower[i..].find("api_key") else {
            out.push_str(&input[i..]);
            break;
        };
        let start = i + rel;
        let end = start + "api_key".len();
        let before_ok = start == 0 || !is_ident_byte(input.as_bytes()[start - 1]);
        let after_ok = end >= input.len() || !is_ident_byte(input.as_bytes()[end]);
        if !before_ok || !after_ok {
            out.push_str(&input[i..start + 1]);
            i = start + 1;
            continue;
        }
        out.push_str(&input[i..end]);
        let after = &input[end..];
        let sep_end = skip_ws_quotes(after);
        let rem = &after[sep_end..];
        let Some(first) = rem.chars().next() else {
            out.push_str(after);
            break;
        };
        if first != '=' && first != ':' {
            i = end;
            continue;
        }
        out.push_str(&after[..sep_end]);
        out.push(first);
        let after_eq = &rem[first.len_utf8()..];
        let quote_end = skip_ws_quotes(after_eq);
        out.push_str(&after_eq[..quote_end]);
        let value = &after_eq[quote_end..];
        let val_len = value_token_len(value);
        out.push_str("***");
        i = end + sep_end + first.len_utf8() + quote_end + val_len;
    }
    out
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn skip_ws_quotes(s: &str) -> usize {
    s.chars()
        .take_while(|c| c.is_whitespace() || *c == '"' || *c == '\'')
        .map(|c| c.len_utf8())
        .sum()
}

fn value_token_len(s: &str) -> usize {
    s.find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ',' | '}' | ';' | '\n' | '\r'))
        .unwrap_or(s.len())
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
