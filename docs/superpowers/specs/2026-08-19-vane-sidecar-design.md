# Vane 本地文档侧车

日期：2026-08-19  
状态：已评审，待实现计划  
范围：本机文档切片、增量索引、RAG 检索；CLI + 常驻守护进程 + MCP。  
实现语言：Rust。检索引擎复用 `vane-core`，不把模型、监听、HTTP 打进 core。

本文是产品侧车的设计合同。`docs/REQUIREMENTS.md` / `docs/SPEC.md` 仍只约束嵌入式检索库；侧车不得静默改 core 的公共 API、持久化格式或 Won't-have。

---

## 1. 目标与非目标

### 1.1 一句话

一个本机 Rust 守护进程盯若干文档目录，按内容哈希复用切片与向量，只把当前磁盘上仍引用的文件送进 Vane；Agent 通过 MCP 做混合检索，人通过 `vane` CLI 配置和验收。

### 1.2 目标用户

主要使用者是本机编码 Agent（Claude / Codex / Cursor）。人只负责 `init`、登记目录、改项目策略。

### 1.3 成功标准

- `vane init` 四步后，装上用户服务，对已登记目录可 `search` / `read`。
- 同一文件内容在分支间往返：提取与 embedding 都命中 CAS，不重算。
- 换模型：切片可复用则不重切，向量全部按新模型重算，旧维数库不与新库混排。
- 一个进程、一个 OS 级文件监听；exclude 目录不注册 watch。
- Embedding 不可达时，已入库文档仍可 BM25 检索，结果带降级标记。

### 1.4 非目标（第一版）

- 不修改 `vane-core` 合同：不内置 embedding、不引入 tokio、不把 `std::fs` 扩出 VFS 例外。
- 不解析 `.gitignore`。工作集 = 磁盘存在的文件 − 合并后的 exclude。
- 不内置 PDF / Word / PPT 解析。规则表和文档 schema 按「以后同一项目库检索」预留。
- 不跑 VLM / CLIP；图片只建元数据文档，原图经 `read` 交给 Agent 的多模态模型。
- 不做 Windows 服务（第一版：macOS launchd + Linux systemd --user）。
- 不在 MCP 里写配置。
- 不为每个目录起进程。
- 不把网络盘 / FUSE 上的 `notify` 可靠性当作承诺；启动对账补洞。
- **GC 永不删除用户源文件**（登记根目录里的文档）。只删 `$VANE_HOME/rag` 下的 CAS / 陈旧索引。

---

## 2. 与 Vane 库的关系

| 层 | 职责 |
|----|------|
| `vane-core` | 文本 + 向量存储、BM25、HNSW、hybrid、filter、flush、compact |
| `crates/vane`（本产品，binary） | CLI、守护进程、watcher、CAS、提取器、embedding HTTP、MCP、用户服务安装 |

`crates/vane` 只依赖 `vane-core` 的公开 API（`Db::open` + `StdFsVfs`）。禁止把 `notify`、HTTP 客户端、启动项逻辑放进 core。

工作区约束：

- 本 crate **禁止**依赖 `tokio`（`deny.toml` 黑名单）。运行时用标准库线程 + channel + 阻塞 HTTP（`ureq`）。
- glob 用 `globset` / `ignore` 一类不把 `regex` 引进工作区运行时的库。
- WASM / `wasm32-unknown-unknown` CI **排除** `crates/vane`。
- `crates/vane` 加入 workspace `members` 与 `default-members`（native 默认测试覆盖）；`cargo test --workspace` 已有的 wasm/fuzz 排除规则保持不变，并再排除该 crate 的 wasm job。

命令名与 crate 名都是 `vane`。库 crate 继续叫 `vane-core`。

---

## 3. 目录布局

默认家目录是 `~/.vane/`，不另起产品名目录。解析顺序：**`--home <dir>`（CLI/daemon 参数）> 环境变量 `VANE_HOME` > `~/.vane`**。测试必须设 `VANE_HOME` 指向隔离临时目录，禁止写真实家目录。下文路径均相对该家目录。

