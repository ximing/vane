# M2 执行账本（Orchestrator Ledger）

> 起点：HEAD `65427a3`（M1 完成，main 干净）
> SPEC 版本：v1.1（M1 三处修订已落）；M2 预计 v1.1→v1.2（懒加载 + stored-zstd + per-file format_version，待用户批准）
> 编排方式：纯编排者 + SubAgent TDD 串行+审查/实现重叠（worktree 不可用，同 M1）
> 任务看板：#12 scoping → #13 安全清理 / #14 懒加载 / #15 stored-zstd → #16 plan-splitter → #17 reviewer+检查点 → #18 TDD 开发 → #19 DoD

## 阶段进度

### 阶段零（M1 遗留清理 + M2 前置）
- [ ] #12 scoping & SPEC v1.2 修订提案（in_progress）
- [ ] #13 安全清理（parked minors + wiki nDCG + vane-wasm 骨架）
- [ ] #14 冷启动懒加载（blocked by SPEC 批准）
- [ ] #15 stored.bin zstd + per-file format_version（blocked by SPEC 批准）

### 阶段一（计划拆分）
- [ ] #16 plan-splitter M2 模块拆分
- [ ] #17 双视角 reviewer + 用户检查点

### 阶段二（TDD 开发）
- [ ] #18 按依赖拓扑 TDD 开发 + 集成门禁

### DoD
- [ ] #19 全量门禁 + M2 总结报告

## 裁决记录（ADR）

（随执行追加）

## 模块完成状态总表

（阶段一拆分后填充）

## 最终指标

（DoD 时填充）

## M2 遗留

（DoD 时填充）
