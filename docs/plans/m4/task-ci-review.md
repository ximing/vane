# M4 阶段六 a：CI 新增 5 job — Task Reviewer 审查报告

> 审查人：task reviewer SubAgent（sonnet，只读）。
> 审查对象：commit b4aa743 `ci: 新增 fuzz-smoke/fuzz-long/compat/stress/crash-recovery job（M4 阶段六 a）`。
> 审查范围：spec 合规 + 设计质量（cron-tradeoff / 不破现有 job / 冗余）+ yml 语法/结构 + fuzz job 设计 + concerns 定性。
> 审查依据：M4-PLAN 阶段六 DoD + phase0-design §3.2 草案 + task-ci-report.md + ci.yml 全文件（450 行）。

## A. Spec 合规：✅

| DoD 项（M4-PLAN §69 + DoD §81） | 实现 | 验证 |
|---|---|---|
| fuzz-smoke（每 target 60s，push/PR） | `ci.yml:339-364`，5 targets × `-max_total_time=60 -max_len=4096` | ✅ |
| fuzz-long（cron + workflow_dispatch） | `ci.yml:366-395`，5 targets × `-max_total_time=600 -max_len=65536`，cron `0 3 * * 0` | ✅ |
| compat job | `ci.yml:397-412`，`cross_version_compat --all-features --release` | ✅ |
| stress job | `ci.yml:414-432`，`stress_concurrency --release` ×3 multi-run | ✅ |
| crash-recovery job | `ci.yml:434-449`，`crash_recovery --features fault-injection --release` | ✅ |
| 顶层 on: 加 schedule + workflow_dispatch | `ci.yml:9-14` | ✅ |
| fuzz-smoke/fuzz-long 用 nightly + cargo-fuzz | `nightly-2026-07-01` + `cargo install cargo-fuzz --locked` | ✅ |
| compat/stress/crash-recovery 用 --release | 三 job 均 `--release`（比 test job debug 更彻底） | ✅ |

**Spec 合规结论**：5 job 覆盖 M4-PLAN 阶段六 DoD 全部要求，nightly + cargo-fuzz + max_total_time + --release + schedule/workflow_dispatch 全到位。

## B. Findings

### Critical：无

### Important：无

### Minor

1. **cron 触发全 workflow（非仅 fuzz-long）** | `ci.yml:9-13` + 现有 16 job 无 `if:` guard
   | schedule 加顶层 `on:` → 周日 03:00 UTC ALL 21 job 跑（含现有 16 job 无 `if: != schedule` 门控）。
   | 失败场景：cron 跑现有 16 job 产生 flaky 失败噪音（邮件通知、红 X）；CI 分钟浪费（public repo 免费但仍有运行时开销）。
   | **定性**：acceptable Minor。public repo ubuntu-latest 标准运行器免费无限制；周度全量回归/flaky 检测本身有价值（early warning）；implementer 已文档化取舍；brief 约束"git status 只动 ci.yml"阻止拆 `fuzz-long.yml` 隔离。后续若需隔：拆独立 workflow 文件 或 给 16 现有 job 加 `if: github.event_name != 'schedule'`（仍只动 ci.yml）。

2. **concurrency group schedule/push 共享** | `ci.yml:19-21`
   | `group: ci-${{ github.ref }}` + `cancel-in-progress: true`。schedule 触发时 `github.ref = refs/heads/main`，与 push-to-main 同 group。周日 03:00 UTC cron 跑中若有人 push 到 main → cron run 被 cancel（含 fuzz-long）。
   | 失败场景：fuzz-long 被中途 cancel，周度 fuzz 覆盖静默缺失。
   | **定性**：Minor。周日 03:00 UTC push 概率低 + 次周自愈 + 人工 dispatch 可补。后续可改 group 为 `ci-${{ github.event_name }}-${{ github.ref }}` 让 schedule/push 不共享。

3. **nightly pin 未本地验证** | `ci.yml:355,379`
   | `nightly-2026-07-01` 未本地安装验证；若该日期 nightly 不存在或 cargo-fuzz 不兼容，CI 首次跑失败。
   | 失败场景：nightly 不存在 → fuzz-smoke + fuzz-long 均 toolchain install 失败。
   | **定性**：acceptable Minor defer。2026-07-01 距今 ~6 周，nightly 日期合理存在；fuzz-smoke（无 `|| true`）首次 push/PR 即验证 nightly 兼容性，失败即报警；bump 日期一行修复。

