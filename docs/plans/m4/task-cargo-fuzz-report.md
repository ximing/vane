# M4 阶段一 a — cargo-fuzz 集成 report

> Task #2 Phase 1a implementer 报告。BASE=0458942。分支 `feat/m4-prod-readiness`。
> 设计依据：`docs/plans/m4/phase0-design.md` §3.2（crate 布局 + Cargo.toml + 5 targets + CI 草案 + 取舍/风险）。
> 状态：**DONE_WITH_CONCERNS**（见 §6 concerns）。

## 1. crate 结构

```
crates/vane-fuzz/
├── Cargo.toml              # name=vane-fuzz, publish=false, license=Apache-2.0
│                           # [package.metadata] cargo-fuzz=true（0.13+ 检测机制，设计未预见但必需）
│                           # deps: vane-core(path) + libfuzzer-sys=0.4
│                           # 5 [[bin]] targets，path=fuzz_targets/<name>.rs
└── fuzz_targets/
    ├── common.rs           # 共享 ByteCursor 字节→结构 decoder（不引 arbitrary）
    ├── brute_search_fuzz.rs
    ├── hnsw_search_fuzz.rs
    ├── persist_roundtrip_fuzz.rs
    ├── merge_fuzz.rs
    └── dict_load_fuzz.rs
```

**Cargo.toml 关键字段**（设计 §3.2 按字面采用 + 2 必要增补）：
- `publish = false` + `version = "0.0.0"` + `edition = "2021"`（设计原文）
- `license = "Apache-2.0"`（**增补**：cargo-deny 要求 crate 有 license 字段，否则 `unlicensed` error；从 workspace `license = "Apache-2.0"` 对齐）
- `[package.metadata] cargo-fuzz = true`（**增补**：cargo-fuzz 0.13+ 的检测机制——无此字段 `cargo fuzz build` 报 "does not look like a cargo-fuzz manifest"；设计未预见，必需）
- `[dependencies] vane-core = { path = "../vane-core" }` + `libfuzzer-sys = "0.4"`（设计原文，**不启 jieba feature**——见 §4 取舍）

## 2. 5 targets 实现摘要

每个 target：`#![no_main]` + `libfuzzer::fuzz_target!(|data: &[u8]| { ... })`。输入经 `common::ByteCursor` 从 `&[u8]` 确定性消费（耗尽返 0，saturating_add，**decoder 自身绝不 panic**）。f32 经 NaN/Inf 过滤→0.0（保 score 算术良定义）。

| target | 输入 decode | 不变量 |
|---|---|---|
| `brute_search_fuzz` | dim(1..=16) + n_docs(0..=8) + top_k(1..=10) + mode(Vector/Text/Hybrid) + 每 doc 的 text+vector + query 的 text/vector | `search_brute_baseline` 不 panic；`hits.len() ≤ top_k`；每 `hit.score` 非 NaN |
| `hnsw_search_fuzz` | dim(2..=8) + n_docs(1..=20) + top_k(1..=5) + 每 doc 的 vector + query 向量 | `search`（HNSW）不 panic；`hits.len() ≤ top_k`；score 非 NaN；hit.id ∈ 已知 id 集（无 phantom）；`search_brute_baseline` 同结构合法。**不做严格 recall 断言**（HNSW 近似，随机小图 recall 未必 100%，严格断言易误报；recall 质量由 proptest §3.3 覆盖） |
| `persist_roundtrip_fuzz` | dim(1..=8) + n_docs(1..=9) + query 向量（reopen 前后复用同一向量）+ 每 doc 的 text+vector | open→add→flush→search→close→reopen→search：reopen 后 topK 合法、score 非 NaN、hit id 全在原 id 集合（external_id 回填）、reopen 前后 id 集合相同（round-trip 一致） |
| `merge_fuzz` | dim(1..=8) + n_flushes(1..=4) + docs_per_flush(1..=5) + n_delete（cursor 驱动）+ 各 doc 的 vector + query 向量 | 多轮 add+flush（多段）→ delete（tombstone）→ compact → search top_k=1000：tombstoned id 不可见、hit id 全已知（无 phantom）、无重复 id（docid 连续）、live id 全可见（不丢文档） |
| `dict_load_fuzz` | n_entries(0..=16) + 每 entry 的 word(lossy UTF-8)+freq | M2-04 铁律：`build_tokenizer(Jieba,..)` 返 Err（DictUnavailable，不 panic）；`build_tokenizer(CjkBigram,..)` 降级成功（不 panic）；`build_tokenizer(Standard,..)` 成功；`Collection::set_user_dict(fuzzer entries)` Ok/Err 不 panic |

