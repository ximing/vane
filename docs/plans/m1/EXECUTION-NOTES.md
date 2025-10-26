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

## 阶段一 · M1 计划拆分（Task #10）

plan-splitter（opus，agentId a18d8dbc）产出 12 模块计划 + README + 报告。提出 6 个裁决项 R-1~R-6。

### R-item 编排者裁决

- **R-1（export 归属）→ 批准**：export 保留 M2 占位（E_UNSUPPORTED）。SPEC §15 M2 行明确列 export 快照；plan-splitter-brief 误列 M1，以 SPEC 为准。M1 不实装 export。
- **R-2（reindex 签名）→ 批准**：`reindex()` 落实为 `Result<ReindexHandle>`（SPEC §4.1 M0 冻结 IDL）。M0 的 `Result<()>` 是文档化占位（M0 README 标注 "ReindexHandle 留 M1"），此为回归冻结签名非破坏。同步 Node（VaneReindexHandle napi struct）+ FFI（vane_reindex 返回 handle）。
- **R-3（TokenizerId 词典版本注入）→ 暂定推翻方案 A，需用户确认**：
  - 方案 A（JiebaTokenizer 二次哈希叠加 dict.version()+sha256_prefix）**错误**——会使词典升级时 TokenizerId 变化，旧段与 collection 新 TokenizerId 不符 → E_TOKENIZER_MISMATCH → 实质强制重建，违反 SPEC §3.3「词典升级打开老库仅警告不强制重建」+ I-4「禁止查询期多版本词表合并」。
  - 正解：`builtin_dict_version(Jieba)` = **编译期常量**（dict 格式 spec 版本，如 `b"jieba-lite-v1"`）；`compute_tokenizer_id` 内部用之，**不改签名、不二次哈希**。运行时词典日历版本 + sha256_prefix 仅供 §12.3 三渠道一致性校验 + §3.3 升级警告，不进 TokenizerId。
  - 这是 SPEC §5.4（TokenizerId 含 builtin_dict_version）与 §3.3（词典升级不强制重建）的张力 → **SPEC 澄清提议**：§5.4 的 `builtin_dict_version` 应澄清为"编译期 dict 格式 spec 版本"而非"运行时日历版本"。需用户确认。
- **R-4/R-6（rayon / 并行 / cfg）→ 需用户裁决**：
  - 选项 a：M1 引入 Executor 抽象（SPEC §11，cfg 仅在 Executor impl），native 用 rayon（SPEC 忠实，加依赖）或 std::thread::scope（避依赖，偏离 §11 "rayon"）。honors「并行」Must + I-5。
  - 选项 b：M1 全串行搜索（plan-splitter 倾向），并行+Executor 延后 M2。避 cfg 污染（I-5 干净），但**延后 §3.1/§8.1「多段并行搜索」Must** → Must 降级需用户裁决。10万×384 HNSW 串行 <50ms 或许可达，50万/M2 100万需并行。
  - 编排者暂推荐：选项 a（Executor 抽象 + std::thread::scope native + wasm 串行，cfg 仅在 Executor 模块），既 honor Must 又守 I-5，不引 rayon。但偏离 §11 "rayon" 表述 → 需用户确认是否接受 std::thread::scope 替代 rayon，或要求 rayon。
- **R-5（stored.bin zstd）→ 批准**：stored.bin zstd 延后 M2。SPEC §15 M1 未列；core 加 zstd 会撑爆 800KB wasm 红线（且 I-5 禁止 cfg 隔离压缩）。M0 I10「M1 补 zstd」为愿景笔记非合同。M1 保持 stored.bin 裸 JSON（corpus 兼容测试不断链）。

### 需用户确认项（阶段一检查点）
- R-3：§5.4 builtin_dict_version 释义（编译期 spec 版本 vs 运行时日历版本）+ 推翻方案 A。
- R-4/R-6：M1 并行策略（Executor+thread::scope / Executor+rayon / 串行延后 M2）。