4. **fuzz-long `|| true` 吞编译错误** | `ci.yml:386`
   | `cargo fuzz run $target -- ... || true` 吞 ALL 非零退出（含 crash + 编译失败 + nightly 不兼容）。nightly break 时 fuzz-long 静默成功（step exit 0），不报警。
   | 失败场景：nightly break → fuzz-long 静默成功 → 周度 fuzz 覆盖静默缺失，用户无感知。
   | **定性**：acceptable Minor。fuzz-smoke（同 nightly pin，无 `|| true`）下次 push/PR 即捕获 nightly break 并失败报警——fuzz-smoke 是 gate，fuzz-long 是 explorer，职责分离设计正确。若需更稳：拆 `cargo fuzz build` + `cargo fuzz run`，编译失败 `continue`、crash `|| true`。

5. **fuzz-long timeout 60min 紧** | `ci.yml:374`
   | 5 targets × 600s = 3000s = 50min fuzz + cargo-fuzz install (~1-2min) + 5 targets 首次编译 (~5-10min，无 cache) ≈ 56-62min。60min timeout 紧。首次无 cache 可能 timeout。
   | 失败场景：首次 fuzz-long timeout → job killed → `if: always()` upload 步骤不执行（GitHub Actions timeout kill 非 step failure，`if: always()` 不触发）→ crash artifact 丢失。
   | **定性**：Minor。后续 cache hit 编译快，50min fuzz + ~3min overhead = 53min < 60min 应够。首次可手动 dispatch 验证或调 70-75min 留余量。

6. **cargo-fuzz 未 version-lock** | `ci.yml:358,382`
   | `cargo install cargo-fuzz --locked` 装最新版（`--locked` 用 Cargo.lock 锁传递依赖，但不锁 cargo-fuzz 自身版本）。未来 cargo-fuzz 新版改 CLI/行为 → 不可复现。
   | 失败场景：cargo-fuzz 0.14 改 CLI flag → `cargo fuzz run` 命令失败。
   | **定性**：Minor。后续 pin `--version 0.13.0` 提升可复现性。implementer 已在 concerns §6.4.2 记录。

7. **crash artifact 路径未运行时验证** | `ci.yml:394`
   | `path: crates/vane-fuzz/fuzz/artifacts/` 是 cargo-fuzz 默认 artifact 目录。若 vane-fuzz standalone fuzz manifest 模式下路径不同 → 上传空。
   | 失败场景：路径错 → `if-no-files-found: ignore` 不失败，但 crash artifact 不上传 → 首次 crash 丢失。
   | **定性**：acceptable Minor。`if-no-files-found: ignore` 是安全网（不阻断 job）。首次 fuzz-long 发现 crash 时需验证路径。implementer 已在 concerns §6.4.3 记录。

8. **`if:` 表达式风格不一致** | `ci.yml:345,372`
   | fuzz-smoke `if: github.event_name != 'schedule'`（裸表达式）；fuzz-long `if: ${{ github.event_name == 'schedule' || ... }}`（`${{ }}` 包裹）。两者均合法 GitHub Actions 语法。
   | **定性**：Cosmetic Minor。不影响功能。

## C. cron-tradeoff 定性

**acceptable Minor**（不须拆文件或加 guard）。

理由：
1. **public repo CI 免费**：GitHub Actions 对 public repo 的 ubuntu-latest 标准运行器提供无限免费分钟。无成本浪费。
2. **周度全量回归/flaky 检测有价值**：现有 16 job（含 cold-start 60min、wasm-recall 30min、go-cross 30min）周度跑可作 early warning——若某 job 在非 push/PR 时段 flaky 失败，是潜在回归信号。
3. **brief 约束阻止干净隔离**：brief 明确"git status 只动 ci.yml"，阻止新建 `fuzz-long.yml` 独立 workflow 文件。单文件内实现 cron-only 的唯一路径是顶层 `schedule` + job-level `if:` 门控——但给 16 现有 job 加 `if:` guard 技术上"修改现有 job 定义"，与"不破现有 job"有张力。implementer 选不加 guard 保持现有 job 定义不变，是更保守的选择。
4. **implementer 文档化充分**：`ci.yml:10-12` 注释 + report §2 + §6.4.1 三处记录此取舍，可审计。
5. **后续可逆**：若噪音过大，可拆 `fuzz-long.yml` 或加 `if:` guard，均一行级改动。

## D. 不破现有 job 定性

**确认（不破）**。

