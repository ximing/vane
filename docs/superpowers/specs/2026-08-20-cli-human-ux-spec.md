# Vane CLI 人性化交互规范（CLI Human UX v1.0 草案）

日期：2026-08-20（v1.1：两份评审意见闭环修订；v1.2：复审遗留 1 条矛盾 + 3 条建议修正）
状态：已冻结（代码可行性复审 APPROVE；完整性复审 APPROVE_WITH_CHANGES 的唯一必须项已在 v1.2 闭环）
范围：`crates/vane` 产品 CLI（`vane` binary）的终端交互层。不改 daemon RPC schema、不改 `vane-core` 任何合同、不改持久化索引格式。
上游依据：维护者验收反馈（2026-08-20），逐条对应本文 §2–§6。

本文是产品 CLI 交互层的执行合同。`docs/REQUIREMENTS.md` / `docs/SPEC.md` 仍只约束嵌入式检索库；本文与 `docs/superpowers/specs/2026-08-19-vane-sidecar-design.md` 同级，冲突时以后者的架构边界（不改 core、无 tokio、无新运行时重依赖）为准。

---

## 1. 目标与非目标

### 1.1 一句话

让「人配一次、Agent 天天搜」中的「人配一次」和偶尔的人工验收（`vane query` / `vane status` / `vane doctor`）读得懂、接得上：知道在搜哪里、结果能接着打开、状态说人话、命令面不吓人。

### 1.2 成功标准（验收锚点）

- 在一个已登记 root 里敲 `vane query foo`，第一行能回答「搜的哪个目录、多少文件、走的哪条召回路径（降级可见）」。
- `vane query` 之后敲 `vane read 2` 能看到第 2 条命中的正文，全程不碰 MCP、不抄路径。
- `vane status` 不出现 Unix 秒和裸数字；跳过数能指到 `vane issues`。
- 裸敲 `vane` 得到当前状态而不是一屏 help；`vane --help` 分「常用 / 管理 / 运维」三组。
- `LANG=zh_CN.UTF-8` 下，init 向导、doctor 的 TTY 报告、空结果原因、probe 失败为中文。
- `vane add` 结束有一句人话总结；索引进度是 `34/120` 的条，不是只有 spinner。

### 1.3 非目标（明确不做）

- 交互 REPL、全屏 TUI、点击打开编辑器、GUI——那是另一款应用。
- 全量 CLI 翻译：clap help 正文、`logs` / `inspect` / `df` / `gc` 输出保持英文。**一切进入 JSON 输出、持久化文件（`last_error.json` 等）、以及非 TTY stderr 的文案字段取值一律保持英文**——i18n 只发生在 TTY 渲染瞬间（§5.1 硬规则）。
- 不改 daemon 的 `search` / `status` RPC 返回字段；不改 `progress.json` / `skips.json` / `state.json` / `dirty.json` 磁盘格式（lib 内新增读取 API 可以，改 schema 不行）。
- 不引入新运行时依赖：i18n 不引 `fluent`/`rust-i18n`，路径输入不引 `rustyline`/`reedline`（理由见 §5.2）。`indicatif` / `cliclack` / `console` 为既有依赖。
- Windows 支持仍不在范围（sidecar 设计合同不变）。

---

## 2. 搜索体验（对应反馈 2）

### 2.1 范围头行（TTY 专有）

`vane query` 在 TTY 输出命中列表前，先打印一行范围头：

```
searching ~/notes · 12 live files · hybrid
```

精确格式：

- 单 root 作用域（cwd 推断或 `--root`）：`searching {root} · {n} live files · {mode}`
  - `{root}`：绝对路径做 `~` 折叠（`$HOME` 前缀 → `~`）；不做其他缩写。
  - `{n}`：该 root 的 live files 数，客户端从本地 `LiveSet::load_for_project` 读取（与 `main.rs` 既有 `count_live_files` 同一路径），**不新增 RPC**。
  - `{mode}`：`hybrid`；当响应中任一 hit 带 `degraded: true`（hit 级字段，`SearchHit.degraded`）时，显示黄色 `BM25 (degraded: embedder unreachable)`。