### 输入 decode 取舍

**不引 `arbitrary` crate**：设计 §3.2 Cargo.toml 只列 libfuzzer-sys。自研 `ByteCursor`（~70 行）从 fuzzer `&[u8]` 确定性消费字节构造结构化输入。libfuzzer-sys 0.4 的 `fuzz_target!` 宏本身经 `cfg(fuzzing)` 门控——plain `cargo check` 不设此 cfg → `libfuzzer` 模块不可见（见 §5 #4）。`arbitrary` 虽大概率不触黑名单（已验证 `bans ok`），但多一个传递依赖多一份 deny 风险；自研 decoder 零外部依赖、可控。

## 3. workspace + CI 改动

### workspace `Cargo.toml`
```toml
members = [..., "crates/vane-fuzz"]
default-members = ["crates/vane-core", "crates/vane-ffi", "crates/vane-node", "crates/vane-dict-zh", "crates/vane-wasm"]
```
- `members` 加 `crates/vane-fuzz`。
- `default-members` 排除 vane-fuzz → `cargo build`/`cargo test`（不带 `--workspace`）默认不含 fuzz。
- **`cargo test --workspace` 显式列全部 members，仍需 `--exclude vane-fuzz`**（见下 CI 改动）。

### `.github/workflows/ci.yml`
- **clippy job（line 44→）**：`cargo clippy --all-targets --all-features` → `cargo clippy --all-targets --all-features --workspace --exclude vane-fuzz -- -D warnings`。
  - **增补 `--workspace`**：原 clippy job 无 `--workspace`（靠 default-members），但 `--exclude` 须配 `--workspace`（否则 cargo 报 "`--exclude can only be used together with --workspace`"）。default-members 也排除 vane-fuzz，双保险。
- **test job（line 55→）**：`cargo test --workspace --all-features` → `cargo test --workspace --all-features --exclude vane-fuzz`。
  - 原 test job 已有 `--workspace`，加 `--exclude` 直接生效。
- **不动其他 14 jobs**。fuzz-smoke/fuzz-long 新 job 是 Phase 6（非本任务）。

### `Cargo.lock`
- 新增 `libfuzzer-sys v0.4.13` + `arbitrary v1.4.2`（libfuzzer-sys 传递依赖）。
- **commit 含 Cargo.lock**：项目追踪 Cargo.lock（git ls-files 确认），新增 crate 的 lockfile 更新是直接后果，不入 commit 会留仓库状态不一致。brief 的 commit 清单未列 Cargo.lock，但它是必要组成部分（同 `--exclude` 是"保 CI 绿的必要修改"）。

## 4. 取舍 / 偏离

### 4a. 不启 jieba feature（dict_load_fuzz 的取舍）

**设计 §3.2 Cargo.toml 字面**：`vane-core = { path = "../vane-core" }`（无 features）。

**dict_load_fuzz 的设计意图**（§3.2 target 表）："畸形词典字节 → 降级 bigram 不抛错"——需 `JiebaDict::load`/`load_zstd`，二者在 `#[cfg(feature = "jieba")]` 后。

**若启 jieba**：vane-fuzz dep 加 `features = ["jieba"]` → workspace feature unification 会使 vane-core 在**所有** `-p vane-core` 构建（含 wasm32-check）都带 jieba。设计 §3.2 明确不启以避此风险（"绝不污染 vane-core/wasm/ffi 生产构建"）。

**采用方案**：按字面不启 jieba。dict_load_fuzz 验 M2-04 的 **API 层降级不变量**：
- `build_tokenizer(BuiltinTokenizer::Jieba, &user_dict)` → `Err(DictUnavailable)`（无 dict 实例，不 panic）
- `build_tokenizer(BuiltinTokenizer::CjkBigram, &user_dict)` → `Ok`（降级 bigram 成功）
- `build_tokenizer(BuiltinTokenizer::Standard, &user_dict)` → `Ok`
- `Collection::set_user_dict(&user_dict)` → Ok/Err 不 panic