```
~/.vane/
  config/
    config.toml                 # 全局默认 + 已登记项目名单 + 可选项目覆盖
  rag/
    cas/
      extract/<extract_key>/    # 切片结果
      embed/<embed_key>/        # 单 chunk 向量
    projects/<project_id>/
      state.json                # root_path、model_id、dim、chunk_strategy_id、重建状态
      live.json                 # 工作集：path → 哈希与 chunk 数
      db/                       # 当前模型的 Vane 库（StdFsVfs 根）
      db.prev/                  # 换模型时旧库，切成功后删除
    dirty.json                  # 待重试的 (project, path) 队列
  run/
    vane.sock                   # Unix socket，权限 0600
    vane.pid
  log/
    daemon.YYYY-MM-DD.log       # 按本地日历日切分，见 §9.6
```

`<root>/.vane.toml` 属于项目仓库，不放密钥。

`project_id` = 根路径 `realpath` 后 UTF-8 字节的 SHA-256 十六进制前 16 个字符。目录被移动视为新项目；旧 `projects/<id>/` 残留直到以后 `gc`。第一版不在 `.vane.toml` 里写稳定 UUID。

---

## 4. 配置

### 4.1 优先级

从高到低：

1. `<root>/.vane.toml`
2. `~/.vane/config/config.toml` 里对应 `[[projects]]` 的内联覆盖
3. 同文件中的 `[defaults]` 与顶层 `exclude` / `[[types]]`

密钥（API key）只允许出现在全局 config 或环境变量（`OPENAI_API_KEY`、`VANE_EMBED_API_KEY`）。项目文件出现 `api_key` 字段则启动报错。

### 4.2 合并规则（钉死）

| 项 | 项目未写 | 项目写了 |
|----|----------|----------|
| `exclude` | 用全局 | **并集**（全局 ∪ 项目）。项目不能「关掉」全局排除，避免漏掉 `node_modules` |
| `[[types]]` / `include` | 用全局类型表 | **整表替换**，不用并集。这样才能把某项目收窄到只有 `md` |
| `[embed]` | 用默认模型 | 字段级覆盖（可只改 `model`） |
| `[chunk]` | 用默认切片 | 字段级覆盖 |
| `[rerank]` | 用默认（第一版 `none`） | 字段级覆盖 |
| `[log]` | 用全局 | **不可项目覆盖**。日志是守护进程级，不是项目级 |
| `[gc]` | 用全局 | **不可项目覆盖**。CAS 保留天数是守护进程级 |

项目若同时写 `include = ["**/*.md"]` 和 `[[types]]`，以 `[[types]]` 为准。`include` 是糖：每条 glob 编译成 `extractor = "text"` 的一条 `[[types]]`。因为 types 是整表替换，项目一旦写 `include` / `[[types]]`，就不会再继承全局的 `image` 等类型；要在该项目索引图片，必须在项目表里再写一条 `extractor = "image"`。

### 4.3 全局配置形状

```toml
[defaults.embed]
provider = "ollama"                 # ollama | openai_compat
model = "nomic-embed-text"
base_url = "http://127.0.0.1:11434"
# api_key 仅 openai_compat；建议改用环境变量

[defaults.rerank]
provider = "none"

[defaults.chunk]
split = "markdown"                  # markdown | plain
max_chars = 1200
overlap_chars = 200
min_chars = 50

[log]
retain_days = 3                     # 按天切分后最多保留几天（含今天）；最小 1

[gc]
cas_retain_days = 365               # 未被任何工作集引用超过该天数的 CAS 条目删除；最小 1

exclude = [
  "**/.git/**",
  "**/node_modules/**",
  "**/target/**",
  "**/dist/**",
  "**/.venv/**",
  "**/*.log",
  "**/*.lock",
  "**/package-lock.json",
  "**/pnpm-lock.yaml",
  "**/*.min.js",
  "**/*.map",
  "**/.DS_Store",
  "**/.env",
  "**/.env.*",
]

[[types]]
glob = "**/*.{md,mdx,txt,rst,org,html}"
extractor = "text"

[[types]]
glob = "**/*.{png,jpg,jpeg,webp,gif}"
extractor = "image"
enabled = false

[[projects]]
path = "/absolute/or/tilde/path"
```

`[[projects]]` 可内联覆盖 `embed` / `chunk` / `exclude` / `types`，与 `.vane.toml` 同键。日常应把策略写在仓库的 `.vane.toml`，全局名单只登记 `path`。

### 4.4 项目文件形状

```toml
# <root>/.vane.toml
[embed]
provider = "openai_compat"
model = "text-embedding-3-small"
base_url = "https://api.openai.com/v1"

[chunk]
max_chars = 800

exclude = ["**/generated/**"]

include = ["**/*.{md,rst}"]
```

### 4.5 当前项目如何解析

