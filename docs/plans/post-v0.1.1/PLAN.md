# SDD ledger — plan: docs/plans/post-v0.1.1/PLAN.md

# post-v0.1.1 计划：f32 距离 SIMD128 向量化

基线（BASE）：`16c9fe44e8f3891531f079b3c4b35bedbffa2897`（main，v0.1.1 发版后）

## 背景

M2-05 落地 SIMD128 双变体（simd/scalar），但 f32 距离（cosine/L2/dot）未显式向量化：
simd 变体中距离循环靠 LLVM 自动向量化，因 FP 非结合性实际未向量化（见
`crates/vane-wasm/src/simd_probe.rs` 头注释与 M2-05 报告）。本任务用 wasm32 simd128
intrinsics（f32x4）显式向量化三 metric 的 f32 距离循环。

## 全局约束（Global Constraints）

- **数值一致性**：simd 与 scalar 双变体距离结果必须保证跨变体 top-10 集合一致
  （CI wasm-recall：recall@10≥0.95 且跨变体 Jaccard≥0.99；top-10 下 Jaccard≥0.99
  实际要求两变体 top-10 集合完全相同）。推荐策略：标量核与向量核采用相同的
  4 路累加归约顺序（scalar 也改为 4 累加器），使两变体逐位一致；由实现者验证。
- **I-5 修订（前置门禁）**：`cfg(target_feature="simd128")` 属 target 分支，须先获
  用户批准 SPEC v1.3→v1.4 修订（I-5 释义扩展：cfg(target_feature) 算法向量化允许，
  同一算法的向量化/标量两实现算能力开关而非平台分支），批准后才可动代码。
- core 禁 std::fs/std::net/mmap；cfg 仅 VFS/Executor + 本任务批准的 cfg(target_feature)。
- 依赖黑名单：regex/tokio/prost/tonic/openssl/lindera/ndarray/wee_alloc/dashmap/parking_lot；
  本任务不应新增任何依赖（直接用 `core::arch::wasm32` intrinsics）。
- 不改 M0/M1/M2/post-M2 冻结 pub API（`brute_search`/`sq8_distance` 等签名不变）。
- 词典永不进 wasm；双变体 ≤800KB gzip 红线。
- 遵循 rustfmt.toml；clippy `-D warnings` 通过。
- 门禁全集：cargo test --workspace --all-features / clippy --all-targets --all-features -D warnings /
  cargo fmt --check / wasm32 check（vane-core + vane-wasm）/ check-no-std-fs.sh /
  cargo deny check / wasm 双变体体积 / wasm-recall 双变体回归。

## 任务分解

- **T0 SPEC v1.4 修订批准**（orchestrator）：AskUserQuestion 检查点，未批准不动代码。
- **T1 f32 距离 SIMD128 向量化**：`vector/mod.rs` cosine/L2/dot 三核，
  `cfg(target_feature="simd128")` 双路径（f32x4 intrinsics / 标量），归约顺序对齐
  保证逐位一致；补充单元测试（含 simd 路径的 host 侧可测性评估——host 无 simd128
  target_feature 时走标量，wasm 侧由 wasm-recall 覆盖）。
- **T2 SQ8 距离评估**：SQ8 三 metric（`vector/sq8.rs`）是否共用/受益同一向量化；
  若代价低则同步向量化，否则记录为 carry-forward 并说明理由。
- **T3 双变体回归与体积**：本地跑 scripts/run-wasm-recall.sh（双变体 recall+Jaccard）
  与 scripts/build-wasm-variants.sh 体积检查（≤800KB gzip）。
- **T4 SPEC v1.4 文档落地**：SPEC §14 I-5 释义 + §6.2/§13.2 相关条目 + changelog；
  本计划总结报告（docs/plans/post-v0.1.1/ 下 SUMMARY）。
- **T5 全量门禁 + 最终整支 review**。

## 进度台账（ledger）

（任务完成/修复轮次/停放 findings 记录于此）

- T0: complete — 用户批准 SPEC v1.4 修订（I-5 释义扩展 cfg(target_feature) 算法向量化允许）+ 直接 main 提交（2026-08-10）
- T1: complete (commits 16c9fe4..5668479, review clean — Spec ✅ + Approved)
- T1: minor (deferred): ① acc4 小整数逐位测试 docstring 过度宣称（零舍入数据无法区分归位错误）② acc4_reduction_is_deterministic 近永真无回归价值 ③ SPEC v1.4 文档未落地前可追溯性依赖 T4；build-wasm-variants.sh:8 注释「core 算法零 cfg」已过时，T4 一并更新
- T1: ⚠️ 已闭环（T3）：simd 路径运行时逐位一致性经 wasm-recall 双变体门禁确认
- T3: complete — 体积 simd 400.3KB / scalar 403.1KB gzip（≤800KB ✅，simd 变体含 3192 行 simd128 指令、scalar 0）；run-wasm-recall 全绿：五档 recall=1.000、跨变体 30 查询 min Jaccard=1.000000 ✅（2026-08-10）
- T2: complete — 结论 carry-forward（无代码改动）。依据经编排者核实：vane-wasm/vane-node/vane-ffi 均未启用 sq8 feature，向量化即死代码、无运行时门禁背书；结构可行（0.5-1d），重触发条件见 task-2-report.md。遗留：sq8.rs 头注释 v1.3 口径待 T4 更新；SQ8 无交付物接线状态记入总结
- T4: review clean (Spec ✅ + Approved, commits 5668479..afd34ce)。Minor M2（其余 7 处「零 cfg(target)」旧措辞为事实性描述，保留）defer 至最终 review
- T4: fix round 1/5 — ① build-wasm-variants.sh 过时头注释（原 T1 Minor③ 遗留）② SPEC I-5 补 all(target_arch, target_feature) 组合门控澄清（审查 Minor M1）；已派发原实现者
- T4: complete (commits 5668479..ce30c5a, fix round 1: 2 addressed, 0 open, re-review clean)。SUMMARY 总结报告由编排者在 T5 收尾时写（docs/plans/post-v0.1.1/ 属编排者维护区）
- T5: 全量门禁全绿（test --workspace --all-features / clippy -D warnings / fmt / wasm32 check×2 / no-std-fs / deny）。整支终审（opus）：无 Critical/Important；新 Minor 1 项——vane-wasm/Cargo.toml:42 过时注释（同类清理遗漏），已派发修复；T1 Minor①②、T4 M2 经 triage 全部可 defer；SUMMARY.md 已写
- T5: complete — 终审修复波 1（3913b8e）scoped 复审 clean；PLAN+SUMMARY 入库 4a2b83f。**全部 DoD 达成，计划关闭**（16c9fe4..4a2b83f，5 commits）
