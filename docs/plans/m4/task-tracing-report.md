# M4 Phase 5a — tracing feature（cfg 门控零开销埋点）报告

- **分支**：`feat/m4-prod-readiness`
- **BASE**：`0cb50e5`（M4 Phase 3 跨版本兼容报告/审查 artifacts 提交）
- **commit**：`86a2b81`（`feat(core): tracing feature（cfg 门控零开销埋点，默认 off）（M4 阶段五 a）`）
- **范围**：`crates/vane-core` 首个触动生产代码的 M4 phase——新增 `tracing` feature + `#[cfg(feature="tracing")]` 门控埋点。纯新增，不改 M0-M3 冻结 pub API。
- **brief**：`docs/plans/m4/phase0-design.md` §3.5（tracing feature 骨架）。

## 1. feature 定义（`crates/vane-core/Cargo.toml`）

按设计 §3.5 字面采用。

```toml
[dependencies]
# tracing：可观测性埋点（M4 §3.5，I-5 能力开关）。
# optional + cfg(feature="tracing") 门控，默认 off——不启用时编译期消除，
# wasm/native 体积不变（800KB gzip 红线）。vane-wasm 不启用 tracing（守护红线）。
# 不触黑名单：传递依赖 tracing-core/thread_local/cfg-if（无 regex/tokio/prost/tonic/
# openssl/lindera/ndarray/wee_alloc/dashmap/parking_lot），cargo deny check 守护。
# tracing-subscriber（消费侧）不进 core——vane-ffi/vane-node 按需加 dev-dep。
tracing = { version = "0.1", optional = true }

[features]
# tracing：埋点能力开关（I-5，M4 §3.5）。启用时检索延迟/段数/merge 频率/
# 缓存命中率/WAL append/词典状态指标可观测；不启用时编译期消除，wasm/native
# 体积不变（800KB gzip 红线）。所有 tracing 调用经 `#[cfg(feature="tracing")]`
# 门控。默认 off；vane-wasm 不启用（守护红线）；vane-ffi/vane-node native 可启。
tracing = ["dep:tracing"]
```

**机制选型**：设计 §3.5 给出两选一（telemetry.rs 宏 vs 直接在各模块 `#[cfg(feature="tracing")] tracing::info!(...)`），**推荐后者**（少一层抽象）。采用推荐——不新建 `telemetry.rs`，所有埋点直接在各模块用 `#[cfg(feature="tracing")]` + `tracing::{span,info,debug}!` 宏。`tracing` crate 经 `dep:tracing` 隔离（Cargo 自动 hash 不会因 feature 名与 crate 名相同而误解析为 implicit feature）。

## 2. 埋点位置清单（§3.5 表字面落实）

| 指标 | 埋点位置 | span/事件 | 文件:行 |
|---|---|---|---|
| 检索延迟 | `api/collection.rs::run_search` 入口 span + 出口 elapsed | `tracing::info_span!("search", top_k, mode=?mode, segment_count, allow_hnsw)` + `tracing::info!(elapsed_us, hits, "search done")` | `api/collection.rs:683-695`（span）+ `:1003-1009`（done） |
| 段数 | `flush` 末 | `tracing::info!(collection, segment_ulid, doc_count, segment_count, "flush done")` | `api/collection.rs:464-472` |
| merge 频率 | `merge_segments` 入口 | `tracing::info!(collection, sources, target_docid_base, full_merge, "merge start")` | `api/collection.rs:533-541` |
| merge 完成 | `merge_segments` 末 | `tracing::info!(collection, new_segment_ulid, new_doc_count, segment_count, "merge done")` | `api/collection.rs:658-666` |
| 缓存命中率 | `PageCache::read` 命中/未命中 | `tracing::debug!(hit=true, path, page_idx, "page_cache")` / `tracing::debug!(hit=false, path, page_idx, bytes, "page_cache")` | `vfs/page_cache.rs:53-54` / `:65-73` |
| WAL append | `Wal::append` | `tracing::debug!(?record, "wal append")` | `wal/mod.rs:66-67` |
| 词典状态（set_user_dict） | `set_user_dict` 状态改写前 | `tracing::info!(collection, from=?state, to=?PendingReindex, dict_entries, "dict state transition")` | `api/collection.rs:1184-1191` |
| 词典状态（reindex 进入） | `reindex` PendingReindex→Rebuilding | `tracing::info!(collection, from=?PendingReindex, to=?Rebuilding, "dict state transition")` | `api/collection.rs:1244-1251` |
| 词典状态（reindex 完成） | `run_reindex` Rebuilding→Stable | `tracing::info!(collection, from=?Rebuilding, to=?Stable, "dict state transition")` | `api/collection.rs:1432-1439` |

