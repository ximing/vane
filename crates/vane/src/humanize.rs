use crate::i18n::{tr, Lang};

/// Human relative time. `ts == 0` → "never"; future ts (clock skew) → "just now".
pub fn rel_time(ts: u64, now: u64, lang: Lang) -> String {
    if ts == 0 {
        return tr(lang, "time.never").to_string();
    }
    if ts >= now {
        return tr(lang, "time.just_now").to_string();
    }
    let d = now - ts;
    let (key, n) = if d < 10 {
        return tr(lang, "time.just_now").to_string();
    } else if d < 60 {
        ("time.seconds_ago", d)
    } else if d < 3_600 {
        ("time.minutes_ago", d / 60)
    } else if d < 48 * 3_600 {
        ("time.hours_ago", d / 3_600)
    } else if d < 30 * 86_400 {
        ("time.days_ago", d / 86_400)
    } else {
        return abs_date(ts);
    };
    tr(lang, key).replace("{n}", &n.to_string())
}

/// `YYYY-MM-DD` from unix seconds (Howard Hinnant's civil_from_days; no chrono).
pub fn abs_date(ts: u64) -> String {
    let days = (ts / 86_400) as i64;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}
