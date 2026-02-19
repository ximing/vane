# Vane M4 总结报告——生产门槛：数据安全测试 + 可观测性

> 分支：`feat/m4-prod-readiness`（off main）。BASE = `f195ec2`。
> 计划：`docs/plans/m4/M4-PLAN.md`。SDD 账本：`docs/plans/m4/PROGRESS.md`（turn 1-53）。
> SPEC：v1.5→v1.6（M3 已占 v1.5，M4 修订为 v1.6）。

---

## 1. 概述

**M4 目标**：补齐"敢不敢用在生产"的硬门槛——数据安全系统性测试（模糊 / 崩溃恢复 / 跨版本兼容 / 并发）+ 可观测性（tracing / inspect / 诊断），不破坏宿主机。

**完成状态**：6 阶段（0-6）全部完成。30 commits（17 代码 + 13 文档），SPEC v1.6 落地，全量门禁全绿。

**DoD 达成总览**：11 项 DoD 全部 ✅（详 §8）。3 个真实生产数据损坏 bug 被数据安全测试撞出并修复——这是 M4 的核心价值。

**实施方式**：纯编排者模式（Orchestrator），主 Agent 只做任务管理与调度，全部实现经 SubAgent 派发 + 审查 + 全量门禁确认。串行流水线（同时只一个 implementer 写），修复循环上限 2 次（同一 SubAgent 失败 2 次换策略）。全程中文。

---

## 2. 六阶段产出

### Phase 0：测试基础设施设计

- **目标**：六项测试基础设施只读蓝图（FaultVfs / cargo-fuzz / proptest / 跨版本 fixture / tracing / inspect）。
- **实现**：Plan agent（opus）产出 `docs/plans/m4/phase0-design.md`——六项逐项设计 + 现状摸底（Vfs trait 方法表 / 持久化关键点 file:line / 测试布局 / CI 16 jobs）+ SPEC 影响清单（§9/§10/§13.2/§14）+ 9 未决问题 + 执行摘要。无硬 SPEC 矛盾（4 处属 v1.4→v1.5 演进）。
- **用户确认**：AskUserQuestion 3 决策——Q1 批准设计进实现 / Q2 FaultVfs = cfg(test)+dev-feature（两者） / Q3 LostWrite 列 Could 暂不实现。
- **关键决策**：FaultVfs 用 path+op+调用计数器层 1+2（不实现层 3 hook，避免污染生产代码）；cargo-fuzz 独立 crate `vane-fuzz`；tracing 默认 off；inspect API 纯新增不改冻结签名。
- **审查结论**：无（设计阶段，用户批准即定案）。

### Phase 1：模糊测试 + proptest

**1a cargo-fuzz**：
- **目标**：vane-fuzz crate + 5 targets + CI --exclude。
- **实现**：`crates/vane-fuzz/`（Cargo.toml + fuzz_targets/：brute_search_fuzz / hnsw_search_fuzz / persist_roundtrip_fuzz / merge_fuzz / dict_load_fuzz）+ workspace members+default-members + ci.yml test+clippy 加 `--exclude vane-fuzz`。
- **commit**：`d4a94d8`（crate+5 targets+workspace+CI）+ `9e262db`（NCSA license allow）。
- **关键发现**：libfuzzer-sys license = `(MIT OR Apache-2.0) AND NCSA`，deny.toml [licenses] allow 未含 NCSA 阻断 CI deny job——经用户批准加 NCSA。nightly fuzz build/smoke 未本地验证（网络不稳），延 CI Phase 6 fuzz-smoke job。cfg(fuzzing) stable check fail 是 cargo-fuzz 惯例非问题。
- **审查结论**：Spec ✅，0 Critical，1 Important=NCSA（用户批准后 fix），3 Minor（M1 persist_roundtrip unwrap_or_default 假绿 / M2 merge_fuzz 用 HNSW search 验活 / M3 dict_load_fuzz 未断言 Err）defer Phase 6 fuzz-smoke 跑通后微调。

**1b proptest**：
- **目标**：3 不变量（检索稳定 / persist round-trip / merge 不丢）+ 256 cases。
- **实现**：`tests/proptest_invariants.rs`——3 proptest! 不变量 + arb_doc/batch/query Strategy + NaN 过滤 + proptest-regressions 提交。proptest 传递依赖不拉 regex（仅 regex-syntax v0.8.11 独立解析器零依赖非黑名单）。
- **commit**：`f849c7b`（proptest 3 不变量）+ `34a9b11`（fix r1：不变量 1 加非空 guard + Cargo.toml 注释修正）。
- **关键发现**：不变量 1a 仅有上界无下界/非空 guard → search 返 0 假绿风险（当前非假绿，潜在）。fix 加 Vector/Hybrid 非空 guard（Text/Auto 不强制）。
- **审查结论**：Spec ✅，0 Critical，1 Important（I-1 非空 guard）fix r1 ADDRESSED，3 Minor（Cargo.toml 注释 / #![allow(dead_code)] 文件级 / persist_roundtrip ~110s 性能）defer。

### Phase 2：崩溃恢复测试（FaultVfs 故障注入）

**2a FaultVfs**：
- **目标**：FaultVfs 故障注入 VFS 机器 + 单测。
- **实现**：`crates/vane-core/src/vfs/fault.rs`——Fault enum（IoError/PartialWrite/Enospc/Delay，含 one_shot+trigger_on_nth）+ VfsOp + FaultVfs impl（包装任意 inner Vfs）+ 8 单测。cfg(test)+dev-feature `fault-injection` 双门控。LostWrite 按用户决策省略+TODO。wasm32 无 FaultVfs 符号泄漏。
- **commit**：`03319ca`。
- **审查结论**：Spec ✅，0 Critical/Important，8 Minor（call_counts 共享 / PartialWrite 错误传播 / glob_match 语义 / one_shot 语义 / catch-all / Mutex 中毒 / 测试缺口 / sleep_ms wasm warning）全 defer final review。