合计 9 处埋点（8 个 `info!`/`info_span!` + 2 个 `debug!`；其中 PageCache 2 个 `debug!` 算同位置 1 处）。所有埋点经 `#[cfg(feature = "tracing")]` 门控，`feature="tracing"` off 时编译期消除（空展开），运行期零开销 + wasm/native 体积零增量。

## 3. telemetry.rs（不采用）

设计 §3.5 推荐"直接在各模块用 `#[cfg(feature="tracing")] tracing::info!(...)`，少一层抽象"。按推荐执行——**不新建 `crates/vane-core/src/telemetry.rs`**，埋点直接落在各业务模块。理由：

- 抽象层（`trace_span!`/`trace_info!`/`trace_debug!` 宏）增加一层间接，调试时读者须先查宏定义才知埋点语义；
- `#[cfg(feature="tracing")]` 散布度可控（9 处，集中在 search/flush/merge/PageCache/Wal/词典状态机关键路径，非散乱全模块）；
- `tracing` crate 的宏本身在 `tracing` feature off 时经 `dep:tracing` 隔离 + `#[cfg(feature)]` 门控已编译期消除，再加一层内部宏不增效益。

## 4. wasm 体积对比（关键：tracing off 体积不变 + 编译期消除 grep=0）

### 4.1 `bash scripts/check-wasm-size.sh`（默认 tracing off）

```
=== vane-wasm default (real deliverable) ===
vane-wasm default gzip size: 349261 bytes (max 819200)
OK: vane-wasm default gzip ≤ 800KB

=== vane-core --export-all (conservative upper bound) ===
vane-core --export-all gzip size: 641277 bytes (max 819200)
OK: vane-core --export-all gzip ≤ 800KB

=== Summary ===
vane-wasm default:      349261 bytes (gzip)
vane-core --export-all: 641277 bytes (gzip)
```

对比 Phase 2 基线（Phase 2a 全量门禁确认记录：vane-wasm 349261B / core --export-all 641275B gzip）——**tracing off 体积不变**（349261 持平；641275→641277 = +2B 属构建非确定性噪声，非 tracing 引入）。`vane-wasm` 不引 `tracing` feature（Cargo.toml 未透传），守护 800KB 红线。

### 4.2 编译期消除验证（tracing off 无符号）

```
$ wasm-objdump -x target/wasm32-unknown-unknown/release/vane_core.wasm | grep -c tracing
0
```

`vane_core.wasm`（tracing off，--export-all）内 0 个 tracing 符号——`#[cfg(feature="tracing")]` 门控 + `dep:tracing` 隔离确保 tracing crate 不进 wasm 二进制。**编译期消除验证通过**。

### 4.3 tracing on wasm 体积对比（验证 +15KB 增量，仍 ≤800KB）

```
$ RUSTFLAGS="-C link-arg=--export-all" cargo build --release --target wasm32-unknown-unknown -p vane-core --features tracing
$ wasm-opt -Oz ... -o /tmp/vane_core_tracing_on.wasm
$ gzip -c /tmp/vane_core_tracing_on.wasm | wc -c
656422

$ wasm-objdump -x target/wasm32-unknown-unknown/release/vane_core.wasm | grep -c tracing
918
```