- `--all` / `--global` 作用域：`searching {k} roots · {n} live files · {mode}`（n 为各 root live 数之和）。
- 颜色：`searching` 与分隔符 `·` 用 dim，`{root}` 用 accent；degraded 文案黄色。`NO_COLOR` / 非 TTY 时无任何样式。
- 空结果时头行照打，随后是既有的 why 行（§2.5 语言规则适用）。
- 非 TTY（管道）输出：不打印头行，JSON 输出格式与字段**一字不变**（agent 消费面不动）。

### 2.2 命中条目精简

`ui::print_hits` 调整为：

- 单 root 作用域：不再逐条打印 dim 的 root 行（头行已回答）；hit 的 `path` 本身即 root 相对路径（`index.rs` 写入的 stored `path`），原样打印。
- `--all` 作用域：保留逐条 dim root 行（此时 root 有区分价值）。
- 内部 `id` 行默认**不打印**；`vane query` 新增**子命令局部** flag `--verbose`（仅作用于 `query`，不进 clap 全局参数）时恢复打印。
- 头行已聚合显示 degraded 时，逐条的 degraded 黄字**省略**（不重复报警）；逐条 degraded 行仅在不可能有头行的调用路径（未来内部复用）保留。
- 其余元素（序号、分数、绿色路径、snippet）不变。

### 2.3 无参数的 `vane query`

- `q` 改为 `Option<String>`。
- TTY（stdin+stdout 均为终端）：用 cliclack `input` 提示输入查询词（文案随 §5 语言），空输入视为取消，exit 0 不打印错误。此分支为 cliclack 交互，自动化测试不覆盖，靠人工验收（§8 明确豁免）。
- 非 TTY：打印单行英文错误 `missing query — usage: vane query <text>`（非 TTY 文案一律英文，§5.1 硬规则），exit code 2。**不得**落入 clap 的 missing-argument 长文。

### 2.4 `vane read <n>`：接着读刚才那条

新子命令，让人用 CLI 完成「搜 → 读」闭环验收，不走 MCP。

- 缓存：`vane query` 在 **TTY 交互调用**成功返回后（含空结果），将本次结果原子写入 `$VANE_HOME/run/last_query.json`：
  ```json
  {
    "query": "foo",
    "at": 1755700000,
    "scope": { "kind": "root", "root": "/abs/path" },
    "hits": [{ "id": "...", "path": "notes/a.md", "root": "/abs/path", "score": 0.42 }]
  }
  ```
  - `scope.kind` 为 `"root"` 或 `"all"`；hit 的 `id` 编码为 `{project_id}:{path}#{chunk_index}`，经既有的 pub `parse_doc_id`（`search.rs`）可拆出 chunk 定位，`SearchHit` 无需加字段。
  - **写入决策（评审仲裁）**：只有 TTY query 写缓存；非 TTY（agent 管道）query **不写**，避免脚本一次 `vane query | jq` 覆盖人刚查的缓存。取舍记录：脚本想连用 `read` 时需自己走 MCP 或重跑 TTY query——agent 消费面本来就有无状态的 MCP `read`，CLI 缓存是面向人的。
  - 原子写：抽出共享 helper（把 `progress.rs` 私有的 `atomic_write` 提为 `pub(crate)` 共享函数，顺带收敛 `live.rs` / `mcp.rs` 里的两份私有拷贝；实现细节允许先提一份共用、旧拷贝逐步清理）。
  - 这是**客户端缓存**，daemon 无感知；损坏/解析失败按「无缓存」处理，不得 panic。