`vane include` / `exclude` / `model` / `query` 若未传 `--global` 或 `--root`：从 cwd 向上找**已登记根目录**中路径最长的那个。cwd 不在任何根下则报错，提示 `vane add` 或 `--global`。

`vane add` 在登记时把 `~` 展开为绝对路径并 `realpath`。若新路径等于已有根、或落在已有根之内 / 把已有根包进去，拒绝登记（第一版不做嵌套根）。

写策略时：若根下已有或不准备提交配置，默认写入 `<root>/.vane.toml`；`--global` 改 `[defaults]` 或该项目的 `[[projects]]` 覆盖。

---

## 5. 文件分类与提取器

### 5.1 匹配顺序

对根内相对路径：

1. 命中合并后的 `exclude` → 跳过（不读、不哈希、不注册其子目录 watch）
2. 按 `[[types]]` **从上到下**第一条命中且 `enabled != false` → 用该提取器
3. 其余 → 跳过

不存在「能当 UTF-8 读就索引」。lock / log 必须靠 exclude 或未进 types 才能避开。

额外硬限制（不可配置，防误伤）：

- 跟踪符号链接但不走出根目录；环与逃逸一律跳过。
- 文本提取器跳过大于 8 MiB 的文件。
- 图片提取器跳过大于 20 MiB 的文件（仍可被 `read` 拒绝，见 MCP）。

### 5.2 第一版提取器

**`text`**

- 按 UTF-8 读取（非法 UTF-8 跳过并记日志）。
- 用项目生效的 `[chunk]` 切片（§6）。
- 产出 0..N 条规范文档。

**`image`**

- 不切片。一条文档：`text` = 文件名（无扩展名可带上相对路径），`modality = image`。
- 不跑视觉模型。

**预留（第一版不实现，配置里出现则警告并跳过）：**  
`pdf` / `docx` / `pptx`。它们必须产出与 `text` 相同的规范文档，写入**同一项目**的同一 Vane collection，以便和 md/图片一起检索。扫描件 OCR、CLIP 旁路库不在第一版。

### 5.3 规范文档

提取器只许产出：

```
text:            用于 BM25 与展示的字符串
vector:          由 embedding 提供者写入，提取阶段可空
modality:        "text" | "image"   （以后可加 "pdf" 等）
path:            根内相对路径，POSIX 分隔符
headings:        标题面包屑，可空
chunk_index:     从 0 连续
start_byte, end_byte
extractor:       名称
```

禁止把第二套向量空间（CLIP 等）写入该项目主 collection。

---

## 6. 切片策略

只对 `text`（及以后的办公格式在页/段切分之后）生效。按**字符**预算，不绑定某个 embedding 模型的 tokenizer（Ollama 通常没有同一套词表）。

### 6.1 参数

| 字段 | 默认 | 含义 |
|------|------|------|
| `split` | `markdown` | `markdown`：按 ATX/Setext 标题切开；`plain`：不认标题 |
| `max_chars` | 1200 | 单块上限（Unicode 标量计） |
| `overlap_chars` | 200 | 下一块前置重叠；须 `< max_chars` |
| `min_chars` | 50 | 小于此长度的块丢弃，除非全文只有一块 |

非法组合（`overlap >= max`、`min > max`）在加载配置时拒绝。

`chunk_strategy_id` = 对规范 JSON  
`{"split","max_chars","overlap_chars","min_chars","extractor_ver"}`  
做 SHA-256，取十六进制前 16 位。`extractor_ver` 第一版 `text` 为 `1`。

### 6.2 `markdown` 算法

1. 扫描标题，得到区间与面包屑（如 `API > 鉴权`）。无标题则全文一块，进入第 3 步。
2. 每个标题区间先作为候选。
3. 候选长度 `> max_chars`：依次按空行、换行、硬切拆开。
4. 相邻块：后块正文前加前块尾部 `overlap_chars`；**每块检索文本前都加上面包屑一行**（面包屑不计入 overlap 来源）。
5. 丢弃过短块（全文单块除外）。
6. `chunk_index` 按文件内最终块顺序编号。

`plain`：跳过 1–2，全文走第 3–6 步。`html` 第一版按 `plain` 处理（即使 `split = markdown` 也不解析 HTML 标题）。

改 `[chunk]` 会使该项目 `chunk_strategy_id` 变化 → 提取 CAS 失效 → 重切再 embed。

---

## 7. CAS 与工作集

