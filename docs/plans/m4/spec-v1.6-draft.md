# SPEC v1.6 修订草案（v1.5 → v1.6，用户已批准）

本草案是 M4（生产门槛：数据安全测试 + 可观测性）产出的 SPEC 回写提案。用户已批准（2026-08-12），§9/§10 节已根据最终实现更新（FFI 已实现 + ErrorContext 结构化），由 6b-apply 步骤应用到 `docs/SPEC.md`。

## 版本号校正说明

`docs/SPEC.md` 第 1 行 `# Vane 技术规范（SPEC v1.5）`。v1.5 由 M3 于 2026-08-11 落地（commit `5d092f8`），changelog 见 SPEC.md 末尾 v1.5 条目（约 466 行），覆盖 §12.1/§12.2/§12.3/§12.4/§13.2，**明确"不触碰 §1-§11 / §13.1 / §13.3 / §14 / §15"**。因此 M4 的 §9/§10/§13.2-新增/§14 修订是 **v1.5 → v1.6**。

> **注意**：`docs/plans/m4/phase0-design.md` §5 标题（816 行）写的"v1.4 → v1.5"已**过时**——该设计文档写于 M4 Phase 0，误以为 SPEC 还在 v1.4。本草案更正为 v1.5 → v1.6。草案文件名用 `spec-v1.6-draft.md`（不是 spec-v1.5-draft.md）。

## 四节修订概览

| 节 | 修订类型 | 要点 |
|---|---|---|
| §9 FFI 规范 | 补列 inspect API 函数 | core + FFI/Node/Wasm 全实现 [M4] |
| §10 错误码 | 表后补注释 | ErrorContext 结构化字段替代 String 拼接，错误码 -1..-11 不变 |
| §13.2 质量门禁 | 新增第 6-11 项 | fuzz/崩溃恢复/兼容/压测/proptest |
| §14 不变量 I-5 | 扩展注释 | tracing feature 能力开关释义 |

---

## §9 — FFI 规范（C ABI）— inspect API 补列

**v1.5 现状**：

SPEC.md §9.2 函数面（305-325 行）列有 M0-M3 冻结的 FFI 函数签名（`vane_open` / `vane_collection` / `vane_add` / `vane_flush` / `vane_search` / `vane_delete` / `vane_compact` / `vane_reindex` / `vane_export` / `vane_close` / `vane_string_free` / `vane_last_error_message`）+ v1.1 补列（`vane_reindex_progress` / `vane_reindex_wait` / `vane_load_dict` / `vane_dict_version`）。无 inspect/stats 相关函数。§9.3 Node 例外（327-329 行）说明 Node 不经 C ABI。

**v1.6 修订提案**：

在 §9.2 函数面 v1.1 补列块之后、325 行参数/返回注释之前，加 **v1.6 补列**块：

```
**v1.6 补列**（M4 inspect API，core + FFI/Node/Wasm 全实现）：
vane_db_stats(db_h, out_arena*) -> i32              // DbStats JSON
vane_db_segment_info(db_h, out_arena*) -> i32       // Vec<SegmentInfo> JSON
```

> **实现状态**：core 层 `Db::stats()` / `Db::segment_info()` / `Db::collection_segment_info()` 已实现 [M4, commit `684a112`]（`crates/vane-core/src/api/inspect.rs` + `crates/vane-core/src/api/db.rs`）。FFI `vane_db_stats` / `vane_db_segment_info` + Node `stats()`/`segmentInfo()` + Wasm 2 函数全实现 [M4, commit `5143885`]。core 层 inspect API 纯新增，不改 M0-M3 冻结 FFI 签名。

**返回结构体字段概要**（7 structs/enums，源自 `crates/vane-core/src/api/inspect.rs`）：