- `vane read <n>`（n 为 1-based 序号，对应上次 TTY query 的命中序号）：
  - 默认输出该命中的**切片正文**：复用 pub 的 `search::read_by_id`（输入 `Cas` + `LiveSet`，返回含 canonical text 的 `ReadChunk`）；定位所需 chunk_index 由 `parse_doc_id` 从缓存的 `id` 拆出。
  - 输出头部一行 dim 元信息：单 root scope 为 `{root 相对路径} · score {score} · chunk {k}`；scope 为 `all` 时加 root：`{root} :: {相对路径} · score {score} · chunk {k}`（消歧同名相对路径）。随后空行 + 正文。
  - `--file`：改为直接打印磁盘上的源文件全文（`root/path`），**不依赖缓存的 chunk 定位**，因此不受缓存陈旧影响；源文件已不存在时给出明确错误。二进制/图片类型不打印内容，提示路径与类型。
  - **缓存陈旧（评审仲裁）**：query 之后文件被编辑/reconcile，`read_by_id` 找不到对应 chunk 时，报单行错误 `this hit is stale — the file changed since the last query; re-run vane query or use vane read {n} --file`（zh 对应），exit 1；`--file` 路径不受此影响。
  - 不依赖 daemon 运行（CAS 与 live set 均在 `$VANE_HOME` 磁盘上，daemon 停机也可读）。
- 错误路径（均为单行；TTY 随 §5 语言，非 TTY 一律英文）：
  - 无 `last_query.json`：`no recent query — run vane query first`。
  - n 越界：`no hit {n} — last query has {k} hits (1..={k})`。
  - 上次 query 为空结果：`last query had no hits`。
- 非 TTY：`vane read` 输出正文本身（不加颜色），元信息行省略；便于管道。

### 2.5 空结果与错误语言

`explain_empty_query` 现有 **8 类**原因（`not_initialized` / `not_registered` / `still_indexing` / `embedder` / `excluded` / `wrong_root` / `empty_index` / `no_match`，doctor.rs 实际分支数）的 message 全部进入 §5 的 i18n key，zh 环境的 TTY 输出为中文；`id` 与判断逻辑不变，非 TTY stderr 的英文不变（§1.3 硬规则）。

---

## 3. 状态、时间与数量说人话（对应反馈 3）

### 3.1 相对时间格式化

新模块 `crates/vane/src/humanize.rs`，纯函数、可单测：

```
rel_time(ts: u64, now: u64, lang: Lang) -> String
```

- `< 10s`：`just now` / `刚刚`
- `< 60s`：`{n}s ago` / `{n} 秒前`
- `< 60min`：`{n} min ago` / `{n} 分钟前`
- `< 48h`：`{n} hours ago` / `{n} 小时前`
- `< 30d`：`{n} days ago` / `{n} 天前`
- 否则：绝对日期 `YYYY-MM-DD`
- `ts > now`（时钟回拨）：按 `just now` 处理，不打印负数。
- `ts == 0` / None：调用方显示 `never` / `从未`。

应用点（仅 TTY 展示层，JSON 字段保持 Unix 秒不变）：

- `status` 每 root 的 `last_reconcile`：`indexed 3 min ago`；None → `never indexed`。
- `status` / `issues` 中 skip 的 `at`、last_error 的 `at`：随文括号 `(3 min ago)`。
- daemon 进行中（`progress.json` phase != idle）：root 行显示 `indexing {scanned}/{total_estimate}`，替代时间。

### 3.2 `vane status` 看板增强

在现有 dashboard 基础上：

- daemon 运行且无进行中 progress → 顶部显示 `watching`（zh：`正在监听`）；有 progress → 显示 `indexing {scanned}/{total}`；**daemon 停止时顶部维持现有 `daemon not running — vane start` 警告不变**。
- 每 root 的 `skips` 行：`12 skipped — run vane issues`（zh：`12 个文件被跳过 — 运行 vane issues 查看`）；skip 为 0 或缺失时不打印该行。
- `dirty` 行改文案为 `{n} pending changes`（zh：`{n} 个待处理变更`）；0 时不打印。
- JSON 输出：字段与取值不变，只改 TTY 渲染。

### 3.3 `vane add` 结束总结

现有 `added {root} scanned … skipped N` 机器行保留（管道解析可能依赖），TTY 下在其后追加一句人话：

- skipped == 0：`indexed 80 files`（zh：`已索引 80 个文件`）。
- skipped > 0：`indexed 80 files, 3 skipped — run vane issues`（zh：`已索引 80 个文件，跳过 3 个 — 运行 vane issues 查看`）。
- 「indexed 数」= 响应的 `added + unchanged`（工作集总量），不是单指新增。

