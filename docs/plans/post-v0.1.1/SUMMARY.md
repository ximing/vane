# post-v0.1.1 总结：f32 距离 SIMD128 向量化

日期：2026-08-10 ｜ 分支：main（16c9fe4 → 最终 HEAD）｜ 编排：SDD（SubAgent 驱动，全程串行单实现者 + 逐任务审查 + 整支终审）

## 目标与结论

M2-05 落地 SIMD128 双变体后遗留的 carry-forward 项——f32 距离（cosine/L2/dot）
显式向量化——已完成。simd 变体（`RUSTFLAGS=-Ctarget-feature=+simd128`）的 f32
距离三核走 `core::arch::wasm32` f32x4 intrinsics 显式路径，scalar 变体走同归约
顺序的 4 累加器标量路径，**双变体输出逐位一致**。

## DoD 对照

| 项 | 结果 |
|---|---|
| f32 三核 SIMD128 向量化 | ✅ commit 5668479；simd 变体 asm 含 3192 行 simd128 指令（scalar 0） |
| 双变体召回回归不退步 | ✅ 五档 selectivity recall=1.000；跨变体 30 查询 min Jaccard=**1.000000** |
| wasm 双变体 ≤800KB gzip | ✅ simd 400.3KB / scalar 403.1KB（向量化后 simd 变体反而更小） |
| SPEC v1.4 + changelog | ✅ 用户批准后落地（afd34ce + ce30c5a）：§11/§14 I-5 释义扩展 + 组合门控澄清 |
| 全量门禁不回退 | ✅ test --workspace --all-features 全绿、clippy -D warnings 零警告、fmt、wasm32 check×2、check-no-std-fs、cargo deny 全过 |
| 性能基准（可选） | 未做——正确性优先；simd 路径运行时性能对比留作后续可选任务 |
| 计划 + 总结 | ✅ docs/plans/post-v0.1.1/PLAN.md（含台账）+ 本文 |

## 关键设计：跨变体逐位一致

CI wasm-recall 要求跨变体 top-10 Jaccard≥0.99（top-10 下实际等价于集合完全相同），
而 FP 加法非结合。解法：**两路径统一归约顺序**——

- 主循环 4 路累加：simd lane k 与标量 acc[k] 各自累加 i≡k (mod 4) 的项；
- 尾部（len%4）两路径**共用同一段标量代码**（j → 累加器 j%4）；
- 最终归约统一走固定顺序 `((a0+a1)+a2)+a3` 的 `reduce4`（simd 路径先按 lane
  顺序 extract 再进同一 reduce4，不引入第二种水平归约）；
- wasm f32x4 逐 lane IEEE 单精度、无 FMA 融合；host 无 fast-math 不会重结合——
  两变体逐位相等，Jaccard=1.000000 实测背书。

## SPEC v1.4（用户批准，2026-08-10）

I-5 释义扩展：`cfg(target_feature)` 用于同一算法的向量化/标量双实现视为能力开关
（类似 cfg(feature)），允许出现在算法代码中；可与 target_arch 组合
（`cfg(all(target_arch = "wasm32", target_feature = "simd128"))`，组合整体仍是
能力开关）；`cfg(target_arch)`/`cfg(target_os)` 平台分支仍仅限 VFS/Executor。
同步修订 §11 表述、changelog v1.4 条目，并清理 simd_probe.rs / sq8.rs /
build-wasm-variants.sh / vane-wasm Cargo.toml 四处过时注释。

## SQ8 决策：carry-forward

评估结论（task-2-report.md）：当前无任何交付物（vane-wasm/vane-node/vane-ffi）
启用 `sq8` feature，向量化即死代码且无运行时门禁背书。结构可行（预估 0.5-1 天，
lane 规则可照搬 f32 核），重触发条件：① 某交付物启用 sq8；② 先补 SQ8 双变体
召回门禁；③ 若确认 native-only 定位则本决策为终态。

## 已知遗留（终审 triage：全部可 defer）

- `acc4_tail_lane_rule_bitwise_exact_on_ints` 测试 docstring 宣称过度（零舍入数据
  无法区分归位错误；测试仍有 off-by-one 捕获价值）；
- `acc4_reduction_is_deterministic` 测试近永真（无害，零维护成本）；
- hnsw/mod.rs 等 7 处「零 cfg(target)」注释为事实性描述，与 v1.4 不冲突；
- HNSW 路径（hnsw/mod.rs metric_distance）未向量化——两变体编译同一标量源码，
  逐位一致不受影响；如需进一步提速可作为后续任务；
- simd vs scalar 性能基准对比未做（可选 DoD 项）。

## 过程记录

- T0 SPEC 修订用户批准（AskUserQuestion 检查点）→ T1 实现（审查 clean）
  → T2 SQ8 评估（carry-forward，关键论断经编排者独立核实）
  → T3 双变体体积+召回门禁（编排者直跑）→ T4 文档（审查 clean + fix round 1
  两处收尾，复审全 ADDRESSED）→ T5 全量门禁 + 整支终审（1 Minor 修复）。
- 修复轮次总计：T4 round 1 + 终审 fix wave 1，均未触上限。