```
DbStats {
    db_path: String,
    collections: [CollectionStats],
    dict_available: bool,          // jieba 词典是否加载
    executor_kind: ExecutorKind,   // Serial | Rayon
}

CollectionStats {
    name: String,
    segment_count: usize,
    total_docs: u64,               // 含 tombstoned
    live_docs: u64,                // total - tombstoned
    tombstoned_docs: u64,
    index_bytes: u64,              // 各段文件大小之和
    dict_state: DictState,
    tokenizer_id: TokenizerId,
    health: Health,
}

SegmentInfo {
    ulid: String,
    doc_count: u32,
    docid_base: u64,
    tombstoned_count: u64,
    format_versions: FormatVersions,
    file_sizes: SegmentFileSizes,
    health: Health,
}

FormatVersions {
    header: u32, vectors: u32, stored: u32, idmap: u32,
    scalars: u32, inverted: u32, hnsw: u32,
}

SegmentFileSizes {
    header: u64, vectors: u64, stored: u64, idmap: u64,
    scalars: u64, inverted: u64, hnsw: Option<u64>,  // None = 无 hnsw.bin
}

Health = Healthy | Degraded | Corrupt
  // Healthy: 段可 open + hnsw 存在
  // Degraded: 词典降级 / hnsw 缺失 fallback brute / 段文件部分缺失但可读
  // Corrupt: SegmentReader::open 失败（magic/version/CRC 校验失败）

ExecutorKind = Serial | Rayon
```

> **健康检查语义**：inspect 重新 open 段做健康检查——`SegmentReader::open` 失败 → `Corrupt`；open 成功但 hnsw 缺失 → `Degraded`（fallback brute）；否则 `Healthy`。inspect 非热路径，重新 open 可接受且能真实检测段损坏。`index_bytes` / `file_sizes` 用 `read_at` 探测 EOF 累计推算（Vfs trait 无 `size()` 方法，M0 冻结签名）。

**rationale**：

M4 Phase 5b 交付了 core 层 inspect API（commit `684a112`）+ FFI/Node/Wasm 三绑定层（commit `5143885`），提供 DB 级统计与段级信息，支持健康检查（词典降级/段损坏/hnsw 缺失）。phase0-design.md §5（820-825 行）假设 FFI 层同步加 `vane_db_stats`/`vane_db_segment_info`——实际实现与设计一致，FFI/Node/Wasm 全部落地。`crates/vane-ffi/src/lib.rs`（`vane_db_stats`/`vane_db_segment_info`）+ `crates/vane-node/src/`（`stats()`/`segmentInfo()`）+ `crates/vane-wasm/src/`（2 函数）已验证。

**不触碰**：

§9.1 约定（句柄/错误/内存铁律/并发，298-303 行）不变；§9.2 M0-M3 冻结函数签名（308-323 行）不变；§9.3 Node 例外（327-329 行）不变；参数/返回 JSON 序列化约定（325 行）不变——inspect 返回结构体经 FFI 层序列化为 JSON（binding 薄壳原则）。

---

## §10 — 错误码 — 诊断上下文注释

**v1.5 现状**：

SPEC.md §10（333-350 行）列错误码表（0=OK, -1=E_IO … -11=E_INVALID_ARG），末行"三侧绑定透传 code，不得吞并/重编"（350 行）。无诊断上下文相关注释。`VaneError` 各变体 String payload 是诊断信息，但无上下文结构化约定。

**v1.6 修订提案**：

**不改错误码表**（-1..-11 不变，三侧绑定透传不变）。在 §10 表后、350 行"三侧绑定透传"之后加注释：