### 3.4 索引进度条

`add_root_poll_progress` 的展示升级：

- `progress.total_estimate > 0`：用 `indicatif::ProgressBar::new(total)`，模板 `{bar:30} {pos}/{len} {phase} {root}`（如 `██████░░░░ 34/120 embed ~/notes`）。`total_estimate` 会在扫描中途增长（`sync.rs` 的 `apply_report` 兜底上调），轮询到更大值时用 `set_length` 更新；`pos` 以 `scanned` 为准、封顶不超过 `len`。
- `total_estimate == 0`（扫描初期窗口）：退回现有 spinner + `spinner_message`。
- 「选 bar 还是 spinner」抽成纯函数（输入 `total_estimate`），入 §8 测试。
- 完成时 `finish_and_clear`，随后打 §3.3 总结行。
- 非 TTY：无任何进度输出（现状不变）。

---

## 4. 命令面收敛（对应反馈 4）

### 4.1 裸 `vane`

`Commands` 改为 `Option<Commands>`。无子命令时：

- 非 TTY：手动 `Cli::command().print_help()` + exit 2（`Option<Commands>` 之后 clap 不再自动这样做，需显式保持现状语义）。
- TTY：
  - 未初始化（无 `config.toml`）→ 打印一句引导（zh/en 随 §5）：`vane is not initialized — run vane init to set up`，exit 0。
  - daemon 未运行 → 直接渲染 `vane doctor` 的 TTY 报告（复用 `doctor::run` + `ui::print_doctor`），exit code 同 doctor。
  - 正常 → 渲染 `vane status` 的 TTY dashboard，exit 0。
- 该 dispatch 决策（三分支选择）下沉到 lib 的纯决策函数（输入：是否 TTY、home 下是否有 config、daemon 是否运行——均可注入），main.rs 只做 IO 绑定（§7.1）。

### 4.2 `--help` 分组

用 clap 4 derive 的 `next_help_heading`（Cargo.lock 已锁定 clap 4.x，支持）分三组（组标题保持英文，见 §1.3）：

- **Common**：`init` `add` `query` `read` `status` `doctor`
- **Manage**：`rm` `include` `exclude` `issues` `logs` `inspect` `model` `mcp` `watch`
- **Ops**：`daemon` `start` `stop` `service` `df` `gc`

各子命令 help 文案不变。声明顺序保证 Common 在 `vane --help` 输出最前。

### 4.3 `vane mcp install` 收尾文案

TTY 报告末尾固定追加一行（zh/en 随 §5）：

```
done — start a new agent session (Claude / Cursor / Codex) for the vane tools to load
```

zh：`完成 — 新开一轮 Agent 会话（Claude / Cursor / Codex）后 vane 工具才会加载`。

### 4.4 skill 安装并入 `vane mcp install`

- 触发条件（评审仲裁，写死）：`--client claude` 显式指定时**总是**安装；默认全量（未指定 `--client`）时，当且仅当 `~/.claude` 目录已存在才安装（不存在则不安装、不报告——没装 Claude 的用户不应被新建目录）。显式 `--client claude` 而 `~/.claude/skills` 不存在时**创建**该目录树（显式指令即授权）。
- 内容：把 `skills/vane/SKILL.md` 以 `include_str!("../../../skills/vane/SKILL.md")` 编译期内嵌（相对 `crates/vane/src/mcp.rs` 深度已核对；crate `publish = false`，仓库内单一真源，不复制副本），写入 `~/.claude/skills/vane/SKILL.md`。
- 幂等：已存在且字节一致 → 报告 `skip (~/.claude/skills/vane/SKILL.md: up-to-date)`；存在但不同 → 覆盖并报告 `updated`；新建 → `wrote`。
- `--dry-run` 对 skill 同样只报告不写入。
- `cursor` / `codex` 无对应 skill 目录约定，本版不处理。
- 兼容性表述（评审修正）：`McpInstallReport` 当前仅 derive `Serialize`；行的 `kind: "config" | "skill"` 是 **additive 可选字段新增**，依据 §7.2「只允许新增可选字段」即向后兼容，不涉及反序列化容忍机制。
- 成功报告中 skill 行与 config 行同样受 §4.3 收尾文案覆盖。