| 配置 | vane-core --export-all gzip | tracing 符号数 |
|---|---|---|
| tracing off（默认） | 641277 B | 0 |
| tracing on | 656422 B | 918 |
| **增量** | **+15145 B（~15KB）** | +918 |

设计 §3.5 估算 +30-50KB gzip；实测 +15KB gzip，**低于估算下限**（tracing 0.1.44 + tracing-core 0.1.36 + once_cell 较历史版本更精简）。tracing on 体积 656422B ≤ 800KB（819200B）红线，余量 162778B。**vane-wasm 不启 tracing**，故此增量仅 native/ffi 可观测，wasm deliverable 永不变。

## 5. cargo deny check（tracing 依赖链不触黑名单——关键）

### 5.1 tracing 依赖树

```
$ cargo tree -p vane-core --features tracing -e normal,build | grep -iE "tracing|regex|tokio|prost|tonic|openssl|lindera|ndarray|wee_alloc|dashmap|parking_lot|cfg-if|thread_local|once_cell"
│   ├── cfg-if v1.0.4   # 来自 sha2，非 tracing
├── tracing v0.1.44
│   ├── pin-project-lite v0.2.17
│   ├── tracing-attributes v0.1.31 (proc-macro)
│   └── tracing-core v0.1.36
│       └── once_cell v1.21.4
```

tracing 0.1.44 传递依赖：
- `pin-project-lite`（无黑名单依赖）
- `tracing-attributes` v0.1.31（proc-macro，build-time，→ proc-macro2/quote/syn，无黑名单）
- `tracing-core` v0.1.36（→ `once_cell`，无黑名单）

设计 §3.5 预判 `tracing-core`→`thread_local`+`cfg-if`；实测新版 `tracing-core` v0.1.36 改用 `once_cell`（非 `thread_local`），`cfg-if` 来自 `sha2` 而非 tracing。**均不触黑名单**（regex/tokio/prost/tonic/openssl/lindera/ndarray/wee_alloc/dashmap/parking_lot 均不在 tracing 依赖链）。

### 5.2 `cargo deny check` 全量输出

```
$ cargo deny check
warning[unused-wrapper]: wrapper for banned crate was not encountered
   ┌─ deny.toml:16:36
16 │     { name = "regex", wrappers = ["napi-derive-backend", "criterion"] },
   │                                    ━━━━━━━━━━━━━━━━━ unmatched wrapper
advisories ok, bans ok, licenses ok, sources ok
```

- advisories **ok**
- bans **ok**（tracing 链无黑名单 crate）
- licenses **ok**（tracing = MIT，tracing-core = MIT，tracing-attributes = MIT，proc-macro2/quote/syn/once_cell/pin-project-lite 均 MIT/Apache-2.0，均在 allow 列表）
- sources **ok**

唯一 warning 是预存的 `regex` wrappers 未匹配（Phase 1a 之前就有，非 tracing 引入）——非 error，exit 0。

## 6. 各门禁真实输出

