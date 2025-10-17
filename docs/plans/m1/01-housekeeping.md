# 阶段零-B：清理（FF5 benchmark 回退门禁 + parked 次要项）派发简报

> 这是你（housekeeping SubAgent）的需求文档。你是 Rust 实现者，遵循 TDD + 小步增量提交。
> 完成后自证全量门禁，报告写入 `docs/plans/m1/01-housekeeping-report.md`。

## 背景

Vane M0 已完成，阶段零-A（格式冻结）已完成并通过门禁（HEAD c287458）。本任务是阶段零收尾的 housekeeping：修复 FF5 benchmark 回退门禁 + 清理 M0 各模块 parked 次要项。不实现任何 M1 功能。

## 环境

- 工作目录：`/Users/ximing/project/mygithub/vane`（main，HEAD c287458）。
- 直接在 main 工作，每项小步 commit。
- 全程中文。

## 范围

### A. FF5：benchmark.yml 回退门禁修复（中等，必做）

**问题**：`.github/workflows/benchmark.yml` 用 `git worktree add ../vane-main main` 在对侧目录跑 main baseline（`--save-baseline main`），但 criterion baseline 存在各自 worktree 的 `target/criterion`，repo 根的 `critcmp main current` 读不到对侧 baseline → 回退门禁实际失效（`|| true` + python 容错兜底 exit 0 掩盖）。

**目标**：让回退>10% 报警真正生效。修法（择一，你判断更稳健的）：
- 方案 1（criterion 原生 --baseline）：在同一 checkout 顺序跑 main 与 current，main 用 `--save-baseline main`，current 用 `--baseline main`（criterion 在同一 target/criterion 比较）。需在单次 job 内切换 checkout（main → 跑 → 切回触发分支 → 跑）。注意保留触发分支上下文。
- 方案 2（同目录 critcmp）：两个 baseline 都在 repo 根 `target/criterion`，用 `--save-baseline main` / `--save-baseline current`，再 `critcmp main current`（critcmp 默认读 `target/criterion`）。
无论哪个方案，移除掩盖失败的 `|| true`（除非 critcmp 自身退出码不可靠，则保留 `|| true` 但确保 python 脚本解析正确并在回退时 exit 1）。验证 `scripts/check-bench-regression.py` 的 regex 能匹配 criterion/critcmp 实际输出格式（M0 记录其 regex 不匹配 ASCII `us` 单位——顺手修）。

> 注意：benchmark.yml 是 `schedule` + `workflow_dispatch` 触发，不在本地实跑（耗时长）。本地只验证 `check-bench-regression.py` 能正确解析一份样例 compare.txt 并在回退>10% 时 exit 1、不回退时 exit 0。构造样例输入测试该脚本。

### B. parked 次要项（低风险，逐项小步 commit + 测试）

来源 `docs/plans/m0/EXECUTION-NOTES.md` 与 `docs/plans/m0/M0-SUMMARY.md` §3.4。逐项处理，每项跑 `cargo test -p vane-core` 确认无回归：

**01-vfs**（`crates/vane-core/src/vfs/`）：
- P1：`memory.rs` 约 117 行注释 "I11" 应为有效不变量编号（I-1~I-8 无 I11）或改描述性文字。
- P2：`MemoryVfs::list` 排序与 `StdFsVfs::list` 不一致——统一（都排序或都不排序，conformance 测试用 `.contains()` 不受影响，但一致性更好）。
- P3：`PageCache::put`（若有）加同 key 去重防御；当前调用流无此路径，防御性即可。
- P4：`StdFsVfs::resolve` 每次 `create_dir_all`——加缓存（生产化）。注意不改 pub 签名。

**02-tokenizer**（`crates/vane-core/src/tokenizer/`）：
- cjk_bigram.rs 仍用 `let mut position`，与 standard 的 zip 写法不一致——统一写法（无功能问题，clippy 未告警）。

**03-fusion**（`crates/vane-core/src/fusion/`）：
- P3：NaN 测试命名 "rejected" 略误导（NaN 非首元素仍 NaN，但调用方契约不含 NaN）——改名或加注释。
- P4：模块文档措辞 `vane_core::types` 与代码 `crate::types` 不一致——统一。