---

## 5. 语言与默认值（对应反馈 5）

### 5.1 i18n 层（最小实现）

新模块 `crates/vane/src/i18n.rs`：

- `Lang::{En, Zh}`；`Lang::detect()` 优先级：环境变量 `VANE_LANG` > `LC_ALL` > `LANG`；值以 `zh` 开头（大小写不敏感）→ `Zh`，否则 `En`。`VANE_LANG=en|zh` 显式覆盖。
- `tr(lang, key) -> &'static str`：两张静态表（`const EN: &[(&str, &str)]` / `ZH`），key 缺失时编译期测试兜底（§8 测试要求两表 key 集相等）。
- 插值约定：占位符一律**命名**（`{n}` / `{added}` / `{skipped}` / `{root}` …），用 `str::replace` 按名替换；zh 文案允许自由调整语序，禁止按位置参数拼接锁死语序。

**硬规则（评审仲裁，覆盖全节）**：i18n 只作用于 **TTY 渲染瞬间**。进入 JSON 输出的字段值（如 `DoctorReport.checks[].message/fix`）、写入持久化文件的文案（`last_error.json`）、以及非 TTY 的 stderr/stdout 文案，**一律英文**。据此：

1. init / add 向导全部提示语与 next-steps card：向导本身是 TTY/交互专属，直接用 `tr` 出当前语言。
2. `doctor`：check 结构内 message/fix 保持英文（JSON 消费面不变）；TTY 渲染（`print_doctor`）按 check `id` 查 i18n 表出中文。check `id` 因此成为稳定键，不得随意改名。
3. 空结果 why 8 类 message：TTY 的 `print_why` 出中文；非 TTY stderr 保持英文。
4. embed probe 失败、`require_init` 等错误：TTY 出中文；写入 `last_error.json` 的持久化文案保持英文。
5. 本规范新增文案：范围头行、status 新行、add 总结行、mcp install 收尾、query 提示输入、read 错误、watch 首行/daemon 停机提示/四条失败态错误（TTY 中文；非 TTY 一律 stderr 英文）。
6. §3.1 相对时间词（仅 TTY 使用）。

### 5.2 首个 root 的路径输入

init 向导「First project root」与 `vane add` 的路径参数：

- 接受 `.`、`~`、`~/…`、相对路径（`expand_tilde` + canonicalize 语义），本项把它变成**经过测试的承诺**；实现时顺带收敛 `wizard.rs` 与 `main.rs` 里两份私有 `expand_tilde` 拷贝为一处共享。
- TTY 向导中输入后立即校验「存在且为目录」：cliclack 0.3 的 `Input::validate` 闭包原生支持报错重问；管道版（`run_init` 的 `R: Read, W: Write` 泛型）在 prompt 外包 loop 重问。校验复用 `normalize_root` 语义。
- TTY 向导的默认值建议：提示语显示 cwd 作为参考（`First project root (empty to skip) [current: /abs/cwd]`），不自动填入。
- **不做 tab 补全**：cliclack 无此能力，引入 line-editor 依赖与本 crate 的依赖纪律（deny.toml、无重型运行时依赖）不符；以「接受 `.` + 即时校验 + 显示 cwd」替代。此决定记录在案，未来若换输入库再议。

---

## 6. 索引活跃度可见（对应反馈 6）

### 6.1 status 的 watching / indexing 行

见 §3.2。数据全部来自既有本地文件（`progress.json`、`state.json`），无 daemon 改动。

### 6.2 `vane watch`（前台观察模式）

新子命令，满足「想看时有得看」，默认工作方式（静默 daemon）不变。