```
> **v1.6 注**（M4 诊断架构重构）：`VaneError` 11 变体统一携带 `ErrorContext`
> struct（`message: String` + `seg: Option<String>` + `docid: Option<u64>`
> + `op: Option<&'static str>` + `hint: Option<String>`），替代旧 String
> payload。`ErrorContext` 提供 builder 链式 `.seg()`/`.docid()`/`.op()`/`.hint()`
> + `From<String>`/`From<&str>`；`VaneError` 提供 `with_seg()`/`with_docid()`
> /`with_op()`/`with_hint()` pub(crate) 方法（替代旧 `append_context`）。
> `VaneError::context()` pub 方法返回 `&ErrorContext`（消费者程序化访问字段，
> 无需 parse Display）。**错误码 -1..-11 + 名称 E_IO 等不变**（§10 表硬约束）。
> Display 新格式 `E_CODE: message [seg=... op=... docid=... hint=...]`
> （None 字段省略）。实现：commit `c34e473`（主重构）+ `d9dcc5f`（fix）。
```

**rationale**：

M4 Phase 6b 重构 `VaneError` 诊断架构（commit `c34e473` + `d9dcc5f`）：11 变体统一携带 `ErrorContext` struct，替代旧 String payload + `append_context` 拼接模式。消费者经 `context()` pub 方法程序化访问 seg/docid/op/hint 字段，无需 parse Display 字符串。phase0-design.md §5（829-833 行）推荐"先丰富 String，结构化上下文列为 Could"——实际实现超越设计提议，直接结构化落地（ErrorContext struct + builder 链式 + `context()` pub 方法 + `with_*` pub(crate) 替代 `append_context`）。错误码 -1..-11 + 名称不变（§10 表硬约束），Display 新格式 `E_CODE: message [seg=... op=... docid=... hint=...]`（None 省略）。

**不触碰**：

错误码表（335-348 行）不变；错误码名称/含义不变；"三侧绑定透传 code，不得吞并/重编"不变。

---

## §13.2 — 质量门禁 — 新增第 6-11 项

**v1.5 现状**：

SPEC.md §13.2（418-424 行）列 5 项质量门禁：① hybrid recall@10 ≥ 0.95；② 中文分词（M1）；③ 体积 wasm gzip ≤ 800KB；④ 平台四包管理器安装矩阵；⑤ Web npm 安装门禁 [M3]。第 5 项由 v1.5（M3）新增。

**v1.6 修订提案**：

在 §13.2 第 5 项（424 行）之后新增第 6-11 项：

```
6. fuzz-smoke [M4]：cargo-fuzz 每 target 60s 短跑（push/PR），nightly
   toolchain + `-Z sanitizer`，5 targets（brute_search_fuzz /
   hnsw_search_fuzz / persist_roundtrip_fuzz / merge_fuzz /
   dict_load_fuzz）无 panic/crash。CI job `fuzz-smoke`（commit `b4aa743`）。
7. fuzz-long [M4]：cargo-fuzz 每 target 10min 长跑（cron 周日 03:00 UTC
   + workflow_dispatch），`-max_total_time=600 -max_len=65536`，crash
   不阻断 job（`|| true` 容错）但上传 crash artifact。CI job `fuzz-long`
   （commit `b4aa743`）。
8. 崩溃恢复 [M4]：FaultVfs 注入 5 场景（meta_slot 翻转 / WAL flush
   中断 / merge 中断 / ENOSPC / 部分写）全通过，崩溃后 manifest 指向
   完整状态、数据一致（`tests/crash_recovery.rs` --features fault-injection
   --release）。CI job `crash-recovery`（commit `b4aa743`）。
9. 跨版本兼容 [M4]：v0.1.0 真实 fixture 当前版本读取通过
   （`tests/cross_version_compat.rs` --all-features --release，覆盖
   zstd-encode 分支）。CI job `compat`（commit `b4aa743`）。
10. 并发压测 [M4]：多线程 search+insert+flush+merge N 轮，timeout 内
    无 panic/死锁/数据不一致（`tests/stress_concurrency.rs` --release
    ×3 multi-run 捕捉低概率竞态）。CI job `stress`（commit `b4aa743`）。