### R-item 状态
| 项 | 裁决 | 状态 |
|---|---|---|
| R-1 export M2 | 批准 | ✅ 自主决策 |
| R-2 reindex ReindexHandle | 批准 | ✅ 自主决策 |
| R-3 TokenizerId | 暂定推翻方案 A | ⏳ 需用户确认（SPEC 澄清） |
| R-4/R-6 并行/Executor | 暂推荐 Executor+thread::scope | ⏳ 需用户裁决 |
| R-5 stored zstd M2 | 批准 | ✅ 自主决策 |

### 双视角 reviewer 审查

**可行性 reviewer（sonnet，agentId aed95126）→ CHANGES_REQUESTED**。报告 `docs/plans/m1/review-feasibility.md`。阻塞项：
- **B1【已编排者核实为真】M0 未持久化文档原文**：`api/collection.rs:195-223` flush 中 stored_json 仅由 doc.meta 构造，doc.text 仅 tokenize 进倒排后丢弃，原文不入 stored.bin/任何段文件。违反 SPEC §6.2（stored.bin 应含"原文/JSON meta"），M0 README 偏离记录未记载。→ **06 reindex（换分词器重建倒排）不可实现**（reindex 是 M1 Must）。
  - **编排者裁决**：M1 必须补原文持久化（§6.2 合约补全，非新需求）。12 计划均未覆盖，是真实计划缺口。M1 在 stored.bin 加原文（或新增 text.bin），corpus 兼容测试同步更新（阶段零冻结的是头纪律 magic+version LE，非文件集不可变；M1 扩展格式如 hnsw.bin 同理）。zstd 仍延后 M2（R-5）。**检查点报备用户**。
- **B2 MergeTask::new 签名缺 tokenizer 参数**：README §02 契约无 tokenizer，02/06 又假设传 Box<dyn Tokenizer>。→ 扩展 MergeTask::new 签名（M1 新类型，非 M0 冻结，可改）。
- **M1**：12-recall Task 2 测试含 `unimplemented!()`（语义 placeholder）→ 改真实测试。
- **M2**：README §05 文本"扩展 builtin_dict_version 填词典版本"与 05 方案 A 矛盾。**注**：可行性 reviewer 认为方案 A 正确，**与编排者 R-3 判定（方案 A 违反 §3.3+I-4）冲突**——可行性 reviewer 未深入 §3.3 分析。**待 opus SPEC 契约 reviewer 独立结论裁决**。
- 可行性 reviewer 对 R-4/R-6 倾向串行方案；M0 API 对接核查总体可信（唯一重大遗漏即 B1）。

**SPEC 契约 reviewer（opus，agentId af760dd0）→ CHANGES_REQUESTED**。报告 `docs/plans/m1/review-spec.md`。
- **R-3：opus 完全支持编排者判定**——方案 A 违反 REQUIREMENTS §3.3+I-4，推翻。正解 `builtin_dict_version(Jieba)`=编译期格式常量 `b"jieba-fmt-v1"`，无二次哈希。发现 M0 `id.rs:23` 注释编码错误假设须修。
- **B-2（opus 独有发现）04-wal truncate 缺陷**：清空整个 WAL→`flush→delete→flush→崩溃` 后 AddTombstone 丢失→已删除文档复活。修复：flush 不 truncate，仅 compact truncate。
- **R-4/R-6：opus 强论证 M1 全串行**——thread::scope 在 hnsw 违反 I-5；"多段并行搜索"是 §3.1 架构描述非 §2 Must，10万×384≤10段串行满足 <50ms，不构成 Must 降级。

### 用户确认（阶段一检查点，2026-08-09）
- **SPEC 三处修订全部批准**：S1 §5.4 builtin_dict_version=编译期格式常量；S2 §9.1 DashMap→std::sync::RwLock；S3 §9.2 补 reindex_progress/wait + load_dict/version。
- **M1 全串行策略批准**：M1 全串行搜索，Executor+并行延后 M2，文档化为阶段性偏离。

### plan-splitter 第二轮修订（Task #10 续）
派发 opus plan-splitter（简报 `docs/plans/m1/revision-brief.md`）：应用 SPEC v1.0→v1.1 + 按审查反馈修订全部计划。修订项：B-1 新增 00-text-persistence、B-2 WAL truncate 修复、R-3 推翻方案 A、R-4/R-6 全串行、M-2 MergeTask::new 补 tokenizer、M-3/M1 测试修复、Q-5~Q-8 细化。

