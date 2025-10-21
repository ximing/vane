# M1 执行账本（编排者维护）

> 本文件是 M1 阶段编排者的恢复地图：记录所有裁决、派发状态、集成节点门禁结果、遗留项。
> 防上下文压缩丢失——压缩后信任本文件 + `git log`，而非记忆。
> 上游契约：`docs/REQUIREMENTS.md` v1.1 + `docs/SPEC.md` v1.0 + `docs/plans/m0/M0-SUMMARY.md` + `docs/plans/m0/EXECUTION-NOTES.md`。

---

## 阶段零 · M0 格式冻结清理（进行中）

M1 的 HNSW 会扩展 segment 格式，必须先把 M0 segment 格式冻结。分两批派发：

### 阶段零-A：格式冻结关键路径（Task #8）

派发一个 opus cleanup SubAgent，范围 = FF1/FF3/FF2/corpus-test/FF6。简报见 `docs/plans/m1/00-cleanup.md`。

**SubAgent 派发前编排者已核实的真实状态**（基于 git HEAD 538db51）：
- FF1：`crates/vane-core/src/segment/mod.rs:104-112` vectors.bin 写纯 f32 LE，无 magic+version 头；`SegmentReader::open:215-223` 直接 `chunks_exact(4)` 读全文件。违反 SPEC §6.2"所有文件以 magic+version 开头"。
- FF3：`segment/header.rs:16` + `mod.rs` stored/idmap/scalars 写入均用 `FORMAT_VERSION.to_be_bytes()`，payload 字段用 LE——字节序混合。`header.rs:40` decode 用 `from_be_bytes`。`segment/tests.rs:30` 断言 `bytes[4..8]==[0,0,0,1]`（BE）。
- FF2：代码注释 `mod.rs:66-67` 已正确（"局部 docid，全局=base+local"）；`segment_writer_docid_base_nonzero` 测试（tests.rs:188）已测 base>0 的 meta 读回，但**未断言 add_doc 返回值是局部 docid**（base=2 时首 doc 应返回 0 而非 2）——这是 FF2 剩余缺口。
- FF5：`benchmark.yml:23-29` 用 `../vane-main` worktree 跑 main baseline，critcmp 在 repo 根读不到对侧 `target/criterion`；line 33 `|| true` 掩盖失败。
- FF6：`ci.yml` 无 wasm32 体积门禁 job；已有注释化 `corpus-compat` job（line 82-84）待落地。

**裁决**：
- FA1：FF1 加 8 字节头（magic LE + format_version LE，与 FF3 统一 LE）。SegmentReader 加载 vectors.bin 时跳过 8 字节头，保证 `vectors()` 仍返回纯 f32（brute_search 不受影响）。doc_count=0 时仍写头（空段合规）。
- FA2：FF3 统一全 LE——header.bin / stored.bin / idmap.bin / scalars.col 的 format_version 一律 `to_le_bytes()`；decode 用 `from_le_bytes()`；更新 header.rs 注释 + tests.rs:30 断言为 `[1,0,0,0]`。decode_kv_map 当前跳过 version 不校验，可顺手加 version 校验（轻量，属 FF4 范畴的可接受部分）。
- FA3：corpus 兼容测试（§13.3）骨架 = `crates/vane-core/tests/corpus_compat.rs`：用 StdFsVfs 建 DB→灌若干文档→flush→close→reopen→验证 search 结果与 stored/external_id 一致；文档化"格式变更须保持此测试通过或 bump version+迁移器"。uncomment `ci.yml` corpus-compat job。
- FA4：FF6 加 deferred wasm32 size job 注释（≤800KB gzip，M1 jieba 起生效），不实跑。
- FA5：M0 未发布任何产物（fresh repo，commit 在 main，无 published artifacts），故 vectors.bin 头变更无向后兼容约束；corpus 兼容测试冻结的是清理后格式。

### 阶段零-B：清理（Task #9，A 通过后）

派发 sonnet cleanup SubAgent，范围 = FF5 benchmark 修复 + parked 次要项。排除 M1 落点项。