11. proptest 不变量 [M4]：检索稳定 / round-trip / merge 不丢 256 cases
    全通过（`tests/proptest_invariants.rs`，`proptest-regressions/` 提交
    确保 CI 复现）。CI test job 覆盖（commit `f849c7b` + `34a9b11`）。
```

### §13.3 工程纪律门禁 — 补注

**v1.5 现状**：

SPEC.md §13.3（426-431 行）列 4 项工程纪律：① core 出现 `std::fs` 即失败；② cargo-deny + cargo bloat + 依赖黑名单（regex / tokio 全套 / prost / tonic / openssl / lindera / ndarray / wee_alloc）；③ 冻结 corpus 格式兼容测试；④ benchmark CI 性能回退 >10% 报警。

**v1.6 修订提案**：

**不改黑名单列表本身**。在 §13.3 第 2 项（429 行 cargo-deny 行）之后补注：

```
> **v1.6 注**（M4 dev/optional 依赖）：dev/optional 依赖（tracing /
> proptest / cargo-fuzz / libfuzzer-sys）不触运行时依赖黑名单
> （regex / tokio / prost / tonic / openssl / lindera / ndarray /
> wee_alloc / dashmap / parking_lot），cargo-deny 守护。libfuzzer-sys
> license = `(MIT OR Apache-2.0) AND NCSA` 已在 `deny.toml` [licenses]
> allow NCSA（commit `9e262db`）——NCSA 是 libFuzzer C++ 库许可证
> （OSI approved + FSF Free/Libre），仅 license 允许，不改 [bans]
> crate 黑名单语义。
```

**rationale**：

M4 Phase 6a 新增 5 个 CI job（commit `b4aa743`）：fuzz-smoke / fuzz-long / compat / stress / crash-recovery，对应 M4 Phase 1-4 产出的测试套件。proptest 不变量由现有 test job 覆盖（`tests/proptest_invariants.rs`）。5 个 fuzz targets 在 `crates/vane-fuzz/fuzz_targets/`（brute_search_fuzz / hnsw_search_fuzz / persist_roundtrip_fuzz / merge_fuzz / dict_load_fuzz）。proptest-regressions 目录在 `crates/vane-core/proptest-regressions/`（提交确保 CI 复现）。

§13.3 补注澄清 dev/optional 依赖与运行时黑名单的边界：tracing（optional feature）、proptest（dev-dependency）、cargo-fuzz + libfuzzer-sys（fuzz crate，workspace default-members 排除）均不进入 vane-core/wasm/ffi 生产构建，cargo-deny 守护。libfuzzer-sys 的 NCSA 许可证已在 deny.toml allow（commit `9e262db`）。

phase0-design.md §5（838-845 行）提议的门禁编号与实现一致（6-11 项），但第 8 项描述"meta_slot/WAL/merge/ENOSPC/部分写"与实现完全一致（`crash_recovery.rs` 5 场景）。无偏差。

**不触碰**：

§13.2 第 1-5 项（420-424 行）不变；§13.3 黑名单列表（429 行）不变；§13.3 第 1/3/4 项（428/430/431 行）不变；§13.1 性能承诺（408-416 行）不变。

---

## §14 — 不变量 I-5 — tracing feature 扩展

**v1.5 现状**：

SPEC.md §14 I-5（441-442 行）：

> **I-5 核心零平台分支**：core 算法代码无 `cfg(target_arch)`/`cfg(target_os)` 平台分支；平台差异仅在 VFS/Executor 实现。
> - 注：`cfg(feature)` 用于存储编解码能力开关（如 zstd-encode）允许出现在 segment 编解码处；`cfg(target_feature)` 用于同一算法的向量化/标量双实现（如 f32 距离核的 simd128/标量双路径）视为能力开关（类似 `cfg(feature)`），允许出现在算法代码中；向量化能力门控可与 `target_arch` 组合（如 `cfg(all(target_arch = "wasm32", target_feature = "simd128"))`——simd128 仅在 wasm32 有意义，`target_arch` 在此是能力定位而非平台分支），组合整体仍视为能力开关；`cfg(target_arch)`/`cfg(target_os)` 平台分支仍仅限 VFS/Executor 实现。

v1.4（post-v0.1.1）已扩展 `cfg(target_feature)` 释义。无 tracing feature 相关注释。

**v1.6 修订提案**：

I-5 核心（核心零平台分支）**不变**。在 442 行注释末尾扩展：

```
  - 注（v1.6，M4 可观测性）：`cfg(feature="tracing")` 是可观测性能力开关
    （类似 zstd-encode），允许出现在 api/segment/persistence/wal 模块的
    埋点位置（span/info/debug 宏调用）。不启用时编译期消除
    （`grep -c tracing = 0` 验证），wasm/native 体积不变
    （800KB gzip 红线守护，tracing off 时 vane-wasm ~352KB / core
    --export-all ~650KB）。tracing crate 传递依赖
    （pin-project-lite / tracing-attributes / tracing-core / once_cell）
    不触依赖黑名单，cargo-deny 守护。参照 commit `dae29c6`。