### 聚焦复审（Task #10 收尾）
sonnet 聚焦复审（`review-revision.md`）→ **APPROVED_WITH_MINOR，可进入阶段二**。B-1/B-2/R-3/R-4-R-6/M-*/Q-* 全部闭环。2 实装期 minor：① 00 测试 `TokenizerId::from_bytes` 不存在→改 `TokenizerId([0u8;32])`；② 06 line 68 陈旧注释与 R-3 矛盾→实装时改。02 posting remap 需新增 postings 迭代方法（M1 扩展非破坏，已声明）。

**阶段一完成** ✅（commit ba67d51：SPEC v1.1 + 13 计划 + 评审闭环）。

---

## 阶段二 · M1 TDD 开发（Task #11）

串行 + 审查/实现重叠流水线（worktree 不可用）。依赖拓扑序：
L0：00-text-persistence → 01-hnsw → 05-jieba-lite → 09-go-cgo(可后移)
L1：02-tombstone-merge(需 01+00) → 07-dict-node(需 05)
L2：03-pre-filter(01+02) / 04-wal(02) / 06-userdict-reindex(05+02+00) / 08-dict-go(05+09)
L3：11-cold-start(01+02) / 12-recall(01+03)
L4：10-ci-m1（收尾）

每模块：developer SubAgent（TDD，简报=计划文件）→ reviewer → fix 循环 → 层边界集成门禁。模型：opus 算法/集成（01/05/02/06），sonnet 机械（00/03/04/07/08/09/10/11/12）。

### 模块完成状态
| 模块 | 状态 | 模型 | commits | 审查 |
|---|---|---|---|---|
| 00-text-persistence | ✅ 完成 | sonnet | 91c8d7d..97823d0 | APPROVED_WITH_MINOR |
| 01-hnsw | ✅ 完成 | opus | aa252ca..919936f | APPROVED（fix 后） |
| 05-jieba-lite | ✅ 完成 | opus | 12eb209..19c03d1 | APPROVED_WITH_MINOR |
| 09-go-cgo | ⏸ 待(可后移) | sonnet | — | — |
| 02-tombstone-merge | ✅ 完成 | opus | 407bafb..72bb641 | APPROVED_WITH_MINOR（fix 后） |
| 07-dict-node | ⏸ 待 | sonnet | — | — |
| 03-pre-filter | ✅ 完成 | sonnet | 57785ce..5260c49 | APPROVED_WITH_MINOR |
| 04-wal | ⏸ 待 | sonnet | — | — |
| 06-userdict-reindex | ⏸ 待 | opus | — | — |
| 08-dict-go | ⏸ 待 | sonnet | — | — |
| 11-cold-start | ⏸ 待 | sonnet | — | — |
| 12-recall | ⏳ 实现中 | sonnet | — | — |
| 10-ci-m1 | ⏸ 待 | sonnet | — | — |

### 00-text-persistence 裁决（reviewer APPROVED_WITH_MINOR）
- **R-00-1 text() 契约**：以实现为准——无原文返回 `Some("")`、docid 不存在返回 `None`（便于 06 reindex 始终拿 `&str`）。02/06 派发据此对齐（非 Produces 段的 None）。
- **R-00-2 format_version 保持 1**：背书「M0 stored.bin = 未发布占位」定性（SPEC §6.2 始终要求原文+meta，M0 仅 meta 是占位不完整，00 补全非破坏，无发布产物）。**护栏**：仅因 M0 未发布才可；首个正式发布后 stored.bin v1 冻结，后续布局变更须 per-file format_version + bump + 迁移。遗留 M0 持久库须重建（可能 misparse）。

### 并发纪律（worktree 不可用）
同一工作目录，**同时只允许一个 implementer 写**。重叠仅限 implementer(写) || reviewer(只读)。故 05 实现期间不派 01；05 提交后，01 实现 与 05 审查 并行（01 建在 00+05 已提交状态，文件域隔离：01=hnsw/+api，05=tokenizer/）。

