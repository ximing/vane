# M4 阶段六 a：CI 新增 5 job 实现报告

> 任务：M4-PLAN 阶段六 a + 设计 §3.2 fuzz workflow 草案。
> 分支：`feat/m4-prod-readiness`。
> 前置：Phase 1（vane-fuzz crate + 5 fuzz targets）、Phase 2（crash_recovery 5 场景）、Phase 3（cross_version_compat + v0.1.0 fixture）、Phase 5（tracing/inspect，不涉本批）、Phase 4（stress_concurrency）均就位。

## 1. 改动范围

仅改 `.github/workflows/ci.yml`（+124 行，0 删减——`on:` 块编辑是纯追加，5 job 全新）。不碰 vane-core/vane-fuzz 源码、SPEC.md、fault.rs、crash_recovery.rs、proptest、cross_version、tracing、inspect、diagnostics。

## 2. `on:` 触发器扩展

原 `on:` 仅 `push`（main, paths-ignore website/docs-site）+ `pull_request`。新增：

- `schedule: - cron: '0 3 * * 0'`（每周日 03:00 UTC）
- `workflow_dispatch:`（手动触发）

**取舍说明**：fuzz-long 需要 cron 触发，而 GitHub Actions 的 `on:` 是 workflow 级（非 job 级）。在单文件内实现 fuzz-long cron-only 的两条路：
- (a) 加 `schedule` 到顶层 `on:` + 给 fuzz-long 加 `if:` 门控（只在 schedule/dispatch 跑）——代价是 cron 时全 workflow（含现有 16 job）都会触发。
- (b) 独立 workflow 文件 `fuzz-long.yml`——干净但违反 brief 的"只动 ci.yml / git status 只动 ci.yml"约束。

**选 (a)**：brief 明确要求 `git status 确认只动 ci.yml`，故不能新建文件。cron 触发全 workflow 的代价可接受——Vane 是 public repo（CI 分钟免费），周度全量跑反而可作 flaky/回归检测。fuzz-long 用 `if:` 门控只在 schedule/dispatch 跑，fuzz-smoke 用 `if: event_name != schedule` 避免与 fuzz-long 重复。现有 16 job 无 `if:` 门控——cron 时也会跑，但其 job 定义不变（`on:` 块是 workflow trigger，非 job 定义），满足"不改现有 job"约束。

## 3. 5 个新 job 设计

### 3.1 fuzz-smoke

| 属性 | 值 |
|---|---|
| trigger | push/PR + workflow_dispatch（`if: github.event_name != 'schedule'` 跳过 cron） |
| needs | 无（独立 nightly，不依赖 test job——fuzz 是不同验证维度，并行省时） |
| timeout | 15min |
| toolchain | nightly-2026-07-01（pin，`dtolnay/rust-toolchain@master` + `toolchain:` 输入） |
| steps | checkout → nightly → rust-cache → `cargo install cargo-fuzz --locked` → 循环 5 targets × `-max_total_time=60 -max_len=4096`（working-directory: crates/vane-fuzz） |

5 fuzz targets：brute_search_fuzz / hnsw_search_fuzz / persist_roundtrip_fuzz / merge_fuzz / dict_load_fuzz（Phase 1 已建）。

### 3.2 fuzz-long

| 属性 | 值 |
|---|---|
| trigger | cron `0 3 * * 0` + workflow_dispatch（`if: ${{ github.event_name == 'schedule' \|\| github.event_name == 'workflow_dispatch' }}`） |
| needs | 无（独立 nightly） |
| timeout | 60min |
| toolchain | nightly-2026-07-01 |
| steps | checkout → nightly → rust-cache → install cargo-fuzz → 循环 5 targets × `-max_total_time=600 -max_len=65536`（`\|\| true` 容错） → upload-artifact（`if: always()` + `if-no-files-found: ignore`） |

**`|| true` + `if: always()` 取舍**：设计 §3.2 草案用 `|| true` 容错 + `if: failure()` 上传——但 `|| true` 使 step 永不失败，`if: failure()` 永不触发，矛盾。改为 `if: always()` + `if-no-files-found: ignore`：无 crash 时不上传、不失败；有 crash 时上传 artifact 供人工分析。crash artifact 路径 `crates/vane-fuzz/fuzz/artifacts/`（cargo-fuzz 默认 artifact 目录）。

### 3.3 compat（跨版本兼容）

| 属性 | 值 |
|---|---|
| trigger | push/PR/schedule/dispatch（无 `if:` 门控） |
| needs | test |
| timeout | 15min |
| toolchain | stable |
| steps | checkout → stable → rust-cache → `cargo test --test cross_version_compat -p vane-core --all-features --release` |