**未覆盖**：`JiebaDict::load`/`load_zstd` 的畸形字节→Err 路径（需 jieba feature）。**defer Phase 6**：若需，vane-fuzz 加 optional `jieba` feature（`vane-core/jieba`）+ cfg-gated `JiebaDict::load` 调用，fuzz-smoke CI 加 `--features jieba` 跑 dict_load_fuzz。

### 4b. cargo-fuzz = true metadata（设计未预见）

设计 §3.2 Cargo.toml 未含 `[package.metadata] cargo-fuzz = true`。cargo-fuzz 0.13+ 的检测机制要求此字段，否则 `cargo fuzz build` 报 "does not look like a cargo-fuzz manifest"。**必需增补**（无此字段 cargo-fuzz 无法识别 crate）。

### 4c. vane-fuzz license 字段

cargo-deny 对无 license 的 crate 报 `unlicensed` error。设计 §3.2 Cargo.toml 未列 license。**增补** `license = "Apache-2.0"`（与 workspace `[workspace.package] license = "Apache-2.0"` 对齐）。

### 4d. CI clippy job 增补 `--workspace`

设计 §3.2 取舍段说"test job 改为 `--workspace --exclude vane-fuzz`"，但 clippy job 原**无** `--workspace`。`--exclude` 须配 `--workspace`（cargo 报错）。**增补** `--workspace` 到 clippy job。default-members 也排除 vane-fuzz，双保险。

## 5. 验证结果（stable 口径）

| # | 门禁 | 命令 | 结果 |
|---|---|---|---|
| 1 | fmt | `cargo fmt --all -- --check` | **PASS** rc=0，无 diff |
| 2 | clippy | `cargo clippy --all-targets --all-features --workspace --exclude vane-fuzz -- -D warnings` | **PASS** Finished，0 warning |
| 3 | test | `cargo test --workspace --all-features --exclude vane-fuzz` | **PASS** 322 unit + 全集成测试 0 FAILED（recall/corpus_compat/crash_recovery/wal_crash 等） |
| 4 | vane-fuzz stable check | `cargo check -p vane-fuzz --all-features` | **FAIL（预期）** `error[E0432]: unresolved import libfuzzer` ×5。原因：`libfuzzer-sys` 的 `libfuzzer` 模块在 `cfg(fuzzing)` 后，plain `cargo check` 不设此 cfg（仅 `cargo fuzz` 设）。**5 targets 编译验证延至 CI nightly fuzz-smoke（Phase 6）**。API 签名已逐个 grep 确认（见 §7）。 |
| 5 | deny | `cargo deny check` | **bans ok / licenses FAILED / advisories ok / sources ok**（见 §6） |
| 6 | wasm32 | `cargo check --target wasm32-unknown-unknown -p vane-core` | **PASS** Finished 0.13s。vane-fuzz 不在 wasm check 范围（`-p vane-core`），不影响 wasm。 |

### #4 的 E0432 详情
```
error[E0432]: unresolved import `libfuzzer`
  |     ^^^^^^^^^ use of unresolved module or unlinked crate `libfuzzer`
```
每个 target 1 个 E0432（`use libfuzzer::fuzz_target;` 无法解析 `libfuzzer` 模块）。`libfuzzer-sys 0.4` 在 `cfg(fuzzing)` 下才导出 `libfuzzer` 模块——`cargo fuzz` 构建时设 `--cfg fuzzing`，plain `cargo check` 不设。**非 API 错误、非 nightly 问题**（nightly 也不设 `cfg(fuzzing)`，须 `cargo fuzz`）。warning 全是"unused import"（MemoryVfs/Vfs）——是 E0432 导致 `fuzz_target!` 宏不展开、宏体内的 import 使用点不可见的副作用，**非真 unused**。

## 6. cargo deny 结果 + NCSA license concern

### bans（黑名单）：**ok**
libfuzzer-sys v0.4.13 + arbitrary v1.4.2 的传递依赖**不触** deny.toml 黑名单（regex/tokio/prost/tonic/openssl/lindera/ndarray/wee_alloc/dashmap/parking_lot）。设计 §3.2 预判正确。

