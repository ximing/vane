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
    (
        "query.missing_query",
        "missing query — usage: vane query <text>",
    ),
    ("query.prompt", "Search query"),
    (
        "header.searching_one",
        "searching {root} · {n} live files · {mode}",
    ),
    (
        "header.searching_all",
        "searching {k} roots · {n} live files · {mode}",
    ),
    ("header.hybrid", "hybrid"),
    ("header.degraded", "BM25 (degraded: embedder unreachable)"),
    ("read.no_cache", "no recent query — run vane query first"),
    ("read.empty", "last query had no hits"),
    (
        "read.out_of_range",
        "no hit {n} — last query has {k} hits (1..={k})",
    ),
    (
        "read.stale",
        "this hit is stale — the file changed since the last query; re-run vane query or use vane read {n} --file",
    ),
    (
        "read.file_missing",
        "source file no longer exists: {path}",
    ),
    (
        "read.binary",
        "{path} is not a text file (extractor {extractor}) — open it directly",
    ),
    ("add.summary", "indexed {n} files"),
    (
        "add.summary_skipped",
        "indexed {n} files, {skipped} skipped — run vane issues",
    ),
    ("status.watching", "watching"),
    ("status.indexing", "indexing {scanned}/{total}"),
    ("status.indexed_ago", "indexed {ago}"),
    ("status.never_indexed", "never indexed"),
    ("status.skipped_hint", "{n} skipped — run vane issues"),
    ("status.pending_changes", "{n} pending changes"),
    (
        "bare.init_hint",
        "vane is not initialized — run vane init to set up",
    ),
];

pub const ZH_TABLE: &[(&str, &str)] = &[
    ("time.just_now", "刚刚"),
    ("time.seconds_ago", "{n} 秒前"),
    ("time.minutes_ago", "{n} 分钟前"),
    ("time.hours_ago", "{n} 小时前"),
    ("time.days_ago", "{n} 天前"),
    ("time.never", "从未"),
    (
        "query.missing_query",
        "缺少查询词 — 用法：vane query <文本>",
    ),
    ("query.prompt", "搜索关键词"),
    ("header.searching_one", "搜索 {root} · {n} 个文件 · {mode}"),
    (
        "header.searching_all",
        "搜索 {k} 个目录 · {n} 个文件 · {mode}",
    ),
    ("header.hybrid", "混合检索"),
    ("header.degraded", "BM25（降级：embedder 不可达）"),
    ("read.no_cache", "还没有可查的结果 — 先运行 vane query"),
    ("read.empty", "上次查询没有命中"),
    (
        "read.out_of_range",
        "没有第 {n} 条 — 上次查询共 {k} 条（1..={k}）",
    ),
    (
        "read.stale",
        "该结果已过期 — 文件在上次查询后发生变化；请重新 vane query 或用 vane read {n} --file",
    ),
    ("read.file_missing", "源文件已不存在：{path}"),
    (
        "read.binary",
        "{path} 不是文本文件（类型 {extractor}）— 请直接打开",
    ),
    ("add.summary", "已索引 {n} 个文件"),
    (
        "add.summary_skipped",
        "已索引 {n} 个文件，跳过 {skipped} 个 — 运行 vane issues 查看",
    ),
    ("status.watching", "正在监听"),
    ("status.indexing", "索引中 {scanned}/{total}"),
    ("status.indexed_ago", "{ago}完成索引"),
    ("status.never_indexed", "从未索引"),
    (
        "status.skipped_hint",
        "{n} 个文件被跳过 — 运行 vane issues 查看",
    ),
    ("status.pending_changes", "{n} 个待处理变更"),
    (
        "bare.init_hint",
        "vane 尚未初始化 — 运行 vane init 开始设置",
    ),
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