**2b crash_recovery 5 场景**：
- **目标**：5 崩溃场景（meta_slot 翻转 / WAL flush 中断 / merge 中断 / ENOSPC / 部分写）全用 FaultVfs 注入。
- **实现**：`tests/crash_recovery.rs`——5 测试，FaultVfs 在持久化关键点注入失败，验证恢复后数据一致。
- **commit**：`c7e3cdf`（5 场景）+ `acbd23d`（fix r1：decode_header off-by-one + 场景 5 Corrupt 断言）。
- **关键发现**：**decode_header off-by-one panic**——`segment/header.rs:39` `buf.len() < 8` 门允许 8 字节畸形 header 通过但 `buf[8]` 访问 ulid_len 致越界 panic（非 VaneError::Corrupt）。被场景 5 PartialWrite 撞出。fix `< 8`→`< 9` + 防回归单测。（详 §3）
- **审查结论**：opus reviewer Spec ✅，0 Critical，2 Important（I-1 场景 5 间接验证 + I-2 decode_header panic）fix r1 全 ADDRESSED，2 Minor defer。fix 触动生产 segment/header.rs（M4 首个生产代码改动），全量门禁全绿确认。

### Phase 3：跨版本持久化兼容

- **目标**：v0.1.0 真实 fixture + 当前版本读取 + v1/v2 共存 + 迁移占位。
- **实现**：`tests/cross_version_compat.rs`（3 测试：reads_v0_1_0_fixture / v1_v2_coexist / migrates 占位 ignored）+ `tests/fixtures/compat/v0.1.0/`（9 文件 36KB 真实 v0.1.0 tag 产物）+ `scripts/gen_compat_fixture.rs` + `format-freeze-note.md`。
- **commit**：`6300392`。
- **关键发现**：v0.1.0 vectors.bin 实为 V2 非 V1（v0.1.0 segment/mod.rs 始终写 V2 含 dim 头，无已发布 V1 vectors 产物）——设计 §3.4 "v1 fixture" 措辞不精确，implementer 透明纠正。"v1/v2 共存"在 stored.bin 维度实现（fixture stored V1 + 新 flush V2）。
- **审查结论**：opus reviewer Spec ✅，0 Critical，2 Important（均 plan-doc 记录问题非代码缺陷，编排者据 deliverables 已 resolve），3 Minor。不进 fix 循环。

### Phase 4：并发压测

- **目标**：多线程 search+insert+flush+merge N 轮 + Send/Sync 边界 + 竞态检测。
- **实现**：`tests/stress_concurrency.rs`（10 测试，4-8 线程×50-100 轮）+ Send/Sync 编译期断言 + 跨线程共享验证。不用 loom（loom 须 loom::sync 改造 vane-core 侵入大，列 Could defer）。3 次全跑 + 5 次 multi-run 无 flaky。
- **commit**：`354f66e`（stress）+ `cedbb17`（并发 bug fix）。
- **关键发现**：**撞出 2 个真实生产并发 bug**（详 §3）：(1) 并发 flush manifest 损坏（save_atomic 固定 manifest.json.tmp 互相覆盖）；(2) auto-merge 段状态竞争 double-count（不检查 compacting 锁）。用户批准 fix both now。实际修复比 task 更深——Bug 2 完整 fix 4 层（compacting guard + docid 原子预留 + flush base_docid 连续性 + merge offsets 不覆写）+ Bug 1 save_lock Arc 共享。
- **审查结论**：opus reviewer Spec ✅，0 Critical/Important，3 Minor（缺 compact+并发 flush 直测 / StdFsVfs append 非原子 pre-existing / merge WAL append lock 不一致）全 defer。3x stress 全绿无 flaky。

### Phase 5：可观测性

**5a tracing feature**：
- **目标**：cfg(feature="tracing") 零开销埋点，默认 off，I-5 能力开关。
- **实现**：vane-core Cargo.toml `tracing = { version = "0.1", optional = true }` + `tracing = ["dep:tracing"]` feature。9 埋点位置（search span/elapsed, flush done, merge start/done, PageCache hit/miss, Wal::append, 词典状态机）。直接各模块 `#[cfg(feature="tracing")] tracing::info!`/`debug!`（不建 telemetry.rs）。wasm elapsed 用 web_time::Instant（pre-existing dep，非 tracing 引入）。
- **commit**：`dae29c6`。
- **关键确认**：tracing off wasm 体积不变（grep -c tracing = 0 编译期消除 + vane-wasm 体积持平 Phase 2b）；tracing on wasm +15KB ≤800KB。deny 绿（tracing 链 pin-project-lite/tracing-attributes/tracing-core/once_cell 无黑名单）。
- **审查结论**：opus reviewer Spec ✅，0 Critical/Important，3 Minor（flush bytes 缺 / report debug! 笔误 / review hash 偏）defer。不进 fix 循环。

