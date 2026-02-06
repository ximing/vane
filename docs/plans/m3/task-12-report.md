# Task 12 Report — bump version 0.1.2→0.2.0

## 状态

✅ 完成（commit `ca19dc5`，分支 `feat/m3-web-npm`）

## Commits

- `ca19dc5` chore(release): bump 0.1.2→0.2.0（Cargo workspace + @vane-rs/node + install-matrix）

## 一行测试摘要

cargo build --workspace ✓ / cargo fmt --check ✓ / cargo clippy -D warnings ✓ / cargo test --workspace --all-features ✓ / npm test 17/17 ✓

## 改动文件清单（4 文件，11 行改动）

| 文件 | 改动 |
|---|---|
| `Cargo.toml`（root） | `[workspace.package] version` "0.1.2" → "0.2.0" |
| `Cargo.lock` | 4 条 version 同步：vane-core / vane-ffi / vane-node / vane-wasm 各 0.1.2→0.2.0（cargo build 自动重生成，无依赖版本变化） |
| `crates/vane-node/package.json` | 主包 `version` + 4 平台包 `optionalDependencies`（@vane-rs/node-linux-x64-gnu / -darwin-arm64 / -darwin-x64 / -win32-x64-msvc）共 5 处 0.1.2→0.2.0 |
| `.github/workflows/install-matrix.yml` | `inputs.version` default "0.1.2" → "0.2.0" |

### version 来源确认（开工前 Read 核对）

- root `Cargo.toml`：`[workspace.package] version` = workspace 继承源 ← 改此一处
- `crates/vane-core/Cargo.toml`：`version.workspace = true` ← 自动继承，不改
- `crates/vane-ffi/Cargo.toml`：`version.workspace = true` ← 自动继承，不改
- `crates/vane-node/Cargo.toml`：`version.workspace = true` ← 自动继承，不改
- `crates/vane-wasm/Cargo.toml`：`version.workspace = true` ← 自动继承，不改
- `crates/vane-dict-zh/Cargo.toml`：`version = "2026.8.0"` 硬编码 ← 日历版不动（Task 5 已设）
- `bindings/web/package.json`：`version = "0.2.0"` ← Task 2 已设，不动

### 对齐 post-M2 bump 0.1.1→0.1.2 模式（commit 03d3ac2）

上次 bump 改了 4 个文件（install-matrix.yml + Cargo.lock + Cargo.toml + vane-node/package.json），本次同模式。上次 Cargo.lock 变 131 行因 napi 3.x 配对版本同步；本次仅 version bump，Cargo.lock 只变 4 条 version 字段（8 行），干净。

## 残留 0.1.2 检查结果

### 代码/配置文件（*.toml/*.json/*.yml/*.yaml/*.rs/*.go/*.js/*.ts/*.h）

仅 1 处：
- `.github/workflows/release.yml:10` `description: '版本号（如 0.1.2）'` — workflow_dispatch input 的 description 示例文字（非 default，`required: true` 无 default）。按任务指令「release.yml 不硬编码 version（npm publish 读 package.json），无需改」不改。npm publish 流程不读此 description，不影响发版。上次 03d3ac2 实际也未改此行（尽管 post-v0.1.1/napi-cli-3x-plan.md:148 曾标注 "cosmetic 顺手" 改 description 示例，实际 commit 未包含 release.yml）。

### *.md 文件（排除 docs/plans/）

无残留。README badge 未硬编码版本号（npm version badge 动态读取 npm registry latest，不硬编码）。

### docs/plans/（不改）

多处引用 0.1.2 作为历史/上下文（M3-PLAN.md 阶段零发版闭环、PROGRESS.md v0.1.2 发版证据、post-v0.1.1/napi-cli-3x-plan.md 历史 bump 0.1.1→0.1.2 计划）——编排者产出 + 历史记录，不改。

### docs/SPEC.md changelog

未发现 0.1.2 硬编码（Task 13 将加 v0.2.0 changelog）。

## 冻结约束遵守

- `crates/vane-wasm/` 的 .rs 文件：未改（仅 Cargo.toml version 经 workspace 自动继承，未手改 Cargo.toml）
- `bindings/web/src/`：未改
- `crates/vane-dict-zh/src+data/`：未改
- MoSCoW Won't-have：未触碰

git diff --name-only 确认改动文件仅 4 个（install-matrix.yml / Cargo.lock / Cargo.toml / vane-node/package.json），无任何 .rs 文件。

## Concerns

1. **release.yml:10 description 示例文字 "如 0.1.2" 未改**：按任务指令不改（release.yml 不硬编码 version，npm publish 读 package.json）。该 description 是 workflow_dispatch input 的提示文字，非 default 值，不影响发版功能。如编排者希望保持示例版本号与当前版本一致，可顺手改为 "如 0.2.0"（cosmetic，1 行改动）。上次 03d3ac2 也未改此行。

2. **工作区有编排者已有的未提交修改**：`docs/plans/m3/task-1-design.md`（unstaged）+ 多个未跟踪 `docs/plans/m3/*` 文件（M3-PLAN.md / PROGRESS.md / 各 task report/review-package）。这些是编排者产出，非 Task 12 范围，未 commit。Task 12 commit 只含 4 个 version-bump 文件。