### 7.1 两层缓存

**提取键**  
`sha256(file_bytes) + extractor + extractor_ver + chunk_strategy_id`

值：该文件全部规范文档（尚无向量）。

**向量键**  
`sha256(chunk.text 的 UTF-8) + embed_model_id`

`embed_model_id` = `provider + ":" + model + ":" + dim`（dim 为探测得到的维数）。  
值：`Vec<f32>`。

换模型、维数变了：提取可命中，向量全部未命中。  
换切片：提取未命中，随后向量也按新文本计算。

每条 CAS 对象旁存 `last_seen`（Unix 秒，本地对账触及工作集时刷新）。删除策略见 §7.5。

### 7.2 工作集 `live.json`

```json
{
  "files": {
    "docs/auth.md": {
      "content_sha256": "...",
      "extract_key": "...",
      "chunk_count": 3
    }
  }
}
```

路径为相对根的 POSIX 路径。原子写：`live.json.tmp` → `sync` → `rename`。

### 7.3 工作集同步到 Vane

Vane 文档 `id` = `{project_id}:{rel_path}#{chunk_index}`。

| 情况 | 动作 |
|------|------|
| 路径仍在且 `content_sha256`、`extract_key` 未变，且 `state.json` 的 `embed_model_id` 与当前库一致 | 空操作 |
| `state.json` 标明需要或正在按新 `embed_model_id` 重建 | 不适用空操作，走 §7.4 |
| 新路径或文件哈希变，提取 CAS 命中 | 不重切；按需重嵌；删旧 id；`add` 新块 |
| 提取未命中 | 提取 → 写 CAS → embed → `add` |
| 路径消失 | 按旧 `chunk_count` `delete` 所有 id；**不删 CAS** |

Git checkout 没有专用路径：就是一批上述事件。debounce 结束后做集合 diff，每个项目一次 `flush`。

文件重命名（同根内 path 变、`content_sha256` 不变）= 旧 path `delete` + 新 path `add`。提取与向量 CAS 均应命中，禁止因 rename 调用 embed。

无 `.git` 的目录同一套规则。未跟踪文件只要在盘上且通过分类，就会进工作集。

### 7.4 换模型

1. 向提供者发一条探测 embedding，得到 `dim`。与 `state.json` 中记录不同，或 `embed_model_id` 不同，即视为换模型。
2. 在 `projects/<id>/db.new/` 建**新** collection（新 vector dim）。旧 `db/` 继续服务查询。
3. 遍历当前 `live.json` 的路径：提取 CAS 按新 `chunk_strategy_id`（通常不变）复用；向量按新 `embed_model_id` 全量计算。
4. 新库 `flush` 成功后：`db/` → `db.prev/`，`db.new/` → `db/`，更新 `state.json`，再删 `db.prev/`。
5. 失败则保留旧库，`state.json` 标 `reindex_error`，查询不受影响。

`vane status` 显示该项目重建百分比。

禁止把旧维数向量写入新 collection。

### 7.5 CAS / 索引清理（gc）

**永不删除**登记根里的用户文档。只清理 `$VANE_HOME/rag`：提取 CAS、向量 CAS、`db.prev`、已 `rm` 项目留下的 `projects/<id>/`。

**引用：** 某 `extract_key` / `embed_key` 出现在**任意仍登记项目**的 `live.json` 中（embed 可由 live 的 extract 文档 + 该项目 `embed_model_id` 推出）。写入或保留工作集条目时，刷新对应键的 `last_seen = now`。

**自动（TTL）：** `[gc] cas_retain_days`（默认 **365**，最小 **1**，按整天计）。`last_seen` 距今超过该天数、且当前不被任何 live 引用的 CAS 对象删除。仍在工作集里的键每次对账都会刷新，不会因 TTL 掉。

时机：守护进程启动；每个本地日历日一次（与日志跨天 prune 一起）；`reload_config` 后。

**手动 `vane gc`：** 不等 TTL，立刻丢掉「当前无引用」的 CAS。

| 命令 | 范围 |
|------|------|
| `vane gc` | 当前项目（§4.5） |
| `vane gc --root <path>` | 指定项目 |
| `vane gc --all` | 全部项目 + 全局孤儿 CAS |

对指定项目还会：`compact` 该 Vane 库、删除 `db.prev/`。CAS 键若仍被**其他登记项目**的 live 引用，则**不删**（避免误伤共享内容）。