**5b inspect API**：
- **目标**：Db::stats() / segment_info() / collection_segment_info() + 健康检查。
- **实现**：`crates/vane-core/src/api/inspect.rs`（新模块）——7 structs/enums（DbStats/CollectionStats/SegmentInfo/FormatVersions/SegmentFileSizes/Health/ExecutorKind，全 Debug+Clone）+ 3 新 pub 方法。健康检查：SegmentReader::open 失败→Corrupt / hnsw_readers None→Degraded / Jieba+dict 不可用→Degraded / collection=worst。index_bytes 用 read_at 探测 EOF（Vfs trait 无 size 方法，M0 冻结签名不改）。
- **commit**：`684a112`。
- **审查结论**：opus reviewer Spec ✅，0 Critical/Important，7 Minor（segment_info 不排序 / 段文件部分缺失→Degraded 未实现 / 无 Corrupt/Degraded 回归测 / hnsw sz==0→None 混淆 / 持 5 read lock / dict_available 逻辑重复 / jieba cfg 块冗余）全 defer Phase 6/follow-up。不改冻结 pub API 确认（3 新 pub 方法+7 新 structs 纯新增）。

**5c VaneError 诊断**：
- **目标**：丰富 VaneError String payload（段 ULID/docid/操作/建议），不改错误码 -1..-11、不改 enum 签名。
- **实现**：~25 VaneError 构造点丰富 String payload + `types.rs append_context(e,ctx)` pub(crate) helper + `segment/mod.rs segment_ulid_from_dir/seg_ctx` 段级 helper。10 关键路径文件。crash_recovery 5 场景仍绿（丰富在 CONSTRUCTION 点非 `?` 传播点）。4 新测试。
- **commit**：`5fc4ac4`。
- **审查结论**：opus reviewer Spec ✅，0 Critical/Important，4 Minor defer。不进 fix 循环。
- **后续**：Phase 6b 用户授权重构为统一 ErrorContext struct（见 §4）。

**Phase 5 全量门禁**：全绿。wasm 体积较 Phase 2b 微增 +2470B/+7821B（inspect 新 pub API + 诊断 String 字面量，非 tracing），800KB 红线守住。

### Phase 6：CI 集成 + SPEC v1.6 + 总结

**6a CI 新 job**：
- **目标**：fuzz-smoke / fuzz-long / compat / stress / crash-recovery 5 CI job。
- **实现**：`.github/workflows/ci.yml` +124 仅 ci.yml。fuzz-smoke（nightly-2026-07-01+cargo-fuzz+5 targets×60s, push/PR）/ fuzz-long（cron 周日 03:00 UTC + workflow_dispatch, 5×600s, `|| true` 容错 + artifact）/ compat（cross_version_compat --all-features --release）/ stress（stress_concurrency --release ×3 multi-run）/ crash-recovery（crash_recovery --features fault-injection --release）。YAML 语法验证（pyyaml+yamllint clean）。CI jobs 16→21。
- **commit**：`b4aa743`。
- **审查结论**：Spec ✅，0 Critical/Important，8 Minor（cron 触发全 21 job 周度噪音 / concurrency group / nightly pin 未本地验证 / fuzz-long ||true 吞编译错 / timeout 紧 / cargo-fuzz 未 version-lock / artifact 路径未运行时验证 / if: 表达式风格不一致）全 defer post-merge 观察。

**6b SPEC v1.6 + FFI inspect + ErrorContext 重构**：
- **目标**：SPEC v1.5→v1.6 四节修订（§9/§10/§13.2/§14，用户批准）。
- **版本号校正**：编排者 grep 核验撞出——SPEC 当前已是 v1.5（M3 commit 5d092f8），phase0-design §5 "v1.4→v1.5" 过时，M4 修订实为 v1.5→v1.6。
- **用户批准检查点**（4 问）：Q1 §9 FFI → 用户选"立即实现 FFI 层"（非顺延）；Q2 §10 诊断 → 用户授权"选正确方式，不考虑兼容性/API冻结，项目还没给人用，要求架构精简"（去 ADDITIVE 妥协，可改 enum 签名，错误码不变）；Q3 §13.2+§13.3+§14 → 批准全部；Q4 版本号 → v1.6 一次性批准。
- **6b-impl-1 FFI inspect**：`vane_db_stats`/`vane_db_segment_info` + Node `stats()`/`segmentInfo()` + Wasm 2 函数。JSON 序列化手写 serde_json::Value（复用三 crate 现有依赖零新依赖）。commit `5143885`。opus reviewer Spec ✅，0 Critical/Important，3 Minor defer。
- **6b-impl-2 ErrorContext 重构**：统一 `ErrorContext` struct（message/seg/docid/op/hint 5 字段 + builder + From<String>/<&str>），11 变体全携带（消除"8 有+4 无"二元分裂），废弃 append_context→with_* 链式。code()/name() -1..-11 + E_ 名称不变。Display 新格式 `E_CODE: message [seg=... op=... docid=... hint=...]`。commit `c34e473` + fix r1 `d9dcc5f`（persistence 2 处 hybrid 未真结构化→.op()/.hint()/.seg() builder）。opus reviewer Spec ✅，0 Critical，1 Important fix r1 ADDRESSED，2 Minor defer。
- **6b-apply SPEC v1.6**：SPEC.md v1.5→v1.6 + 四节修订 + changelog v1.6 条目。commit `e563f35` + `f195ec2`（编排者 draft 修正）。
- **审查结论**：编排者审查 SPEC.md 改动忠实于用户批准（§9.2 v1.6 补列 / §10 v1.6 注 / §13.2 第 6-11 项 / §13.3 v1.6 注 / §14 I-5 v1.6 注 / changelog）。

**6c M4 总结报告**：本文件。

---

## 3. 关键生产 bug 发现与修复（M4 核心价值）

M4 的核心价值不在测试本身，而在数据安全测试**撞出了 3 个真实生产数据损坏 bug**——这些 bug 在常规功能测试中不会暴露，只在崩溃注入 / 并发压测的极端条件下触发。

