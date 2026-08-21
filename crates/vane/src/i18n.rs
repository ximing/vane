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
    (
        "mcp.done_new_session",
        "done — start a new agent session (Claude / Cursor / Codex) for the vane tools to load",
    ),
    // Empty-query why (spec §2.5): en copy mirrors doctor::explain_empty_query.
    ("why.not_initialized", "not initialized — run `vane init`"),
    (
        "why.not_registered",
        "cwd is not a registered root — run `vane add` or pass --root / --all",
    ),
    ("why.still_indexing", "still indexing — wait and retry"),
    (
        "why.embedder",
        "embedder is down or degraded — check `vane doctor` and the embed provider",
    ),
    (
        "why.excluded",
        "query looks like an excluded or untyped path — adjust include/exclude or search another file",
    ),
    (
        "why.wrong_root",
        "no hits in this root — try `vane query --all` or pass --root",
    ),
    (
        "why.empty_index",
        "index is empty — run `vane add` and wait for reconcile",
    ),
    (
        "why.no_match",
        "no matching chunks — try different terms or `vane query --all`",
    ),
    // Init / add wizard prompts (wizard is interactive-only, so tr directly).
    (
        "wizard.already_initialized",
        "already initialized — empty answers keep the current value",
    ),
    (
        "wizard.provider",
        "Embedding provider (ollama / openai_compat)",
    ),
    ("wizard.provider_short", "Embedding provider"),
    ("wizard.model", "Model"),
    ("wizard.base_url", "Base URL"),
    ("wizard.api_key", "API key ({hint})"),
    ("wizard.api_key_keep_openai", "empty keeps OPENAI_API_KEY"),
    ("wizard.api_key_keep_vane", "empty keeps VANE_EMBED_API_KEY"),
    (
        "wizard.api_key_use_env",
        "empty uses OPENAI_API_KEY / VANE_EMBED_API_KEY",
    ),
    (
        "wizard.api_key_tty",
        "API key (empty keeps env / stored key)",
    ),
    (
        "wizard.dim",
        "Vector dimension (empty to probe from the API)",
    ),
    ("wizard.dim_tty", "Vector dimension (empty to probe)"),
    ("wizard.split", "Chunk split (markdown / plain)"),
    ("wizard.split_tty", "Chunk split"),
    ("wizard.max_chars", "Chunk max_chars"),
    ("wizard.overlap", "Chunk overlap_chars"),
    ("wizard.min_chars", "Chunk min_chars"),
    (
        "wizard.no_api_key",
        "warning: no API key; probe will likely 401. Enter a key, or export OPENAI_API_KEY / VANE_EMBED_API_KEY",
    ),
    ("wizard.first_root", "First project root (empty to skip)"),
    ("wizard.root_not_dir", "directory does not exist: {path}"),
    ("wizard.exclude_defaults", "Default excludes:"),
    (
        "wizard.exclude_drop",
        "Numbers to uncheck (comma-separated, empty to keep all)",
    ),
    (
        "wizard.exclude_extra",
        "Additional exclude glob or folder (empty to skip)",
    ),
    ("wizard.images", "Enable image types?"),
    ("wizard.install_service", "Install user service?"),
    (
        "wizard.write_project_toml",
        "Write .vane.toml in this repo (chunk / types)?",
    ),
    (
        "wizard.write_project_toml_tty",
        "Write .vane.toml in this repo?",
    ),
    ("wizard.intro", "Vane sidecar"),
    ("wizard.add_project_intro", "Add project"),
    ("wizard.hint_ollama", "local Ollama"),
    ("wizard.hint_openai_compat", "OpenAI-compatible HTTP"),
    ("wizard.hint_markdown", "split on ATX/Setext headings"),
    ("wizard.hint_plain", "ignore headings"),
    (
        "wizard.using_global_defaults",
        "using global chunk defaults",
    ),
    ("wizard.project_ready", "project policy ready"),
    ("wizard.continue_anyway", "Continue anyway?"),
    ("wizard.probe_ok", "probe ok, dim={dim}"),
    (
        "init.required",
        "not initialized: missing {path}; run `vane init`",
    ),
    // `vane watch` (spec §6.2): start header, daemon-down hint, scope errors,
    // interval guard, and the four per-event TTY lines.
    (
        "watch.start",
        "watching {root} — Ctrl-C to stop",
    ),
    (
        "watch.daemon_down",
        "daemon not running — index changes need vane start; showing current state only",
    ),
    (
        "watch.not_registered",
        "root is not registered: {path}",
    ),
    (
        "watch.bad_interval",
        "--interval-ms must be 100..=60000",
    ),
    ("watch.added", "added {path}"),
    ("watch.updated", "updated {path}"),
    ("watch.removed", "removed {path}"),
    ("watch.queued", "queued {path}"),
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
    (
        "mcp.done_new_session",
        "完成 — 新开一轮 Agent 会话（Claude / Cursor / Codex）后 vane 工具才会加载",
    ),
    // 空结果 why（spec §2.5）：id 与 doctor::explain_empty_query 一致。
    ("why.not_initialized", "尚未初始化 — 运行 vane init"),
    (
        "why.not_registered",
        "当前目录不是已注册的 root — 运行 vane add 或加 --root / --all",
    ),
    ("why.still_indexing", "仍在索引中 — 请稍后重试"),
    (
        "why.embedder",
        "embedder 不可用或已降级 — 运行 vane doctor 并检查 embed provider",
    ),
    (
        "why.excluded",
        "查询词看起来像被排除或未启用类型的路径 — 调整 include/exclude 或换个文件试试",
    ),
    (
        "why.wrong_root",
        "该 root 下没有命中 — 试试 vane query --all 或加 --root",
    ),
    (
        "why.empty_index",
        "索引为空 — 运行 vane add 并等待 reconcile",
    ),
    (
        "why.no_match",
        "没有匹配的 chunk — 换个关键词或试试 vane query --all",
    ),
    // init / add 向导提示语（向导仅交互使用，直接 tr）。
    ("wizard.already_initialized", "已初始化 — 留空则保留当前值"),
    (
        "wizard.provider",
        "Embedding provider（ollama / openai_compat）",
    ),
    ("wizard.provider_short", "Embedding provider"),
    ("wizard.model", "模型"),
    ("wizard.base_url", "Base URL"),
    ("wizard.api_key", "API key（{hint}）"),
    ("wizard.api_key_keep_openai", "留空则沿用 OPENAI_API_KEY"),
    ("wizard.api_key_keep_vane", "留空则沿用 VANE_EMBED_API_KEY"),
    (
        "wizard.api_key_use_env",
        "留空则使用 OPENAI_API_KEY / VANE_EMBED_API_KEY",
    ),
    (
        "wizard.api_key_tty",
        "API key（留空则沿用环境变量 / 已存 key）",
    ),
    ("wizard.dim", "向量维度（留空则从 API 探测）"),
    ("wizard.dim_tty", "向量维度（留空自动探测）"),
    ("wizard.split", "Chunk 切分方式（markdown / plain）"),
    ("wizard.split_tty", "Chunk 切分方式"),
    ("wizard.max_chars", "Chunk max_chars"),
    ("wizard.overlap", "Chunk overlap_chars"),
    ("wizard.min_chars", "Chunk min_chars"),
    (
        "wizard.no_api_key",
        "警告：未配置 API key；probe 很可能 401。请输入 key，或 export OPENAI_API_KEY / VANE_EMBED_API_KEY",
    ),
    ("wizard.first_root", "第一个项目目录（留空跳过）"),
    ("wizard.root_not_dir", "目录不存在：{path}"),
    ("wizard.exclude_defaults", "默认排除规则："),
    (
        "wizard.exclude_drop",
        "要取消勾选的编号（逗号分隔，留空全部保留）",
    ),
    (
        "wizard.exclude_extra",
        "额外的排除 glob 或目录（留空跳过）",
    ),
    ("wizard.images", "启用图片类型？"),
    ("wizard.install_service", "安装用户服务？"),
    (
        "wizard.write_project_toml",
        "在该仓库写入 .vane.toml（chunk / types）？",
    ),
    (
        "wizard.write_project_toml_tty",
        "在该仓库写入 .vane.toml？",
    ),
    ("wizard.intro", "Vane sidecar"),
    ("wizard.add_project_intro", "添加项目"),
    ("wizard.hint_ollama", "本地 Ollama"),
    ("wizard.hint_openai_compat", "OpenAI 兼容 HTTP"),
    ("wizard.hint_markdown", "按 ATX/Setext 标题切分"),
    ("wizard.hint_plain", "忽略标题"),
    ("wizard.using_global_defaults", "使用全局 chunk 默认值"),
    ("wizard.project_ready", "项目策略已就绪"),
    ("wizard.continue_anyway", "仍要继续吗？"),
    ("wizard.probe_ok", "探测成功，dim={dim}"),
    ("init.required", "尚未初始化：缺少 {path}；请运行 vane init"),
    // `vane watch`（spec §6.2）：启动提示、守护进程未运行提示、scope 校验、
    // interval 边界、四种事件的 TTY 行（zh 仅 TTY，非 TTY 由 pick 退回 en）。
    (
        "watch.start",
        "正在监听 {root} — Ctrl-C 停止",
    ),
    (
        "watch.daemon_down",
        "守护进程未运行 — 索引变更需要 vane start；当前仅展示现有状态",
    ),
    (
        "watch.not_registered",
        "目录未登记：{path}",
    ),
    (
        "watch.bad_interval",
        "--interval-ms 必须在 100..=60000 之间",
    ),
    ("watch.added", "新增 {path}"),
    ("watch.updated", "更新 {path}"),
    ("watch.removed", "移除 {path}"),
    ("watch.queued", "排队 {path}"),
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