`vane rm` 之后：`projects/<id>/` 仍在，但已不登记 → 该项目不再计入「引用」。`vane gc --root` 对已移除路径：若 `state.json` 仍记录该 `root_path`，删除整个 `projects/<id>/`，再按全局 live 并集删 CAS 孤儿。`state.json` 必须含 `root_path` 以便 rm 后还能 gc。

提取对象删除时，级联删除其写出时记录的 `embed_keys`。写 extract CAS 时保存该列表。

失败：单条 CAS 删失败记日志，继续其余项；不回滚已删对象。

`.vane.toml` 写 `cas_retain_days` 忽略并 warn。

---

## 8. Vane collection 契约

每个项目当前模型对应**一个** collection，名称固定 `docs`。

Schema（创建时冻结 dim）：

| 字段 | 类型 | 用途 |
|------|------|------|
| `body` | text | BM25；切片正文（含面包屑前缀） |
| `embedding` | vector `{dim, metric: cosine}` | 语义 |
| `root` | scalar keyword | 根绝对路径，过滤 |
| `path` | scalar keyword | 相对路径 |
| `modality` | scalar keyword | `text` / `image` |
| `extractor` | scalar keyword | |
| `chunk_index` | scalar int | |
| `start_byte` | scalar int | |
| `end_byte` | scalar int | |

分词器：`jieba`；词典不可用时该项目回退 `cjk_bigram`，并在 `status` 说明。全库同一分词身份，遵守 core 的 reindex 语义；侧车第一版不提供改分词入口。

`auto_commit`：索引批次里关闭，由 writer 在每个 debounce 窗口末尾显式 `flush`。查询进程读已 flush 快照。

**Compact：** 单次窗口 `delete` 次数 ≥ 1000，或 tombstone 文档数 / 活文档数 ≥ 0.2，则在 flush 之后调用 `compact()`。切分支造成的堆积必须靠这条消化。

---

## 9. 守护进程

### 9.1 生命周期

- `vane init` 第 4 步默认建议安装用户服务（launchd plist 或 systemd --user unit），开机拉起 `vane daemon`。
- `vane start` / `vane stop` 手动起停。
- `vane service uninstall` 移除用户服务定义并 `stop`；不删配置与 `rag/` 数据。
- 未运行时，CLI 与 `vane mcp` 报错并提示启动，不在 MCP 进程内嵌索引引擎。
- **单实例：** `run/vane.pid` 上 `flock`（或等价独占锁）。第二个 `vane daemon` 立即以非 0 退出并指出已有 pid，不得再听 socket。

### 9.2 线程（无异步运行时）

| 线程 | 职责 |
|------|------|
| 主线程 | `UnixListener` 接受 CLI/MCP；只做查询与状态读 |
| watch 线程 | `notify`，多根目录；debounce 500ms，最长等待 2s 后出批 |
| writer 线程 | 唯一允许调用 `add` / `delete` / `flush` / `compact` 与写 CAS / `live.json` |
| embed 辅助 | writer 内同步调用 HTTP；本地模型并发 1，`openai_compat` 并发上限 4（同一时刻仍只从一个 writer 逻辑批次发出） |

Vane 单写者：禁止从请求线程 `add`。检索可并发读。

### 9.3 Watch 与 exclude

注册递归 watch 时**不得**下降到 exclude 命中的目录（`node_modules`、`.git`、`target` 等）。事件到达后再过滤一次作为兜底。

启动与配置热加载：对每个根做一次全量对账（walk − exclude），再进入监听。服务停机丢事件靠这次对账补。

不跟踪根外文件。网络盘不额外保证，对账仍执行。

`.vane.toml` 变更：按配置热加载处理，必要时触发该项目策略失效（换模型 / 换切片走 §7.4 或提取失效逻辑），不是普通文件内容更新。

### 9.4 Embedding 提供者

统一 trait：`embed(texts: &[String]) -> Result<Vec<Vec<f32>>>`，外加 `probe_dim()`。

- `ollama`：`POST {base_url}/api/embeddings`
- `openai_compat`：`POST {base_url}/v1/embeddings`（或配置的 path）

维数以第一次成功探测为准，写入 `state.json`。之后若返回维数不一致，该批失败并标脏，不入库。

查询时若 embed 失败：该项目改走 `SearchMode::Text`（BM25），命中带 `degraded: true`。已在盘上的向量索引不删。