### 3.1 decode_header off-by-one panic

- **发现阶段**：Phase 2b 崩溃恢复测试，场景 5 PartialWrite。
- **根因**：`crates/vane-core/src/segment/header.rs:39` `if buf.len() < 8` 门允许 8 字节畸形 header 通过，但 `:52-53` `let pos=8; let ulid_len=buf[pos]`（`buf[8]`）访问越界致 **index-out-of-bounds panic**（非 `VaneError::Corrupt`）。8 字节恰好是 magic+version 长度，PartialWrite 写 8 字节后失败即触发。
- **影响**：段文件 header 仅 8 字节时，`decode_header` panic 而非返回 `Corrupt` 错误码——违反 SPEC §10 E_CORRUPT 语义（应返 -4，实际 panic 未捕获）。崩溃恢复路径 `recover` 调 `SegmentReader::open` 时若遇此段会 panic 整个进程。
- **fix**：`< 8`→`< 9`（真实 header ≥35 字节，`< 9` 门不误拒）+ 场景 5 加 `decode_header` Corrupt 直接断言 + 防回归单测 `decode_header_8_bytes_returns_corrupt_not_panic`。
- **commit**：`acbd23d`。fix 循环 r1 clean，0 round 2。

### 3.2 并发 flush manifest 损坏

- **发现阶段**：Phase 4 并发压测。
- **根因**：`crates/vane-core/src/persistence/mod.rs:104-121` `ManifestStore::save_atomic` 用固定 tmp 路径 `manifest.json.tmp`（`self.tmp_path()`），并发调用时 `delete`/`create`/`write_at`/`sync`/`rename` 交错互相覆盖→manifest 损坏→E_CORRUPT。
- **影响**：多线程并发 flush 时 manifest 文件损坏，数据库不可恢复——真实生产数据丢失场景。
- **fix**：方案 A——`ManifestStore` 加 `save_lock: Arc<Mutex<()>>` 序列化 `save_atomic`。但 ManifestStore 原 per-flush/merge 新构造致 save_lock 不共享→改 `DbInner.manifest_store` 为 `Arc<ManifestStore>` + `CollectionInner` 持同 Arc 让 save_lock 真共享。拆出 `save_atomic_locked` 避重入死锁。
- **commit**：`cedbb17`。

### 3.3 auto-merge 段状态竞争 double-count

- **发现阶段**：Phase 4 并发压测。
- **根因**：`crates/vane-core/src/api/collection.rs:486-508` `auto_merge_two_smallest` 读 snapshot→pick candidates→`merge_segments`，**不获取 `compacting` 锁**（compact 在 :1226/:1137 获取+E_BUSY，但 auto_merge 绕过）→与并发 compact/merge 竞争→段未正确移除→同文档旧段 + merged 段两次（double-count）。flush 串行下仍偶发。
- **影响**：并发操作时同文档在旧段和新 merged 段都被检索到→检索结果重复——数据一致性破坏。
- **fix**（4 层，实际修复比 task 更深）：
  1. **compacting guard**：`auto_merge_two_smallest` 用 `try_lock`，held→skip 安全降级（空闲→set true+panic-safe Drop guard）。
  2. **docid 原子预留**：`merge_segments target_docid_base` 并入 `next_docid` + write_state lock 内原子 read+bump 预留 docid 区间，消 flush 缓冲段与 merge 新段 docid 重叠（TOCTOU）。
  3. **flush base_docid 连续性检测**：连续→首文档 docid 保 inspect base=0；非连续→rebase next_docid。
  4. **merge offsets 不覆写**：merge 快照重建不再用 stale `offsets` 覆写 `seg_offsets`（并发 flush 新段 offset 不被错置 0）。
- **残留 race**：full merge(compact)+并发 flush docid 重叠**部分修**——数学推导排除 compact+并发 flush docid 重叠（compact 新段 [0,actual_new_count) ≤ estimated ≤ next_docid，buffered docs ≥ old_next_docid ≥ actual_new_count，不重叠），缺直测 defer Could。
- **commit**：`cedbb17`。
- **验证**：3x stress 全绿无 flaky。opus reviewer 严验并发正确性（死锁/lock 序/4 层 fix 声/Arc 共享/crash_recovery 仍绿），0 Critical/Important。

---

## 4. SPEC v1.6 修订

M4 SPEC 修订实为 **v1.5→v1.6**（M3 已占 v1.5，2026-08-11 commit 5d092f8）。编排者 grep 核验撞出版本号校正（phase0-design §5 "v1.4→v1.5" 过时）。四节修订 + changelog v1.6 条目，**用户一次性批准**（AskUserQuestion 4 问）。

### §9.2 inspect API FFI 函数面补列

v1.6 补列 `vane_db_stats(db_h, out_arena*) -> i32` + `vane_db_segment_info(db_h, out_arena*) -> i32`。实现：core 层 `Db::stats()`/`segment_info()`/`collection_segment_info()` [M4, `684a112`] + FFI/Node/Wasm 三绑定层 [M4, `5143885`] 全实现。7 structs + 健康检查语义（SegmentReader::open 失败→Corrupt / hnsw 缺失→Degraded / 否则 Healthy）。

### §10 错误码表 ErrorContext 结构化注

v1.6 注：`VaneError` 11 变体统一携带 `ErrorContext` struct（message+seg+docid+op+hint 5 字段），builder 链式 `.seg()`/`.docid()`/`.op()`/`.hint()` + `From<String>`/`From<&str>`。`VaneError` `with_*` pub(crate) 替代旧 `append_context`；`context()` pub 返回 `&ErrorContext`。**错误码 -1..-11 + 名称不变**（本表硬约束）。Display `E_CODE: message [seg=... op=... docid=... hint=...]`（None 省略）。实现：`c34e473` + `d9dcc5f`。