**冗余取舍：独立 job（选 a）vs 合并 corpus-compat（选 b）**：选 **(a) 独立 job**。
- 理由 1：DoD 列 5 job（fuzz-smoke/fuzz-long/compat/stress/crash-recovery），独立 job 让 DoD 检查清单 1:1 映射到 CI job，可审计。
- 理由 2：cross_version_compat（跨版本 fixture 读取）与 corpus_compat（同版本 round-trip）是不同关注点，独立失败信号更清晰。
- 理由 3：`--release` + `--all-features` 是 test job（debug + all-features）的 release 增强。
- 与 test job 的冗余：test job 已跑 cross_version_compat（debug + all-features）。本 job 加 --release（release 优化下跨版本读取一致性）。`--all-features` 覆盖 `cross_version_compat.rs:335 #[cfg(feature="zstd-encode")]` 分支。

### 3.4 stress（并发压测）

| 属性 | 值 |
|---|---|
| trigger | push/PR/schedule/dispatch |
| needs | test |
| timeout | 20min |
| toolchain | stable |
| steps | checkout → stable → rust-cache → 循环 3 次 `cargo test --test stress_concurrency -p vane-core --release` |

**冗余取舍**：test job 已跑 stress_concurrency（debug，单次，all-features）。本 job 价值增量：
- `--release`：验 release 优化下并发安全（debug 的 Mutex 断言在 release 可能被优化，需独立验证）。
- 3 次 multi-run：降低低概率竞态的 flaky 漏检（单次跑可能恰好没命中竞态窗口）。
- default features（不加 --all-features）：stress 测试关注并发安全，与存储格式（zstd）正交。brief 指定 `cargo test --test stress_concurrency --release`（无 --all-features）。

### 3.5 crash-recovery（崩溃恢复）

| 属性 | 值 |
|---|---|
| trigger | push/PR/schedule/dispatch |
| needs | test |
| timeout | 15min |
| toolchain | stable |
| steps | checkout → stable → rust-cache → `cargo test --test crash_recovery -p vane-core --features fault-injection --release` |

**冗余取舍**：test job 已跑 crash_recovery（all-features 含 fault-injection，debug）。本 job 加 --release（release 优化下崩溃恢复一致性）。crash_recovery.rs 门控 `#![cfg(feature = "fault-injection")]` → 须 `--features fault-injection`。**仅 fault-injection，不需 zstd-encode**：crash 场景用 v1 stored 格式（`crash_recovery.rs:555 assert_eq!(ver, 1, ...format_version=1)`），本地 `cargo test --test crash_recovery --features fault-injection --no-run` 编译验证通过。

## 4. 新 job 清单

| # | job | needs | trigger | timeout | toolchain | 核心命令 |
|---|---|---|---|---|---|---|
| 17 | fuzz-smoke | — | push/PR/dispatch（skip cron） | 15min | nightly-2026-07-01 | `cargo fuzz run <t> -- -max_total_time=60 -max_len=4096` ×5 |
| 18 | fuzz-long | — | cron Sun 03:00 + dispatch | 60min | nightly-2026-07-01 | `cargo fuzz run <t> -- -max_total_time=600 -max_len=65536 \|\| true` ×5 |
| 19 | compat | test | push/PR/cron/dispatch | 15min | stable | `cargo test --test cross_version_compat --all-features --release` |
| 20 | stress | test | push/PR/cron/dispatch | 20min | stable | `cargo test --test stress_concurrency --release` ×3 |
| 21 | crash-recovery | test | push/PR/cron/dispatch | 15min | stable | `cargo test --test crash_recovery --features fault-injection --release` |

现有 16 job（fmt/clippy/test/recall/wasm32-check/deny/corpus-compat/cold-start/wasm32-size/dict-size/dict-hash/jieba-compat/ndcg-wiki/go-host/go-cross/wasm-recall）定义不变。

## 5. 自证

### 5.1 YAML 语法验证

```
$ python3 -c "import yaml; d = yaml.safe_load(open('.github/workflows/ci.yml')); print('jobs:', list(d['jobs'].keys()))"
OK — top-level keys: ['name', True, 'permissions', 'concurrency', 'env', 'jobs']
jobs: ['fmt', 'clippy', 'test', 'recall', 'wasm32-check', 'deny', 'corpus-compat',
'cold-start', 'wasm32-size', 'dict-size', 'dict-hash', 'jieba-compat', 'ndcg-wiki',
'go-host', 'go-cross', 'wasm-recall', 'fuzz-smoke', 'fuzz-long', 'compat', 'stress',
'crash-recovery']
```
（`True` 键是 YAML 将 `on:` 解析为布尔 True key 的已知行为，GitHub Actions 正确处理。）

