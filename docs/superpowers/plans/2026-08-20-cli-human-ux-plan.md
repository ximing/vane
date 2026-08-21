# CLI Human UX 实施计划（v0.4.0）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 vane 产品 CLI 的搜索、状态、命令面、文案对人说人话（scope 头行、`vane read`、相对时间、命令分组、中文文案、`vane watch`），不改变任何 agent 消费面。

**Architecture:** 全部新逻辑进 `crates/vane` lib 的纯函数/数据结构层（`i18n` / `humanize` / `last_query` / `watch_diff` / `dispatch` / `fsutil`），`main.rs` 只做 clap 解析、IO 绑定与退出码。i18n 硬规则：只作用于 TTY 渲染瞬间；JSON 输出、持久化文件、非 TTY stderr 一律英文。不改 daemon、不改 RPC schema、不改磁盘格式、不加依赖。

**Tech Stack:** Rust 2021、clap 4 derive、cliclack 0.3、indicatif 0.17、console 0.15、serde_json。测试：std 进程（`env!("CARGO_BIN_EXE_vane")`）+ lib 集成测试。

**Spec:** `docs/superpowers/specs/2026-08-20-cli-human-ux-spec.md`（v1.2，已冻结）。执行者必须先读 spec；本计划每个 Task 标注对应 spec 章节。

**评审状态:** v1.1（2026-08-20）——评审 APPROVE_WITH_CHANGES 的 6 条必须修正已全部闭环：① i18n 双表直接 pub 比较（无 KEYS 别名，parity 测试不失效）；② 临时目录用仓库手写 TempHome 惯例（无 tempfile 依赖）；③ 空结果分支同样打头行 + 写缓存；④ help 分组先重排 Commands variant 声明顺序；⑤ wizard 测试先建目录再切 cwd、VANE_LANG=en 固定语言；⑥ interval 校验下沉 `valid_interval` 纯函数。

## Global Constraints

- 不新增任何 Cargo 依赖（spec §1.3 / §7.2）。
- 非 TTY 的 JSON 输出字段、取值、顺序一律不变；只允许新增可选字段（spec §7.2）。
- 进入 JSON / 持久化文件 / 非 TTY stderr 的文案一律英文；zh 只出现在 TTY 渲染（spec §5.1 硬规则）。
- `vane-core` 零改动；`crates/vane` 不引入 tokio/regex 等黑名单依赖（`deny.toml` 守护）。
- 提交信息用 conventional commits（`feat(vane): …` / `refactor(vane): …`），**不加任何 Co-Authored-By trailer**（仓库历史已清除该署名的先例）。
- 每个 Task 完成后门禁：`cargo fmt --all -- --check && cargo clippy -p vane --all-targets -- -D warnings && cargo test -p vane`。
- 新测试集中放 `crates/vane/tests/cli_ux.rs`（一个文件内分 mod 组织）。
- spec §8 测试编号在 Task 里以 `(spec 测试 N)` 标注。

## File Structure

| 文件 | 责任 | 新建/修改 |
|---|---|---|
| `crates/vane/src/fsutil.rs` | 共享 `atomic_write` + `expand_tilde`（收敛 atomic_write 3 份、expand_tilde 3 份） | 新建（Task 1） |
| `crates/vane/src/i18n.rs` | `Lang`、`detect`、`tr`、`pick`、EN/ZH 静态表 | 新建（Task 2，后续 Task 增 key） |
| `crates/vane/src/humanize.rs` | `rel_time`、`abs_date` | 新建（Task 2） |
| `crates/vane/src/dispatch.rs` | 裸 vane 三分支、query 缺参分支的纯决策函数 | 新建（Task 3/7） |
| `crates/vane/src/last_query.rs` | `last_query.json` 缓存结构与读写 | 新建（Task 4） |
| `crates/vane/src/watch_diff.rs` | LiveSet / dirty 队列快照 diff 纯函数 | 新建（Task 10） |
| `crates/vane/src/ui.rs` | 头行/hits/status/add/mcp 渲染改造；`collapse_home` | 修改（Task 3/5/6/8） |
| `crates/vane/src/main.rs` | clap 定义（Option<Commands>、Read/Watch、--verbose、help 分组）+ 薄 dispatch | 修改（Task 3/4/7/10） |
| `crates/vane/src/doctor.rs` | `DoctorCheck` 增加 serde-skip zh 字段，全部 check 站点双语 | 修改（Task 9） |
| `crates/vane/src/wizard.rs` | 提示语 tr 化 + first_root 即时校验重问 | 修改（Task 9） |
| `crates/vane/src/dirty.rs` | 新增 pub 只读列举 `paths_for` | 修改（Task 10） |
| `crates/vane/src/mcp.rs` | install 报告加 `kind`、skill 安装 job | 修改（Task 8） |
| `crates/vane/src/lib.rs` | 注册新模块 | 修改（各 Task） |
| `crates/vane/tests/cli_ux.rs` | 全部新测试 | 新建（Task 1 起累积） |

---

### Task 1: 共享工具层 `fsutil`（收敛 atomic_write / expand_tilde 私有拷贝）

对应 spec §2.4（原子写复用）、§5.2（expand_tilde 收敛）。

**Files:**
- Create: `crates/vane/src/fsutil.rs`
- Modify: `crates/vane/src/lib.rs`（加 `pub mod fsutil;`）、`crates/vane/src/progress.rs`、`crates/vane/src/live.rs`、`crates/vane/src/mcp.rs`、`crates/vane/src/main.rs`、`crates/vane/src/wizard.rs`、`crates/vane/src/doctor.rs`
- Test: `crates/vane/tests/cli_ux.rs`

**Interfaces:**
- Produces（后续所有 Task 可用）:
  - `vane::fsutil::atomic_write(path: &Path, bytes: &[u8], label: &str) -> Result<(), VaneCliError>`
  - `vane::fsutil::expand_tilde(path: &Path) -> PathBuf`

- [ ] **Step 1: 写失败测试**

新建 `crates/vane/tests/cli_ux.rs`：

```rust
mod fsutil_tests {
    use std::path::{Path, PathBuf};

    #[test]
    fn atomic_write_creates_parent_and_renames() {
        let dir = std::env::temp_dir().join(format!("vane-fsutil-{}", std::process::id()));
        let path = dir.join("a").join("b.json");
        vane::fsutil::atomic_write(&path, b"{}", "b.json").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"{}");
        assert!(!dir.join("a").join("b.json.tmp").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn expand_tilde_handles_all_forms() {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap();
        assert_eq!(vane::fsutil::expand_tilde(Path::new("~")), home);
        assert_eq!(
            vane::fsutil::expand_tilde(Path::new("~/notes")),
            home.join("notes")
        );
        assert_eq!(
            vane::fsutil::expand_tilde(Path::new("/abs/x")),
            PathBuf::from("/abs/x")
        );
        assert_eq!(
            vane::fsutil::expand_tilde(Path::new("rel/x")),
            PathBuf::from("rel/x")
        );
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p vane --test cli_ux`
Expected: 编译失败 `unresolved import vane::fsutil`

- [ ] **Step 3: 实现 fsutil 并替换调用方**

`crates/vane/src/fsutil.rs`（函数体逐字搬自现有实现）：

```rust
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::VaneCliError;

/// Atomic write: create parents, write `<name>.tmp` beside `path`, fsync, rename.
/// `label` only appears in error messages.
pub fn atomic_write(path: &Path, bytes: &[u8], label: &str) -> Result<(), VaneCliError> {
    let dir = path.parent().ok_or_else(|| {
        VaneCliError::new(format!("{label} path has no parent: {}", path.display()))
    })?;
    fs::create_dir_all(dir)
        .map_err(|e| VaneCliError::new(format!("create {} parent {}: {e}", label, dir.display())))?;
    let tmp = dir.join(format!(
        "{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or(label)
    ));
    {
        let mut f = File::create(&tmp)
            .map_err(|e| VaneCliError::new(format!("create {} temp {}: {e}", label, tmp.display())))?;
        f.write_all(bytes)
            .map_err(|e| VaneCliError::new(format!("write {} temp {}: {e}", label, tmp.display())))?;
        f.sync_all()
            .map_err(|e| VaneCliError::new(format!("sync {} temp {}: {e}", label, tmp.display())))?;
    }
    fs::rename(&tmp, path).map_err(|e| {
        VaneCliError::new(format!("rename {} -> {}: {e}", tmp.display(), path.display()))
    })
}

/// `~` / `~/…` expand to `$HOME`; everything else passes through unchanged.
pub fn expand_tilde(path: &Path) -> PathBuf {
    let Some(s) = path.to_str() else {
        return path.to_path_buf();
    };
    if s == "~" {
        return std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| path.to_path_buf());
    }
    if let Some(rest) = s.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    path.to_path_buf()
}
```

`lib.rs` 加一行（按字母序插在 `extract` 后）：`pub mod fsutil;`

替换调用方（全部删私有拷贝、改调共享版）：
- `progress.rs`：删 `fn atomic_write`（288 行附近）；`save_progress` 与 `persist_skips` 里的 `atomic_write(&path, &payload, "…")` 改 `crate::fsutil::atomic_write(...)`（签名一致，仅路径不同）。
- `live.rs`：删 `fn atomic_write`；`LiveSet::save_atomic` 里改 `crate::fsutil::atomic_write(path, &payload, "live.json")`。
- `mcp.rs`：删 `fn atomic_write`（681 行附近）；调用点 `atomic_write(&job.path, &bytes, job.client)` 改 `crate::fsutil::atomic_write(&job.path, &bytes, job.client)`。
- `main.rs`：删 `fn expand_tilde`（1394 行附近），调用点改 `vane::fsutil::expand_tilde(...)`。
- `wizard.rs`：删 `fn expand_tilde`（514 行附近），调用点改 `crate::fsutil::expand_tilde(...)`。
- `doctor.rs`：删 `fn expand_tilde`（670 行附近），调用点改 `crate::fsutil::expand_tilde(...)`。
- `cas.rs` 的 2 参 `atomic_write` **不动**（签名不同，留待以后）；`index.rs:482` 另有一份 2 参拷贝，同样不动。
- `expand_tilde` 在仓库共 6 份：本 Task 收敛 `main.rs` / `wizard.rs` / `doctor.rs` 三份；`config.rs:620` / `daemon.rs:1301` / `gc.rs:357` 三份本里程碑不动（避免无收益的大 diff，记录在案）。

