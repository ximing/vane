# M4 阶段五 c：VaneError 诊断上下文——审查结论

> 来源：Phase 5c task reviewer SubAgent（opus，只读，禁编辑源码）。
> 审查对象：commit 5fc4ac4（14 files +520 -52）。
> 依据：`docs/plans/m4/phase0-design.md` §10、`docs/plans/m4/M4-PLAN.md` 阶段五 3、`docs/SPEC.md` §10、`crates/vane-core/src/types.rs`、`crates/vane-core/tests/crash_recovery.rs`。

## A. Spec 合规：✅

| 项 | 结论 | 证据 |
|---|---|---|
| 丰富 VaneError String payload 含段 ULID/docid/操作/建议（§10） | ✅ | 25+ 构造点均含 `seg=<ulid>` / `db=<path>` / `path=<wal>` / `op=<操作>` / `建议=<suggestion>`；diff 全 ADDITIVE（`format!("{}{}", m, ctx)` 或 `format!("original{}{}", ..., seg_ctx(...))`） |
| 不改错误码 -1..-11（§10） | ✅ | `git diff 8959337..5fc4ac4 -- types.rs` grep `^[-+].*(fn code|enum VaneError|=> -[0-9])` 返回空——code() impl / enum 定义 / => -N 映射全无 +/- 行；SPEC §10 表与 types.rs:54-67 code() 实现逐行对齐 |
| 不改 VaneError enum 签名 | ✅ | 11 变体签名不变（Io(String)/Schema(String)/.../InvalidArg(String) + 4 无 payload 变体）；diff 仅新增 `pub(crate) fn append_context`（types.rs:117）+ `pub(crate) fn segment_ulid_from_dir`（segment/mod.rs:365）+ `pub(crate) fn seg_ctx`（segment/mod.rs:376），均 pub(crate) 非 pub API |
| 不改 M0-M3 冻结 pub API | ✅ | append_context/seg_ctx/segment_ulid_from_dir 全 pub(crate)；Display impl / Error impl / code() / name() 不变 |
| 5a/5b 产出未误改 | ✅ | diff 不含 SPEC.md / CI yml / fault.rs / crash_recovery.rs / tracing 埋点 / inspect API；`#[cfg(feature="tracing")]` 块在 api/collection.rs / wal/mod.rs 上下文行保留未动 |

## B. 代码质量

### B.1 无错误码变更：✅ CONFIRMED

- types.rs:54-67 `fn code(&self) -> i32` 实现逐行核对：Io→-1 / Schema→-2 / NotFound→-3 / Corrupt→-4 / Version→-5 / TokenizerMismatch→-6 / DictTooLarge→-7 / DictUnavailable→-8 / Busy→-9 / Unsupported→-10 / InvalidArg→-11，与 SPEC §10 表完全一致。
- diff 中 code() 实现无 - 或 + 行（grep 验证）。
- `error_code_matches_spec_section_10` 测试（types.rs:274）未在 diff 中，仍存在且未改——自动验证 code 与 SPEC 对齐。

### B.2 无 enum 签名变更：✅ CONFIRMED

- types.rs:39-50 `pub enum VaneError` 11 变体字段定义未改（diff 无 +/- 行）。
- 仅新增 `pub(crate) fn append_context`（crate 内部辅助，非 pub API surface）。
- `segment_ulid_from_dir` / `seg_ctx` 同为 pub(crate)。

### B.3 String 丰富有意义：✅ NON-VACUOUS

抽样 25 构造点，每处 String 均含至少 2 项上下文：

| 路径 | 丰富样例 | 含信息字段 |
|---|---|---|
| segment open vectors.bin bad magic | `vectors.bin bad magic (seg=<ulid>, op=open vectors.bin; 建议: 检查段文件完整性或从备份恢复)` | seg + op + 建议 |
| manifest parse | `manifest parse: {} (db={}, op=load manifest; 建议: 检查 manifest.json 完整性或从备份恢复)` | db + op + 建议 |
| wal parse | `wal parse: {} (path={}, op=wal recover; 建议: wal.log 损坏，检查崩溃恢复或清除 wal.log 重试)` | path + op + 建议 |
| add vector dim mismatch | `vector dim mismatch: got {} expected {} (op=add, collection={}, doc_id={}; 建议: 对齐 doc vector 维度与 schema 声明)` | op + collection + doc_id + 建议 |
| dict load | 经 append_context 追加 `(op=dict load; 建议: 词典数据损坏，重新构建或联系支持)` | op + 建议（dict 无 seg/docid 上下文，合理） |