### 01-hnsw 裁决（reviewer CHANGES_REQUESTED → fix 循环）
- **R-hnsw-vec（阻塞，已派 fix）**：hnsw.bin 嵌入向量（write_hnsw 每节点追加 dim*4，HnswReader::open 读 Node.vector，search 用 self.nodes[e].vector）。违反 §6.2 + README graph-only；双存 10万≈321MB 逼近 500MB、50万≈1.6GB 违反 §3.3 不塌红线。**裁决方案 B（零冗余）**：hnsw.bin 改 graph-only，HnswReader::search 增 `vectors: &[f32]` 参数（api 传 reader.vectors()，与 SegmentReader 共享单一副本）。01-fix（opus）修复中。
- **测试名实不符**：`api_hnsw_recall_vs_brute_at_least_95pct` 仅断言 10 条 → 改名（并入 fix）。
- **ef_search 公式** `max(ef_construction, want*4)`：接受（recall 有利，12-regression 验证）。
- 11 维度全 ✅（算法/filter/Q-5/串行/I-3/M0 签名/hnsw.bin 头/测试质量）。

### 02-tombstone-merge 裁决（reviewer APPROVED_WITH_MINOR）
- **next_docid 重置**：无需修（stale-high 经 seg_offsets 按段映射无害；重置反与 buffer docid 碰撞）。
- **partial auto-merge base=0 碰撞（真实缺陷，排队 02-fix）**：`merge_segments` 硬编码 base=0，auto_merge 合并 2/N 段时若非合并段含 base=0 段→docid 重叠→search 误命中/fusion 丢文档/污染 filter。**Option A 修复**：partial merge `target_docid_base=max(非合并段 base+count)`，compact 全合并保 0。**06 reindex 复用 MergeTask 前必须修**（03 后派 02-fix）。
- M-minor-2：header.bin tombstone abs/local 语义 04 WAL 前明确。
- M-minor-1：compacting 标志非 panic-safe，建议 Drop guard。

### Parked minors（后续 housekeeping pass 修）
- 05: is_cjk 代码复制（jieba/mod.rs:113-127 复制 cjk_bigram.rs:97-111）→ 改 cjk_bigram is_cjk 为 `pub(crate)` 共享（~3 行）。
- 05: UserTrie 重复词条 last-write-wins（seg.rs:46）vs §5.3「freq 高者优先」→ 改 `self.freqs[node].max(freq)`（~1 行）。不影响正确性（DAG DP 自然保证高频胜出）。
- 00: R-00-1/R-00-2 见上（text() 契约 / format_version v1-keep 护栏）。

### 10-ci-m1 必修（deny 门禁预存问题，非任何模块引入）
- ci.yml `cargo deny check --workspace` 错误参数（cargo-deny 0.16.4 不接受 `--workspace`）→ 改 `cargo deny check`（M0 10-ci-gates 遗留，CI 从未真正跑 deny）。
- deny.toml regex ban 须豁免 build-dep/proc-macro：regex 来自 napi-derive-backend（build-dep，编译期工具不进 core/wasm 运行时），非运行时依赖。ban 意图是 core/wasm 运行时，应 skip build-deps。
- cargo-deny 0.16.4 不能解析 CVSS 4.0 advisory（RUSTSEC-2026-0073）→ 升级 cargo-deny 或 pin advisory-db。
- 05 验收① 200 句 fixture 需在有 jieba-rs 环境离线生成 `tests/fixtures/jieba_200.txt`（jieba-rs 因 regex 黑名单不引入运行时/dev-dep）。

### 待 plan-splitter 修订项（两名 reviewer 收敛后一次性派发）
- B1 原文持久化（新增计划任务或并入 02/06）+ corpus 兼容测试更新。
- B2 MergeTask::new 扩展 tokenizer 参数。
- M1 12-recall Task 2 placeholder 修复。
- M2 README §05 文本统一（依 R-3 最终裁决）。
- R-3/R-4/R-6 依用户裁决落实。

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