- [ ] **Step 4: 跑测试确认通过 + 回归**

Run: `cargo test -p vane`
Expected: cli_ux 2 个新测试 PASS；既有 18 个测试文件全绿。

- [ ] **Step 5: 门禁 + Commit**

Run: `cargo fmt --all -- --check && cargo clippy -p vane --all-targets -- -D warnings`

```bash
git add crates/vane/src/fsutil.rs crates/vane/src/lib.rs crates/vane/src/{progress,live,mcp,wizard,doctor}.rs crates/vane/src/main.rs crates/vane/tests/cli_ux.rs
git commit -m "refactor(vane): converge atomic_write and expand_tilde into fsutil"
```

---

### Task 2: i18n 机制 + humanize 相对时间

对应 spec §3.1、§5.1；覆盖 spec 测试 1、2。

**Files:**
- Create: `crates/vane/src/i18n.rs`、`crates/vane/src/humanize.rs`
- Modify: `crates/vane/src/lib.rs`
- Test: `crates/vane/tests/cli_ux.rs`

**Interfaces:**
- Produces:
  - `vane::i18n::Lang::{En, Zh}`；`Lang::detect() -> Lang`；`Lang::detect_with(get: &dyn Fn(&str) -> Option<String>) -> Lang`
  - `vane::i18n::tr(lang: Lang, key: &str) -> &'static str`（zh miss → en；双 miss → 静态 `"missing-i18n-key"`，不 panic）
  - `vane::i18n::pick(lang: Lang, tty: bool, key: &str) -> &'static str`（tty 用 lang，非 tty 恒 En——硬规则的唯一闸门）
  - `vane::i18n::EN_TABLE` / `ZH_TABLE`（`pub const &[(&str, &str)]`，pub 的唯一理由是让 parity 测试直接比两张表，**禁止**用别名/互相引用——否则漏 key 时测试恒真失效）
  - `vane::humanize::rel_time(ts: u64, now: u64, lang: Lang) -> String`
  - `vane::humanize::abs_date(ts: u64) -> String`（`YYYY-MM-DD`，无 chrono）

- [ ] **Step 1: 写失败测试**

`cli_ux.rs` 追加：

```rust
mod i18n_tests {
    use vane::i18n::{pick, tr, Lang};

    #[test]
    fn tables_have_identical_keys() {
        let mut en: Vec<&str> = vane::i18n::EN_TABLE.iter().map(|(k, _)| *k).collect();
        let mut zh: Vec<&str> = vane::i18n::ZH_TABLE.iter().map(|(k, _)| *k).collect();
        en.sort_unstable();
        zh.sort_unstable();
        assert_eq!(en, zh, "EN/ZH key sets diverged");
    }

    #[test]
    fn detect_priority_and_zh_prefix() {
        let env = |pairs: &[(&str, &str)]| {
            move |k: &str| pairs.iter().find(|(key, _)| *key == k).map(|(_, v)| v.to_string())
        };
        assert_eq!(Lang::detect_with(&env(&[])), Lang::En);
        assert_eq!(Lang::detect_with(&env(&[("LANG", "zh_CN.UTF-8")])), Lang::Zh);
        assert_eq!(
            Lang::detect_with(&env(&[("LANG", "zh_CN.UTF-8"), ("LC_ALL", "en_US.UTF-8")])),
            Lang::En
        );
        assert_eq!(
            Lang::detect_with(&env(&[("LC_ALL", "en_US"), ("VANE_LANG", "zh")])),
            Lang::Zh
        );
        assert_eq!(Lang::detect_with(&env(&[("LANG", "ZH_TW")])), Lang::Zh);
    }

    #[test]
    fn pick_forces_english_off_tty() {
        assert_eq!(pick(Lang::Zh, false, "time.just_now"), "just now");
        assert_eq!(pick(Lang::Zh, true, "time.just_now"), "刚刚");
        assert_eq!(tr(Lang::En, "nonexistent.key"), "missing-i18n-key");
    }
}

mod humanize_tests {
    use vane::humanize::{abs_date, rel_time};
    use vane::i18n::Lang;

    const NOW: u64 = 1_755_700_000; // fixed reference

    #[test]
    fn all_buckets_en_and_zh() {
        assert_eq!(rel_time(0, NOW, Lang::En), "never");
        assert_eq!(rel_time(NOW + 100, NOW, Lang::En), "just now"); // clock skew
        assert_eq!(rel_time(NOW - 5, NOW, Lang::En), "just now");
        assert_eq!(rel_time(NOW - 30, NOW, Lang::En), "30s ago");
        assert_eq!(rel_time(NOW - 180, NOW, Lang::En), "3 min ago");
        assert_eq!(rel_time(NOW - 5 * 3600, NOW, Lang::En), "5 hours ago");
        assert_eq!(rel_time(NOW - 10 * 86_400, NOW, Lang::En), "10 days ago");
        assert_eq!(rel_time(NOW - 40 * 86_400, NOW, Lang::En), abs_date(NOW - 40 * 86_400));
        assert_eq!(rel_time(NOW - 180, NOW, Lang::Zh), "3 分钟前");
        assert_eq!(rel_time(0, NOW, Lang::Zh), "从未");
    }

    #[test]
    fn abs_date_known_epoch() {
        assert_eq!(abs_date(0), "1970-01-01");
        assert_eq!(abs_date(1_755_700_000), "2025-08-20");
    }
}
```

注：`abs_date(1_755_700_000)` 的期望值先用算法跑一遍再填死（若与算法输出不一致，以算法对已知日期的输出为准并核对日历）。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p vane --test cli_ux`
Expected: 编译失败 `unresolved import vane::i18n`

- [ ] **Step 3: 实现**

`crates/vane/src/i18n.rs`：

```rust
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
```

`crates/vane/src/humanize.rs`：

```rust
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
```

`lib.rs` 注册：`pub mod humanize;`、`pub mod i18n;`（字母序）。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p vane --test cli_ux`
Expected: 全部 PASS

- [ ] **Step 5: 门禁 + Commit**

```bash
cargo fmt --all -- --check && cargo clippy -p vane --all-targets -- -D warnings
git add crates/vane/src/{i18n,humanize}.rs crates/vane/src/lib.rs crates/vane/tests/cli_ux.rs
git commit -m "feat(vane): add i18n layer and human relative time"
```

---

### Task 3: query 范围头行 + 命中精简 + --verbose + 无参数分支

对应 spec §2.1–§2.3；覆盖 spec 测试 3、4、11。

**Files:**
- Create: `crates/vane/src/dispatch.rs`
- Modify: `crates/vane/src/ui.rs`（`collapse_home`、`format_scope_header`、`print_hits` 重写）、`crates/vane/src/main.rs`（`Query` 改 `q: Option<String>` + `--verbose`，`run_query`/`print_search_result` 改造）、`crates/vane/src/i18n.rs`（加 key）、`crates/vane/src/lib.rs`
- Test: `crates/vane/tests/cli_ux.rs`

**Interfaces:**
- Consumes: Task 2 的 `tr`/`pick`/`Lang`。
- Produces:
  - `vane::ui::collapse_home(path: &str) -> String`
  - `vane::ui::format_scope_header(root: Option<&str>, roots: usize, live: u64, degraded: bool, lang: Lang, colors: bool) -> String`（`root=None` 即 all）
  - `vane::ui::print_hits(hits: &[serde_json::Value], all: bool, verbose: bool, header_degraded: bool)`（签名变更，唯一调用方 main.rs）
  - `vane::dispatch::QueryArg::{Run(String), Prompt, MissingError}`；`vane::dispatch::decide_query_arg(q: Option<String>, tty: bool) -> QueryArg`
- i18n 新 key（**两张表同步各加一条**，parity 测试自动守护）：`query.missing_query`（`missing query — usage: vane query <text>` / `缺少查询词 — 用法：vane query <文本>`）、`query.prompt`（`Search query` / `搜索关键词`）、`header.searching_one`（`searching {root} · {n} live files · {mode}` / `搜索 {root} · {n} 个文件 · {mode}`）、`header.searching_all`（`searching {k} roots · {n} live files · {mode}` / `搜索 {k} 个目录 · {n} 个文件 · {mode}`）、`header.hybrid`（`hybrid` / `混合检索`）、`header.degraded`（`BM25 (degraded: embedder unreachable)` / `BM25（降级：embedder 不可达）`）。

- [ ] **Step 1: 写失败测试**

```rust
mod query_display_tests {
    use vane::i18n::Lang;

    #[test]
    fn collapse_home_folds_home_prefix() {
        let home = std::env::var("HOME").unwrap();
        assert_eq!(vane::ui::collapse_home(&format!("{home}/notes")), "~/notes");
        assert_eq!(vane::ui::collapse_home("/var/tmp"), "/var/tmp");
    }

    #[test]
    fn scope_header_single_root_and_all() {
        let one = vane::ui::format_scope_header(Some("~/notes"), 1, 12, false, Lang::En, false);
        assert_eq!(one, "searching ~/notes · 12 live files · hybrid");
        let all = vane::ui::format_scope_header(None, 3, 42, false, Lang::En, false);
        assert_eq!(all, "searching 3 roots · 42 live files · hybrid");
        let deg = vane::ui::format_scope_header(Some("~/notes"), 1, 12, true, Lang::Zh, false);
        assert!(deg.contains("BM25（降级：embedder 不可达）"));
    }
}

mod dispatch_tests {
    use vane::dispatch::{decide_query_arg, QueryArg};

    #[test]
    fn query_arg_branches() {
        assert_eq!(
            decide_query_arg(Some("foo".into()), true),
            QueryArg::Run("foo".into())
        );
        assert_eq!(decide_query_arg(None, true), QueryArg::Prompt);
        assert_eq!(decide_query_arg(None, false), QueryArg::MissingError);
    }
}
```

`print_hits` 的行为测试（spec 测试 4）直接测打印不现实，改为测一个纯装配函数——实现时在 ui.rs 加：

```rust
pub struct HitLineOpts { pub all: bool, pub verbose: bool, pub header_degraded: bool }
pub fn hit_lines(hit: &serde_json::Value, index: usize, opts: &HitLineOpts, colors: bool) -> Vec<String>
```

测试：