### licenses：**FAILED**（concern）

**libfuzzer-sys v0.4.13** 的 license = `(MIT OR Apache-2.0) AND NCSA`。其中 `NCSA`（University of Illinois/NCSA Open Source License，即 LLVM/libFuzzer 的许可证）**不在** deny.toml `[licenses] allow` 列表 → cargo-deny 报 `error[rejected]: failed to satisfy license requirements`。

- NCSA 是 **OSI approved** + **FSF Free/Libre**（cargo-deny 自报）。
- 是 libFuzzer C++ 库的许可证（LLVM 项目），libfuzzer-sys 绑定它。
- **不影响 bans 黑名单**（不是 banned crate，是 license allow-list 缺口）。

**修复**：deny.toml `[licenses] allow` 加 `"NCSA"`（1 行，trivial，不改 ban 语义）。
**未修**：brief 指令"勿自行改 deny.toml 语义"+ commit scope 限 `crates/vane-fuzz/** + Cargo.toml + Cargo.lock + ci.yml`。**上报编排者批准** deny.toml 的 NCSA 增补——否则 CI deny job 会在 push/PR 时 FAIL。

**vane-fuzz 自身 unlicensed**：已修（加 `license = "Apache-2.0"`），deny 不再报 vane-fuzz unlicensed。

### 其他 warning（非阻断）
- `warning[unused-wrapper]`：criterion wrapper for regex 未匹配——**预存**（criterion 0.5 不直接拉 regex），非 vane-fuzz 引入。
- `warning[duplicate]`：syn 2 个版本（2.0 + 3.0）——workspace 预存多版本，非 vane-fuzz 引入。

## 7. 自审

### API 签名逐个 grep 确认（防 #4 掩盖 API 错误）
#4 的 E0432 在宏不展开后可能掩盖 vane-core API 误用。已 grep 确认全部方法存在且签名匹配：

| 调用 | 源文件:行 | 签名 |
|---|---|---|
| `Db::open(vfs, path, opts)` | api/db.rs | `pub fn open(vfs: Arc<dyn Vfs>, path: &str, opts: OpenOptions) -> Result<Self>` |
| `Db::close(&self)` | api/db.rs:178 | `pub fn close(&self) -> Result<()>`（`&self` 非 `self`，不消费） |
| `db.collection(name, schema, opts)` | api/db.rs | `pub fn collection(&self, name: &str, schema: Schema, opts: CollectionOptions) -> Result<Collection>` |
| `col.add(&docs)` | api/collection.rs:253 | `pub fn add(&self, docs: &[Doc]) -> Result<AddReport>` |
| `col.flush()` | api/collection.rs | `pub fn flush(&self) -> Result<()>` |
| `col.search(&query)` | api/collection.rs | `pub fn search(&self, query: &SearchQuery) -> Result<Vec<Hit>>` |
| `col.search_brute_baseline(&query)` | api/collection.rs:648 | `pub fn search_brute_baseline(&self, query: &SearchQuery) -> Result<Vec<Hit>>`（`#[doc(hidden)]` 仍 pub） |
| `col.delete(&[id])` | api/collection.rs:1019 | `pub fn delete(&self, ids: &[String]) -> Result<u64>` |
| `col.compact()` | api/collection.rs:1076 | `pub fn compact(&self) -> Result<()>` |
| `col.set_user_dict(&dict)` | api/collection.rs:1129 | `pub fn set_user_dict(&self, dict: &[UserDictEntry]) -> Result<()>` |
| `build_tokenizer(kind, &dict)` | tokenizer/mod.rs:74 | `pub fn build_tokenizer(kind: BuiltinTokenizer, user_dict: &[UserDictEntry]) -> Result<Box<dyn Tokenizer>>` |
| `BuiltinTokenizer::Jieba` | tokenizer/mod.rs:33 | 非 cfg-gated，无 jieba feature 也可用 |
| `MemoryVfs::new()` / `StdFsVfs::with_root` | vfs/ | corpus_compat.rs 同路径 |

