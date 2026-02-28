# Vane

![License](https://img.shields.io/badge/license-Apache--2.0-blue)
![CI](https://github.com/ximing/vane/actions/workflows/ci.yml/badge.svg)
![Rust](https://img.shields.io/badge/Rust-stable-orange)
[![Docs](https://img.shields.io/badge/docs-ximing.github.io%2Fvane-3b9eff)](https://ximing.github.io/vane/)

[English](README.md) | **中文**

Vane 是一个轻量级 **向量 + BM25 混合检索库**，以单一 Rust 核心嵌入桌面、Node.js、Go 与浏览器。
它把分段 HNSW 向量检索与 Block-Max WAND BM25 配对，再用 RRF 融合——sqlite-vec 的嵌入式形态、
Tantivy 级别的文本检索、一体化混合排序，收在一个库里。

- **一个核心，四处可嵌。** 同一份 Rust 引擎驱动 Node（napi-rs）、Go（cgo 静态库）和浏览器
  （wasm-bindgen + Web Worker）。绑定层是薄壳——不在各语言间复制逻辑。
- **默认混合。** `mode: "hybrid"` 并行跑向量 + BM25，用 RRF（`k = 60`）融合，零调参。
  召回 recall@10 ≥ 0.95 是 CI 硬门禁。
- **中文一等公民。** 内置 `jieba` 分词器（前缀 DAG + HMM，精简词典 ~20 万词），与 `standard`、
  `cjk_bigram` 并列。中英混排按脚本边界正确切分；用 `userDict` 注入领域词条。
- **嵌入式、耐用。** 目录多段文件 + `manifest` 原子切换，薄 WAL 崩溃恢复，`export()` 单文件快照。
  无服务端、无 GPU、无 mmap。
- **不内置 embedding。** Vane 只负责存储、索引、融合你给的向量。OpenAI / ollama / transformers.js
  几行即可接入（见 `examples/`）。

> Vane 不生成 embedding、不跑模型、不说 SQL、不做分布式。它是一个检索库——快、可嵌、可预期。

---

## 目录

- [Vane 是什么](#vane-是什么)
- [功能](#功能)
- [安装](#安装)
  - [Node.js](#nodejs) · [Go](#go) · [浏览器](#浏览器) · [从源码构建](#从源码构建)
- [本机侧车 CLI](#本机侧车-cli)
  - [安装](#安装-cli) · [第一次跑](#第一次跑) · [诊断与维护](#诊断与维护) · [MCP](#mcp) · [Agent skills](#agent-skills)
- [快速开始](#快速开始)
  - [Node.js](#快速开始nodejs) · [Go](#快速开始go) · [浏览器](#快速开始浏览器)
- [API 参考](#api-参考)
  - [Schema 与文档](#schema-与文档) · [分词器](#分词器) · [检索模式与融合](#检索模式与融合) · [自定义词表与 reindex](#自定义词表与-reindex) · [过滤](#过滤)
- [架构](#架构)
- [性能](#性能)
- [状态](#状态)
- [示例](#示例)
- [贡献](#贡献)
- [协议](#协议)

---

## Vane 是什么

Vane 是一个你**嵌进自己进程**的混合检索库。你把文档交给它——每条含一个文本字段、一个向量、可选的
标量元数据——它在同一份数据上同时构建倒排索引（BM25）和向量索引（HNSW）。查询时你可以用文本、向量、
或两者一起，Vane 返回两路信号融合排序后的命中结果。

它存在的理由是：现有的常见方案各自都让出了一些东西：

| 你想要… | 典型方案 | 你付出的代价 |
|---|---|---|
| 进程内的 向量+文本 检索 | sqlite-vec **+** FTS5 + 手写融合胶水 | 原子混合排序、统一 filter 模型、约 200 行管道代码 |
| 浏览器端语义搜索 | 纯 JS 引擎 | 性能天花板、Node/Go 无法复用的 Rust 核心 |
| 中文友好的分词 | Tantivy + 分词 crate | 浏览器构建，或客户端再上一套引擎 |

Vane 的赌注是：**一份保持无 mmap、平台洁净的 Rust 核心**，能从同一条代码路径同时服务桌面（Node）、
服务端（Go）和浏览器（WASM）——并且 BM25 与向量检索从一开始就为融合而设计，而非事后拼凑。

典型场景：AI Agent 的本地记忆库、边缘/端侧 RAG 检索层、浏览器内对笔记/PKM 库的隐私语义搜索。

## 功能

- **Collection 管理** —— 建/列/删；文档 = `id` + `text` + `vector` + JSON 元数据。
- **向量索引** —— 分段 HNSW，过滤后候选集过小时自动回退暴力精确扫描。度量：`cosine` / `l2` / `dot`，维数上限 4096。
- **BM25** —— Block-Max WAND top-k，128 文档跳块的 posting 列表；`k1=1.2`、`b=0.75`。每集合可多文本字段。
- **混合融合** —— 默认 RRF（`k=60`，零校准）；可选 `{ linear: { alpha } }` + min-max 归一化。
- **分词器** —— `standard`（unicode + 小写 + Porter 词干）、`cjk_bigram`（零词典 CJK 兜底）、`jieba`（中文精确分词）。
- **自定义词表** —— 建库时或运行期注入领域词条，`reindex()` 原子生效。用户词优先级永远高于内置词典。
- **元数据过滤** —— 标量字段 `eq` / `in` / `gte` / `lte`，字段间 AND，作为 pre-filter 推进 HNSW 遍历与 WAND 扫描
  （不是 post-filter）。*当前绑定覆盖见 [过滤](#过滤)。*
- **持久化** —— 目录多段 + `manifest.json` 经 `rename` 原子切换；薄 WAL 崩溃恢复；`export()` 单文件快照。
- **删除与合并** —— tombstone 位图；`compact()` 物理回收空间。
- **可见性** —— `flush()` 是新读快照原子可见的边界；auto-commit 默认开启（1s 或 1000 条触发）。
- **并发** —— 单写者 + 无锁并发读；所有公开 API 线程/goroutine 安全。

## 安装

### Node.js

```bash
npm install @vane-rs/node
```

预编译原生产物通过 `optionalDependencies` 自动按平台选择：

| 平台 | npm 子包 |
|---|---|
| Linux x64（glibc） | `@vane-rs/node-linux-x64-gnu` |
| macOS arm64 | `@vane-rs/node-darwin-arm64` |
| macOS x64 | `@vane-rs/node-darwin-x64` |
| Windows x64（MSVC） | `@vane-rs/node-win32-x64-msvc` |

这些平台无需从源码编译。`jieba` 词典随包捆绑，open 时自动加载——中文搜索开箱即用。

### Go

Go 绑定通过 cgo 链接按平台预编译的静态库（`libvane_ffi.a`）。

```bash
# 1. 构建静态库（或从 GitHub Releases 下载 libvane_ffi-<lib_dir>.a）
cargo build --release -p vane-ffi

# 2. 放到 cgo 约定的位置（bindings/go/lib/ 下的 os-arch 子目录）
mkdir -p bindings/go/lib/$(go env GOOS)-$(go env GOARCH)
cp target/release/libvane_ffi.a bindings/go/lib/$(go env GOOS)-$(go env GOARCH)/

# 3. 加入你的 module
go get github.com/ximing/vane/bindings/go
```

预编译静态库覆盖 `linux-amd64`、`linux-arm64`、`darwin-amd64`、`darwin-arm64`。`jieba` 词典内嵌在
`bindings/go/dict` 包中——`Open` 后调用一次 `db.LoadDict(dict.DictBytes())` 即可。`vane_nodict` 构建标签
可裁掉内嵌词典（退化为 `cjk_bigram`）。

> cgo 路径不支持 `CGO_ENABLED=0`；纯 Go（更慢）的变体请用 `wazero` 构建标签。

### 浏览器

```bash
# 需要 wasm32-unknown-unknown target + wasm-opt（binaryen）
bash scripts/build-wasm-variants.sh
# → target/wasm-variants/vane_wasm_simd.wasm   (~312 KB gzip)
#   target/wasm-variants/vane_wasm_scalar.wasm (~314 KB gzip)
```

产出两个变体（SIMD128 + scalar），运行时用 `WebAssembly.validate` 探针选其一。词典**永不**打进 wasm
（体积红线：核心 ≤ 800 KB gzip）——从 CDN 拉取、sha256 校验、OPFS 缓存，离线时自动降级 `cjk_bigram`
不抛错。

### 从源码构建

```bash
# 构建全部
cargo build --release --workspace

# 只构建侧车 CLI
cargo build --release -p vane

# CI 使用的完整测试 + 质量门禁
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace --all-features

# WASM 基线（core 必须不含 std::fs / mmap）
cargo check --target wasm32-unknown-unknown -p vane-core
```

## 本机侧车 CLI

检索库本身仍然不生成 embedding。可选的 **`vane` CLI**（`crates/vane`）是叠在上面的本机侧车：
一个守护进程盯你登记的文件夹，做 markdown/纯文本切片（可选图片元数据），调用 Ollama 或
OpenAI 兼容 embedding 接口，人用 `vane query` 检索，Agent 走 MCP。

第一版只支持 macOS 与 Linux（Unix socket + launchd / systemd --user），不做 Windows 服务。

### 安装 CLI

`v*` GitHub Release 提供 Linux x86_64、macOS arm64、macOS x86_64 预编译：

```bash
curl -fsSL https://raw.githubusercontent.com/ximing/vane/main/scripts/install-vane-cli.sh | sh
# 安装到 ~/.local/bin/vane（PREFIX=/usr/local 可改）
export PATH="$HOME/.local/bin:$PATH"
vane --version
```

从源码（其他架构，或还没有对应 tarball 时）：

```bash
cargo install --path crates/vane --locked --force
# 或不克隆仓库：
cargo install --git https://github.com/ximing/vane.git --locked --bin vane
```

默认 embedding 是 Ollama（`nomic-embed-text`）。先拉一次模型，或在 `vane init` 里选
`openai_compat`：

```bash
ollama pull nomic-embed-text
```

### 第一次跑

```bash
vane init                 # embedding、API key、向量维度、第一个目录、用户服务
vane add ~/notes          # 再登记一个根
vane start                # 若没装用户服务
vane status               # 终端仪表盘（管道输出为 JSON）
vane query "鉴权怎么做"
vane query "发版" --all
```

家目录：`--home` > `VANE_HOME` > `~/.vane`。项目策略写在 `<root>/.vane.toml`（禁止放
`api_key`）。相同文件字节在分支切换时复用提取/向量缓存。`vane gc` 永不删除用户源文件。

`vane init` 会探测 embedder，**探测失败则中止**。终端里可以确认「仍继续」。脚本里设
`VANE_ALLOW_EMBED_FAIL=1` 才会在探测失败时仍写出配置——在 provider 恢复前，检索只走
BM25（命中带 `degraded`）。

### 诊断与维护

终端里 `vane status` 是一块仪表盘：守护进程是否在跑、脏队列、磁盘，以及每个根的
live 文件数、模型和跳过数。管道输出是 JSON。

检索为空或看起来过期时，按这个顺序看：

```bash
vane doctor                 # 配置、socket、守护进程、embedder、根目录、磁盘
vane status
vane query "鉴权"            # 空结果会打印 why（退出码 0）
vane issues                 # 当前根里被跳过的文件
vane issues --all
vane logs                   # 最近 50 行已脱敏的守护进程日志
vane logs --follow --lines 200
vane inspect                # 解析后的 embed / chunk / exclude / types
vane inspect --global
vane inspect --root ~/notes
vane df                     # $VANE_HOME、CAS、各项目 db
vane gc --dry-run           # 只统计未引用缓存，不删除
vane gc --all --dry-run
```

空的 `vane query` 仍会成功退出。CLI 会说明 **原因**（命中第一条即停）：尚未初始化、
当前目录不是已登记根、仍在建索引、embedder 不可用、查询像被排除的路径、查错了根
（试 `--all` / `--root`）、索引为空、或没有匹配的 chunk。接着跑 `vane doctor` /
`vane issues` / `vane logs`。

改 embedding 模型会重嵌 live 文件。终端会确认；非交互必须加 `--yes`（`-y`）：

```bash
vane model --model nomic-embed-text --yes
```

### MCP

把 stdio MCP 桥交给 Claude Code / Cursor / 其他 MCP 客户端。守护进程必须已经在跑
（`vane mcp` 不会替你启动）：

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

也可以把 `mcpServers.vane` 合并进 `$HOME` 下 Claude / Cursor / Codex 的配置
（默认：所有已知客户端）：

```bash
vane mcp install --dry-run              # 只打印将要写入的内容
vane mcp install
vane mcp install --client claude        # claude | cursor | codex
```

Agent 应先 `list_roots`，再 `search` / `read`，不要自己扫盘。

### Agent skills

[`skills/vane`](skills/vane) 是一份给多个编程 Agent 用的 skill：CLI 不在就先装/启动，
然后走 MCP（`list_roots` → `search` → `read`），不要扫文件系统。

一次性装进本机常见 Agent 目录：

```bash
curl -fsSL https://raw.githubusercontent.com/ximing/vane/main/scripts/install-vane-skill.sh | sh
```

或手动把 [`skills/vane`](skills/vane) 拷到：

| 运行时 | Skill 目录 |
|---|---|
| Claude Code | `~/.claude/skills/vane/` |
| Codex / Grok | `~/.agents/skills/vane/` |
| Cursor | `~/.cursor/skills/vane/` |
| Grok Build CLI | `~/.grok/skills/vane/` |

Claude Code（本仓库就是插件市场）：

```text
/plugin marketplace add ximing/vane
/plugin install vane@vane
```

Codex：

```bash
codex plugin marketplace add ximing/vane
codex plugin add vane@vane
```

Cursor：把 `skills/vane` 拷到 `~/.cursor/skills/` 或项目 `.cursor/skills/`。
Kimi Code：`/plugins install https://github.com/ximing/vane`，然后 `/new`。

完整说明：[本机侧车 CLI](https://ximing.github.io/vane/guides/sidecar)。

## 快速开始

下方示例用 4 维伪向量以便直接跑通。生产环境请把 `vector` 换成模型产出的真实 embedding——Vane 只索引、
检索你给它的向量。

### 快速开始：Node.js

```js
import vane from '@vane-rs/node';
const { open } = vane;

// 打开一个数据库目录（不存在则创建）。autoCommit: 'off' 表示
// 我们用 flush() 自己控制可见性边界。
const db = await open('./mydb', { autoCommit: 'off' });

// 声明 schema：一个 text 字段 + 一个 vector 字段。（每集合恰好一个 vector 字段。）
const col = await db.collection('docs', {
  fields: [
    { name: 'body', type: 'text' },
    { name: 'vec',  type: 'vector', dim: 4, metric: 'cosine' },
  ],
}, { tokenizer: 'standard' });

// 按 id 批量幂等 upsert。返回 { accepted, visibleAfterFlush }。
await col.add([
  { id: 'a', text: 'hello world',  vector: [1.0, 0.0, 0.0, 0.0] },
  { id: 'b', text: 'foo bar baz',  vector: [0.0, 1.0, 0.0, 0.0] },
  { id: 'c', text: 'hello foo',    vector: [0.7, 0.3, 0.0, 0.0] },
]);
await col.flush();                       // 此刻起数据可搜

// 混合搜索：BM25(text) + 向量相似度，RRF 融合。
const hits = await col.search({
  text: 'hello',
  vector: [1.0, 0.0, 0.0, 0.0],
  topK: 3,
  mode: 'hybrid',                        // 'vector' | 'text' | 'hybrid'
  fusion: 'rrf',                         // 默认；或 { linear: { alpha: 0.5 } }
});
// hits = [{ id, score, fields }, ...]

await db.close();
```

### 快速开始：Go

```go
package main

import (
	"fmt"
	"log"

	"github.com/ximing/vane/bindings/go"
	"github.com/ximing/vane/bindings/go/dict"
)

func main() {
	db, err := vane.Open("./mydb", nil) // nil opts = 默认值
	if err != nil { log.Fatalf("Open: %v", err) }
	defer db.Close()

	// 加载内嵌的 jieba 词典（dict 包内嵌）。
	if b, err := dict.DictBytes(); err == nil {
		_ = db.LoadDict(b) // 失败时 jieba 降级为 standard——collection 创建不会失败
	}

	schema := vane.Schema{Fields: []vane.SchemaField{
		{Name: "body", Type: "text"},
		{Name: "vec",  Type: "vector", Dim: 4, Metric: "cosine"},
	}}
	col, err := db.Collection("docs", schema, &vane.CollectionOptions{Tokenizer: "jieba"})
	if err != nil { log.Fatalf("Collection: %v", err) }
	defer col.Close()

	_ = col.Add([]vane.Doc{
		{ID: "a", Text: "hello world", Vector: []float32{1.0, 0.0, 0.0, 0.0}},
		{ID: "b", Text: "foo bar baz", Vector: []float32{0.0, 1.0, 0.0, 0.0}},
		{ID: "c", Text: "hello foo",   Vector: []float32{0.7, 0.3, 0.0, 0.0}},
	})
	_ = col.Flush()

	hits, _ := col.Search(vane.SearchQuery{
		Text: "hello", Vector: []float32{1.0, 0.0, 0.0, 0.0}, TopK: 3,
	})
	for _, h := range hits {
		fmt.Printf("hit: id=%s score=%.4f\n", h.ID, h.Score)
	}
}
```

### 快速开始：浏览器

浏览器侧是一个 Web Worker：用 `postMessage` 的 Promise 边界包裹 wasm 引擎，OPFS 持久化
（IndexedDB 降级），CDN 拉取词典。端到端范例是一个纯前端的 Markdown 搜索应用：

```bash
bash demo/build.sh                 # 构建 wasm 双变体 + JS 胶水 + dict.bin
cd demo && python3 -m http.server 8765
# 打开 http://localhost:8765/ —— 拖入含 .md 的文件夹即可搜索
```

完整的 Worker 协议、持久化与 SIMD/CDN/离线降级说明见 [`demo/README.md`](demo/README.md)。

## API 参考

六个动词（+ 四个管理函数）在三侧绑定中一致出现；仅命名风格与错误形式不同
（JS `Promise`/`VaneError`；Go `(T, error)`）。

| 操作 | Node.js | Go |
|---|---|---|
| 打开数据库 | `open(path, opts)` → `VaneDb` | `vane.Open(path, *OpenOptions)` |
| 创建/打开 collection | `db.collection(name, schema, opts)` | `db.Collection(name, Schema, *CollectionOptions)` |
| 列出 collection | `db.collections()` | —（尚未暴露） |
| 写入文档（批量 upsert） | `col.add(docs)` → `{accepted}` | `col.Add([]Doc)` |
| 使写入可见 | `col.flush()` | `col.Flush()` |
| 检索 | `col.search(query)` → `Hit[]` | `col.Search(SearchQuery)` |
| 按 id 删除 | `col.delete(ids)` | `col.Delete([]string)` |
| 触发段合并 | `col.compact()` | `col.Compact()` |
| 用新分词器/词表重建 | `col.reindex()` → handle | `col.Reindex()` |
| 单文件快照导出 | `db.export(dest)` | `db.Export(dest)` |
| 关闭 | `db.close()` | `db.Close()` |

### Schema 与文档

schema 是具名字段列表。每集合恰好一个 `vector` 字段；`text` 字段进 BM25；`scalar` 字段可过滤。

```js
{
  fields: [
    { name: 'title', type: 'text' },
    { name: 'body',  type: 'text' },
    { name: 'vec',   type: 'vector', dim: 384, metric: 'cosine' }, // metric: cosine|l2|dot
    { name: 'lang',  type: 'scalar', kind: 'keyword' },           // kind: int|float|bool|keyword
  ],
}
```

文档是 `{ id, text, vector, meta }`，`meta` 是按字段名键控的标量值。`id` 是外部字符串主键
（≤ 512 字节）；`add` 是按 `id` 的幂等 upsert。向量维度必须等于 schema 的 `dim`，否则报 `E_SCHEMA`。

### 分词器

在 `CollectionOptions.tokenizer` 上按集合配置：

| 分词器 | 做什么 | 需要词典？ |
|---|---|---|
| `standard` | unicode 切词 → 小写 → Porter 词干（Latin/digit run） | 否 |
| `cjk_bigram` | CJK run 切二元组；非 CJK run 走 `standard` 管线 | 否 |
| `jieba` | 前缀 DAG 最大概率切分 + HMM 未登录词识别 | 是（已捆绑） |

中英混排按 unicode 脚本边界切 run：CJK run 走 `jieba`/`cjk_bigram`，Latin/digit run 走小写+词干，
token position 全程连续递增（跨语言短语查询因此正确）。

词典加载，分平台：
- **Node** —— open 时自动加载（`@vane-rs/node` 包捆绑 `dict.bin`）。
- **Go** —— `Open` 后 `db.LoadDict(dict.DictBytes())`。
- **浏览器** —— CDN 拉取、sha256 校验、OPFS 缓存；不可用时降级 `cjk_bigram`（console.warn 不抛错）。
  支持内联 `dictData` 注入，用于离线/自托管。

### 检索模式与融合

| `mode` | 召回路径 | 排序依据 |
|---|---|---|
| `vector` | HNSW 段级并行搜索 → 归并；候选 < 2×topK 时暴力精确回退 | 向量距离 |
| `text` | Block-Max WAND top-k | BM25 |
| `hybrid` | 两路各取 `topK × candidateMultiplier` 候选 | 融合 |

融合默认 **RRF**（`score = Σ 1/(60 + rank)`，零调参）。需要显式加权混合时传
`fusion: { linear: { alpha: 0.5 } }`——注意 linear 分数按当次查询归一化，**跨语料不可比**，调参由调用方负责。
默认路径刻意不暴露 `alpha`。

### 自定义词表与 reindex

建库时或运行期注入领域词条：

```js
await col.setUserDict([
  '布地奈德',                       // 裸词条 → 取内置词典最高词频
  { term: 'PD-1抑制剂', freq: 100 }, // 显式词频
]);
```

`setUserDict` 只**暂存**新词表——collection 进入 `pendingReindex` 状态，但所有写入与查询在
你调用 `reindex()` 之前仍用**旧**分词身份。这杜绝了新旧段混排导致的静默不一致。`reindex()`
用新分词器逐一重建每个段（后台增量，旧段只读服务直到原子切换），完成后新词表才生效。可轮询
`reindexHandle.progress()` 或 `await reindexHandle.wait()`。

> 最佳实践：建库前收齐领域术语，作为 `userDict` 在 collection 创建时一次性注入。大库 reindex 不贵但也不免费。

### 过滤

元数据过滤（`eq` / `in` / `gte` / `lte`，字段间 AND，作为 pre-filter 推进 HNSW 遍历与 WAND 扫描，
低选择率时暴力回退）已在 Rust 核心（`vane-core`）实现，并由 `pre_filter` 测试套件覆盖。

**目前尚未通过 Node、Go、浏览器的绑定查询解析器暴露**（它们当前会拒绝 `filter`）。在绑定补齐之前，
过滤只能通过 Rust 核心 API 直接使用。这是绑定完整性缺口，不是核心能力限制。

## 架构

```
                    ┌─────────────────────────────────────────┐
   Node (napi-rs) ──┤                                         │
   Go (cgo/.a)   ───┤   vane-core  (单一 Rust 引擎)            ├── 浏览器 (wasm-bindgen + Worker)
   C ABI (FFI)   ───┤   • VFS trait: std-fs / OPFS / IDB / mem │
                    │   • 不可变段 + manifest 原子切换           │
                    │   • 分段 HNSW + Block-Max WAND BM25       │
                    │   • RRF 融合、pre-filter 位图             │
                    └─────────────────────────────────────────┘
```

- **单核心、无 mmap。** 所有 IO 走 `Vfs` trait；`vane-core` 被禁止触碰 `std::fs`/`std::net`/mmap
  （从第一天起就是 CI 硬门禁）。native 与浏览器共用同一条代码路径——显式 read 进 LRU 页缓存，而非 mmap。
- **不可变段。** 写入先进内存 buffer；`flush` 构建新段并经 `rename` 原子切换 manifest。读持有不可变快照。
- **分段 HNSW。** 每段一个独立 HNSW 图；删除即 tombstone；图只在段合并时从零重建（绝不原地删）。多段搜索并行
  （native 用 rayon，wasm 串行）后归并。
- **薄绑定。** Node 直连核心（不经 C ABI、不引入 tokio）；Go 经 cgo 包 C ABI；浏览器在 Worker 中包裹基于句柄的
  wasm 面。行为测试都在核心；绑定层只做转换与分发。

## 性能

native / Node，10 万文档 × 384 维：

| 指标 | 承诺 |
|---|---|
| Hybrid topK=10，P99 延迟 | < 50 ms（HNSW）；< 150 ms（暴力） |
| 批量写入吞吐 | ≥ 5,000 docs/s（含索引构建） |
| 10 万库冷启动 | 元数据 < 1 s；首次向量查询 < 3 s |
| 常驻内存（全加载） | < 500 MB（SQ8 后 < 200 MB） |
| Hybrid recall@10 | ≥ 0.95，相对暴力双路 + RRF 基线（CI 硬门禁） |
| 浏览器（WASM） | 延迟放宽约 3–5 倍；运行时自动选 SIMD128 变体 |

体积门禁（CI 硬卡）：核心 wasm ≤ 800 KB gzip（含分词器代码、不含词典）；全功能 ≤ 1.2 MB；中文词典每渠道 ≤ 1.5 MB。

## 状态

**v0.3.1** —— 核心引擎按里程碑 M0–M3 功能完成，并附带本机侧车 CLI：

- ✅ 核心 API、VFS、分词器（standard/cjk_bigram/jieba）、BM25、分段 HNSW、RRF 融合
- ✅ 持久化（段 + manifest + WAL）、tombstone 删除、合并、快照导出
- ✅ 核心 pre-filter 位图、SQ8 量化、rayon 并行执行器
- ✅ `setUserDict` / `reindex` 状态机、中文词典三侧分发（Node/Go/WASM）
- ✅ 绑定：Node（napi-rs，4 平台）、Go（cgo，4 平台 + wazero 桩）、浏览器
  （wasm-bindgen + Worker，OPFS/IDB，SIMD 双变体）
- ✅ 本机侧车 CLI（`vane`）：目录监听、CAS、混合检索、MCP stdio（macOS/Linux）

已知缺口：`filter` 已在核心接通但尚未通过绑定查询解析器暴露（见 [过滤](#过滤)）；musl/linux-arm64/
winx64-arm 的 Node 预编译与 wazero 纯 Go 路径顺延。

## 示例

- [`examples/demo/`](examples/demo/) —— Node：灌 1 万条合成维基摘要，并排对比 hybrid / vector / text 排序
  （含与手写 sqlite-vec + FTS5 方案的代码量对比）。
- [`demo/`](demo/) —— 浏览器：拖入 markdown 文件夹做纯前端混合搜索（jieba + OPFS + SIMD 双变体，无后端）。
- [`bindings/go/example/`](bindings/go/example/) —— Go：open → 加载词典 → add → search。

示例使用确定性的**伪向量**（hash bucket），因此无需 embedding 模型即可运行。生产请替换为真实
embedding 提供方——该接入有意不纳入 Vane 自身。

## 贡献

Vane 依据两份合同开发：[`docs/REQUIREMENTS.md`](docs/REQUIREMENTS.md)（做什么/为什么）与
[`docs/SPEC.md`](docs/SPEC.md)（精确接口、文件格式、错误码、数值门禁）。实现不得静默偏离它们——
改动公共 API、持久化格式、错误码或跨语言行为时，先同步更新 SPEC 及对应测试。

提 PR 前本地检查：

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cd crates/vane-node && npm test          # Node 绑定测试
cd bindings/go && go test ./...         # Go 绑定测试（需先构建 vane-ffi）
```

## 协议

Apache-2.0。专利授权条款让嵌入式使用的法务阻力最小。