```rust
    #[test]
    fn hit_lines_omit_root_single_scope_and_id_unless_verbose() {
        let hit = serde_json::json!({
            "id": "p1:notes/a.md#0", "path": "notes/a.md", "root": "/abs/notes",
            "snippet": "hello", "score": 0.42, "degraded": false
        });
        let single = vane::ui::hit_lines(&hit, 0, &vane::ui::HitLineOpts { all: false, verbose: false, header_degraded: true }, false);
        assert!(!single.iter().any(|l| l.contains("/abs/notes")), "single scope must not repeat root");
        assert!(!single.iter().any(|l| l.contains("p1:notes/a.md#0")), "id hidden by default");
        let all = vane::ui::hit_lines(&hit, 0, &vane::ui::HitLineOpts { all: true, verbose: true, header_degraded: true }, false);
        assert!(all.iter().any(|l| l.contains("/abs/notes")));
        assert!(all.iter().any(|l| l.contains("p1:notes/a.md#0")));
        let deg = serde_json::json!({"id":"p:x#0","path":"x","root":"/r","snippet":"","score":0.1,"degraded":true});
        let lines = vane::ui::hit_lines(&deg, 0, &vane::ui::HitLineOpts { all: false, verbose: false, header_degraded: true }, false);
        assert!(!lines.iter().any(|l| l.contains("degraded")), "header already aggregated it");
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p vane --test cli_ux`
Expected: 编译失败（`format_scope_header` / `decide_query_arg` / `hit_lines` 不存在）

- [ ] **Step 3: 实现**

`crates/vane/src/dispatch.rs`：

```rust
/// Pure dispatch decisions, unit-testable without a TTY or daemon (spec §7.1).

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryArg {
    Run(String),
    Prompt,
    MissingError,
}

pub fn decide_query_arg(q: Option<String>, tty: bool) -> QueryArg {
    match (q, tty) {
        (Some(q), _) => QueryArg::Run(q),
        (None, true) => QueryArg::Prompt,
        (None, false) => QueryArg::MissingError,
    }
}
```

`ui.rs` 新增/改造：

```rust
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
        // mode portion yellow: simplest faithful rendering is to color the mode
        // substring; keep it simple — whole line plain except degraded mode.
        text.replace(&mode, &console::style(&mode).yellow().to_string())
    } else {
        text
    }
}
```

`hit_lines`：把现 `print_hits` 的每条命中逻辑搬进来，按 opts 决定 root 行（`opts.all` 才打）、id 行（`opts.verbose` 才打）、degraded 行（`!opts.header_degraded` 才打）；`print_hits` 变成薄壳：`for (i, hit) in hits.iter().enumerate() { for line in hit_lines(hit, i, &opts, colors_enabled()) { println!("{line}") } }`（保留空 hits 的 `warn("no hits")` 分支）。

`main.rs` 改造：
1. `Query` variant：
```rust
    /// Search the current project (or --all / --root)
    Query {
        /// Query text (omit on a TTY to be prompted)
        q: Option<String>,
        #[arg(long)]
        all: bool,
        #[arg(long)]
        global: bool,
        #[arg(long)]
        root: Option<PathBuf>,
        #[arg(long = "type")]
        extractor: Option<String>,
        #[arg(long, default_value_t = 8)]
        top_k: u32,
        /// Also print internal hit ids
        #[arg(long)]
        verbose: bool,
    },
```
2. dispatch 处：
```rust
        Commands::Query { q, all, global, root, extractor, top_k, verbose } => {
            match vane::dispatch::decide_query_arg(q, vane::ui::interactive()) {
                vane::dispatch::QueryArg::Run(q) => run_query(&home, q, all || global, root, extractor, top_k, verbose),
                vane::dispatch::QueryArg::Prompt => match prompt_query_text() {
                    Some(q) => run_query(&home, q, all || global, root, extractor, top_k, verbose),
                    None => ExitCode::SUCCESS, // empty input cancels (spec §2.3)
                },
                vane::dispatch::QueryArg::MissingError => {
                    vane::ui::error(vane::i18n::pick(vane::i18n::Lang::detect(), false, "query.missing_query"));
                    ExitCode::from(2)
                }
            }
        }
```
`prompt_query_text() -> Option<String>`（main.rs 私有）：cliclack input 用 `tr(lang, "query.prompt")`，trim 后空 → None。此分支人工验收（spec 豁免）。
3. `run_query` 加 `verbose: bool` 形参；把 cwd 推断出的 resolved scope（`QueryScope::Root(r)` 的 `r` / `QueryScope::All`）随响应一起传给 `print_search_result`——**签名改为接收已解析的 scope，print 层不重新解析**（`run_query` 在 main.rs:948-955 已算出）。`print_search_result` 的 TTY 渲染顺序（spec §2.1「空结果时头行照打」）：
   - 先算 header 数据：单 root → root 字符串 + `LiveSet::load_for_project` 的 live 数；all → roots 数 + 各 root live 求和（遍历 cfg.projects，复用 `count_live_files` 的模式）；`degraded = hits.iter().any(|h| h.degraded == true)`。
   - **空命中分支也要先打 `format_scope_header`，再打 why 行**（现 main.rs:977-987 的空结果分支在 print_hits 前 return，改造时必须把 header 提到分支之前）。
   - 有命中：`format_scope_header(...)` 后 `print_hits(&hits, all, verbose, degraded)`。
   - 颜色从简（评审认可并记录）：首版只对 degraded mode 上色，`searching`/`·`/root 的 dim/accent 不做——与 spec §2.1 的颜色描述存在意的简化偏差，后续可补。

- [ ] **Step 4: 跑测试确认通过 + 子进程验证（spec 测试 11）**

`cli_ux.rs` 追加子进程测试（复用 lib 直接建 config，不起 daemon）：

```rust
mod subprocess_tests {
    use std::io::Write;
    use std::process::{Command, Stdio};

    #[test]
    fn query_without_arg_non_tty_is_single_line_exit_2() {
        let _g = ENV_LOCK.lock().unwrap();
        let home = init_home(); // helper 定义见下方，紧跟本测试块
        let out = Command::new(env!("CARGO_BIN_EXE_vane"))
            .args(["--home", &home.path.display().to_string(), "query"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2));
        let stderr = String::from_utf8(out.stderr).unwrap();
        assert!(stderr.contains("missing query"), "got: {stderr}");
        assert!(!stderr.contains("Usage:"), "must not dump clap help: {stderr}");
        assert_eq!(stderr.lines().count(), 1);
    }
}
```

`ENV_LOCK` 为文件级 `static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());`，所有碰环境变量的测试（含本测试与 T9 的 wizard 测试）在函数体首行持锁。

`init_home` helper（放测试文件顶部）。**注意：`crates/vane/Cargo.toml` 没有 `[dev-dependencies]`，没有 tempfile**——仓库惯例是手写 TempHome（参照 `tests/config_merge.rs:14-42`），照抄该惯例：

```rust
struct TempHome {
    path: std::path::PathBuf,
}

fn temp_home(tag: &str) -> TempHome {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "vane-ux-{tag}-{}-{nanos}-{n}",
        std::process::id()
    ));
    std::fs::create_dir_all(&path).unwrap();
    TempHome { path }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let tmp = std::env::temp_dir();
        if self.path.starts_with(&tmp) && self.path != tmp {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

fn init_home() -> TempHome {
    let dir = temp_home("init");
    let answers = vane::wizard::InitAnswers {
        install_service: false,
        ..Default::default()
    };
    // VANE_ALLOW_EMBED_FAIL=1 lets the probe fail closed→continue without a live embedder.
    std::env::set_var("VANE_ALLOW_EMBED_FAIL", "1");
    vane::wizard::run_init(&dir.path, std::io::empty(), std::io::sink(), Some(answers)).unwrap();
    dir
}
```

`set_var` 的测试间竞态：cli_ux.rs 内所有碰环境变量的测试用文件级 `static ENV_LOCK: std::sync::Mutex<()>` 串行化（`init_home` 也持有该锁——把锁获取放进 init_home 并在返回前释放会有生命周期问题，改为测试函数内 `let _g = ENV_LOCK.lock().unwrap(); let home = init_home(); …`）。

Run: `cargo test -p vane`
Expected: 全绿。

- [ ] **Step 5: 门禁 + Commit**

```bash
cargo fmt --all -- --check && cargo clippy -p vane --all-targets -- -D warnings
git add crates/vane/src/{dispatch,ui,i18n,lib}.rs crates/vane/src/main.rs crates/vane/tests/cli_ux.rs
git commit -m "feat(vane): query scope header, lean hit lines, --verbose, friendly missing-arg"
```

---

### Task 4: `last_query` 缓存 + `vane read <n>`

对应 spec §2.4；覆盖 spec 测试 5、6。

**Files:**
- Create: `crates/vane/src/last_query.rs`
- Modify: `crates/vane/src/main.rs`（`Read` 子命令、TTY query 成功写缓存）、`crates/vane/src/lib.rs`、`crates/vane/src/i18n.rs`（加 key）
- Test: `crates/vane/tests/cli_ux.rs`

**Interfaces:**
- Consumes: `fsutil::atomic_write`（Task 1）、`search::{read_by_id, parse_doc_id}`（既有 pub）、`LiveSet::load_for_project`、`Cas::new`。
- Produces:
  - `vane::last_query::CachedHit { id: String, path: String, root: String, score: f64 }`（Serialize/Deserialize/PartialEq）
  - `vane::last_query::LastQuery { query: String, at: u64, scope_root: Option<String>, hits: Vec<CachedHit> }`（`scope_root=None` 即 all）
  - `vane::last_query::last_query_path(home: &Path) -> PathBuf`（`home/run/last_query.json`）
  - `vane::last_query::save_last_query(home: &Path, q: &LastQuery) -> Result<(), VaneCliError>`
  - `vane::last_query::load_last_query(home: &Path) -> Option<LastQuery>`（缺文件/损坏 → None）
  - `vane::last_query::read_outcome(home: &Path, q: &LastQuery, n: usize) -> Result<ReadOutcome, ReadError>`——**决策与定位全在这个 lib 函数**，main 只打印：
    ```rust
    pub struct ReadOutcome { pub hit: CachedHit, pub chunk_index: u32, pub text: String }
    pub enum ReadError { OutOfRange { n: usize, k: usize }, Empty, Stale { n: usize } }
    ```
