# Vane M4 计划——生产门槛：数据安全测试 + 可观测性

> 来源：用户 M4 编排者 prompt。本文件为 M4 计划的事实记录（plan of record），供 SDD ledger 引用与压缩恢复。
> 分支：`feat/m4-prod-readiness`（off main，**无 worktree**，主目录串行——同时只一个 implementer 写）。
> ledger：`docs/plans/m4/PROGRESS.md`。
> SPEC 现版本：v1.4；M4 目标版本 v1.5（§9/§10/§13.2/§14 修订，**用户批准**）。

## 背景
Vane（github.com/ximing/vane，public）0.1.x 功能闭环：三端 prebuilt（Node/Go/WASM）+ Web npm（M3）+ f32 SIMD + 文档站。CI 16 jobs + install-matrix 12/12 全绿。但作为真实项目可用的库，缺生产门槛：数据安全系统性测试（模糊/崩溃恢复/跨版本兼容/并发）+ 可观测性（tracing/inspect/诊断）。M4 目标：补齐"敢不敢用在生产"的硬门槛，不破坏宿主机。

**测试安全铁律**：所有破坏性测试（崩溃/磁盘满/IO 错误/中途失败）必须通过 **FaultVfs 故障注入**或 tempdir 隔离模拟，**禁止真断电/真写满宿主机磁盘/真杀进程/真损坏文件**。测试须可控、可复现、CI 友好。

开工前必读：
- docs/SPEC.md v1.4（§6.2 持久化格式 v1/v2 / §9 API / §10 错误码 / §13.2 门禁 / §14 I-5）
- docs/plans/m2/M2-SUMMARY.md（M2-02 OPFS 双 meta_slot+CRC 崩溃恢复 / M2-07 懒加载 / M2-12 export 快照）
- docs/plans/m1/（M1 WAL + reindex + 合并重建）
- crates/vane-core/src/vfs/（Vfs trait + std_fs.rs + memory.rs）—— FaultVfs 在此 trait 上实现
- crates/vane-core/src/segment/（段不可变 + tombstone + merge）+ api/（Db/Collection）
- crates/vane-core/src/types.rs（per-file format_version 常量）
- .github/workflows/ci.yml（现有 16 jobs）

## 角色：纯编排者（Orchestrator）
主 Agent 只做任务管理与调度，**禁止写任何代码、禁止直接编辑任何文件**（唯一例外维护 docs/plans/m4/ 计划状态文件与任务看板）。全部工作通过 TaskCreate 建看板 / Agent+SendMessage 派发驱动 SubAgent / 审查 SubAgent 产出结论 / 每修复节点跑全量本地门禁（cargo test --workspace --all-features / clippy --all-targets --all-features -- -D warnings / wasm32 check / fmt / check-no-std-fs.sh / cargo deny check / wasm 体积 / recall / nDCG）确认不回退。失败重试有上限（同一 SubAgent 任务失败 2 次换策略或上报）。仅遇 SPEC 矛盾/阻塞/需用户侧操作/全部 DoD 达成才打扰用户。串行+审查/实现重叠（同时只一个 implementer 写）。全程中文沟通与文档。每步确认可 commit。

## 并发纪律（worktree 不可用）
同 post-M2：同一工作目录同时只允许一个 implementer 写；重叠仅限 implementer(写) || reviewer(只读)。串行+审查/实现重叠流水线。

## 阶段零：测试基础设施设计（用户确认）
派只读 SubAgent 出设计，**AskUserQuestion 检查点**向用户确认：
1. **FaultVfs 故障注入 VFS**：Vfs trait 的包装实现（cfg(test) 或 dev-feature `fault-injection`），包装任意 inner Vfs（Memory/StdFs），注入可控故障：IO 错误（指定 path/op/offset 返 Err）/ 部分写（写 N 字节后返 Err，模拟中途失败）/ 写后丢（模拟写未落盘）/ 延迟乱序（可选）/ 磁盘满模拟（write 返 ENOSPC，不真写满）。核心：崩溃恢复测试用 FaultVfs 在持久化关键点（meta_slot 翻转前/后、WAL flush 前/后、merge persist 前/后）注入失败，验证恢复后数据一致（不丢/不损/可读）。**不真断电/真写满**。
2. **cargo-fuzz 集成**：fuzz targets 目录 + cargo-fuzz。CI 短跑（60s smoke）+ 定期长跑（cron/workflow_dispatch）。
3. **proptest**：property-based 不变量测试（检索不变量/持久化 round-trip/merge 不丢文档）。
4. **跨版本兼容 fixture 框架**：旧版本数据 fixture（提交仓库或 CI 用旧 tag 二进制生成）+ 新版本读取/迁移测试。
5. **tracing feature 骨架**：cfg(feature="tracing") 埋点（零开销，不启用编译期消除），I-5 能力开关。
6. **inspect API 设计**：Db::stats()/segment_info() 等新 pub API（SPEC §9 修订）。

## 阶段一：模糊测试（cargo-fuzz）
fuzz targets（每个独立 fuzz target）：brute_search_fuzz / hnsw_search_fuzz / persist_roundtrip_fuzz / merge_fuzz / dict_load_fuzz（畸形词典字节降级 bigram 不抛错，M2-04 铁律）。自证：cargo-fuzz 短跑（每 target 60s）无 panic/crash。