无 vacuous 丰富（无 "error occurred" 类无信息占位）。原消息全保留为 String 前缀（ADDITIVE）。

### B.4 4 新测试 non-vacuous：✅ CONFIRMED

| 测试 | 文件 | 断言关键词 | non-vacuous |
|---|---|---|---|
| `append_context_enriches_string_preserves_code` | types.rs | 7 String 变体 code 不变 + 4 无 payload 变体原样返回 + msg 含 seg=01H / op=open / 建议 | ✅ |
| `m4_5c_open_error_contains_segment_context` | segment/tests.rs | 含原消息 "vectors.bin bad magic" + 段 ULID + op=open + 建议 | ✅ |
| `m4_5c_manifest_parse_error_contains_context` | persistence/tests.rs | 含原消息 "manifest parse" + db 路径 "mydb" + op=load manifest + 建议 | ✅ |
| `m4_5c_wal_parse_error_contains_context` | wal/tests.rs | 含原消息 "wal parse" + wal 路径 "mydb" + op=wal recover + 建议 | ✅ |

每测试断言 ≥3 关键词 + 原消息保留，非单字符串匹配。

### B.5 crash_recovery 仍绿：✅ CONFIRMED

crash_recovery.rs 5 个 contains 断言全安全（enrichment 在 CONSTRUCTION 点，非 `?` 传播点）：

| 断言 | 行 | 错误来源 | 传播路径 | 受 5c 丰富影响？ |
|---|---|---|---|---|
| `contains("manifest.json.tmp")` | 144 | FaultVfs Io on rename/create tmp | `save_atomic` 的 `vfs.rename(&tmp, &target)?` 传播 | 否——5c 仅丰富 `manifest serialize`（serde 路），vfs op 的 `?` 未碰 |
| `contains("inverted.bin")` | 328 | FaultVfs Io on inverted.bin write/read | write_inverted / InvertedIndexReader::open 的 `vfs.read_at?` 传播 | 否——5c 仅丰富 truncated header/bad magic/version mismatch（Corrupt/Version 构造点），vfs.read_at `?` 未碰；即便碰，"inverted.bin" 仍为前缀保留 |
| `contains("ENOSPC")` | 428 | FaultVfs Enospc Io | `?` 传播 | 否——ENOSPC 是 FaultVfs 构造的 Io msg，5c 未碰 vfs op 传播点 |
| `contains("partial write")` | 521 | FaultVfs PartialWrite Io | `?` 传播 | 否——同上 |
| `contains("too short")` | 561 | `decode_header` Corrupt | **crash_recovery:559 直接调 `decode_header(&buf[..n])`**，不经 SegmentReader::open 的 append_context wrap | 否——decode_header 本体未改，直接调用返回原始 Corrupt("header too short") |

关键：crash_recovery.rs 在 8959337..5fc4ac4 diff 中**无变更**（`git diff --stat` 空）。segment/header.rs 同未改。

### B.6 覆盖度：✅ ADEQUATE

10 关键路径全覆盖：

| 路径 | 丰富点 | 状态 |
|---|---|---|
| open（SegmentReader + InvertedIndexReader + decode_header wrap） | 9 + 3 + 1 | ✅ |
| manifest（load / save_atomic / add_segment） | 3 | ✅ |
| WAL（append / read_all） | 2 | ✅ |
| search（topK / dim mismatch / missing text+vector） | 3 | ✅ |
| add（vector dim mismatch with doc_id） | 1 | ✅ |
| merge（finalize_merge InvalidArg + merge_segments NotFound） | 2 | ✅ |
| reindex（update_manifest_after_reindex NotFound） | 1 | ✅ |
| dict load（load + load_zstd ×2） | 3 | ✅ |
| DB open（schema / tokenizer mismatch） | 2 | ✅ |
| flush | manifest save_atomic 丰富 + leaf 传播 | ✅（manifest 丰富覆盖；SegmentWriter::finalize 文件写错误经 `?` 传播，vfs Io msg 已含路径，adequate） |