- 行为：解析 scope（cwd 推断 root / `--root` / `--all`，与 `vane issues` 同一套解析），打印首行 `watching {root} — Ctrl-C to stop`，随后每 `{interval}`（默认 1000ms，`--interval-ms` 可调，合法范围 100–60000，越界报单行错误 exit 2）轮询一次 `LiveSet` 与 dirty queue，对 diff 逐行打印：
  - `added {relpath}` / `updated {relpath}` / `removed {relpath}`（relpath 为 root 相对路径）。
  - 判断依据：LiveSet 条目的出现/消失/内容键（`LiveFile.extract_key`，pub）变化。
  - dirty queue：轮询的是**队列快照 diff**（不是入队事件——watch 是独立进程，看不到事件）；`DirtyQueue` 目前只有 `len`/`len_for`，需在 lib 内**新增 pub 的只读列举/快照方法**（读取既有 `dirty.json`，不改磁盘格式）。出队（reconcile 排空）**不打印**，是有意的——live set 的 `updated` 行已经表达了结果。
- **失败态（评审仲裁）**：
  - 未初始化：与 `require_init` 相同的单行错误（zh/en 随 §5），exit 1。
  - cwd 不在任何已登记 root 且未给 `--root`/`--all`：与 `issues` 相同的单行错误，exit 1。
  - `--root` 指向未登记路径：单行错误 `root is not registered: {path}`，exit 1。
  - daemon 停机：启动时打印一次 dim 提示 `daemon not running — index changes need vane start; showing current state only`（zh 对应），随后继续轮询（本地 `vane gc`/rebuild 仍可能改 live set）。
- 纯客户端轮询，**不新增 IPC、不改 daemon、不读 notify**；diff 计算进 lib 新模块 `watch_diff.rs`（避免与既有 daemon 侧 `watch.rs` notify 模块混淆），main.rs 只做参数绑定。
- 非 TTY：每行一个 JSON 对象 `{"event":"updated","path":"notes/a.md","root":"…","at":…}`。
- 退出：Ctrl-C（默认信号行为即可，不装 handler）。

---

## 7. 工程约束

### 7.1 可测试性分层

- 所有新展示/决策逻辑进 lib：`humanize.rs`、`i18n.rs`、新增的 `last_query.rs`（缓存读写）、`watch_diff.rs`（diff 计算）、`ui.rs` 新打印函数。
- **dispatch 决策同样下沉到 lib**（评审仲裁）：裸 `vane` 三分支选择、query 缺参分支等，实现为接受注入参数（is_tty / home / daemon_running 等）的纯决策函数；`main.rs` 只做 clap 解析、IO 绑定与退出码映射。否则 §8 第 9、11 条无法执行。
- TTY 渲染函数与「算数据」分离：渲染函数接收已算好的结构，便于测试断言数据层。

### 7.2 兼容性边界

- 所有现有 JSON 输出（非 TTY）字段、取值、顺序承诺不变；只允许**新增**可选字段（如 `McpInstallReport` 行的 `kind`）。
- `last_query.json` 是新文件，不读旧状态，无迁移问题；损坏/解析失败时按「无缓存」处理，不得 panic。
- 文案 key 与分组标题的英文版本随本规范冻结；zh 文案允许后续修订（不改行为）。
- `crates/vane` 依赖黑名单与 WASM 排除规则不变；本规范不新增任何依赖。

### 7.3 文档同步

- `docs/superpowers/specs/2026-08-19-vane-sidecar-design.md` 中 CLI 命令清单涉及 `read` / `watch` 处追加指针到本文（一句话，不改写历史）。
- `skills/vane/SKILL.md` 的 CLI 命令列表补 `read` / `watch`，并注明二者面向**人工验收**，agent 应继续优先使用无状态的 MCP `read`（CLI `read` 依赖上次 TTY query 的缓存，不适合 agent 脚本）。
- 根 README 命令清单同步。

---

## 8. 验收测试清单（每个编号对应至少一个测试）

新增测试文件 `crates/vane/tests/cli_ux.rs`（或并入既有相近文件），除注明外均不依赖真实 daemon/embedder：