验证项：
1. **push/PR 触发不变**：`on:` 块加 `schedule` + `workflow_dispatch` 是纯追加（diff +124 insertions, 0 deletions）。`push`（branches + paths-ignore）和 `pull_request`（paths-ignore）定义原封不动。push/PR 触发行为与之前完全一致。
2. **现有 16 job needs 链不变**：fmt → clippy → test → [recall/wasm32-check/corpus-compat/cold-start/wasm32-size/dict-size/dict-hash/jieba-compat/ndcg-wiki/go-host→go-cross/wasm-recall]。新 job（compat/stress/crash-recovery）加 `needs: test` 是新增依赖，不改现有 job 的 `needs` 字段。
3. **fuzz-smoke/fuzz-long `if:` 隔离正确**：
   - push/PR：fuzz-smoke `if: github.event_name != 'schedule'` → true → 跑；fuzz-long `if: schedule || dispatch` → false → 跳。✅ push/PR 跑 fuzz-smoke 不跑 fuzz-long。
   - schedule：fuzz-smoke → false → 跳；fuzz-long → true → 跑。✅ cron 跑 fuzz-long 不跑 fuzz-smoke。
   - workflow_dispatch：fuzz-smoke → true → 跑；fuzz-long → true → 跑。✅ 手动触发两者都跑（用户自主选择）。
4. **concurrency 不变**：`group: ci-${{ github.ref }}` + `cancel-in-progress: true` 定义未改。push/PR 之间的 cancel 行为不变。唯一新增交互：schedule 与 push 共享 group（见 Minor #2），但不影响 push/PR 自身行为。

## E. 冗余定性

**acceptable（非 waste）**。

| 新 job | 与 test job 重叠 | 增量价值 | 定性 |
|---|---|---|---|
| compat | test 已跑 cross_version_compat（debug + all-features） | --release（release 优化下跨版本读取） + 独立失败信号（cross-version vs same-version round-trip 分离） + DoD 5 job 1:1 映射可审计 | acceptable |
| stress | test 已跑 stress_concurrency（debug，单次，all-features） | --release（release 优化下 Mutex 断言可能被优化，需独立验） + 3× multi-run（降低低概率竞态 flaky 漏检） | acceptable |
| crash-recovery | test 已跑 crash_recovery（all-features 含 fault-injection，debug） | --release（release 优化下崩溃恢复一致性） + 独立失败信号 | acceptable |

三 job 的 --release 增量是真实价值（debug vs release 优化差异可能暴露不同行为，Mutex 断言在 release 可能被优化掉）。stress 的 3× multi-run 捕捉低概率竞态是正当增量。独立失败信号（compat vs corpus-compat 分离、crash-recovery vs test 分离）提升诊断可见性。不冗余。

**feature gate 验证**（reviewer 补查源码）：
- `stress_concurrency.rs`：无 `#![cfg(feature = ...)]` 文件级门控 → default features 编译 OK。✅
- `cross_version_compat.rs:335`：`#[cfg(feature = "zstd-encode")]`（行级，非文件级）→ `--all-features` 覆盖。✅
- `crash_recovery.rs:1`：`#![cfg(feature = "fault-injection")]`（文件级）→ `--features fault-injection` 覆盖，implementer `--no-run` 本地编译验证通过。✅

## F. yml 语法 + 结构定性

**clean（pyyaml + yamllint 验证 + reviewer 复核）**。

1. **pyyaml parse**：21 job key 正确（16 现有 + 5 新）。`True` 键是 YAML 将 `on:` 解析为布尔 True 的已知行为，GitHub Actions 正确处理。
2. **yamllint clean**：implementer 用 relaxed config 跑通过。reviewer 复核：缩进 2-space 一致，无 trailing space，无结构错误。
3. **job 名唯一**：fuzz-smoke / fuzz-long / compat / stress / crash-recovery，与 16 现有 job 无冲突。
4. **needs 引用正确**：compat/stress/crash-recovery `needs: test`（test 存在于现有 16 job）；fuzz-smoke/fuzz-long 无 needs（独立，符合设计）。
5. **`if:` 表达式语法合法**：fuzz-smoke 裸表达式 + fuzz-long `${{ }}` 包裹，两者 GitHub Actions 均合法（风格不一致见 Minor #8）。
6. **steps 三件套对齐**：checkout@v4 / rust-toolchain / rust-cache@v2 与现有所有 job 风格一致。
7. **diff 纯追加**：+124 insertions, 0 deletions, 1 file。无现有 job 行被修改/删除。

## G. fuzz job 设计定性