**defer 的 leaf 错误（~60 处）——acceptable**：
- segment/mod.rs decode_kv_map / decode_stored / decode_scalars ~30 leaf "too short"——在 decode 函数内无 segment_dir 参数，threading 成本高；外层 load_stored/load_id_map 已有段级捕获；ROI 低，defer 合理。
- bm25.rs InvertedIndexReader ~15 深层 decode（truncated term_len/vbyte/tf/docid）——magic+version 已校验后的罕见损坏路径，defer 合理。
- hmm.rs 5 "hmm_blob too short"——经 dict.rs parse 间接调用，已有 `load` 层 append_context wrap 覆盖，defer 合理。
- api/snapshot.rs ~10 snapshot Corrupt/Version——非核心 flush/merge/search 路径，defer 合理。

## C. 已知 concerns 评估

| concern | 评估 |
|---|---|
| defer leaf 错误 | acceptable——低 ROI，外层已有段级 wrap，深层 decode 无 segment_dir 参数 |
| 结构化上下文 defer Phase 6 | acceptable——§10 注释列为 Could；String 丰富立即可用，结构化需改 enum 签名碰冻结 API，正确推迟 |
| report 在 code commit | minor——5a/5b 同模式，scope 偏差可接受 |

## Findings（Critical→Important→Minor）

**Critical**：无。

**Important**：无。

**Minor**：

1. `docs/plans/m4/task-diagnostics-report.md` | report 随 code commit 5fc4ac4 提交（非独立 docs commit） | minor scope 偏差，5a/5b 同模式，不影响正确性
2. `docs/plans/m4/task-diagnostics-report.md` §8.1 | "flush（经 SegmentWriter/write_inverted/manifest 传播——leaf 丰富已覆盖）" 略 overstated | SegmentWriter::finalize 文件写错误未在 CONSTRUCTION 丰富（经 `?` 传播 vfs Io），但 manifest save_atomic 路径已丰富；vfs Io msg 已含路径信息，adequate
3. `crates/vane-core/src/tokenizer/jieba/dict.rs:39` | dict load 经 append_context 用同一 generic `(op=dict load; 建议: 词典数据损坏，重新构建或联系支持)` wrap 所有 parse leaf | 特定 leaf 原因（magic/version/too short）作为前缀保留，非 vacuous；但 suggestion 泛化，深层诊断仍需看前缀——acceptable
4. `crates/vane-core/src/segment/mod.rs:365` | `segment_ulid_from_dir` 对非 `seg_<ulid>` 路径返回 "unknown" | 防御性 fallback，实践中 segment_dir 恒为 `seg_<ulid>`，不会触发；若未来有非标路径会产出 "seg=unknown" 低信息上下文

## ⚠️ 无法从 diff 验证项

- `cargo test --workspace --all-features --exclude vane-fuzz` 实际返回 "346 passed; 0 failed"（report §6.3 自报）——按指令不重跑 implementer 已跑门禁，trusted。
- `cargo clippy --all-targets --all-features --workspace --exclude vane-fuzz -- -D warnings` 通过（report §6.2）——trusted。
- `cargo fmt --all -- --check` 通过（report §6.1）——trusted。
- `cargo check --target wasm32-unknown-unknown -p vane-core` 通过（report §6.5）——trusted；代码侧核实：append_context 仅用 `format!`，无 std::fs / 平台分支。
- `cargo deny check` 通过（report §6.4）——trusted；diff 无新依赖引入。
- crash_recovery.rs 5 场景在本地 cargo test 实际跑绿——未重跑，但 diff 证实文件未改 + 断言与丰富点无交集（见 B.5 表），逻辑上确定仍绿。

## 总体：不进 fix 循环

任务 spec 合规、错误码不变、enum 签名不变、String 丰富有意义、4 测试 non-vacuous、crash_recovery 逻辑确定仍绿、覆盖度 adequate。可进 5d / 阶段六。