失败的文件路径写入 `rag/dirty.json`，由 writer 重试：初始等待 1s，指数退避至上限 60s，成功则清该项。`openai_compat` 单次 HTTP 最多 64 条文本；`ollama` 按条发送（多数发行版不支持真批量）。

Rerank 第一版只有 `provider = "none"`。配置了其他值则警告并忽略。

### 9.5 本机 IPC

CLI 与 `vane mcp` 通过 `run/vane.sock` 说话。协议：**一行一条 JSON 对象**（换行分隔），UTF-8。请求至少含 `id`（字符串）与 `method`；响应用同一 `id`，成功带 `result`，失败带 `error: { code, message }`。

第一版 method：`status`、`search`、`read`、`list_roots`、`reload_config`、`add_root`、`remove_root`、`gc`。检索类参数与 §11 对齐。`gc` 参数：`root`（可选）或 `all`（bool）。

认证 = socket 文件 `0600` + 同 uid。无 token、无 ACL：能连上就能 `search` / `read` 所有已索引文件。这是本机单用户产品，不在第一版做权限隔离。

### 9.6 日志滚动

守护进程日志只写 `$VANE_HOME/log/`，不写被索引的项目目录。

| 项 | 规则 |
|----|------|
| 文件名 | `daemon.YYYY-MM-DD.log`，日期为**本机本地日历日**（不是 UTC） |
| 切分 | 按天。一次写入前若已打开的文件日期 ≠ 今天，关掉旧文件、打开今天的文件，然后 prune |
| 保留 | `[log] retain_days`（默认 **3**，最小 **1**，加载时 `< 1` 拒绝）。保留「今天」起往前共 `retain_days` 个本地日，更早的 `daemon.YYYY-MM-DD.log` 删除 |
| 例 | `retain_days = 3` 且今天是 2026-08-19 → 保留 19/18/17，删除 16 及更早 |
| prune 时机 | 启动时一次；跨日后第一次写入时一次；`reload_config` 后一次 |
| 遗留名 | 若存在旧的单文件 `daemon.log`（无日期），prune 时删除 |
| 格式 | 一行一条：`YYYY-MM-DDTHH:MM:SS±HHMM LEVEL message`（本地时间）。LEVEL = `INFO` / `WARN` / `ERROR` |
| 失败 | 写日志失败不得让索引/检索崩掉；向 stderr 打一条后继续 |
| 依赖 | 不用 tracing/tokio；`Mutex<File>` + 标准库写 |

`[log]` 只出现在全局 `config.toml`。`.vane.toml` 写 `retain_days` 忽略并 warn。

密钥、API key、文档全文不要写入日志。路径、project_id、错误码可以写。

---

## 10. CLI

| 命令 | 行为 |
|------|------|
| `vane init` | 向导，写全局 config，可选装服务并 `start` |
| `vane add <path>` | 登记根目录（规范化为绝对路径），通知守护进程 watch + 对账 |
| `vane rm <path>` | 从名单移除，停 watch；**保留**该 `projects/<id>/` 与 CAS，直到 `vane gc` |
| `vane gc` | 见 §7.5：当前项目立刻清理无引用 CAS；`--root` / `--all`；不删用户源文件 |
| `vane include add <glob>` / `vane include reset` | 改当前项目（写入 `.vane.toml`）或 `--global` 类型表 |
| `vane exclude add <glob>` / `vane exclude reset` | `reset --global` 恢复默认排除列表；项目级 `reset` 只清空项目追加项 |
| `vane model` | 改 embed；当前项目或 `--global`；触发 §7.4 |
| `vane start` / `vane stop` / `vane daemon` | 前台守护由服务管理器调用 |
| `vane service uninstall` | 卸载 launchd/systemd 用户服务并停止守护进程；保留数据 |
| `vane status` | 各项目文件数、队列、模型、dim、重建进度、分词回退 |
| `vane query <q>` | 默认**当前项目**（§4.5）；`--all` 才跨项目 RRF；`--root` 指定项目 |
| `vane mcp` | stdio MCP，连 `run/vane.sock` |

### 10.1 `init` 四步

1. Embedding：Ollama 或 OpenAI 兼容 → 探测连通，记录 provider / model / base_url / dim。
2. 第一个根目录，可跳过。
3. 排除：预勾选 §4.3 默认列表，可取消单项、可再加一个文件夹（写入全局 `exclude`）。
4. 是否启用图片类型（改全局 `image` 的 `enabled`，默认否）+ 是否安装用户服务（默认建议是）。