1. `rel_time` 全档位 + 时钟回拨 + ts=0（zh/en 双表）。
2. i18n 两表 key 集合相等（含 8 个 `EmptyQueryWhy` id 对应的 key 逐一存在）；`Lang::detect` 优先级与 `zh_CN.UTF-8` / `en_US` / 未设置 / `VANE_LANG` 覆盖四例。
3. 范围头行：单 root `~` 折叠、live 数注入、`--all` 汇总、degraded 分支（渲染函数注入假数据断言字符串）。
4. `print_hits` 单 root 不打 root 行、`--all` 打、`--verbose` 才打 id、头行已聚合时无逐条 degraded。
5. `last_query.json`：TTY query 写入 → `read 1` 取出 chunk 正文；n 越界 / 无缓存 / 空结果 / **缓存损坏按无缓存** / **chunk 失效报 stale** 五条错误路径；**非 TTY query 不写缓存**。
6. `vane read --file` 读磁盘源文件（缓存陈旧也能读）；源文件缺失时报错文案；非 TTY 省略元信息行。
7. add 总结行：skipped=0 / skipped>0 两分支（zh/en）。
8. status 渲染：`watching` / `indexing 34/120` / `indexed 3 min ago` / `never indexed` / `12 skipped — run vane issues` / dirty 为 0 不打印 / daemon 停止时顶部警告不变。
9. 裸 `vane` 三分支 dispatch 决策函数（未初始化 / daemon 停 / 正常 × TTY；非 TTY → help 决策），纯函数注入测试，不起真 daemon。
10. `vane --help` 输出含 Common / Manage / Ops 三标题且 `query` 在 Common 段（`env!("CARGO_BIN_EXE_vane")` 子进程 + 子串断言）。
11. `vane query` 无参数非 TTY：单行错误 + exit 2，无 clap 长文（`CARGO_BIN_EXE_vane` 子进程断言）。TTY 提示分支为 cliclack 交互，**人工验收豁免**（记录在测试文件注释）。
12. mcp install（临时 HOME）：`--dry-run` 报告含 skill 行；首装 `wrote` → 二装 `up-to-date` → 改动目标文件后 `updated`；默认全量且 `~/.claude` 不存在时不安装 skill；收尾文案存在。
13. `watch_diff` 纯函数：构造前后两个 LiveSet 快照，断言 added/updated/removed 分类正确；`--interval-ms` 越界报错。
13b. 进度条选择纯函数：`total_estimate == 0` → spinner、`> 0` → bar 两分支；`pos` 封顶不超过 `len`。
14. init 向导路径校验：输入不存在目录 → 重问后接受合法输入；输入 `.` → 接受并 canonicalize；`~` 前缀展开（驱动 `run_init` 的泛型 stdin/stdout）。

既有测试全部保持绿色；`cargo fmt --check`、`clippy -D warnings`、`cargo test -p vane` 为提交门禁。

---

## 9. 版本与里程碑

- 归属：产品 CLI 独立里程碑 **post-v0.3.1「Human UX」**，目标版本 `v0.4.0`（仅 `crates/vane`；workspace 同步版本政策若要求四端齐发，则在发版计划中显式说明库端为无变化重发）。
- 本规范冻结条件：~~复审通过 + 开发计划评审通过~~ 复审已通过（2026-08-20，v1.2 起冻结；行为条款不得再改，zh 文案与实现细节允许在开发计划评审中微调）。

---

## Changelog

- **v1.0**（2026-08-20）：初稿。
- **v1.1**（2026-08-20）：两份评审（代码可行性 + 完整性/UX）意见闭环。要点：① i18n 硬规则——仅 TTY 渲染瞬间，JSON/持久化/非 TTY stderr 一律英文（消解 §5.1 与 §7.2 冲突）；② `last_query.json` 仅 TTY query 写入，read 复用 pub `read_by_id` + `parse_doc_id`，补缓存陈旧/损坏行为；③ `--verbose` 明确为 query 局部 flag；④ why 计数统一为 8 类；⑤ skill 安装触发条件与目录创建策略写死，`kind` 字段兼容性改为 additive 表述；⑥ watch 补失败态、`--interval-ms` 范围、`DirtyQueue` 需新增只读列举 API、diff 进 `watch_diff.rs`；⑦ dispatch 决策下沉 lib；⑧ 裸 vane 非 TTY 手动 print_help；⑨ 进度条 `set_length` 处理 total 增长；⑩ §8 测试补 5 个缺口并豁免 cliclack 交互分支。