| 维度 | 评估 | 详情 |
|---|---|---|
| timeout fuzz-smoke 15min | ✅ 够 | 5×60s=5min fuzz + install(~1min) + 编译(~5min with cache) ≈ 11min < 15min。首次无 cache 可能 ~13min 仍够。 |
| timeout fuzz-long 60min | ⚠️ 紧（Minor #5） | 5×600s=50min + install + 编译 ≈ 56-62min。首次无 cache 可能 timeout。后续 cache hit 应够。建议 70-75min 更稳。 |
| cargo-fuzz install | ✅ 正确 | `cargo install cargo-fuzz --locked` 标准安装。`--locked` 锁传递依赖。未 version-lock（Minor #6）。 |
| artifact 上传 fuzz-long | ✅ 设计正确 | `if: always()` + `if-no-files-found: ignore`。implementer 修正了 §3.2 草案 `if: failure()` 的矛盾（`|| true` 使 step 永不 failure → `if: failure()` 永不触发）。改为 `if: always()` 正确：有 crash 上传、无 crash 不上传不失败。唯一限制：timeout kill 时 `if: always()` 不执行（GitHub Actions timeout 是 job kill 非步骤失败）。 |
| working-directory | ✅ 正确 | fuzz run 步骤 `working-directory: crates/vane-fuzz`，cargo-fuzz 从 crate 根运行。artifact 上传步骤无 working-directory（默认 repo 根），path `crates/vane-fuzz/fuzz/artifacts/` 相对 repo 根——与 cargo-fuzz 在 crate 根下创建的 `fuzz/artifacts/` 路径一致。 |

## H. nightly pin 定性

**acceptable Minor defer**。

- `nightly-2026-07-01` 距今 ~6 周，nightly 日期合理存在（nightly 每日发布）。
- 本地未验证（implementer 避免 装 nightly）。首次 CI 跑验证。
- **break 检测兜底**：fuzz-smoke（同 nightly pin，无 `|| true`）下次 push/PR 即验证 nightly 兼容性。若 nightly 不兼容 cargo-fuzz，fuzz-smoke 失败 → PR 阻断 → 用户即知。fuzz-long（`|| true`）虽静默，但 fuzz-smoke 是 gate 兜底。
- `dtolnay/rust-toolchain@master` + `toolchain: nightly-YYYY-MM-DD` 是 pin 特定 nightly 的标准用法（`@nightly` tag 自选不读 `toolchain:` 输入）。
- 后续可周期 bump pin 日期（如季度）。

## I. ⚠️ 无法从 diff 验证项

以下项无法从纯 yml diff 静态验证，须 CI 实际执行：

1. **nightly-2026-07-01 + cargo-fuzz 实际兼容性**：需 CI 首次 fuzz-smoke（push/PR）或 fuzz-long（cron/dispatch）执行验证。若 nightly 不存在或 cargo-fuzz 不兼容该 nightly，CI step 失败。
2. **CI 实际执行行为**：GitHub Actions 的 schedule trigger 是否按预期周日 03:00 UTC 触发；workflow_dispatch 手动触发是否正常；fuzz-smoke `if:` 在 schedule 时是否正确 skip；fuzz-long `if:` 在 push/PR 时是否正确 skip——均须 CI 实跑验证。
3. **crash artifact 路径**：`crates/vane-fuzz/fuzz/artifacts/` 是否是 cargo-fuzz 实际写入 crash 文件的路径。须首次 fuzz-long 发现 crash 时验证（`if-no-files-found: ignore` 是安全网）。
4. **fuzz-long 首次 timeout**：首次无 cache 编译时长是否在 60min timeout 内。须首次 CI 执行验证。
5. **cron 时现有 16 job 的 flaky 表现**：周度全量跑现有 16 job 是否产生未预期的 flaky 失败。须首次 cron 执行后观察。

## J. 总体

**不进 fix 循环**。

- Spec 合规 ✅：5 job 覆盖 DoD 全部要求。
- Critical/Important：无。
- 8 项 Minor 全为 acceptable / defer / cosmetic，均被 implementer 文档化或在 concerns §6.4 记录。
- cron-tradeoff acceptable Minor（public repo 免费 + 周度 flaky 检测有价值 + brief 约束阻止干净隔离 + 后续可逆）。
- 不破现有 job 确认（push/PR 触发不变 + needs 链不变 + `if:` 隔离正确）。
- 冗余 acceptable（--release + multi-run + 独立信号是真实增量价值）。
- yml 语法/结构 clean。
- fuzz job 设计 sound（timeout/install/artifact 正确，fuzz-long timeout 紧但可接受）。
- nightly pin acceptable defer（fuzz-smoke gate 兜底检测 break）。
- 无法验证项须 CI 实跑确认，但均有安全网（fuzz-smoke gate / `if-no-files-found: ignore` / `|| true` 容错）。

**建议**：合入。后续观察首次 CI fuzz-smoke（push/PR）验证 nightly 兼容；首次 cron（周日）观察现有 16 job flaky 表现 + fuzz-long 是否 timeout。若 timeout 频发，调 fuzz-long timeout 至 70-75min；若 cron 噪音过大，拆 `fuzz-long.yml` 或给 16 job 加 `if: != schedule` guard。