不在向导里问后缀细节、rerank、caption、切片数字。

未 `init`：所有子命令（除 `init` 自身与 `vane daemon --help`）拒绝执行并指出缺少 `$VANE_HOME/config/config.toml`。

全局可选参数：`--home <dir>`，所有子命令可用。

---

## 11. MCP 与 skill

`vane mcp`：JSON-RPC 2.0 stdio，代理到 Unix socket。不承载 Vane 句柄。

三个工具：

### `search`

参数：

- `query`（必填）
- `root`（可选，绝对路径或已登记 path；缺省 = 全部项目）
- `type`（可选，精确匹配 `extractor` 名：`text` / `image`，不是文件后缀）
- `top_k`（默认 8，上限 50）

行为：

- 单项目：用该项目模型 embed `query`，Vane `hybrid`；embed 失败则 BM25。
- 多项目：按**互异** `embed_model_id` embed 查询（同一模型只 embed 一次），各库检索，**RRF（k=60）按文档 id 融合**。不同空间的原始向量分不得直接比。
- 过滤用 Vane pre-filter（`root` / `modality` / `extractor`），不用搜完再丢。

返回：`id, path, root, title（面包屑或文件名）, snippet, score, modality, extractor, degraded`。

`snippet`：从规范文档 `text` 去掉开头面包屑行之后，取前 240 个 Unicode 标量。不做搜索高亮。重叠切片导致的近重复命中不去重。

无命中或空库：返回 `[]`，不是错误。

### `read`

参数：`id` 或 `path`（+ 可选 `root`）。规范文档（含 `headings`）从**提取 CAS** 读取，不从 snippet 反推，也不依赖 Vane stored 里另存一份标题。

- 只给 `id`：返回那一块。
- 只给 `path`（多 chunk）：按 `chunk_index` 升序返回该文件**全部块**，不要只回第一块。
- 文本块字段：原文、面包屑、字节范围、绝对路径。
- 图片：文件 ≤ 4 MiB 返回 MCP 图像内容；更大只返回绝对路径、MIME、尺寸提示，不塞 base64。

### MCP 客户端配置（开箱）

Claude / Cursor 使用同一条命令（把 `vane` 换成实际 PATH）：

```json
{
  "mcpServers": {
    "vane": {
      "command": "vane",
      "args": ["mcp"]
    }
  }
}
```

守护进程必须事先在跑。`vane mcp` 只做 stdio 桥。

### `list_roots`

返回已登记根、`project_id`、生效 model / dim、活文件数、上次对账时间、是否在重建。

Skill：路径固定为 `crates/vane/SKILL.md`，约定先 `list_roots` 再 `search` / `read`，不要自行扫盘。Skill 不是第二种检索协议。

---

## 12. 错误与降级

| 情况 | 行为 |
|------|------|
| 守护进程未运行 | CLI/MCP 退出或工具错误，提示 `vane start` / 检查用户服务 |
| 配置损坏或项目文件含密钥 | 拒绝启动或拒绝加载该项目，指出路径，不覆盖用户文件 |
| 单文件 IO / 非 UTF-8 / 超限 | 跳过，写当天 `daemon.YYYY-MM-DD.log`，不堵队列 |
| Embedding 不可达 | 查询 BM25 + `degraded`；写入标脏 |
| 探测维数变化（非用户换模型） | 批次失败，不写入错误维数 |
| `read` 图片过大 | 路径 + 元数据，不回二进制 |
| 崩溃 | `live.json` 与 Vane manifest 均原子 rename；下次启动全量对账 |
| socket 权限 | 仅创建用户可读写的 `0600` |

---

## 13. 扩展：PDF / Word / PPT

第一版不实现解析，但下列不变量以后不得破坏：

- 新格式 = 一条 `[[types]]` + 一个提取器，产出 §5.3 文档。
- 写入**该项目当前模型**的同一个 collection，与 md 一次 `search` 混排。
- 禁止为主检索引入第二套向量空间。CLIP / 以图搜图若做，必须是旁路 collection，默认 `search` 不混进该空间。
- 办公提取器版本进入 `chunk_strategy_id` / 提取键，换解析器只重算该类型。
- 依赖留在 `crates/vane` 的 feature 或可选模块，不进 `vane-core`。

---

## 14. 测试

原则与仓库 Rust 测试规则一致：测可观察契约，用隔离临时目录，不依赖本机 Ollama 或已有 `~/.vane`。

至少覆盖：

