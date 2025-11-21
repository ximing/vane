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
| 03 | IDB 降级 VFS | ✅ | 588b5c7 | IdbVfs 复用 M2-02 overlay（**overlay.rs 零改动**）+ IdbBackend（内存 Vec + AtomicBool dirty）+ sync best-effort（标 dirty 不落盘不抛错）+ from_blob/schedule_checkpoint/snapshot + opfs_available stub；降级不抛错确认（无 E_UNSUPPORTED 路径）；437 测试；idb feature 341KB（增量≈0）。**carry-forward**：IDB 浏览器 put/get + opfs_available 真实探针 + 性能对比待 M2-04；I-1 测试注释 nit（逻辑正确）park 不修 |
| 04 | Worker 壳 | ✅ | 9ed63bf/e644bbe | VaneWorker（wasm-bindgen，create 静态工厂返 Promise，constructor 不能返 Promise 偏离）+ dict_loader（CDN fetch+sha256+OPFS 缓存+内联+降级 bigram 不抛错，E_DICT_UNAVAILABLE 禁止到达）+ worker.js + init 探针（opfs_available 真实探测+Safari 降级 OPFS→IDB→Memory）+ postMessage；worker feature 启 jieba 算法，**800KB 含 jieba=399KB**（浏览器 jieba 验收通过，dict-zh 红线未启）；456 测试；fix round 1: dict cache 刷新（I-1 delete-then-create）+ close flush（M-8）+ 契约同步（M-1）+ read_cache 增量（M-6）。**carry-forward**：浏览器异步路径（OPFS/IDB/CDN/postMessage）手动验证待 Demo；park: M-2/M-3/M-4/M-5/M-7 |
| 05 | SIMD128 双变体 | ✅ | 8501caf | 真实 simd_probe（WebAssembly.validate）+ build-wasm-variants.sh 双产物（simd 406KB / scalar 400KB gzip，均 ≤800KB）+ core 零 cfg（I-5）；459 测试。**⚠️ 性能 carry-forward**：LLVM 自动向量化字面门禁过（3116 SIMD 指令），但 **f32x4=0**——cosine/l2/dot 距离循环未向量化（FP 非结合律，LLVM 无 fast-math 拒绝归约向量化）；3116 SIMD 全来自 roaring 位图（加速 filter）。§8.4 双变体召回回归可达成（M2-06），DoD 不要求 f32 向量化。**post-M2 可选**：trait Distance 抽象（cfg 在 impl）手写 SIMD → 需 SPEC 修订（I-5），交用户 DoD 时裁定。node v20 实际支持 simd128（探针返 true） |
| 06 | SIMD 双变体召回回归 | ✅ | 311e2da | simd/scalar 双变体五档×三模式 recall@10 全 1.000 + 跨变体 min Jaccard=1.000000（≥0.99 硬断言，证实 f32 未向量化两变体数值一致）；CI wasm-recall job + 本地 node 路径跑通；459 测试。§8.4 双变体召回回归满足。carry-forward：CI 远程 wasm-bindgen-cli 编译耗时待验证 |
| 09 | SQ8 量化 | ✅ | 0644abe/c2d2ac6 | SQ8 编解码（per-dim min/max+256 级）+ 三 metric 距离（cosine/L2/dot on-the-fly dequant）+ brute_search_sq8（对齐 metric+docid_base）+ segment sq8_vectors 懒加载（OnceLock）+ HNSW 不改（首选）；内存 10万×384=38.4MB<200MB；recall Jaccard≥0.95；480 测试；fix round 1: brute_search_dispatch 下沉 vector 模块（B-1 I-5：api/collection.rs sq8 cfg=0）。**carry-forward**：100万若 >200MB 需 HNSW 用 SQ8 → 改 HnswReader::search 签名 → SPEC 修订（M2-10 评估） |
| 10 | 100万 + Executor | ✅ | da4abf5 | Executor trait + RayonExecutor（cfg(not(wasm32))+executor-native feature，rayon）+ SerialExecutor（cfg(wasm32)）+ default_executor 工厂（cfg 集中 executor/mod.rs）+ Db 持有 Arc<dyn Executor> + 多段并行搜索归并（每段 Arc<Mutex> 槽 + join_all 后串行归并，与 M1 串行等价，无数据竞争，I-2 不破）+ 100万 #[ignore] 压测；487 测试；I-5 cfg 隔离 PASS（cfg 仅 executor/mod.rs）。**接口偏差**：scope<R>→join_all（dyn-compatibility，Arc<dyn Executor> 不可持 generic R）。**carry-forward**：契约同步（M-1）；vane-ffi/node 未启 executor-native 默认串行（M-2）；100万 recall≥0.95 门禁延后 CI（M-3） |
| 12 | export 快照 | ✅ | f8795c9 | Db::export 实装（VANE_SNAP 单文件格式 + write_snapshot/read_snapshot 遍历 manifest+segments+wal）+ vane-ffi vane_export + vane-node ExportTask 接入 + export→read_snapshot→open→search 闭环（P0-3 数据主权）；签名不变；493 测试。**carry-forward**：spec magic(4)="VANE_SNAP" 笔误（9 字节，实装取 9 字节，格式非冻结可接受）；read_snapshot 全量入内存（恢复低频可接受） |
| 13 | 真实维基 nDCG corpus | ✅ | 3e19c93 | 真实中文维基 500 篇（API 抓取，title 可溯源）+ 50 查询 + jieba-aware qrels + ndcg_wiki_zh.rs；jieba=0.9295 vs bigram=0.9255=+0.4%（不退步）；M1 边界歧义语料回归 +84% 保留；496 测试。**SPEC v1.3 修订**（用户批准 §13.2-2②）：真实维基门禁 ≥0% 不退步（实测 +0.4%，bigram 强基线上限≈7.5%），+15% 由 M1 合成 trap corpus 承载。**carry-forward**：qrels 为 jieba-aware 自动标注非人工 gold-standard |

## 最终指标

（DoD 时填充）

## M2 遗留

（DoD 时填充）