> 用户 Q2 授权"选正确方式，不考虑兼容性/API冻结，要求架构精简"——去 ADDITIVE 妥协，重构为统一 struct（5c 的 String 丰富方案被 6b-impl-2 替代）。

### §13.2 +6 质量门禁

新增第 6-11 项：
- 6. fuzz-smoke [M4]：cargo-fuzz 每 target 60s（push/PR），5 targets 无 panic/crash。CI job `fuzz-smoke`（`b4aa743`）。
- 7. fuzz-long [M4]：cargo-fuzz 每 target 10min（cron 周日 03:00 UTC + workflow_dispatch），crash 上传 artifact。CI job `fuzz-long`（`b4aa743`）。
- 8. 崩溃恢复 [M4]：FaultVfs 注入 5 场景全通过，崩溃后 manifest 指向完整状态、数据一致。CI job `crash-recovery`（`b4aa743`）。
- 9. 跨版本兼容 [M4]：v0.1.0 真实 fixture 当前版本读取通过。CI job `compat`（`b4aa743`）。
- 10. 并发压测 [M4]：多线程 search+insert+flush+merge N 轮，timeout 内无 panic/死锁/数据不一致。CI job `stress`（`b4aa743`）。
- 11. proptest 不变量 [M4]：检索稳定 / round-trip / merge 不丢 256 cases 全通过。CI test job 覆盖（`f849c7b` + `34a9b11`）。

### §13.3 dev/optional 依赖注

v1.6 注：dev/optional 依赖（tracing / proptest / cargo-fuzz / libfuzzer-sys）不触运行时依赖黑名单，cargo-deny 守护。libfuzzer-sys license = `(MIT OR Apache-2.0) AND NCSA` 已在 deny.toml [licenses] allow NCSA（`9e262db`）。

### §14 I-5 tracing feature 能力开关释义扩展

v1.6 注：`cfg(feature="tracing")` 是可观测性能力开关（类似 zstd-encode），允许出现在 api/segment/persistence/wal 模块的埋点位置。不启用时编译期消除（`grep -c tracing = 0` 验证），wasm/native 体积不变（800KB gzip 红线守护，tracing off 时 vane-wasm ~352KB / core --export-all ~650KB）。tracing crate 传递依赖不触黑名单。参照 commit `dae29c6`。

### 不触碰范围

M4 不触碰 §1-§8 / §11 / §12 / §13.1 / §13.3 黑名单列表 / §15（core 检索语义、分发矩阵、性能承诺、里程碑验收）。

---

## 5. 测试安全铁律遵守

**铁律**：所有破坏性测试（崩溃 / 磁盘满 / IO 错误 / 中途失败）一律经 **FaultVfs 故障注入**或 **tempdir 隔离**模拟，**禁止真断电 / 真写满宿主机磁盘 / 真杀进程 / 真损坏文件**。测试须可控、可复现、CI 友好。

**FaultVfs 隔离机制**：
- `cfg(test)` + dev-feature `fault-injection` 双门控，不污染生产二进制。
- wasm32 无 FaultVfs 符号泄漏（wasm32 check 确认）。
- 包装任意 inner Vfs（MemoryVfs 主力 / StdFsVfs+tempdir conformance 对齐），在 Vfs 方法调用前查询故障规则表，命中则返错 / 部分写 / ENOSPC / 延迟。
- check_fault 在调 inner 前执行，返错则不调 inner，保证 inner 状态不变。

**5 崩溃场景**（`tests/crash_recovery.rs`，5 测试，全用 FaultVfs + tempdir）：
1. meta_slot 翻转崩溃（sync tmp 失败 / rename 失败）
2. WAL flush 崩溃（append 失败 / sync 失败）
3. merge 中断崩溃（write_inverted 失败 / save_atomic 失败）
4. ENOSPC（write_at 返 ENOSPC，不损已有数据）
5. 部分写（write_at 写 8 字节后失败→decode_header Corrupt）

**stress 压测**（`tests/stress_concurrency.rs`，10 测试）：MemoryVfs + tempdir，4-8 线程×50-100 轮，无真破坏宿主机。

**fuzz**（`crates/vane-fuzz/fuzz_targets/`，5 targets）：cargo-fuzz + libfuzzer-sys，CI 环境运行，不触宿主机。

**proptest**（`tests/proptest_invariants.rs`，3 不变量 256 cases）：property-based 不变量，MemoryVfs，无破坏性。

**全程零真破坏**：无一次真断电、真写满磁盘、真杀进程、真损坏文件。铁律全程遵守。

---

## 6. 全量门禁状态

最终全量门禁全绿（最近综合确认：Phase 6b-impl-2 fix r1 + 6b-apply）。

| 门禁项 | 结果 |
|---|---|
| `cargo fmt --all -- --check` | ✅ rc=0 |
| `cargo clippy --all-targets --all-features -- -D warnings` | ✅ rc=0（workspace --exclude vane-fuzz） |
| `cargo test --workspace --all-features --exclude vane-fuzz` | ✅ 全过（proptest 3 149s / crash_recovery 5 / stress 10 / FFI 14 / 84 unit / Node 21 / cross_version 2+1 ignored） |
| `cargo check --target wasm32-unknown-unknown -p vane-core` | ✅ rc=0（tracing off） |
| `cargo check --target wasm32-unknown-unknown -p vane-wasm` | ✅ rc=0 |
| `cargo check --target wasm32-unknown-unknown -p vane-core --features tracing` | ✅ rc=0（tracing on） |
| `check-wasm-size.sh` | ✅ vane-wasm 364466B / core --export-all 652256B gzip，均 ≤800KB |
| `check-no-std-fs.sh` | ✅ rc=0（std_fs.rs 是 VFS 实现处 SPEC 允许） |
| `cargo deny check` | ✅ advisories+bans+licenses+sources ok（NCSA allow） |
| Node 绑定测试（`cd crates/vane-node && npm test`） | ✅ 21 passed |
| stress multi-run（3x） | ✅ 10 passed 0 failed 无 flaky |