- 配置合并：exclude 并集、types 替换、密钥出现在项目文件则失败。
- 提取 CAS 命中不重切；仅换 `embed_model_id` 不重切、要重嵌。
- 换 `chunk` 参数后提取失效。
- 工作集：删文件 → Vane 中无该 path；同内容换分支回来不调用 embed fixture。
- exclude 目录不出现在「将注册的 watch 路径」清单（单测 mock 或记录用 API）。
- 模型 dim 变更写入新 db，查询在切换完成前仍打旧 db。
- embed 失败时 search 走文本模式且带 `degraded`。
- 切片：长 markdown 标题层级、短文件单块、overlap 不丢标题前缀。
- `VANE_HOME` 隔离：测试不写 `~/.vane`。
- 第二个 daemon 抢锁失败退出。
- `read` 只给 path 时返回该文件全部 chunk。
- rename 不调用 embed。
- `vane query` 默认单项目；`--all` 才融合。
- 按天切分日志：`retain_days=3` 时只留下最近 3 个本地日的 `daemon.YYYY-MM-DD.log`。
- `vane gc --root` 立刻删该项目无引用且不被其他项目 live 占用的 CAS；不删源文件。
- TTL：`last_seen` 超过 `cas_retain_days`（默认 365）且无 live 引用的 CAS 被自动删。

HTTP 提供者全程用本地 fake server 或 trait mock。

---

## 15. 关键决策

1. **侧车不是 core。** 检索合同不变；产品体积与模型生命周期留在 `crates/vane`。
2. **一个守护进程，名单全局，策略可项目化。** 解决「不要 N 个常驻服务」和「include/model 要按仓库差异化」。
3. **每项目一个 Vane 库，按模型冻 dim。** 跨项目用 RRF，绝不混 HNSW。
4. **CAS 拆提取 / 向量。** 换模型不必重切；换切片才重切。
5. **Vane 只含工作集。** 历史向量留在 CAS，避免 HNSW 被已删分支污染。
6. **分类是规则表，不是写死的三类。** PDF/Office 以后只加提取器。
7. **图片 v1 = 元数据 + `read` 原图。** 多模态在 Agent，不在守护进程。
8. **同步线程，不用 tokio。** 符合 deny 名单，也符合 Vane 同步 API。
9. **Watch 在注册期应用 exclude。** 否则大仓库不可用。
10. **切片按字符 + markdown 结构。** 不绑模型 tokenizer，策略进入提取键。
11. **家目录统一 `~/.vane/`。** 命令就叫 `vane`。
12. **CAS 可回收。** 手动 `vane gc` 清当前无引用；TTL 默认 365 天清长期未引用。源文件不动。

---

## 16. 实现分期（供后续计划拆分，不是另起产品）

1. crate 骨架、`~/.vane`、`init` / 配置合并、`daemon` socket。  
2. watcher（exclude 下钻）+ 工作集对账 + `text` 切片 + 假 embedding。  
3. 两层 CAS + 真实 Ollama / OpenAI 兼容 + 按项目 Vane 库 + compact。  
4. `search`/`query`、BM25 降级、换模型重建。  
5. MCP 三工具 + 图片提取器 + 用户服务安装。  

---

## 17. 已关闭的歧义

- 项目 exclude **并集**，types/include **替换**。  
- 当前项目 = cwd 向上匹配的已登记根，不是任意含 `.vane.toml` 的目录。  
- 未 `vane add` 的目录即使有 `.vane.toml` 也不被监听。  
- `vane rm` 不删 CAS / 项目目录。  
- 跨项目原始向量分不可比，只许 RRF。  
- `html` 第一版当 `plain` 切。  
- 第一版不分发 Windows 服务。
- 家目录：`--home` > `VANE_HOME` > `~/.vane`。
- 单实例 flock；IPC 为 Unix socket 上的 JSON 行，无 ACL。
- `read(path)` 回该文件全部 chunk；`snippet` = 去面包屑后 240 字。
- `vane query` 默认当前项目；MCP `search` 默认全部项目。
- rename = delete + add，CAS 命中。
- embed：dirty 指数退避（1s…60s）；OpenAI 兼容批量 ≤64。
- `vane service uninstall` 只卸服务，不删数据。
- 日志按本地日切分，`[log] retain_days` 默认 3，不可项目覆盖。
- `[gc] cas_retain_days` 默认 365；手动 `vane gc` 立刻清无引用 CAS；永不删用户源文件。