## 阶段二：崩溃恢复测试（FaultVfs 故障注入）
**核心：用 FaultVfs 模拟崩溃，不真断电。**
1. meta_slot 翻转崩溃：FaultVfs 在 persist_meta 翻转前/后注入失败 → 验证恢复后用上一致 meta_slot + CRC 校验 + 数据一致。
2. WAL flush 崩溃：WAL 写一半注入失败 → 重放恢复 + 不丢已确认事务。
3. merge 中断崩溃：merge persist 前/后注入失败 → 验证原段未损 + merge 可重试 + 数据不丢。
4. 磁盘满（ENOSPC）：FaultVfs write 返 ENOSPC → 验证优雅降级（不损已有数据 + 错误码可操作）。
5. 部分写：write 写 N 字节后失败 → 验证 CRC 校验拒绝损坏段 + 恢复路径正确。
自证：每个崩溃场景测试用 FaultVfs + tempdir，可复现，CI 友好。**全程不真写满/真断电**。

## 阶段三：跨版本持久化兼容
1. fixture 数据：v0.1 格式段数据（提交仓库 fixtures/ 或 CI 用 v0.1.0 tag 二进制生成）。
2. 新版本读取测试：当前版本读 v0.1 fixture → 数据一致（per-file format_version v1/v2 双模，M2-08 已有，验证覆盖）。
3. 迁移测试（若格式升级）：旧格式 → 新格式迁移器 + 迁移后数据一致。
4. 格式冻结承诺文档：SPEC §6.2 哪些格式冻结、哪些可演进、迁移策略。
自证：CI job 用 v0.1 fixture 测当前版本读取 + 迁移。

## 阶段四：并发压测
1. 多线程读写压测：多线程并发 search + insert + flush + merge，N 轮，验证无 panic/死锁/数据不一致。
2. 竞态检测：可选 loom（若适用）或压力测试 + 段/Meta 锁竞争场景。
3. Send/Sync 边界：验证 Db/Collection 跨线程共享 + 并发安全。
自证：压测脚本 + CI job（timeout 内无竞态）。

## 阶段五：可观测性
1. tracing feature（cfg(feature="tracing")，零开销，I-5 能力开关）：检索延迟（p50/p99 span）/ 段数/索引大小/merge 频率/缓存命中率 指标；不启用时编译期消除（无运行时开销）。
2. inspect API（新 pub API，SPEC §9 修订）：Db::stats()（段数/文档数/索引大小/词典状态）/ Db::segment_info()（各段状态/格式版本/大小）/ 健康检查（词典是否降级/段是否损坏标记）。
3. VaneError 诊断信息：错误码 + 上下文（哪段/哪文档/哪操作/建议操作）。SPEC §10 修订。
自证：tracing 零开销（不启用时 wasm/native 体积/性能不变）+ inspect API 测试 + VaneError 诊断完整。

## 阶段六：CI 集成 + SPEC 修订 + 总结
1. CI 新增 job：fuzz-smoke（每 target 60s，push/PR）/ fuzz-long（cron 或 workflow_dispatch）/ compat / stress / crash-recovery。
2. SPEC 修订（用户批准）：§9 API 加 inspect/stats / §10 错误码诊断上下文 / §13.2 DoD 加 fuzz/崩溃恢复/兼容/压测门禁 / §14 I-5 tracing feature（cfg(feature) 能力开关，零开销）。SPEC v1.4→v1.5。
3. tracing feature 默认不启用（避免影响 wasm 体积 800KB 红线 + native 零开销）。
4. docs/plans/m4/ 总结报告。

## 完成定义（DoD）
- FaultVfs 故障注入 VFS 实现 + 崩溃恢复测试套件（meta_slot/WAL/merge/ENOSPC/部分写，**全用 FaultVfs 模拟，不真破坏宿主机**）；
- cargo-fuzz targets（检索/持久化/合并/词典）+ CI fuzz-smoke + fuzz-long；
- proptest property-based 不变量；
- 跨版本兼容 fixture + 迁移测试 + CI compat job；
- 并发压测 + 竞态检测 + CI stress job；
- tracing feature（零开销，cfg(feature)，I-5）+ inspect API（stats/segment_info/健康检查）+ VaneError 诊断上下文；
- CI 新增 fuzz-smoke/fuzz-long/compat/stress/crash-recovery job 全绿；
- SPEC v1.5（§9 inspect API / §10 诊断 / §13.2 测试门禁 / §14 tracing feature，用户批准）+ changelog；
- cargo test --workspace 全绿 + clippy/fmt/wasm32 check/check-no-std-fs/deny 不回退；
- wasm 体积 ≤800KB gzip（tracing 不启用时不变）；
- docs/plans/m4/ 计划 + 总结报告。

## 约束
- **测试安全铁律**：破坏性测试一律用 FaultVfs 故障注入或 tempdir 隔离，**禁止真断电/真写满磁盘/真杀进程/真损坏文件**。测试须可控可复现 CI 友好。
- MoSCoW 即合同；Won't-have（内置 embedding/GPU/SQL/分布式）不碰。
- 词典永不进 wasm（800KB gzip 红线）。tracing feature 不启用时 wasm 体积不变。
- core 禁 std::fs/std::net/mmap；cfg 仅 VFS/Executor + cfg(target_feature) 向量化 + cfg(feature) 能力开关（tracing，I-5）。FaultVfs 是 cfg(test)/dev-feature，不污染生产二进制。
- 依赖黑名单（regex/tokio/prost/tonic/openssl/lindera/ndarray/wee_alloc/dashmap/parking_lot）；rayon 仅 Executor impl。cargo-fuzz/proptest/tracing 是 dev/optional 依赖，需符合黑名单 + 体积约束。
- 不改 M0-M3 冻结 pub API（inspect API 是新增，SPEC 修订）；SPEC 矛盾上报不绕行。
- SPEC 修订（§9 inspect / §10 诊断 / §13.2 门禁 / §14 tracing）需用户批准（AskUserQuestion 检查点）。
- 全程中文沟通与文档。
- 每步确认可 commit。