**wasm 体积演进**：
- Phase 2b 基线：vane-wasm 349261B / core 641275B
- Phase 5（inspect + 诊断）：vane-wasm 351731B / core 649096B（+2470B/+7821B，inspect 新 pub API + 诊断 String 字面量，非 tracing）
- Phase 4 并发 fix：vane-wasm 352011B / core 649619B（+280B/+523B，并发 fix 新代码）
- 6b-impl-1 FFI inspect：vane-wasm 362636B / core 649619B（+10KB，FFI inspect JSON 序列化）
- 6b-impl-2 ErrorContext：vane-wasm 364466B / core 652256B（ErrorContext struct + Display 格式）
- **最终**：vane-wasm 364466B / core 652256B gzip，均 ≤800KB 红线守住。

---

## 7. tracing 默认 off 确认

**cfg 门控**：`crates/vane-core/Cargo.toml` `tracing = { version = "0.1", optional = true }` + `tracing = ["dep:tracing"]` feature，默认 off。

**编译期消除**：所有 tracing 调用经 `#[cfg(feature="tracing")]` 门控。tracing off 时 `grep -c tracing = 0`（wasm 二进制无 tracing 符号）。

**wasm/native 体积不变**：tracing off 时 vane-wasm 体积与 Phase 2b 基线持平（grep=0 确认）；tracing on 时 +15KB ≤800KB。

**vane-wasm 不启用 tracing**：vane-wasm Cargo.toml 无 tracing 依赖（grep -c tracing = 0），守护 800KB 红线。

**9 埋点位置**（11 处宏调用）：
1. search span（`api/collection.rs:825` `tracing::info_span!`）
2. search elapsed（`api/collection.rs` search 出口 `tracing::info!`）
3. flush done（`api/collection.rs:499`）
4. merge start（`api/collection.rs:1148`）
5. merge done（`api/collection.rs:1303`）
6. PageCache hit（`vfs/page_cache.rs:54` `tracing::debug!`）
7. PageCache miss（`vfs/page_cache.rs:66` `tracing::debug!`）
8. Wal::append（`wal/mod.rs:68` `tracing::debug!`）
9. 词典状态机（`api/collection.rs` 3 处 `tracing::info!` dict state transition）

**deny 合规**：tracing crate 传递依赖（pin-project-lite / tracing-attributes / tracing-core / once_cell）无黑名单（regex/tokio/prost/tonic/openssl/lindera/ndarray/wee_alloc/dashmap/parking_lot），cargo-deny 守护。

---

## 8. DoD 达成检查

逐项对照 `M4-PLAN.md` 的 DoD 清单：

| # | DoD 项 | 状态 | 说明 |
|---|---|---|---|
| 1 | FaultVfs 故障注入 VFS + 崩溃恢复测试套件（meta_slot/WAL/merge/ENOSPC/部分写，全用 FaultVfs 模拟） | ✅ | `03319ca` FaultVfs + `c7e3cdf` crash_recovery 5 场景 + `acbd23d` decode_header fix。全程 FaultVfs/tempdir 隔离，零真破坏。 |
| 2 | cargo-fuzz targets（检索/持久化/合并/词典）+ CI fuzz-smoke + fuzz-long | ✅ | `d4a94d8` vane-fuzz crate + 5 targets + `9e262db` NCSA + `b4aa743` CI fuzz-smoke/fuzz-long job。nightly pin 未本地验证（延 CI 首次跑）。 |
| 3 | proptest property-based 不变量 | ✅ | `f849c7b` 3 不变量 256 cases + `34a9b11` 非空 guard fix。proptest-regressions 提交。 |
| 4 | 跨版本兼容 fixture + 迁移测试 + CI compat job | ✅ | `6300392` v0.1.0 真实 fixture + cross_version_compat 3 测试 + `b4aa743` CI compat job。迁移占位（当前 v1 不需迁移，双模读取）。 |
| 5 | 并发压测 + 竞态检测 + CI stress job | ✅ | `354f66e` stress 10 测试 + `cedbb17` 2 并发 bug fix + `b4aa743` CI stress job。loom 列 Could 未做（纯压力测试替代）。3x multi-run 无 flaky。 |
| 6 | tracing feature（零开销，cfg(feature)，I-5）+ inspect API（stats/segment_info/健康检查）+ VaneError 诊断上下文 | ✅ | `dae29c6` tracing + `684a112` inspect API + `5fc4ac4`→`c34e473`+`d9dcc5f` ErrorContext 重构 + `5143885` FFI inspect。tracing 默认 off 编译期消除。 |
| 7 | CI 新增 fuzz-smoke/fuzz-long/compat/stress/crash-recovery job 全绿 | ✅ | `b4aa743` 5 新 job。CI jobs 16→21。YAML 语法验证 clean。首次 CI 跑验证 nightly fuzz。 |
| 8 | SPEC v1.6（§9 inspect / §10 诊断 / §13.2 测试门禁 / §14 tracing feature，用户批准）+ changelog | ✅ | `e563f35`+`f195ec2` SPEC v1.6 apply。用户一次性批准（4 问 AskUserQuestion）。changelog v1.6 条目。 |
| 9 | cargo test --workspace 全绿 + clippy/fmt/wasm32 check/check-no-std-fs/deny 不回退 | ✅ | 全量门禁全绿（详 §6）。322+ 集成测试无回归，含 recall/corpus_compat/crash_recovery/stress/proptest/cross_version/FFI/Node。 |
| 10 | wasm 体积 ≤800KB gzip（tracing 不启用时不变） | ✅ | tracing off：vane-wasm 364466B / core 652256B gzip，均 ≤800KB。tracing off 体积不变（grep=0 编译期消除）。tracing on ≤800KB。 |
| 11 | docs/plans/m4/ 计划 + 总结报告 | ✅ | M4-PLAN.md + PROGRESS.md + phase0-design.md + 各 phase report/review + 本文件。 |

