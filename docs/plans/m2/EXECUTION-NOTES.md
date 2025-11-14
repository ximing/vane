# M2 执行账本（Orchestrator Ledger）

> 起点：HEAD `65427a3`（M1 完成，main 干净）
> SPEC 版本：v1.1（M1 三处修订已落）；M2 预计 v1.1→v1.2（懒加载 + stored-zstd + per-file format_version，待用户批准）
> 编排方式：纯编排者 + SubAgent TDD 串行+审查/实现重叠（worktree 不可用，同 M1）
> 任务看板：#12 scoping → #13 安全清理 / #14 懒加载 / #15 stored-zstd → #16 plan-splitter → #17 reviewer+检查点 → #18 TDD 开发 → #19 DoD

## 阶段进度

### 阶段零（M1 遗留清理 + M2 前置）
- [x] #12 scoping & SPEC v1.2 修订提案（完成：00-scoping-report；SPEC v1.2 三处修订用户批准，commit c2bd0bb）
- [ ] #13 安全清理
  - [x] vane-wasm 骨架（commit 6458247；340 测试未回退；wasm32 双 check 通过；体积 9.46KB default / 151KB --export-all <<800KB；CI job 更新留 M2-01）
  - [x] parked minors（6 项，commits f81be11/70622b2/d490b43/4ff9203/f964864/ff1d527 + 报告 4034440；347 测试=340+7；JS 17 全绿；2.1.3 行为变更 filter 非标量字段→E_INVALID_ARG 端到端验证通过；2.1.5 新增 #[cfg(test)] set_jieba_dict_for_test 注入）
  - [~] 真实维基 nDCG corpus → fold 为 M2-13（Phase One 计划，Phase Two 执行；需真实维基数据获取 + qrels 方法论，宜正式计划非 ad-hoc）
- [ ] #14 冷启动懒加载（SPEC v1.2 已批准；fold 为 M2-07，Phase One 计划后 Phase Two 执行）
- [ ] #15 stored.bin zstd + per-file format_version（SPEC v1.2 已批准；fold 为 M2-08，Phase One 计划后 Phase Two 执行）

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

| # | 模块 | 状态 | commit | 备注 |
|---|---|---|---|---|
| 00 | Phase Zero 安全清理 | ✅ | 6458247/af2d895 | vane-wasm 骨架 + parked minors（6 commits）；wiki nDCG fold M2-13 |
| 07 | 冷启动懒加载 | ✅ | 1337b1a/afd690a | OnceLock 按需加载 vectors/stored；open 100k 1573ms→752ms<1s；fix round 1: open 期廉价头探测恢复损坏 loud 失败（I-1）+ v2 dim 预存（M-3）；361 测试。**M2-08 交接**：落实 VECTORS_FORMAT_V2 常量 + finalize 写 v2 头后，替换 mod.rs 字面量 `2u32` + 将 `build_v2_stub_segment` 测试切真实 finalize 产物回归 |
| 08 | stored-zstd + per-file version | ✅ | b91f28f/a3e9ae2 | per-file 常量（6 段文件）+ vectors v2 头（M2-07 交接落实）+ stored v2 zstd 双模 + zstd-encode/zstd-decode feature 解耦（ruzstd 进 wasm）；368→370 测试；fix round 1: 压缩失败回退 v1 保可读（I-1）。**carry-forward**：wasm 11.4KB 占位 LTO 剥离 decode，真实 800KB 门禁须 M2-01 接入真实 API 后重测；3 Minor（inverted per-file / dict.rs 改名 / 注释）接受不修 |
| 11 | Go cgo 绑定 | ✅ | ff266a7/7b478c8 | vane-ffi C ABI 实装（1070 行，16 extern C，M1 README §09 契约）+ 手写 vane.h + Go cgo 薄壳 + dict embed 1.48MB<2MB + wazero 骨架 + host demo（含 jieba loadDict+三模式 search）+ CI go-cross matrix；vane-core additive Db::set_jieba_dict；379 cargo + 7 Go 测试；fix round 1: catch_unwind 全入口 panic 安全（B-1）+ 锁 map_err + Go LockOSThread（I-1）+ collection jieba_dict TOCTOU snapshot（I-3）+ reindex_wait 锁外 clone（I-4）+ README vane_collection 签名同步（I-2）。**carry-forward**：多平台 zig cc 交叉待 CI 触发；wazero 仅骨架；panic 测试未构造真实输入（stdlib 机制保证） |
| 01 | vane-wasm cdylib + 体积门禁 | ✅ | 7165710/6818eeb | 真实检索 API 胶水（wasm-bindgen，MemoryVfs 后端）+ SIMD 探针占位 + 800KB 门禁（default 340KB / --export-all 622KB，M2-08 carry-forward 消解）+ CI 切 vane-wasm + check-no-std-fs 扩展；vane-core 增 web-time（修 wasm32 Instant/SystemTime panic，native 零开销，I-5 守护）；380 cargo + 4 wasm-bindgen-test。**carry-forward**：CI wasm-bindgen-test job 未加（需 wasm-bindgen-cli） |
| 02 | OPFS VFS | ✅ | 39650ea/4483676 | 单容器 `vane.db` + MemOverlay（文件表/extent/free list/双 meta_slot+CRC）+ OverlayBackend trait（OpfsBackend/MemoryBackend）+ Vfs 8 方法全同步 + 3 时点崩溃恢复 + superblock 损坏恢复；**core/Vfs 零改动**（diff 空）；417 测试；体积 343KB（opfs +2.2KB）；fix round 1: compaction shadow-write 崩溃安全（M-1）+ truncate f64（I-1）+ 时点 B 语义（I-3）。**carry-forward**：OpfsBackend 浏览器运行时验证待 M2-04；meta slot 256KB（~9k 项，5万场景远低，合理） |

## 最终指标

（DoD 时填充）

## M2 遗留

（DoD 时填充）