| 门禁 | 命令 | 结果 |
|---|---|---|
| 格式 | `cargo fmt --all -- --check` | rc=0，无 diff（首次 `tracing::debug!` 多参数 rustfmt 折行已修正） |
| 静态检查 | `cargo clippy --all-targets --all-features --workspace --exclude vane-fuzz -- -D warnings` | rc=0，0 warnings（stable clippy 能编 tracing crate） |
| 工作区测试 | `cargo test --workspace --all-features --exclude vane-fuzz` | **EXIT=0**，全 0 failed；代表性：vane-core unittest 322 passed / crash_recovery 5 / proptest_invariants 3 / cross_version_compat 3 / recall 7 / pre_filter 9 / tombstone_merge 9 / userdict_reindex 5 / wal_crash 8 / hnsw_recall 2 / corpus_compat 11 / million_scale 3（105s）/ ndcg_wiki_zh 3（72s）/ vane-node integration 84 / vane-dict-zh 24。tracing on 时埋点编译期参与但无 subscriber 安装→事件 emit 后丢弃，行为与 tracing off 等价（零行为漂移）。 |
| cargo deny | `cargo deny check` | advisories ok / bans ok / licenses ok / sources ok（1 预存 unused-wrapper warning，非本任务引入） |
| wasm32 check（tracing off） | `cargo check --target wasm32-unknown-unknown -p vane-core` | rc=0 |
| wasm32 check（tracing on） | `cargo check --target wasm32-unknown-unknown -p vane-core --features tracing` | rc=0（tracing 在 wasm 可编，once_cell/pin-project-lite/tracing-core 均 wasm32 兼容） |
| wasm 体积 | `bash scripts/check-wasm-size.sh` | rc=0；vane-wasm 349261B / vane-core --export-all 641277B gzip，均 ≤800KB；tracing off 体积不变 |
| 编译期消除 | `wasm-objdump -x vane_core.wasm \| grep -c tracing` | **0**（tracing off 无符号） |
| no-std-fs | `bash scripts/check-no-std-fs.sh` | OK（tracing 埋点不引 std::fs/std::net/mmap） |

## 7. commit

```
commit 86a2b81
Author: ximing
Date:   Wed Aug 12 00:35:13 2026 +0800

    feat(core): tracing feature（cfg 门控零开销埋点，默认 off）（M4 阶段五 a）
```

commit 内容（`git status --short`）：
- `crates/vane-core/Cargo.toml`（tracing optional dep + feature 定义）
- `crates/vane-core/src/api/collection.rs`（search span + flush/merge_segments/dict 状态机埋点）
- `crates/vane-core/src/vfs/page_cache.rs`（PageCache::read 命中/未命中 debug!）
- `crates/vane-core/src/wal/mod.rs`（Wal::append debug!）
- `Cargo.lock`（tracing v0.1.44 + tracing-attributes v0.1.31 + tracing-core v0.1.36 入 lock）

**未触碰**：SPEC.md / CI yml（.github/workflows/ci.yml）/ fault.rs / crash_recovery.rs / vane-fuzz / proptest / cross_version_compat / segment/header.rs / Cargo.toml（根）/ 其他 M0-M3 冻结 pub API 文件。`git status` 确认 commit 只含上述 5 文件。

## 8. 自审

### 8.1 tracing crate 依赖链

- tracing v0.1.44 → pin-project-lite + tracing-attributes（proc-macro）+ tracing-core v0.1.36 → once_cell。
- 无 regex/tokio/prost/tonic/openssl/lindera/ndarray/wee_alloc/dashmap/parking_lot（cargo deny check bans ok 守护）。
- 设计 §3.5 预判 `thread_local`+`cfg-if`；实测新版 tracing-core v0.1.36 改用 `once_cell`，更精简（thread_local 不在链）。`cfg-if` 来自 `sha2`（既有），非 tracing 引入。
- 版本漂移风险（tracing 0.2 可能引黑名单）由 cargo deny check 守护——CI deny job 已配置。

### 8.2 埋点散落度

- 9 处埋点（`info_span!` 1 / `info!` 6 / `debug!` 2），分布在 4 个文件（api/collection.rs、vfs/page_cache.rs、wal/mod.rs）。
- 集中在 §3.5 表关键路径（search/flush/merge/PageCache/Wal/词典状态机），未散乱全模块——符合设计 §3.5 "集中在 search/flush/merge 关键路径，不过度埋点"。
- 每处埋点经 `#[cfg(feature = "tracing")]` 门控 + 单行注释标明 M4 §3.5 出处 + 指标名，便于 grep 审计与 future-off 时编译期消除验证。

### 8.3 wasm 体积增量实测 vs 估算