```

**9 埋点位置**（commit `dae29c6`，`cfg(feature="tracing")` 门控）：

| # | 位置 | 语义 |
|---|---|---|
| 1 | `vfs/page_cache.rs` | PageCache hit |
| 2 | `vfs/page_cache.rs` | PageCache miss |
| 3 | `wal/mod.rs` | Wal::append |
| 4 | `api/collection.rs` | flush done（段写入完成） |
| 5 | `api/collection.rs` | merge start（源段 → 目标段） |
| 6 | `api/collection.rs` | merge done（新段可见） |
| 7 | `api/collection.rs` | search span（检索起点） |
| 8 | `api/collection.rs` | search elapsed（检索延迟 p50/p99） |
| 9 | `api/collection.rs` | 词典状态机转换（Stable→PendingReindex→Rebuilding→Stable，3 transition 点） |

**rationale**：

M4 Phase 5a 交付 tracing feature（commit `dae29c6`）：`tracing = { version = "0.1", optional = true }` + `tracing = ["dep:tracing"]` 在 `crates/vane-core/Cargo.toml`，默认 off。9 个埋点位置经 `#[cfg(feature = "tracing")]` 门控，不启用时编译期消除（宏调用展开为空），wasm/native 体积不变。tracing crate 传递依赖（pin-project-lite / tracing-attributes / tracing-core / once_cell）不触运行时依赖黑名单（regex/tokio/prost/tonic/openssl/lindera/ndarray/wee_alloc/dashmap/parking_lot），cargo-deny 守护。

phase0-design.md §5（849-856 行）提议的 tracing feature 释义与实现一致。I-5 不变量核心（核心零平台分支）不变——tracing 是 feature 能力开关，非平台分支。

**不触碰**：

I-5 核心断言（441 行"core 算法代码无 `cfg(target_arch)`/`cfg(target_os)` 平台分支；平台差异仅在 VFS/Executor 实现"）不变；I-5 v1.2/v1.4 释义（`cfg(feature)` zstd-encode / `cfg(target_feature)` simd128）不变；I-1 至 I-4、I-6 至 I-8（437-446 行）不变。

---

## Changelog 草案条目（待 apply 时追加到 SPEC.md changelog v1.5 条目之后）