- i18n 新 key：`read.no_cache`（`no recent query — run vane query first` / `还没有可查的结果 — 先运行 vane query`）、`read.empty`（`last query had no hits` / `上次查询没有命中`）、`read.out_of_range`（`no hit {n} — last query has {k} hits (1..={k})` / `没有第 {n} 条 — 上次查询共 {k} 条（1..={k}）`）、`read.stale`（`this hit is stale — the file changed since the last query; re-run vane query or use vane read {n} --file` / `该结果已过期 — 文件在上次查询后发生变化；请重新 vane query 或用 vane read {n} --file`）、`read.file_missing`（`source file no longer exists: {path}` / `源文件已不存在：{path}`）、`read.binary`（`{path} is not a text file (extractor {extractor}) — open it directly` / `{path} 不是文本文件（类型 {extractor}）— 请直接打开`）。

- [ ] **Step 1: 写失败测试**

```rust
mod last_query_tests {
    use vane::last_query::*;

    fn sample() -> LastQuery {
        LastQuery {
            query: "foo".into(),
            at: 1_755_700_000,
            scope_root: Some("/abs/notes".into()),
            hits: vec![CachedHit {
                id: "p1:notes/a.md#0".into(),
                path: "notes/a.md".into(),
                root: "/abs/notes".into(),
                score: 0.42,
            }],
        }
    }

    #[test]
    fn roundtrip_and_corrupt_cache() {
        let dir = std::env::temp_dir().join(format!("vane-lq-{}", std::process::id()));
        save_last_query(&dir, &sample()).unwrap();
        assert_eq!(load_last_query(&dir).unwrap(), sample());
        std::fs::write(last_query_path(&dir), b"not json").unwrap();
        assert!(load_last_query(&dir).is_none(), "corrupt cache must be None");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_outcome_errors() {
        let dir = std::env::temp_dir().join(format!("vane-lq2-{}", std::process::id()));
        let q = sample();
        // no live set / CAS → stale (chunk not found)
        assert!(matches!(read_outcome(&dir, &q, 1), Err(ReadError::Stale { n: 1 })));
        assert!(matches!(
            read_outcome(&dir, &q, 5),
            Err(ReadError::OutOfRange { n: 5, k: 1 })
        ));
        let empty = LastQuery { hits: vec![], ..sample() };
        assert!(matches!(read_outcome(&dir, &empty, 1), Err(ReadError::Empty)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_outcome_reads_chunk_from_cas() {
        // 搭最小 live set + CAS：一个文件一个 chunk。
        let dir = std::env::temp_dir().join(format!("vane-lq3-{}", std::process::id()));
        let home = dir.as_path();
        let pid = "p1";
        // live.json
        let mut live = vane::live::LiveSet::default();
        live.files.insert(
            "notes/a.md".into(),
            vane::live::LiveFile {
                content_sha256: "x".into(),
                extract_key: "k1".into(),
                chunk_count: 1,
            },
        );
        live.save_for_project(home, pid).unwrap();
        // CAS extract
        let cas = vane::cas::Cas::new(home.join("rag").join("cas"));
        cas.put_extract(
            "k1",
            &[vane::extract::CanonicalDoc {
                path: "notes/a.md".into(),
                headings: vec![],
                text: "hello world chunk".into(),
                chunk_index: 0,
                start_byte: 0,
                end_byte: 17,
                modality: "text".into(),
                extractor: "text".into(),
            }],
        )
        .unwrap();
        let out = read_outcome(home, &sample(), 1).unwrap();
        assert_eq!(out.text, "hello world chunk");
        assert_eq!(out.chunk_index, 0);
        std::fs::remove_dir_all(&dir).ok();
    }
}
```