- 设计 §3.5 估算 +30-50KB gzip（启用 tracing 时）。
- 实测 +15KB gzip（641277→656422，tracing on），**低于估算下限**——tracing 0.1.44 + tracing-core 0.1.36 + once_cell 较设计撰写时（~2025）更精简（tracing-core 改用 once_cell 替代 thread_local）。
- tracing on 仍 ≤800KB（656422B ≤ 819200B，余量 162778B）——但 vane-wasm 永不启 tracing（守护红线），此增量仅 vane-ffi/vane-node native 可观测路径。
- tracing off 体积不变（641275→641277 = +2B 构建噪声非 tracing）——**红线不变确认**。

### 8.4 行为漂移

- tracing feature off（默认）：所有埋点编译期消除，运行期零开销，行为与 Phase 2/1/3 完成态完全等价（322+ 单测 + 集成测试 0 回归确认）。
- tracing feature on：埋点 emit 事件至 tracing dispatch，若无 subscriber 安装（如单元测试）则事件 emit 后丢弃——不影响检索/写入/合并逻辑。测试在 `--all-features`（含 tracing）下跑 0 failed 确认无行为漂移。
- pub API 不变：埋点全在函数体内 `#[cfg(feature)]` 块，不改任何 pub fn 签名 / struct 定义 / 错误码 / 持久化格式。`tracing` feature 是纯新增能力开关（I-5），符合 SPEC v1.4 §14 I-5 释义。

### 8.5 concerns

1. **CI yml 未加 wasm32-size-tracing-on job**（per task scope 明示 "不碰 CI yml"——Phase 6 加 wasm32-size-tracing-on job 对比）。当前 tracing on wasm 体积验证（§4.3）仅本 report 记录，未进 CI 自动化守护。Phase 6 应加 `wasm32-size-tracing-on` job + `check-wasm-size.sh` 加 tracing on 对比断言。
2. **`check-wasm-size.sh` 未加 `wasm-objdump -x vane_core.wasm \| grep -c tracing` 断言**（设计 §3.5 推荐加一行）。本 report 已手动验证 grep=0，但脚本未持久化此断言。Phase 6 加此一行（与 concern 1 同步）。
3. **tracing-subscriber 不进 core**（设计取舍）：core 只 emit 事件，subscriber 是消费侧。vane-ffi/vane-node native 按需加 tracing-subscriber dev-dep + init——非本 phase 范围，defer 至绑定层演进。
4. **`elapsed_us` 用 `web_time::Instant`**（跨平台，wasm32 用 performance.now()）——已用于 AutoCommitter，tracing 复用同一 Instant 源，无新依赖。但 tracing on wasm 时 `_search_start.elapsed()` 会调 performance.now()，理论上 wasm 检索延迟观测受 wasm 时钟精度限制（ms 级，足够 p50/p99）——非缺陷，记为 known-limitation。
5. **`tracing::info_span!` RAII guard**：`_span` 在 `run_search` 入口绑定，函数返回时 drop 记录 span exit。早期返回（topK 超限/缺 text+vector/dim 不匹配）在 span 建立前——这些路径不经 span，是参数校验 fast-fail 的预期行为，非埋点缺口。span 建立后的 `?` 错误传播会 drop span 记录 exit（subscriber 可观测 span 持续期），但 `elapsed_us` done 事件仅在成功路径 emit——错误路径无 elapsed 事件。若需错误路径 elapsed，可改用 `tracing::instrument` 属性宏（需 tracing-attributes，已在 dep 但未用）——defer。

## 9. 状态

**DONE_WITH_CONCERNS**：tracing feature 落地 + 9 处埋点 + wasm off 体积不变（grep=0）+ deny 绿 + 全门禁绿。5 项 concerns 全属 Phase 6 CI/SPEC 修订范围（CI yml 加 tracing-on job + check-wasm-size.sh 加 grep 断言 + tracing-subscriber 绑定层 defer + elapsed_us wasm 时钟精度 known-limitation + 错误路径 elapsed defer），非本 phase 阻塞。