类型路径：`vane_core::api::{Db,Doc,SearchQuery,SearchMode,FusionSpec,OpenOptions,CollectionOptions}` + `vane_core::types::{Schema,FieldDef,Metric}` + `vane_core::vfs::{memory::MemoryVfs, Vfs}` + `vane_core::tokenizer::{build_tokenizer, BuiltinTokenizer, UserDictEntry}`——全 `pub mod`/`pub fn`/`pub enum`，与 corpus_compat.rs 同路径。

### libfuzzer-sys + arbitrary 依赖链
- `libfuzzer-sys v0.4.13`：license `(MIT OR Apache-2.0) AND NCSA`。传递依赖：`arbitrary v1.4.2`（license MIT/Apache-2.0）+ `cc`（build-dep）。**不触 bans 黑名单**（deny `bans ok` 确认）。**NCSA license 需 deny.toml 增补**（见 §6）。
- `arbitrary v1.4.2`：libfuzzer-sys 的传递依赖（非我直接引）。MIT/Apache-2.0，不触黑名单。

### nightly 可用性
- 本地 nightly 安装**失败**：首次前台安装成功（rustc 1.99.0-nightly 2026-08-10），但后续重装超时（网络不稳，600s 无进展被 watchdog 杀）。
- **按编排者指令换策略**：不再装 nightly，纯 stable 验证 + commit。fuzz build/smoke 延至 CI nightly（Phase 6 fuzz-smoke job）。
- `cargo-fuzz v0.13.2` 已装（`cargo install cargo-fuzz --locked`），但无 nightly 无法 `cargo +nightly fuzz build/run`。

### 是否需 cargo-fuzz install
- 本地已装 `cargo-fuzz 0.13.2`（`/Users/ximing/.cargo/bin/cargo-fuzz`）。
- CI 的 fuzz-smoke job（Phase 6）须 `cargo install cargo-fuzz --locked` + `dtolnay/rust-toolchain@nightly`。

## 8. commit

- **hash**：`d4a94d8`
- **分支**：`feat/m4-prod-readiness`
- **msg**：`feat(fuzz): vane-fuzz crate + 5 targets + CI --exclude vane-fuzz（M4 阶段一 a）`
- **不含** Co-Authored-By trailer，未 push。
- **文件**（10 个，667 insertions, 3 deletions）：
  - `crates/vane-fuzz/Cargo.toml`（新）
  - `crates/vane-fuzz/fuzz_targets/{brute_search_fuzz,common,dict_load_fuzz,hnsw_search_fuzz,merge_fuzz,persist_roundtrip_fuzz}.rs`（6 新）
  - `Cargo.toml`（members+default-members）
  - `Cargo.lock`（libfuzzer-sys + arbitrary）
  - `.github/workflows/ci.yml`（test+clippy --exclude vane-fuzz）
- **git status 确认**：未动 SPEC.md / fault.rs / crash_recovery.rs / vane-core 源码 / PROGRESS.md / CLAUDE.md（预存 M 非本人）。

## 9. concerns

1. **NCSA license 阻断 CI deny job**（**阻断性**）：libfuzzer-sys 的 NCSA license 不在 deny.toml allow 列表 → `cargo deny check` licenses FAILED。修复=deny.toml 加 `"NCSA"` 到 `[licenses] allow`（1 行，trivial，OSI/FSF approved，不改 ban 语义）。**上报编排者批准**——否则 CI deny job 在 push/PR 时 FAIL。
2. **nightly fuzz build/smoke 未本地验证**（非阻断，延 CI）：stable `cargo check -p vane-fuzz` 因 `cfg(fuzzing)` 门控失败（预期）。5 targets 的编译 + run 验证延至 Phase 6 CI nightly fuzz-smoke job。API 签名已逐个 grep 确认（§7），但宏展开后的完整编译验证未做。
3. **dict_load_fuzz 未覆盖 JiebaDict::load 畸形字节**（非阻断，设计取舍）：不启 jieba feature（避 feature unification 触 wasm32-check），dict_load_fuzz 验 API 层降级（Jieba→Err→CjkBigram→Ok），未验 `JiebaDict::load`/`load_zstd` 的畸形字节→Err 路径。defer Phase 6（vane-fuzz 加 optional `jieba` feature + cfg-gated 调用）。
4. **Cargo.lock 入 commit**（超出 brief 清单）：brief commit scope 列 `crates/vane-fuzz/** + Cargo.toml + ci.yml`，未列 Cargo.lock。但 Cargo.lock 是 tracked 文件，新增 vane-fuzz 的 lockfile 更新是直接后果，不入 commit 会留仓库状态不一致。已入 commit（同 `--exclude` 是"保 CI 绿的必要修改"逻辑）。上报编排者知悉。
5. **CI clippy job 增补 `--workspace`**（超出 brief 字面）：brief 说 clippy 加 `--exclude vane-fuzz`，但原 clippy job 无 `--workspace`，`--exclude` 须配 `--workspace`。已增补（不改语义——default-members 也排除 vane-fuzz，双保险）。