**注意**：`CanonicalDoc` 的真实字段以 `crates/vane/src/extract.rs` 为准——写测试前先读该 struct 定义并逐字段对齐（上面是按语义的示意，字段名/类型必须与 extract.rs 完全一致，否则编译失败）。`save_for_project` 需要 `home/rag/projects/p1/` 布局，`Cas` 需要 `home/rag/cas`——均与生产路径一致，无需 mock。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p vane --test cli_ux last_query`
Expected: 编译失败（模块不存在）

- [ ] **Step 3: 实现**

`crates/vane/src/last_query.rs`：

```rust
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::cas::Cas;
use crate::error::VaneCliError;
use crate::fsutil::atomic_write;
use crate::live::LiveSet;
use crate::search::{parse_doc_id, read_by_id};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CachedHit {
    pub id: String,
    pub path: String,
    pub root: String,
    pub score: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LastQuery {
    pub query: String,
    pub at: u64,
    /// None = fused --all scope.
    pub scope_root: Option<String>,
    #[serde(default)]
    pub hits: Vec<CachedHit>,
}

pub struct ReadOutcome {
    pub hit: CachedHit,
    pub chunk_index: u32,
    pub text: String,
}

#[derive(Debug, PartialEq)]
pub enum ReadError {
    OutOfRange { n: usize, k: usize },
    Empty,
    Stale { n: usize },
}

pub fn last_query_path(home: &Path) -> PathBuf {
    home.join("run").join("last_query.json")
}

pub fn save_last_query(home: &Path, q: &LastQuery) -> Result<(), VaneCliError> {
    let payload = serde_json::to_vec_pretty(q)
        .map_err(|e| VaneCliError::new(format!("serialize last_query.json: {e}")))?;
    atomic_write(&last_query_path(home), &payload, "last_query.json")
}

/// Missing or corrupt cache is "no cache", never an error (spec §7.2).
pub fn load_last_query(home: &Path) -> Option<LastQuery> {
    let bytes = std::fs::read(last_query_path(home)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Resolve hit `n` (1-based) of `q` to its chunk text via LiveSet + CAS.
/// No daemon involved (spec §2.4).
pub fn read_outcome(home: &Path, q: &LastQuery, n: usize) -> Result<ReadOutcome, ReadError> {
    if q.hits.is_empty() {
        return Err(ReadError::Empty);
    }
    if n == 0 || n > q.hits.len() {
        return Err(ReadError::OutOfRange { n, k: q.hits.len() });
    }
    let hit = q.hits[n - 1].clone();
    let (pid, _, chunk_index) =
        parse_doc_id(&hit.id).map_err(|_| ReadError::Stale { n })?;
    let live = LiveSet::load_for_project(home, &pid).map_err(|_| ReadError::Stale { n })?;
    let cas = Cas::new(home.join("rag").join("cas"));
    let chunk = read_by_id(&cas, &live, "", Path::new(&hit.root), &hit.id)
        .map_err(|_| ReadError::Stale { n })?;
    Ok(ReadOutcome { hit, chunk_index, text: chunk.text })
}
```

`main.rs`：
1. `Commands` 加（放在 `Query` 之后）：
```rust
    /// Read the n-th hit of the last TTY query (chunk text; --file for the source file)
    Read {
        n: usize,
        /// Print the whole source file from disk instead of the chunk
        #[arg(long)]
        file: bool,
    },
```
dispatch：`Commands::Read { n, file } => run_read(&home, n, file),`
2. `run_read`（薄壳）：
```rust
fn run_read(home: &Path, n: usize, file: bool) -> ExitCode {
    let lang = vane::i18n::Lang::detect();
    let tty = vane::ui::stdout_tty();
    let Some(q) = vane::last_query::load_last_query(home) else {
        vane::ui::error(vane::i18n::pick(lang, tty, "read.no_cache"));
        return ExitCode::from(1);
    };
    if file {
        return run_read_file(&q, n, lang, tty);
    }
    match vane::last_query::read_outcome(home, &q, n) {
        Ok(out) => {
            if tty {
                let meta = match &q.scope_root {
                    Some(_) => format!("{} · score {:.3} · chunk {}", out.hit.path, out.hit.score, out.chunk_index),
                    None => format!("{} :: {} · score {:.3} · chunk {}", out.hit.root, out.hit.path, out.hit.score, out.chunk_index),
                };
                println!("{}\n", vane::ui::dim(&meta));
            }
            println!("{}", out.text);
            ExitCode::SUCCESS
        }
        Err(e) => {
            let key = match e {
                vane::last_query::ReadError::Empty => "read.empty",
                vane::last_query::ReadError::OutOfRange { .. } => "read.out_of_range",
                vane::last_query::ReadError::Stale { .. } => "read.stale",
            };
            let msg = pick(lang, tty, key); // .replace("{n}"/"{k}") 按变体填充
            vane::ui::error(&msg);
            ExitCode::from(1)
        }
    }
}
```
`run_read_file`：取 hit（复用 Empty/OutOfRange 检查——把这段抽成 `last_query::hit_at(q, n) -> Result<&CachedHit, ReadError>` 供两处用），`Path::new(&hit.root).join(&hit.path)`；不存在 → `read.file_missing`；读前 8192 字节含 `\0` → `read.binary`（extractor 从 hit 无字段，填 `"binary"`）；否则 `print!` 全文（非 TTY 也无元信息，管道友好）。
3. TTY query 写缓存（spec §2.4 仲裁：**仅 TTY**、**含空结果**——空结果的 why 分支也要写，否则 `read.empty` 路径在真实流程不可达）：在 `run_query` 拿到 `Ok(v)` 且 `stdout_tty()` 时（覆盖有命中与空结果两个分支，最简位置是 rpc 返回后、渲染前）构造 `LastQuery`（hits 从响应 JSON 取 id/path/root/score，`at` 用 `vane::progress::unix_now()`）调 `save_last_query`。
4. **与冻结 spec 的已批准偏差（记录在案）**：spec §2.4 缓存 JSON 写的是 `"scope": {"kind":"root","root":…}` 标签枚举，本计划用 `scope_root: Option<String>`（`None` = all）。语义等价、`last_query.json` 是新文件无任何既有消费方，评审认可。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p vane`
Expected: 全绿（含既有 search/mcp 测试）

- [ ] **Step 5: 门禁 + Commit**

```bash
cargo fmt --all -- --check && cargo clippy -p vane --all-targets -- -D warnings
git add crates/vane/src/{last_query,lib,i18n}.rs crates/vane/src/main.rs crates/vane/tests/cli_ux.rs
git commit -m "feat(vane): vane read <n> over the last TTY query cache"
```

---

### Task 5: status 看板人话

对应 spec §3.1–§3.2、§6.1；覆盖 spec 测试 8。

**Files:**
- Modify: `crates/vane/src/ui.rs`（`status_view` + `format_status_lines` + `print_status_dashboard` 重写）、`crates/vane/src/i18n.rs`（加 key）、`crates/vane/src/main.rs`（`run_status` 传 progress/now）
- Test: `crates/vane/tests/cli_ux.rs`

**Interfaces:**
- Consumes: `humanize::rel_time`、`i18n::tr`、`progress::{load_progress, Progress, ProgressPhase}`。
- Produces:
  ```rust
  pub struct RootStatusView {
      pub path: String, pub live: u64, pub last_reconcile: Option<u64>,
      pub model: String, pub dim: Option<u64>, pub dirty: u64,
      pub skips: u64, pub last_error: Option<String>,
  }
  pub struct StatusView {
      pub home: String, pub running: bool,
      pub indexing: Option<(u64, u64)>, // (scanned, total) when progress phase != Idle
      pub dirty_total: u64, pub disk_home: u64, pub disk_cas: u64,
      pub last_error: Option<String>, pub roots: Vec<RootStatusView>,
  }
  pub fn status_view(v: &serde_json::Value, indexing: Option<(u64, u64)>) -> StatusView
  pub fn format_status_lines(view: &StatusView, lang: Lang, now: u64) -> Vec<String> // 无颜色纯文本
  pub fn print_status_dashboard(v: &serde_json::Value) // 薄壳：load_progress + detect + 打印
  ```
- i18n 新 key：`status.watching`（`watching` / `正在监听`）、`status.indexing`（`indexing {scanned}/{total}` / `索引中 {scanned}/{total}`）、`status.indexed_ago`（`indexed {ago}` / `{ago}完成索引`）、`status.never_indexed`（`never indexed` / `从未索引`）、`status.skipped_hint`（`{n} skipped — run vane issues` / `{n} 个文件被跳过 — 运行 vane issues 查看`）、`status.pending_changes`（`{n} pending changes` / `{n} 个待处理变更`）。

- [ ] **Step 1: 写失败测试**

```rust
mod status_tests {
    use vane::i18n::Lang;

    fn sample_status() -> serde_json::Value {
        serde_json::json!({
            "home": "/h/.vane",
            "running": true,
            "dirty_queue_size": 0,
            "disk": { "home_bytes": 2048, "cas_bytes": 1024 },
            "roots": [{
                "path": "/abs/notes", "live_files": 12,
                "last_reconcile": 1_755_699_700u64, // 300s before NOW
                "model": "nomic-embed-text", "dim": 768,
                "dirty_queue_size": 0, "skip_count": 12
            }]
        })
    }

    const NOW: u64 = 1_755_700_000;

    #[test]
    fn watching_and_humanized_root_lines() {
        let view = vane::ui::status_view(&sample_status(), None);
        let lines = vane::ui::format_status_lines(&view, Lang::En, NOW);
        let joined = lines.join("\n");
        assert!(joined.contains("watching"), "{joined}");
        assert!(joined.contains("indexed 5 min ago"), "{joined}");
        assert!(joined.contains("12 skipped — run vane issues"), "{joined}");
        assert!(!joined.contains("1755699700"), "no raw unix seconds: {joined}");
        assert!(!joined.contains("pending changes"), "dirty=0 must not print");
    }

    #[test]
    fn indexing_state_and_never_indexed() {
        let mut v = sample_status();
        v["roots"][0]["last_reconcile"] = serde_json::Value::Null;
        let view = vane::ui::status_view(&v, Some((34, 120)));
        let lines = vane::ui::format_status_lines(&view, Lang::Zh, NOW).join("\n");
        assert!(lines.contains("索引中 34/120"), "{lines}");
        assert!(lines.contains("从未索引"), "{lines}");
        assert!(lines.contains("12 个文件被跳过"), "{lines}");
    }

    #[test]
    fn daemon_stopped_keeps_existing_warning() {
        let mut v = sample_status();
        v["running"] = serde_json::json!(false);
        let view = vane::ui::status_view(&v, None);
        let lines = vane::ui::format_status_lines(&view, Lang::En, NOW).join("\n");
        assert!(lines.contains("daemon not running — vane start"), "{lines}");
        assert!(!lines.contains("watching"));
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p vane --test cli_ux status`
Expected: 编译失败（`status_view` 不存在）

- [ ] **Step 3: 实现**

`status_view`：从 JSON 逐字段提取（`as_u64().unwrap_or(0)` 等防御式读取，与现 `print_status_dashboard` 相同的取值路径），`indexing` 由调用方传入。
`format_status_lines` 逐行拼（顺序与现 dashboard 一致：home → daemon 行 → dirty → disk → last_error → roots）：
- daemon 行：`running && indexing.is_none()` → `tr("status.watching")`；`indexing = Some((s,t))` → `tr("status.indexing")` 替换；`!running` → 固定英文串 `daemon not running — vane start`（现状，两语言相同，不进表）。
- root 行：`indexed {rel_time(ts)}` / `never_indexed`；`dirty > 0` → `pending_changes`；`skips > 0` → `skipped_hint`。
`print_status_dashboard` 重写为：`let indexing = load_progress(home).filter(|p| p.phase != Idle).map(|p| (p.scanned, p.total_estimate));`——**问题**：现签名 `print_status_dashboard(v)` 没有 home。改签名为 `print_status_dashboard(home: &Path, v: &Value)`，main.rs 唯一调用点（`run_status`）同步改。颜色：行内路径/数字用既有 `accent`/`dim` 包一层（`format_status_lines` 出纯文本，print 时对已知前缀做最小着色；若着色侵入大，允许首版颜色从简——只给 root path 与数字上色，其余纯文本）。
`run_status`（main.rs）：`vane::ui::print_status_dashboard(home, &v)`。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p vane`
Expected: 全绿

- [ ] **Step 5: 门禁 + Commit**

```bash
cargo fmt --all -- --check && cargo clippy -p vane --all-targets -- -D warnings
git add crates/vane/src/{ui,i18n}.rs crates/vane/src/main.rs crates/vane/tests/cli_ux.rs
git commit -m "feat(vane): human-readable status dashboard (watching, relative time, skip hints)"
```

---

### Task 6: add 人话总结 + 进度条

对应 spec §3.3–§3.4；覆盖 spec 测试 7、13b。

**Files:**
- Modify: `crates/vane/src/main.rs`（`print_add_report`、`add_root_poll_progress`）、`crates/vane/src/progress.rs`（`choose_progress_style`）、`crates/vane/src/i18n.rs`（加 key）
- Test: `crates/vane/tests/cli_ux.rs`

**Interfaces:**
- Produces:
  - `vane::progress::ProgressStyle::{Spinner, Bar(u64)}`；`vane::progress::choose_progress_style(total_estimate: u64) -> ProgressStyle`
  - `vane::progress::clamp_pos(scanned: u64, total: u64) -> u64`（`scanned.min(total)`，spec 测试 13b 的封顶断言落点）
  - `vane::ui::format_add_summary(added: u64, unchanged: u64, skipped: u64, lang: Lang) -> String`（`added+unchanged` 为 indexed 数）
- i18n 新 key：`add.summary`（`indexed {n} files` / `已索引 {n} 个文件`）、`add.summary_skipped`（`indexed {n} files, {skipped} skipped — run vane issues` / `已索引 {n} 个文件，跳过 {skipped} 个 — 运行 vane issues 查看`）。

- [ ] **Step 1: 写失败测试**

```rust
mod add_summary_tests {
    use vane::i18n::Lang;

    #[test]
    fn summary_with_and_without_skips() {
        assert_eq!(
            vane::ui::format_add_summary(5, 75, 0, Lang::En),
            "indexed 80 files"
        );
        assert_eq!(
            vane::ui::format_add_summary(5, 75, 3, Lang::En),
            "indexed 80 files, 3 skipped — run vane issues"
        );
        assert_eq!(
            vane::ui::format_add_summary(5, 75, 3, Lang::Zh),
            "已索引 80 个文件，跳过 3 个 — 运行 vane issues 查看"
        );
    }

    #[test]
    fn progress_style_choice() {
        assert_eq!(
            vane::progress::choose_progress_style(0),
            vane::progress::ProgressStyle::Spinner
        );
        assert_eq!(
            vane::progress::choose_progress_style(120),
            vane::progress::ProgressStyle::Bar(120)
        );
        assert_eq!(vane::progress::clamp_pos(34, 120), 34);
        assert_eq!(vane::progress::clamp_pos(150, 120), 120); // spec 13b: pos 封顶
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p vane --test cli_ux add_summary`
Expected: 编译失败

- [ ] **Step 3: 实现**

`progress.rs` 加：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressStyle {
    Spinner,
    Bar(u64),
}

/// Bar once the daemon knows the total; spinner during the initial scan window.
pub fn choose_progress_style(total_estimate: u64) -> ProgressStyle {
    if total_estimate == 0 {
        ProgressStyle::Spinner
    } else {
        ProgressStyle::Bar(total_estimate)
    }
}
```

`ui.rs` 加 `format_add_summary`（按 key 替换 `{n}`/`{skipped}`）。

`main.rs`：
- `print_add_report` 的 TTY 分支在现有机器行后追加：`println!("{}", vane::ui::format_add_summary(added, unchanged, skipped, Lang::detect()))`（skipped>0 时该行同时承担提示职责）。
- `add_root_poll_progress` 重写：不再只持有一个 spinner——惰性创建：`let mut bar: Option<ProgressBar> = None;` 每轮 poll 到 progress 后 `match choose_progress_style(p.total_estimate) { Spinner => 维持现有 spinner.set_message, Bar(total) => { let pb = bar.get_or_insert_with(|| { spinner 停掉并 finish_and_clear，新建 ProgressBar::new(total)，模板 "{bar:30} {pos}/{len} {msg}" }); pb.set_length(total); pb.set_position(vane::progress::clamp_pos(p.scanned, total)); pb.set_message(format!("{} {}", p.phase.as_str(), collapse_home(&p.root))); } }`；结束后两者都 `finish_and_clear`。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p vane`
Expected: 全绿

- [ ] **Step 5: 门禁 + Commit**

```bash
cargo fmt --all -- --check && cargo clippy -p vane --all-targets -- -D warnings
git add crates/vane/src/{progress,ui,i18n}.rs crates/vane/src/main.rs crates/vane/tests/cli_ux.rs
git commit -m "feat(vane): human add summary and n/total progress bar"
```

---

### Task 7: 裸 `vane` 三分支 + help 分组

对应 spec §4.1–§4.2；覆盖 spec 测试 9、10。

**Files:**
- Modify: `crates/vane/src/dispatch.rs`（`decide_bare`）、`crates/vane/src/main.rs`（`Option<Commands>`、`next_help_heading`、裸 vane 处理）、`crates/vane/src/i18n.rs`（加 key）
- Test: `crates/vane/tests/cli_ux.rs`

**Interfaces:**
- Produces:
  - `vane::dispatch::BareAction::{InitHint, Doctor, Status}`；`vane::dispatch::decide_bare(initialized: bool, daemon_running: bool) -> BareAction`
- i18n 新 key：`bare.init_hint`（`vane is not initialized — run vane init to set up` / `vane 尚未初始化 — 运行 vane init 开始设置`）。

- [ ] **Step 1: 写失败测试**

```rust
mod bare_dispatch_tests {
    use vane::dispatch::{decide_bare, BareAction};

    #[test]
    fn three_branches() {
        assert_eq!(decide_bare(false, false), BareAction::InitHint);
        assert_eq!(decide_bare(false, true), BareAction::InitHint); // 未初始化优先
        assert_eq!(decide_bare(true, false), BareAction::Doctor);
        assert_eq!(decide_bare(true, true), BareAction::Status);
    }
}
```

help 分组子进程测试（spec 测试 10）：

```rust
    #[test]
    fn help_is_grouped() {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_vane"))
            .arg("--help")
            .output()
            .unwrap();
        let stdout = String::from_utf8(out.stdout).unwrap();
        for heading in ["Common:", "Manage:", "Ops:"] {
            assert!(stdout.contains(heading), "missing {heading} in:\n{stdout}");
        }
        let common_pos = stdout.find("Common:").unwrap();
        let query_pos = stdout.find("query").unwrap();
        assert!(query_pos > common_pos);
        assert!(query_pos < stdout.find("Manage:").unwrap());
    }
```

（标题是否带冒号取决于 clap 渲染，以实际输出为准调整断言。）

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p vane --test cli_ux`
Expected: 编译失败 / help 测试断言失败

- [ ] **Step 3: 实现**

`dispatch.rs` 加：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BareAction {
    InitHint,
    Doctor,
    Status,
}

/// Bare `vane` on a TTY (spec §4.1). Non-TTY never reaches here (main prints help).
pub fn decide_bare(initialized: bool, daemon_running: bool) -> BareAction {
    if !initialized {
        BareAction::InitHint
    } else if !daemon_running {
        BareAction::Doctor
    } else {
        BareAction::Status
    }
}
```

`main.rs`：
1. `struct Cli`：`command: Option<Commands>`。
2. help 分组（**必须先重排 variant 声明顺序**——clap 按声明顺序渲染，现有顺序 Init/Add/Rm/Include/Exclude/Status/Doctor/Issues/Logs/Inspect/Daemon/Start/Stop/Query/Mcp/Model/Service/Df/Gc 下直接加 heading 会把 Query 划进 Ops，自带测试必败）：
   - 先把 `enum Commands` 的 variant 重排为：**Common** = Init, Add, Query, Read, Status, Doctor；**Manage** = Rm, Include, Exclude, Issues, Logs, Inspect, Model, Mcp, Watch（Task 10 加入）；**Ops** = Daemon, Start, Stop, Service, Df, Gc。重排是纯移动，不改任何 variant 内容。
   - 再在每组首个 variant 上加 heading：`Init` → `#[command(next_help_heading = "Common")]`，`Rm` → `#[command(next_help_heading = "Manage")]`，`Daemon` → `#[command(next_help_heading = "Ops")]`。
3. match 前处理裸调用：

```rust
    let Some(command) = cli.command else {
        if !vane::ui::stdout_tty() {
            let mut cmd = Cli::command();
            let _ = cmd.print_help();
            println!();
            return ExitCode::from(2);
        }
        let initialized = home.join("config").join("config.toml").is_file();
        let running = vane::daemon::is_running(&home);
        return match vane::dispatch::decide_bare(initialized, running) {
            vane::dispatch::BareAction::InitHint => {
                println!("{}", vane::i18n::pick(vane::i18n::Lang::detect(), true, "bare.init_hint"));
                ExitCode::SUCCESS
            }
            vane::dispatch::BareAction::Doctor => run_doctor(&home),
            vane::dispatch::BareAction::Status => run_status(&home),
        };
    };
    match command { … }
```

`use clap::CommandFactory;` 别忘了加（`Cli::command()`）。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p vane`
Expected: 全绿

- [ ] **Step 5: 门禁 + Commit**

```bash
cargo fmt --all -- --check && cargo clippy -p vane --all-targets -- -D warnings
git add crates/vane/src/{dispatch,main,i18n}.rs crates/vane/tests/cli_ux.rs
git commit -m "feat(vane): bare vane shows status/doctor, grouped --help"
```

---

### Task 8: `vane mcp install` 收尾文案 + skill 安装

对应 spec §4.3–§4.4；覆盖 spec 测试 12。

**Files:**
- Modify: `crates/vane/src/mcp.rs`（`McpInstallTarget`/`McpInstallSkip` 加 `kind`、`ConfigFormat::Skill`、skill job）、`crates/vane/src/ui.rs`（`print_mcp_install` 收尾行）、`crates/vane/src/i18n.rs`（加 key）
- Test: `crates/vane/tests/cli_ux.rs`

**Interfaces:**
- Produces:
  - `McpInstallTarget { path, client, action, kind }`、`McpInstallSkip { path, client, reason, kind }`，`kind ∈ {"config","skill"}`（additive 字段，spec §4.4）
  - `vane::mcp::SKILL_MD: &str`（`include_str!("../../../skills/vane/SKILL.md")`）
- i18n 新 key：`mcp.done_new_session`（`done — start a new agent session (Claude / Cursor / Codex) for the vane tools to load` / `完成 — 新开一轮 Agent 会话（Claude / Cursor / Codex）后 vane 工具才会加载`）。

- [ ] **Step 1: 写失败测试**

```rust
mod mcp_skill_tests {
    use std::path::Path;

    fn fake_home() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("vane-mcp-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join(".claude")).unwrap();
        dir
    }

    #[test]
    fn skill_lifecycle_and_report_kind() {
        let home = fake_home();
        // dry-run: would_write 含 skill 行
        let report = vane::mcp::install_mcp(Path::new(&home), true, Some(vane::mcp::McpClient::Claude)).unwrap();
        assert!(report.would_write.iter().any(|t| t.kind == "skill" && t.path.ends_with("skills/vane/SKILL.md")));
        assert!(report.would_write.iter().all(|t| t.kind == "skill" || t.kind == "config"));
        assert!(!home.join(".claude/skills/vane/SKILL.md").exists(), "dry-run must not write");
        // 首装 wrote
        let report = vane::mcp::install_mcp(&home, false, Some(vane::mcp::McpClient::Claude)).unwrap();
        let skill = report.written.iter().find(|t| t.kind == "skill").unwrap();
        assert_eq!(skill.action, "wrote");
        assert_eq!(std::fs::read_to_string(home.join(".claude/skills/vane/SKILL.md")).unwrap(), vane::mcp::SKILL_MD);
        // 二装 up-to-date
        let report = vane::mcp::install_mcp(&home, false, Some(vane::mcp::McpClient::Claude)).unwrap();
        assert!(report.skipped.iter().any(|s| s.kind == "skill" && s.reason == "up-to-date"));
        // 内容被改 → updated
        std::fs::write(home.join(".claude/skills/vane/SKILL.md"), "old").unwrap();
        let report = vane::mcp::install_mcp(&home, false, Some(vane::mcp::McpClient::Claude)).unwrap();
        assert!(report.written.iter().any(|t| t.kind == "skill" && t.action == "updated"));
        std::fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn default_all_without_claude_dir_skips_skill() {
        let dir = std::env::temp_dir().join(format!("vane-mcp2-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let report = vane::mcp::install_mcp(&dir, false, None).unwrap();
        assert!(!report.written.iter().any(|t| t.kind == "skill"));
        assert!(!dir.join(".claude").exists(), "must not create ~/.claude for skill");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn closing_line_is_present() {
        // print_mcp_install 是 TTY 打印；改为测纯函数
        let line = vane::i18n::tr(vane::i18n::Lang::Zh, "mcp.done_new_session");
        assert!(line.contains("新开一轮 Agent 会话"), "{line}");
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p vane --test cli_ux mcp_skill`
Expected: 编译失败（`kind` / `SKILL_MD` 不存在）

- [ ] **Step 3: 实现**

`mcp.rs`：
1. 两个 report struct 加 `pub kind: String`；所有现有构造点填 `kind: "config".into()`。
2. `pub const SKILL_MD: &str = include_str!("../../../skills/vane/SKILL.md");`
3. `ConfigFormat` 加 `Skill` variant。`install_jobs` 在 claude 分支后加：

```rust
    let explicit_claude = client == Some(McpClient::Claude);
    if explicit_claude || (all && user_home.join(".claude").is_dir()) {
        jobs.push(InstallJob {
            client: "claude-skill",
            path: user_home.join(".claude").join("skills").join("vane").join("SKILL.md"),
            format: ConfigFormat::Skill,
            create: true,
        });
    }
```

4. `prepare_job` 加 Skill 分支（在现有 JSON/TOML 分支前判断）：

```rust
        if matches!(job.format, ConfigFormat::Skill) {
            let bytes = SKILL_MD.as_bytes();
            let exists = job.path.is_file();
            if exists && std::fs::read(&job.path).ok().as_deref() == Some(bytes) {
                return Ok(PreparedJob::Skip { reason: "up-to-date".into() });
            }
            return Ok(PreparedJob::Write {
                action: if exists { "updated" } else { "wrote" },
                bytes: bytes.to_vec(),
            });
        }
```

5. `install_mcp` 构造 `McpInstallTarget`/`McpInstallSkip` 处按 `job.format` 填 kind：`ConfigFormat::Skill => "skill"`，否则 `"config"`。
6. `ui::print_mcp_install`：行打印处给 skill 行与 config 行同样渲染（现有循环不用改，action 文案已是通用动词），函数末尾加：
```rust
    println!("{}", crate::i18n::tr(crate::i18n::Lang::detect(), "mcp.done_new_session"));
```
（dry-run 也打印，文案仍成立。）

- [ ] **Step 4: 跑测试确认通过 + 回归既有 mcp 测试**

Run: `cargo test -p vane`
Expected: 全绿（特别注意 `tests/mcp.rs` 既有 install 断言若因 `kind` 字段变化失败，更新断言而非改行为）

- [ ] **Step 5: 门禁 + Commit**

```bash
cargo fmt --all -- --check && cargo clippy -p vane --all-targets -- -D warnings
git add crates/vane/src/{mcp,ui,i18n}.rs crates/vane/tests/cli_ux.rs
git commit -m "feat(vane): mcp install also installs the agent skill, with next-session hint"
```

---

### Task 9: zh 文案落地（doctor / 向导 / why / probe）+ 向导路径即时校验

对应 spec §2.5、§5；覆盖 spec 测试 2（why 键）、14。

**Files:**
- Modify: `crates/vane/src/doctor.rs`（`DoctorCheck` 加 zh 字段、全部 check 站点双语）、`crates/vane/src/ui.rs`（`print_doctor`/`print_why` 按 lang 渲染）、`crates/vane/src/wizard.rs`（提示语 tr 化、first_root 校验重问）、`crates/vane/src/main.rs`（`require_init`、empty-query why 传 id）、`crates/vane/src/i18n.rs`（加 key）
- Test: `crates/vane/tests/cli_ux.rs`

**Interfaces:**
- Produces:
  - `DoctorCheck` 增加 `#[serde(skip)] pub message_zh: String` 与 `#[serde(skip)] pub fix_zh: String`；构造器 `DoctorCheck::bi(id: &str, level: CheckLevel, message_en: &str, message_zh: &str, fix_en: &str, fix_zh: &str) -> DoctorCheck`（JSON 输出零变化——serde skip）
  - `ui::print_why(id: &str, fallback_en: &str)`（签名变更：TTY 按 `why.{id}` 查表，查不到用 fallback_en）
  - wizard 全部提示语经 `tr(lang, "wizard.*")`；`run_init` / `run_init_tty` 开头 `let lang = Lang::detect();`
- i18n 新 key：`why.not_initialized` / `why.not_registered` / `why.still_indexing` / `why.embedder` / `why.excluded` / `why.wrong_root` / `why.empty_index` / `why.no_match`（en 文案逐字抄 `doctor.rs::explain_empty_query` 现状，zh 对应翻译）；`wizard.*` 全套（provider/model/base_url/api_key/dim/split/max_chars/overlap/min_chars/first_root/exclude_drop/exclude_extra/images/install_service/write_project_toml/project_images 等，逐站点列举）；`wizard.root_not_dir`（`directory does not exist: {path}` / `目录不存在：{path}`）；`init.required`（`not initialized: missing {path}; run vane init` / `尚未初始化：缺少 {path}；请运行 vane init`）。

- [ ] **Step 1: 写失败测试**

```rust
mod zh_copy_tests {
    use vane::i18n::{tr, Lang};

    #[test]
    fn all_why_ids_have_zh_keys() {
        for id in [
            "not_initialized", "not_registered", "still_indexing", "embedder",
            "excluded", "wrong_root", "empty_index", "no_match",
        ] {
            let key = format!("why.{id}");
            assert_ne!(tr(Lang::Zh, &key), "missing-i18n-key", "missing {key}");
            assert_ne!(tr(Lang::En, &key), "missing-i18n-key", "missing {key}");
        }
    }

    #[test]
    fn doctor_json_stays_english_tty_renders_zh() {
        let check = vane::doctor::DoctorCheck::bi(
            "daemon",
            vane::doctor::CheckLevel::Red,
            "daemon is not running",
            "守护进程未运行",
            "run `vane start`",
            "运行 vane start",
        );
        let json = serde_json::to_value(&check).unwrap();
        assert_eq!(json["message"], "daemon is not running");
        assert!(json.get("message_zh").is_none(), "zh fields must not serialize");
    }

    #[test]
    fn wizard_rejects_missing_dir_then_accepts_dot() {
        // run_init 泛型驱动：先输入不存在的目录，重问后输入 "."（spec 测试 14）
        let _g = ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("vane-wiz-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap(); // 必须先建目录，否则 set_current_dir 静默失败
        std::env::set_var("VANE_ALLOW_EMBED_FAIL", "1");
        std::env::set_var("VANE_LANG", "en"); // 固定语言，避免 zh locale 机器上断言英文失败
        let answers_script = "\n\n\n\n\n\n\n\n/no/such/dir\n.\n\n\n\n\nn\n"; // 全部默认 + first_root 两轮 + 不装服务
        let mut out = Vec::new();
        let cwd_restore = std::env::current_dir().unwrap();
        std::env::set_current_dir(&dir).unwrap(); // "." 解析到临时目录
        let result = vane::wizard::run_init(&dir, answers_script.as_bytes(), &mut out, None);
        std::env::set_current_dir(cwd_restore).unwrap();
        assert!(result.is_ok());
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("directory does not exist"), "{text}");
        let cfg = std::fs::read_to_string(dir.join("config/config.toml")).unwrap();
        assert!(cfg.contains("projects"), "first root '.' must register: {cfg}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
```

注：向导答案脚本的确切断行数以实现后的 prompt 序列为准（先跑一遍数 prompt 个数再定稿脚本；`~` 展开用例并入：脚本里把某轮答案换成 `~` 并断言 config 里出现 `$HOME` 展开后的路径）。`set_current_dir`/`set_var` 有测试竞态——该测试与其他用环境变量的测试不得并行冲突；cli_ux.rs 内用 `static ENV_LOCK: std::sync::Mutex<()>` 串行化所有碰环境的测试。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p vane --test cli_ux zh_copy`
Expected: 编译失败（`DoctorCheck::bi` 不存在）/ 断言失败（why key 缺失）

- [ ] **Step 3: 实现**

1. `i18n.rs`：加 `why.*` 8 对 key（en 逐字抄 doctor.rs 现有 message）、`wizard.*`、`init.required`——**两张表同步各加一条**。
2. `doctor.rs`：`DoctorCheck` 加两个 `#[serde(skip)]` 字段与 `bi` 构造器；现有 `DoctorCheck { … }` 字面量全部改 `DoctorCheck::bi(...)`，zh 文案逐站点补写（翻译见下表原则：技术名词 daemon/root/config 保留英文，路径原样嵌入）。`explain_empty_query` 的 message 保持英文不动（非 TTY stderr 用），id 不变。
3. `ui.rs`：`print_doctor`/`print_doctor_check` 开头 `let lang = Lang::detect();`，渲染时 `let message = if lang == Zh && !check.message_zh.is_empty() { &check.message_zh } else { &check.message };`（fix 同理）。`print_why(id, fallback)`：TTY 时 `let key = format!("why.{id}"); let text = tr(lang, &key); if text == "missing-i18n-key" { fallback } else { text }`。main.rs 的 empty-query 调用点改为传 `why.id` + `why.message`；非 TTY 分支仍 `eprintln!("{}", why.message)`（英文，不动）。
4. `wizard.rs`：`run_init`/`prompt_answers`/`run_init_tty`/`prompt_answers_tty`/`prompt_project_setup*` 的提示语字符串全部换 `tr(lang, "wizard.*")`。first_root 校验：
   - 管道版 `prompt_answers`：first_root 提示处包 loop——`loop { let s = prompt(...)?; if s.is_empty() { break None; } let p = crate::fsutil::expand_tilde(Path::new(&s)); if p.is_dir() { break Some(p) } writeln!(stdout, "{}", tr(lang,"wizard.root_not_dir").replace("{path}", &s))?; }`
   - TTY 版：`cliclack::input(...).required(false).validate(|s: &String| { let t = s.trim(); if t.is_empty() { return Ok(()); } if crate::fsutil::expand_tilde(Path::new(t)).is_dir() { Ok(()) } else { Err(tr(Lang::detect(), "wizard.root_not_dir").replace("{path}", t)) } })`（cliclack 0.3 `validate` 的 Err 类型按其实际签名为 `&'static str` 或 `String`，以编译为准；错误时自动重问）。
   - TTY 提示语带 cwd 参考：`format!("{} [current: {}]", tr(lang,"wizard.first_root"), cwd.display())`。
5. `main.rs` `require_init`：`let lang = Lang::detect(); let tty = vane::ui::stdout_tty();` TTY 用 `tr(lang,"init.required").replace("{path}", …)`，非 TTY 保持现有英文格式（现状即英文，不变）。

- [ ] **Step 4: 跑测试确认通过 + 回归**

Run: `cargo test -p vane`
Expected: 全绿（`tests/doctor.rs` / `tests/init_service.rs` 若有断言英文提示语的，TTY 路径默认 lang=En（CI 无 zh LANG），应保持不动）

- [ ] **Step 5: 门禁 + Commit**

```bash
cargo fmt --all -- --check && cargo clippy -p vane --all-targets -- -D warnings
git add crates/vane/src/{doctor,ui,wizard,i18n}.rs crates/vane/src/main.rs crates/vane/tests/cli_ux.rs
git commit -m "feat(vane): zh copy for wizard, doctor TTY, empty-query why; validate first root inline"
```

---

### Task 10: `vane watch`

对应 spec §6.2；覆盖 spec 测试 13。

**Files:**
- Create: `crates/vane/src/watch_diff.rs`
- Modify: `crates/vane/src/dirty.rs`（`paths_for`）、`crates/vane/src/main.rs`（`Watch` 子命令 + `run_watch`）、`crates/vane/src/lib.rs`、`crates/vane/src/i18n.rs`（加 key）
- Test: `crates/vane/tests/cli_ux.rs`

**Interfaces:**
- Consumes: `LiveSet::load_for_project`、`DirtyQueue::load`/`dirty_path`。
- Produces:
  - `vane::dirty::DirtyQueue::paths_for(&self, project_id: &str) -> Vec<String>`（只读列举，排序稳定）
  - `vane::watch_diff::WatchEvent::{Added(String), Updated(String), Removed(String), Queued(String)}`
  - `vane::watch_diff::diff_live(prev: &LiveSet, next: &LiveSet) -> Vec<WatchEvent>`（`extract_key` 变化 → Updated）
  - `vane::watch_diff::diff_queued(prev: &[String], next: &[String]) -> Vec<WatchEvent>`（只在 next 出现的 → Queued；消失不报）
  - `vane::watch_diff::event_line(ev: &WatchEvent, lang: Lang) -> String`（TTY 行）与 `event_json(ev, root, at) -> serde_json::Value`（非 TTY 行）
  - `vane::watch_diff::valid_interval(ms: u64) -> bool`（`100..=60000`，spec 测试 13 的越界断言落在这个纯函数上）
- i18n 新 key：`watch.start`（`watching {root} — Ctrl-C to stop` / `正在监听 {root} — Ctrl-C 停止`）、`watch.daemon_down`（`daemon not running — index changes need vane start; showing current state only` / `守护进程未运行 — 索引变更需要 vane start；当前仅展示现有状态`）、`watch.not_registered`（`root is not registered: {path}` / `目录未登记：{path}`）、`watch.bad_interval`（`--interval-ms must be 100..=60000` / `--interval-ms 必须在 100..=60000 之间`）、`watch.added`/`watch.updated`/`watch.removed`/`watch.queued`（`added {path}` 等 / `新增 {path}`、`更新 {path}`、`移除 {path}`、`排队 {path}`）。

- [ ] **Step 1: 写失败测试**

```rust
mod watch_diff_tests {
    use std::collections::BTreeMap;
    use vane::live::{LiveFile, LiveSet};
    use vane::watch_diff::{diff_live, diff_queued, event_line, WatchEvent};
    use vane::i18n::Lang;

    fn file(key: &str) -> LiveFile {
        LiveFile { content_sha256: key.into(), extract_key: key.into(), chunk_count: 1 }
    }

    fn set(entries: &[(&str, &str)]) -> LiveSet {
        LiveSet { files: entries.iter().map(|(p, k)| (p.to_string(), file(k))).collect::<BTreeMap<_, _>>() }
    }

    #[test]
    fn live_diff_classifies() {
        let prev = set(&[("a.md", "k1"), ("b.md", "k2")]);
        let next = set(&[("a.md", "k1"), ("b.md", "k3"), ("c.md", "k4")]);
        let events = diff_live(&prev, &next);
        assert!(events.contains(&WatchEvent::Updated("b.md".into())));
        assert!(events.contains(&WatchEvent::Added("c.md".into())));
        assert!(!events.iter().any(|e| matches!(e, WatchEvent::Removed(_))));
        let gone = diff_live(&next, &prev);
        assert!(gone.contains(&WatchEvent::Removed("c.md".into())));
    }

    #[test]
    fn queued_only_reports_new_entries() {
        let ev = diff_queued(&["a.md".into()], &["a.md".into(), "b.md".into()]);
        assert_eq!(ev, vec![WatchEvent::Queued("b.md".into())]);
        assert!(diff_queued(&["a.md".into()], &[]).is_empty(), "dequeue is silent by design");
    }

    #[test]
    fn event_line_renders() {
        assert_eq!(event_line(&WatchEvent::Updated("n/a.md".into()), Lang::En), "updated n/a.md");
        assert_eq!(event_line(&WatchEvent::Added("n/a.md".into()), Lang::Zh), "新增 n/a.md");
    }

    #[test]
    fn dirty_queue_lists_paths() {
        let mut q = vane::dirty::DirtyQueue::new();
        q.push("p1", "a.md");
        q.push("p1", "b.md");
        q.push("p2", "c.md");
        assert_eq!(q.paths_for("p1"), vec!["a.md".to_string(), "b.md".to_string()]);
    }

    #[test]
    fn interval_bounds() {
        use vane::watch_diff::valid_interval;
        assert!(!valid_interval(0));
        assert!(!valid_interval(99));
        assert!(valid_interval(100));
        assert!(valid_interval(60_000));
        assert!(!valid_interval(60_001));
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p vane --test cli_ux watch_diff`
Expected: 编译失败

- [ ] **Step 3: 实现**

1. `dirty.rs` 加：
```rust
    /// Sorted queued paths for one project (read-only; used by `vane watch`).
    pub fn paths_for(&self, project_id: &str) -> Vec<String> {
        self.items
            .keys()
            .filter(|(pid, _)| pid.as_str() == project_id)
            .map(|(_, p)| p.clone())
            .collect()
    }
```
（BTreeMap key 有序，天然排序稳定。）
2. `watch_diff.rs`：按 Interfaces 的签名实现（纯迭代 diff；`diff_live` 遍历 next 找 Added/Updated、遍历 prev 找 Removed）。
3. `main.rs`：`Commands` 在 `Mcp` 后加：
```rust
    /// Foreground-watch a root for index changes (client-side polling)
    Watch {
        #[arg(long)]
        root: Option<PathBuf>,
        #[arg(long)]
        all: bool,
        #[arg(long, default_value_t = 1000)]
        interval_ms: u64,
    },
```
`run_watch`：
- `require_init`；`!vane::watch_diff::valid_interval(interval_ms)` → `pick(lang,tty,"watch.bad_interval")` exit 2。
- scope 解析复用 `run_issues` 的选择逻辑（all → 全部 projects；--root → resolve + **校验已登记**（在 cfg.projects 的 canonicalize 列表里），不在 → `watch.not_registered` exit 1；否则 cwd 推断 `current_issues_root`）。
- daemon 未运行 → 打一次 `pick(lang,tty,"watch.daemon_down")`（TTY dim），继续。
- 首行 `watch.start` 替换 `{root}`（多 root 时逐 root 各打一行或打 `searching k roots` 式汇总，选逐 root 一行）。
- 状态：`HashMap<root, (LiveSet, Vec<String>)>` 前帧；循环：`sleep(interval)` → 每 root 重载 live + dirty paths → `diff_live` + `diff_queued` → TTY `println!(event_line)` / 非 TTY `println!(event_json)` → flush。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p vane`
Expected: 全绿

- [ ] **Step 5: 门禁 + Commit**

```bash
cargo fmt --all -- --check && cargo clippy -p vane --all-targets -- -D warnings
git add crates/vane/src/{watch_diff,dirty,main,lib,i18n}.rs crates/vane/tests/cli_ux.rs
git commit -m "feat(vane): vane watch foreground observer (client-side polling)"
```

---

### Task 11: 文档同步 + 全量门禁

对应 spec §7.3。

**Files:**
- Modify: `docs/superpowers/specs/2026-08-19-vane-sidecar-design.md`（CLI 命令清单处加一行指针）、`skills/vane/SKILL.md`、`README.md`、`README.zh-CN.md`

- [ ] **Step 1: sidecar 设计文档指针**

在 `2026-08-19-vane-sidecar-design.md` 的 CLI 命令清单段落末尾加一行：

```
> 2026-08-20 起补充：`read` / `watch` 子命令与 TTY 人性化渲染以 `docs/superpowers/specs/2026-08-20-cli-human-ux-spec.md` 为准。
```

- [ ] **Step 2: SKILL.md 更新**

在 CLI 命令相关段落补 `vane read <n>` 与 `vane watch`，并**明确限定**（spec §7.3 仲裁）：

```markdown
`vane read <n>` / `vane watch` 面向人工验收：`read` 依赖上次 TTY `vane query` 的缓存，agent 脚本不要使用——继续优先用 MCP 的 `search` / `read` 工具（无状态）。
```

注意：SKILL.md 是 Task 8 `include_str!` 的内嵌源，本步改动直接改变下次安装的内容，无需额外同步。

- [ ] **Step 3: README 双语命令清单**

两份 README 的 CLI 部分补 `read` / `watch` 一行说明（英文版英文、中文版中文）。

- [ ] **Step 4: 全量门禁（工作区级）**

Run:
```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```
Expected: 全绿。若 `--all-features` 触发 core 的 fault-injection 等重型 feature 导致时间超限，退化为 CLAUDE.md 常用入口逐条跑并在任务记录里注明。

- [ ] **Step 5: Commit**

```bash
git add docs/superpowers/specs/2026-08-19-vane-sidecar-design.md skills/vane/SKILL.md README.md README.zh-CN.md
git commit -m "docs(vane): document read/watch and human-UX spec pointers"
```

---

## Self-Review 记录（计划作者自查，含评审修订后的如实标注）

- **Spec 覆盖**：§2.1→T3，§2.2→T3，§2.3→T3，§2.4→T4，§2.5→T9，§3.1→T2+T5，§3.2→T5，§3.3→T6，§3.4→T6，§4.1→T7，§4.2→T7，§4.3→T8，§4.4→T8，§5.1→T2+T9，§5.2→T1（expand_tilde）+T9（校验重问），§6.1→T5，§6.2→T10，§7.1→T1–T10 分层约束，§7.2→各 Task 门禁与 JSON 不动约束，§7.3→T11。spec 测试 1→T2，2→T2+T9，3→T3，4→T3，5/6→T4，7→T6，8→T5，9→T7，10→T7，11→T3，12→T8，13→T10，13b→T6，14→T9。
- **人工验收豁免（如实标注，非自动化覆盖）**：spec 测试 5 的「非 TTY query 不写缓存」与「TTY query 写缓存」两条依赖真实 TTY + daemon，自动化只覆盖缓存读写与 `read_outcome` 本身——TTY 写入/非 TTY 不写入的分支靠人工验收（同 spec §2.3 的 cliclack 提示豁免）。spec 测试 6 的 `--file` 三分支中 file_missing/binary 判定在 `run_read_file`（main.rs），自动化覆盖靠测试直接构造场景驱动该函数不可行（main 私有），亦记为人工验收 + 评审时抽查。
- **占位符扫描**：T9 的向导答案脚本行数、T7 help 标题是否带冒号、T2 `abs_date` 期望值（评审已演算确认 `1755700000 → 2025-08-20`，可直接填死）三处为执行期一次运行即可确定的机械值，非设计空缺。
- **类型一致性**：`ProgressStyle`/`clamp_pos`（T6 于 progress.rs）、`HitLineOpts`/`hit_lines`（T3）、`status_view`/`format_status_lines`（T5）、`ReadOutcome`/`ReadError`/`hit_at`（T4）、`WatchEvent`/`diff_live`/`diff_queued`/`event_line`/`event_json`/`valid_interval`（T10）、`decide_bare`/`decide_query_arg`（T3/T7 同文件 dispatch.rs）——跨 Task 引用名称已逐一核对一致。`print_status_dashboard` 签名变更（T5 加 `home` 形参）的唯一调用点 `run_status` 已在 T5 Step 3 同步。
- **已批准偏差记录**：① T4 缓存 JSON 用 `scope_root: Option<String>` 替代 spec 字面的 tagged `scope` 对象（新文件无消费方）；② T3 头行颜色首版只对 degraded 上色（dim/accent 简化）；③ T1 不收敛 cas.rs/index.rs 的 2 参 atomic_write 与 config.rs/daemon.rs/gc.rs 的 expand_tilde（避免无收益大 diff）。