---

## 阶段零-A 状态

| 项 | 状态 | 备注 |
|---|---|---|
| 派发 | ✅ | opus cleanup SubAgent（agentId a7d105ea），DONE |
| 自证门禁 | ✅ | test/clippy/wasm32/fmt/no-std-fs/check-thin/corpus/bench 全绿 |
| 编排者集成门禁 | ✅ | 2026-08-09 独立复跑全绿，与自证一致；本轮集成节点未抓出遗漏 |
| reviewer 审查 | ✅ | sonnet reviewer APPROVED_WITH_MINOR，无阻塞。格式冻结核心正确性坐实，pub API 零改动，不变量守住。报告 `docs/plans/m1/00-cleanup-review.md` |
| 提交 | ✅ | 5236257/e329c53/348f946/37a895d/d4dee8b/c287458（HEAD c287458） |

### 编排者对 implementer 疑问的判断
1. inverted.bin 头校验缺失 → **Minor 完整性缺口**。据 M0 README 契约 `write_inverted` 格式 `magic|version|num_terms|...`，inverted.bin 已有头。corpus 测试漏校验它。→ 阶段零-B 补一行测试。非阻塞格式冻结。
2. stored tag 回填带引号 → M0 既有行为（`serde_json::Value::to_string()`），非本次引入。→ 留 M1 07-api 健壮性阶段。非格式冻结问题。

---

## 裁决日志（M1 全程追加）

- **FA1~FA5**（2026-08-09，阶段零-A 派发前）：见上。
- **FB1**（2026-08-09，阶段零-B 集成门禁）：编排者独立复跑全绿（185 lib +2 corpus +1 recall +19 node +4 ffi；clippy --all-targets --all-features/wasm32/fmt/no-std-fs/check-thin 全绿）。
- **FB2**（2026-08-09，FF5 验证）：编排者最初用非 critcmp 格式 heredoc 测试触发误报（"no benchmark results parsed"）。复核后确认解析器针对真实 critcmp 表格格式（每数据行 = name+main+current+变化率）编写，对 SubAgent fixture 与编排者构造的真实格式多行样例均正确（回退>10% exit 1、无回退 exit 0、处理 µs/ms 混合/缺失值`-`/负变化）。FF5 回退门禁确实生效（此前 regex 要求行内 `current` 字面词导致永远空解析→exit 0 兜底，已修）。
- **FB3**（2026-08-09，auto-commit flush 错误暴露）：housekeeping 期保持 eprintln（不改 pub API，wasm32 安全）。`AddReport.auto_commit_flush_error: Option<VaneError>` 字段属 pub API 变更——若做须走 M1 正式变更流程并同步 FFI/Node 绑定，不在 housekeeping 内塞。`log` crate 引入延后 M1（不在黑名单，但 core 加依赖须 wasm32+deny 评估）。→ 记为 M1 可观测性决策项（07-api 或专项）。
- **FB4**（2026-08-09，LTO）：接受延后。`[profile.release] lto="thin"` 仅在远程 CI 验证 `napi build --platform --release` 无 napi 符号边缘问题后加。→ 记为 M1 发布前 checklist。
- **FB5**（2026-08-09，restore base 改读段头）：SubAgent 判定 M0 非真实 bug（连续追加场景累加与段头一致），按防御性改进处理（读段头 docid_base + next_docid=max(base+count)），补多段 restore 测试。M1 compaction（非连续段）场景需此正确性。接受。

## 阶段零-B 状态

| 项 | 状态 | 备注 |
|---|---|---|
| 派发 | ✅ | sonnet housekeeping SubAgent（agentId aad83ae3），DONE_WITH_CONCERNS |
| 自证门禁 | ✅ | 全绿 |
| 编排者集成门禁 | ✅ | FB1，FF5 独立验证 FB2 |
| reviewer 审查 | ⏳ | sonnet reviewer（agentId a328858b）后台审查中 |
| 提交 | ✅ | ae52beb..0a0ce5e（HEAD 0a0ce5e） |