**04-segment**（来自阶段零-A 疑问①）：
- `crates/vane-core/tests/corpus_compat.rs` 的 `corpus_segment_files_have_magic_version_headers` 补 `inverted.bin` 头校验（据 M0 README `write_inverted` 格式 `magic|version|...`，inverted.bin 已有头）。先 Read `crates/vane-core/src/bm25.rs` 确认 `write_inverted` 确实写 magic+version 头再补测试。

**07-api-core**（`crates/vane-core/src/api/`，谨慎，带测试）：
- search 循环内重复 `vector_field()` 调用——hoist 到循环外（perf + 清晰，无语义变化）。
- `wrapping_sub` → `checked_sub`（更安全，带测试覆盖下溢路径）。
- `inv_readers[i]` 索引对齐脆弱——改 `zip` 迭代（无语义变化，带测试）。
- auto-commit flush 吞错——改为 log 或 `AddReport` 暴露失败标志（评估：若改 AddReport 字段是 pub API 变更，**停下标记交编排者**，不要擅自改 pub API；若仅 log 则可）。
- restore 累加 base 未读段头——若确认为真实 bug，**带测试修复**（TDD：先写复现测试 → 修）；若不确定是否 bug，标记交编排者。
- **排除**：recall 硬编码 1.0（M1 HNSW 真实回归 job）、I2 未校验 auto_commit 差异（M1）。

**09-node-binding**（`crates/vane-node/`）：
- `scripts/check-thin.sh` 注释排除管道冗余——清理注释。
- `[profile.release]` 移除后 release 无 LTO——评估是否补 LTO（若补，确认不影响 napi build）。

**10-ci-gates**：
- `install-matrix.yml` `workflow_run` version 回退 '0.1.0'——修为正确版本读取（从 package.json 或 tag）。
- `check-bench-regression.py` regex 不匹配 ASCII `us` 单位——修（与 A 项一起）。
- `crates/vane-core/benches/hybrid_search.rs` 约 71-85 行冗余死代码——删除。

## 自证门禁（全部须绿，clippy 含 --all-targets）

```
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo check --target wasm32-unknown-unknown -p vane-core
cargo fmt --all -- --check
bash scripts/check-no-std-fs.sh
bash crates/vane-node/scripts/check-thin.sh
cargo test --test corpus_compat -p vane-core
# 额外：本地验证 check-bench-regression.py 解析样例 compare.txt 正确（回退>10% exit 1，否则 exit 0）
```

## 全局约束

- core 禁 std::fs/std::net/mmap；cfg 只在 VFS/Executor；不引入 dashmap/parking_lot/黑名单依赖。
- **不得改 M0 已冻结 pub API 签名**（Vfs trait、Schema、SegmentReader/Writer、brute_search、Collection API 等）。若某 parked 项需要改 pub API，停下标记交编排者。
- 冻结常量（BM25 k1/b、RRF k、段数上限、dim、DOC_MAX、topK、用户词表上限）不得改。
- 不变量 I-1~I-8 不得违反。
- MoSCoW 即合同：不新增需求。不碰 HNSW/jieba/tombstone/WAL/Go/FF4 严格化/stored zstd。
- 全程中文。

## 排除项（不要做）

- recall 硬编码 1.0 → M1 HNSW job。
- FF4 严格解码加严（dim 推导/stored 截断）→ M1。
- stored.bin zstd → M1。
- 任何 M1 功能。

## 报告

完成后写入 `docs/plans/m1/01-housekeeping-report.md`：每项实际改动（文件:行）、偏离与裁决、自证门禁结果、提交 hash、遗留/疑问（尤其标记需要编排者裁决 pub API 变更的项）。最终回复只返回：状态、提交 hash 列表、一句话测试摘要、需裁决项。

## 红线

- 不改 pub API；不确定就停下问。
- 每项小步 commit（message 中文 + ``）。
- 若某项越改越复杂或带回归风险，跳过并在报告标记，不要硬改。