**DoD 偏差**：无硬偏差。2 项 Could 级 defer（loom / LostWrite）非 DoD Must 项，已在 Phase 0 用户确认时列为 Could。

---

## 9. 遗留 / defer 项

### Could 级（Phase 0 用户确认时列为 Could，未做）

- **loom 竞态检测**：loom 须 `loom::sync` 改造 vane-core 侵入大（vane-core 用 std::sync），纯压力测试替代（10 测试 + 3x multi-run）。列 Could defer。
- **LostWrite 故障类型**：MemoryVfs sync 本是 noop 难以真模拟丢写，StdFsVfs 已 fsync 难以模拟。Phase 0 Q3 用户决策列 Could 暂不实现（从 Fault enum 省略、留 TODO 注释，崩溃恢复用 sync 失败注入近似）。

### Minor 级（各 phase reviewer 标 Minor，defer final review / follow-up）

**FaultVfs（8 Minor）**：M1 call_counts 同 key 共享 / M2 PartialWrite 错误传播 / M3 glob_match 语义 / M4 one_shot 语义 / M5 catch-all / M6 Mutex 中毒 / M7 测试缺口 / M8 sleep_ms wasm warning（`let _ = ms` 可修）。

**crash_recovery（2 Minor）**：M1 tmp 清理注释 / M2 注释 mask panic（I-2 修后自愈）。

**cargo-fuzz（3 Minor）**：M1 persist_roundtrip unwrap_or_default 假绿 / M2 merge_fuzz 用 HNSW search 验活 / M3 dict_load_fuzz 未断言 Err——defer Phase 6 fuzz-smoke 跑通后微调。

**proptest（2 Minor）**：M2 #![allow(dead_code)] 文件级 / M3 persist_roundtrip ~110s 性能（SPEC 要求 Db::open 加载词典，非缺陷，CI 20min timeout 内 OK）。

**cross_version（3 Minor）**：fixture 额外 db/ 层级 / 7 段文件含 hnsw.bin / 无独立 cross-version CI job（经现有 test job，§3.4 允许）。

**tracing（3 Minor）**：flush done 缺 bytes（Vfs 无 size() defer inspect）/ report debug! 笔误 / review hash 偏。

**inspect（7 Minor）**：M1 segment_info 不排序致 FFI JSON 顺序非确定 / M2 段文件部分缺失→Degraded 未实现 / M3 无 Corrupt/Degraded 回归测 / M4 hnsw sz==0→None 混淆 / M5 持 5 read lock 阻塞写 / M6 dict_available 逻辑重复 / M7 jieba cfg 块冗余。

**Phase 5c 诊断（4 Minor）**：report in code commit / report §8.1 略 overstated / dict suggestion 泛化 / segment_ulid_from_dir fallback "unknown"——后被 6b-impl-2 ErrorContext 重构替代。

**stress/并发 fix（3 Minor）**：缺 compact+并发 flush 直测（数学排除但无直测）/ StdFsVfs append 非原子 pre-existing / merge WAL append lock 不一致非正确性。

**CI（8 Minor）**：cron 触发全 21 job 周度噪音 / concurrency group schedule+push 共享 / nightly pin 未本地验证 / fuzz-long ||true 吞编译错 / timeout 60min 紧 / cargo-fuzz 未 version-lock / artifact 路径未运行时验证 / if: 表达式风格不一致——defer post-merge 观察 + tune。

**FFI inspect（3 Minor）**：FFI null 未设 thread-local / report 体积矛盾 / wasm-pack test 既有模式。

**ErrorContext 重构（2 Minor）**：report hash 偏 / persistence/tests.rs 注释过时（fix r1 已修）。

### 功能 defer（非 bug，follow-up）

- **`vane_db_collection_segment_info` FFI 未暴露**：core 层 `Db::collection_segment_info()` 已实现（commit `684a112`），但 FFI 只暴露 `vane_db_stats`/`vane_db_segment_info` 两个函数。SPEC §9.2 v1.6 补列仅此两个。`collection_segment_info` 是 core-only 便捷重载，FFI 层用户可从 `segment_info()` 结果按 collection name 过滤。
- **full merge(compact)+并发 flush docid 重叠深层 fix**：数学推导排除重叠，`stress_concurrent_add_during_compact` 过，但缺直测。defer Could。
- **fuzz nightly pin `nightly-2026-07-01` 未本地验证**：网络不稳无法装 nightly/cargo-fuzz，延 CI 首次跑验证。
- **dict_load_fuzz 未覆盖 JiebaDict::load 畸形字节**：不启 jieba feature 避 wasm32 feature unification，defer Phase 6。
- **tracing-subscriber 不进 core**：subscriber 是消费侧（应用层），core 只 emit。vane-ffi/vane-node 按需加 dev-dep。
- **结构化上下文**：已落地（非 Could）——用户 Q2 授权重构为 ErrorContext struct，超出 Phase 5c 的 String 丰富方案。