```
- **v1.6**（2026-08-13）：M4 生产门槛（数据安全测试+可观测性）后四处修订（用户批准）。S1 §9.2 补列 inspect API FFI 函数面 vane_db_stats/vane_db_segment_info（core 层 Db::stats/segment_info/collection_segment_info [M4, 684a112] + FFI/Node/Wasm 三绑定层 [M4, 5143885] 全实现）。S2 §10 错误码表后补 ErrorContext 结构化注（11 变体统一携带 ErrorContext struct [message+seg+docid+op+hint]，builder 链式 + with_* pub(crate) + context() pub，替代旧 String 拼接 + append_context，错误码 -1..-11 不变，c34e473+d9dcc5f）。S3 §13.2 新增第 6-11 项质量门禁（fuzz-smoke/fuzz-long/崩溃恢复/跨版本/并发压测/proptest）+ §13.3 补 dev/optional 依赖不触黑名单注。S4 §14 I-5 扩展 tracing feature 能力开关释义（编译期消除，体积不变，不触黑名单，dae29c6）。不触碰 §1-§8 / §11 / §12 / §13.1 / §13.3 黑名单列表 / §15（M4 不碰 core 检索语义、分发矩阵、性能承诺、里程碑验收）。
```

---

## 待用户批准检查点

> **用户已批准**（AskUserQuestion 检查点，2026-08-12）：Q1 §9 FFI inspect 立即实现（非顺延，FFI 已落地 5143885）；Q2 §10 诊断选精简正确方式（ErrorContext 结构化，c34e473+d9dcc5f）；Q3 §13.2+6 门禁 + §13.3 注 + §14 I-5 tracing 全批准；Q4 版本号 v1.6 一次性批准。以下原决策清单保留作历史记录。

供编排者 AskUserQuestion 的决策清单：

1. **版本号校正**：SPEC 已 v1.5（M3），M4 修订为 v1.5 → v1.6（phase0-design §5 "v1.4 → v1.5" 标题过时已更正）。是否接受 v1.6 版本号？

2. **§9 inspect FFI**：core 层 `Db::stats()` / `Db::segment_info()` / `Db::collection_segment_info()` 已实现 [M4, 684a112]，FFI/Node/Wasm 绑定层 inspect 函数已实现 [M4, 5143885]。是否接受已实现标注？

3. **§10 诊断上下文**：ErrorContext 结构化字段替代 String 拼接（11 变体统一携带 ErrorContext struct，builder 链式 + with_* pub(crate) + context() pub，错误码 -1..-11 不变，c34e473+d9dcc5f）。是否接受此路径？

4. **§13.2 +6 门禁 + §13.3 dev/optional 依赖注**：新增第 6-11 项质量门禁 + §13.3 补 dev/optional 依赖不触黑名单注。是否接受？

5. **§14 I-5 tracing feature 扩展**：`cfg(feature="tracing")` 能力开关释义扩展（编译期消除，体积不变，不触黑名单）。是否接受？

6. **批准方式**：四节是否一次性批准（vs 分批 AskUserQuestion）？

---

## 附：M4 实现与 phase0-design.md §5 提议的偏差记录

| 项 | phase0-design.md §5 提议 | 实际实现 | 草案处理 |
|---|---|---|---|
| 版本号 | v1.4 → v1.5（816 行标题） | SPEC 已 v1.5（M3），M4 是 v1.5 → v1.6 | 更正为 v1.5 → v1.6 |
| §9 FFI inspect | 假设 FFI 层加 `vane_db_stats`/`vane_db_segment_info`（820-824 行） | core + FFI/Node/Wasm 全实现（684a112+5143885） | 与设计一致，全实现 |
| §13.2 第 8 项 | "meta_slot/WAL/merge/ENOSPC/部分写"（841 行） | 完全一致（`crash_recovery.rs` 5 场景） | 无偏差 |
| §14 tracing | 传递依赖无 regex/tokio/prost/tonic/openssl/lindera/ndarray/wee_alloc/dashmap/parking_lot（854-855 行） | 实际传递依赖为 pin-project-lite/tracing-attributes/tracing-core/once_cell，确实不触黑名单 | 补充实际传递依赖名称 |
| proptest-regressions | phase0-design §6 风险 3（867 行）提及需提交 | 已提交于 `crates/vane-core/proptest-regressions/`（非 `tests/` 子目录） | 路径精确标注 |