```
$ yamllint -d '{rules: {line-length: disable, document-start: disable, comments-indentation: disable, trailing-spaces: disable, indentation: {spaces: 2}}}' .github/workflows/ci.yml
（无输出，exit 0——yml 干净）
```

### 5.2 diff 确认

```
$ git diff --stat .github/workflows/ci.yml
 .github/workflows/ci.yml | 124 +++++++++++++++++++++++++++++++++++++++++++++++
 1 file changed, 124 insertions(+)
```

纯追加（`on:` 块编辑在旧内容后追加新行，git 视为纯 insertions；5 job 全新）。无现有 job 行被删除/修改。

### 5.3 crash_recovery 本地编译验证

```
$ cargo test --test crash_recovery --features fault-injection --no-run -p vane-core
   Compiling vane-core v0.2.0
    Finished `test` profile [unoptimized + debuginfo] target(s) in 5.41s
  Executable tests/crash_recovery.rs
```
确认 `--features fault-injection`（无 zstd-encode）可编译 crash_recovery.rs。

## 6. 自审

### 6.1 CI 分钟预算

| 事件 | 新增 job 耗时（墙钟） | 说明 |
|---|---|---|
| push/PR | fuzz-smoke(15) ∥ compat(15) ∥ stress(20) ∥ crash-recovery(15)，均 needs test 或独立 | critical path: test(10) → stress(20) ≈ 30min；fuzz-smoke 独立并行 15min。总新增墙钟 ≈ 30min |
| cron 周日 | 全 21 job 跑 | cold-start(60) ∥ fuzz-long(60) 为最长链；public repo CI 免费 |
| workflow_dispatch | 全 21 job 跑 | 同上，手动触发 |

push/PR 新增 ≈30min 墙钟（并行），可接受。cron 周度 60min 最长链，public repo 免费。

### 6.2 nightly pin

- pin `nightly-2026-07-01`（brief 示例日期）。pin 避免 nightly break 中断 CI。
- `dtolnay/rust-toolchain@master` + `toolchain: nightly-YYYY-MM-DD` 是该 action pin 特定 nightly 的标准用法（`@stable`/`@nightly` tag 自选 toolchain 不读 `toolchain:` 输入——README 明确 "you'll want to use @master as the revision"）。
- 后续可周期性 bump pin 日期（如季度）。fuzz-long `|| true` 容错 + artifact 上传，即使 nightly 有小 regression 也不阻断。

### 6.3 与现有 job 风格对齐

- checkout@v4 / rust-toolchain / Swatinem/rust-cache@v2 三件套：对齐现有所有 job。
- `needs: test` 链：compat/stress/crash-recovery 对齐 corpus-compat/cold-start/jieba-compat 等的 `needs: test` 模式。
- timeout-minutes：对齐现有 job 的 10-60min 区间。
- 注释风格：中文 + SPEC/§ 章节引用 + 取舍说明，对齐现有 job 注释风格。
- fuzz-smoke/fuzz-long 用 nightly（`@master` + toolchain），与现有 stable job 区分；fuzz job 无 needs（独立），与 brief "fuzz job 不依赖现有 test job" 一致。

### 6.4 已知限制 / 后续

1. **cron 触发全 workflow**：`schedule` 加到顶层 `on:` 后，cron 时现有 16 job 也会跑。这是单文件实现的代价（brief 约束"只动 ci.yml"）。若后续允许独立文件，可拆 `fuzz-long.yml` 隔离。
2. **cargo-fuzz 未 pin 版本**：`cargo install cargo-fuzz --locked` 装最新版。后续可 pin `--version 0.13.0` 提升可复现性。
3. **crash artifact 路径**：`crates/vane-fuzz/fuzz/artifacts/` 是 cargo-fuzz 默认 artifact 目录。若 vane-fuzz crate 的 standalone fuzz manifest 模式下路径不同，`if-no-files-found: ignore` 保证不失败（但 crash 不会被上传）——首次 fuzz-long 发现 crash 时需验证路径。
4. **nightly-2026-07-01 有效性**：本地未安装 nightly 验证（避免装 nightly）。若该日期 nightly 不存在或 cargo-fuzz 不兼容，CI 首次跑会失败——需 bump 日期。这是 yml-only 改动的固有限制（本地无法跑 GitHub Actions）。

## 7. commit

- 文件：`.github/workflows/ci.yml`（仅此一个文件）。
- 分支：`feat/m4-prod-readiness`。
- 提交信息：`ci: 新增 fuzz-smoke/fuzz-long/compat/stress/crash-recovery job（M4 阶段六 a）`。
- 不 push，不 Co-Authored-By。
