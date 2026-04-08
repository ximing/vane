/// Minimal i18n: two static tables, named placeholders replaced by callers.
/// HARD RULE (spec §5.1): zh only ever reaches TTY rendering. Anything going to
/// JSON, persisted files, or non-TTY stderr must come from `pick(.., false, ..)`.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Zh,
}

impl Lang {
    pub fn detect() -> Self {
        Self::detect_with(&|k| std::env::var(k).ok())
    }

    pub fn detect_with(get: &dyn Fn(&str) -> Option<String>) -> Self {
        for key in ["VANE_LANG", "LC_ALL", "LANG"] {
            if let Some(v) = get(key) {
                let v = v.trim();
                if !v.is_empty() {
                    return if v.to_ascii_lowercase().starts_with("zh") {
                        Self::Zh
                    } else {
                        Self::En
                    };
                }
            }
        }
        Self::En
    }
}

pub const EN_TABLE: &[(&str, &str)] = &[
    ("time.just_now", "just now"),
    ("time.seconds_ago", "{n}s ago"),
    ("time.minutes_ago", "{n} min ago"),
    ("time.hours_ago", "{n} hours ago"),
    ("time.days_ago", "{n} days ago"),
    ("time.never", "never"),
];

pub const ZH_TABLE: &[(&str, &str)] = &[
    ("time.just_now", "刚刚"),
    ("time.seconds_ago", "{n} 秒前"),
    ("time.minutes_ago", "{n} 分钟前"),
    ("time.hours_ago", "{n} 小时前"),
    ("time.days_ago", "{n} 天前"),
    ("time.never", "从未"),
];

pub fn tr(lang: Lang, key: &str) -> &'static str {
    let table = match lang {
        Lang::En => EN_TABLE,
        Lang::Zh => ZH_TABLE,
    };
    lookup(table, key)
        .or_else(|| lookup(EN_TABLE, key))
        .unwrap_or("missing-i18n-key")
}

pub fn pick(lang: Lang, tty: bool, key: &str) -> &'static str {
    if tty {
        tr(lang, key)
    } else {
        tr(Lang::En, key)
    }
}

fn lookup(table: &'static [(&'static str, &'static str)], key: &str) -> Option<&'static str> {
    table.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}