---

## 10. NCSA license fix（用户批准后追加）

**触发**：§6 concern 1——libfuzzer-sys v0.4.13 的 license `(MIT OR Apache-2.0) AND NCSA` 中 `NCSA` 不在 deny.toml `[licenses] allow`，cargo deny licenses FAILED，阻断 CI deny job。用户批准加 NCSA。

### 改了什么

`deny.toml` `[licenses] allow` 列表加 `"NCSA"`（+5 行注释，共 6 insertions）：

```toml
[licenses]
allow = [
    "Apache-2.0",
    "MIT",
    "MIT-0",
    # NCSA = University of Illinois/NCSA Open Source License（LLVM/libFuzzer 许可证）。
    # libfuzzer-sys v0.4.13 的 license = "(MIT OR Apache-2.0) AND NCSA"——
    # NCSA 是 libFuzzer C++ 库的许可证（OSI approved + FSF Free/Libre）。
    # M4 阶段一 a：vane-fuzz crate 引 libfuzzer-sys，需 allow NCSA。
    # 仅 license 允许，不改 [bans] crate 黑名单语义（regex/tokio/prost/... 仍 ban）。
    "NCSA",
    "BSD-2-Clause",
    ...
]
```

- **不改 `[bans]`**：regex/tokio/prost/tonic/openssl/lindera/ndarray/wee_alloc/dashmap/parking_lot 黑名单完全不变。NCSA 是 license allow-list 增补，非 crate ban 语义变更。
- **不动 Cargo.lock**：NCSA 是 license 非依赖变更，Cargo.lock 无变化。

### cargo deny check 真实输出（fix 后）

```
$ cargo deny check
...
warning[unused-wrapper]: wrapper for banned crate was not encountered
   ┌─ deny.toml:16:36
   │  { name = "regex", wrappers = ["napi-derive-backend", "criterion"] },
   │                                    ━━━━━━━━━━━━━━━━━━━ unmatched wrapper

advisories ok, bans ok, licenses ok, sources ok

$ echo $?
0
```

- **licenses 从 FAILED → ok**：NCSA 加入 allow 后，libfuzzer-sys 的 `(MIT OR Apache-2.0) AND NCSA` 满足（MIT/Apache-2.0/NCSA 全在 allow）。
- `bans ok`：libfuzzer-sys + arbitrary 不触黑名单（不变）。
- `advisories ok` / `sources ok`：不变。
- `warning[unused-wrapper]`：**预存**（criterion 0.5 不直接拉 regex），非 vane-fuzz 引入，非 error，不影响 exit code 0。
- exit code = **0**（全绿）。

### commit

- **hash**：`9e262db`
- **分支**：`feat/m4-prod-readiness`
- **msg**：`chore(deny): allow NCSA license for libfuzzer-sys（M4 阶段一 a）`
- **不含** Co-Authored-By，未 push。
- **文件**（1 个，6 insertions）：`deny.toml` 仅。
- **commit 链**：`0458942`（Phase 2 docs）→ `d4a94d8`（vane-fuzz crate）→ `9e262db`（NCSA fix）。

### concern 1 状态更新

§9 concern 1（NCSA license 阻断 CI deny job）→ **已解决**。cargo deny check 全绿（bans+licenses+advisories+sources 全 ok，exit 0）。CI deny job 不再阻断。剩余 concerns（2 nightly deferred / 3 dict_load JiebaDict::load defer / 4 Cargo.lock 入 commit / 5 clippy --workspace）不变。