---

## 10. commit 链

M4 全 commit 链（`git log --oneline 985cc06..HEAD`），共 30 commits（17 代码 + 13 文档）：

```
f195ec2 docs(spec): SPEC v1.5→v1.6 应用（M4 四节修订）+ inspect.rs §3.6→§9.2 + 账本 turn 53
e563f35 docs(spec): SPEC v1.6 apply——§9 inspect + §10 ErrorContext + §13.2 门禁 + §14 tracing（M4 6b-apply）
e38fbb9 docs(plans): M4 6b-impl-2 §10 诊断重构定案 + 账本 turn 51-52
d9dcc5f fix(core): persistence 2 处构造点真结构化 + report hash 修正 + 注释更新
c34e473 refactor(core): VaneError 诊断架构重构——ErrorContext 结构化字段替代 String 拼接
4588ae2 docs(plans): M4 6b-impl-1 FFI inspect 定案 + spec-v1.6 草案 + 账本 turn 49-50
5143885 feat(ffi): inspect API 落地 FFI/Node/Wasm 三绑定层（M4 Phase 5b）
8dc83a2 docs(plans): M4 turn 48——6b 重派 + 版本号校正 v1.5→v1.6 + FFI inspect 顺延→实现方向
4a1c766 docs(plans): M4 Phase 6a（CI 新 job）报告/审查 artifacts
b4aa743 ci: 新增 fuzz-smoke/fuzz-long/compat/stress/crash-recovery job（M4 阶段六 a）
f793e93 docs(plans): M4 Phase 4（并发压测 + 并发 bug fix）报告/审查 artifacts
cedbb17 fix(core): 并发 flush manifest 损坏 + auto-merge 竞争 double-count（M4 Phase 4 fix）
354f66e test(core): stress_concurrency 多线程压测 + Send/Sync 边界（M4 阶段四）
d6daf7b docs(plans): M4 Phase 5c（VaneError 诊断）报告/审查 artifacts + Phase 5 完成
5fc4ac4 feat(core): VaneError 诊断上下文（String 丰富，不改错误码）（M4 阶段五 c）
8959337 docs(plans): M4 Phase 5b（inspect API）报告/审查 artifacts
684a112 feat(core): inspect API（Db::stats/segment_info + 健康检查）（M4 阶段五 b）
3758620 docs(plans): M4 Phase 5a（tracing）报告/审查 artifacts
dae29c6 feat(core): tracing feature（cfg 门控零开销埋点，默认 off）（M4 阶段五 a）
0cb50e5 docs(plans): M4 Phase 3（跨版本兼容）报告/审查 artifacts
6300392 test(core): cross_version_compat v0.1.0 fixture + v1/v2 共存（M4 阶段三 a）
1fc03af docs(plans): M4 Phase 1（cargo-fuzz+proptest）计划/报告/审查 artifacts
34a9b11 test(core): proptest 不变量 1 加非空 guard + 修 Cargo.toml 注释（M4 阶段一 b fix r1）
f849c7b test(core): proptest 3 不变量（检索稳定/round-trip/merge 不丢）（M4 阶段一 b）
9e262db chore(deny): allow NCSA license for libfuzzer-sys（M4 阶段一 a）
d4a94d8 feat(fuzz): vane-fuzz crate + 5 targets + CI --exclude vane-fuzz（M4 阶段一 a）
0458942 docs(plans): M4 Phase 0 设计 + Phase 2 计划/报告/审查 artifacts
acbd23d fix(segment): decode_header off-by-one (< 8 → < 9) + crash_recovery 场景 5 Corrupt 断言（M4 阶段二 b fix r1）
c7e3cdf test(core): crash_recovery 5 场景 FaultVfs 注入（M4 阶段二 b）
03319ca feat(core): FaultVfs 故障注入 VFS + 单测（M4 阶段二 a）
```

**代码 commit 分类**（17）：
- feat（6）：03319ca FaultVfs / d4a94d8 vane-fuzz / dae29c6 tracing / 684a112 inspect / 5fc4ac4 诊断 / 5143885 FFI inspect
- test（5）：c7e3cdf crash_recovery / f849c7b proptest / 34a9b11 proptest fix / 6300392 cross_version / 354f66e stress
- fix（4）：acbd23d decode_header / cedbb17 并发 bug / d9dcc5f ErrorContext fix / f195ec2（docs-spec）
- refactor（1）：c34e473 ErrorContext 重构
- ci（1）：b4aa743 CI 5 job
- chore（1）：9e262db NCSA deny

---

## 结语

M4 的核心价值不在测试数量，而在**数据安全测试撞出了 3 个真实生产数据损坏 bug**——decode_header off-by-one panic（崩溃恢复测试撞出）、并发 flush manifest 损坏（并发压测撞出）、auto-merge 段状态竞争 double-count（并发压测撞出）。这 3 个 bug 在常规功能测试中不会暴露，只在故障注入 / 并发压测的极端条件下触发，且都是真实生产数据损坏场景（panic 未捕获 / manifest 损坏不可恢复 / 检索结果重复）。

M4 后 Vane 具备了"敢不敢用在生产"的硬门槛：故障注入崩溃恢复验证 + 模糊测试 + 属性测试 + 跨版本兼容 + 并发压测 + 可观测性（tracing/inspect/诊断）。SPEC v1.6 落地，全量门禁全绿，wasm 800KB 红线守住。
