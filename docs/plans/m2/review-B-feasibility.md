# Reviewer B 评审报告 — 可实现性 + TDD + 代码兼容视角

> 评审对象：`docs/plans/m2/README.md` + `modules/M2-01~14-*.md`
> 评审视角：TDD 可执行性、文件目标真实性、可实现性核查、800KB 体积门禁、依赖黑名单
> 日期：2026-08-09
> 状态：**PASS_WITH_FINDINGS**（阻塞 1 / 重要 6 / 次要 10）

---

## 阻塞（B）

### B-1 M2-02 OPFS Vfs：SyncAccessHandle API 不覆盖 Vfs trait 的目录操作，同步实现方案缺失

**计划**：`M2-02-opfs-vfs.md` §3 接口契约 / §4 TDD 测试清单
**证据**：
- 计划 §3 声称 `impl Vfs for OpfsVfs { /* 8 方法，均同步：SyncAccessHandle.read/write/flush */ }`，仅用 SyncAccessHandle 的 read/write/flush 覆盖 8 方法。
- 实际 `FileSystemSyncAccessHandle` API 仅提供：`read(buffer, {at})` / `write(buffer, {at})` / `flush()` / `getSize()` / `close()` / `truncate()`。**不提供** 目录操作。
- Vfs trait（`crates/vane-core/src/vfs/mod.rs:5-13`）8 方法中：
  - `create(path)`：创建新文件需 `FileSystemDirectoryHandle.getFileHandle(name, {create:true})` + `createSyncAccessHandle()`，**两者均异步**（返回 Promise）。
  - `rename(from, to)`：需 `FileSystemFileHandle.move()` 或 copy+delete，**异步**。
  - `delete(path)`：需 `FileSystemDirectoryHandle.removeEntry()`，**异步**。
  - `list(dir)`：需 `FileSystemDirectoryHandle.entries()` 异步迭代器，**异步**。
- 计划 §4 测试 9「create("segments/seg_x/header.bin") 自动创建中间目录（OPFS getDirectoryHandle(create=true) 递归）」——`getDirectoryHandle` 是异步方法，无法在同步 Vfs trait 方法内调用。
- 计划未给出任何同步桥设计（M2-03 IDB 有「同步桥」节讨论 Atomics.wait，M2-02 完全没有对应章节）。

**影响**：OpfsVfs 的 4 个方法（create/rename/delete/list）在纯同步 SyncAccessHandle 下无法实现；要么需引入 M2-03 式同步桥（大幅增加复杂度且计划未提及），要么需重新设计 Vfs trait（破坏 M0 冻结签名）。这是 M2-02 能否落地的核心阻塞。
**建议**：计划必须补充「同步桥设计」节，明确 Worker init 预获取 directory/file handles 缓存策略 + create/rename/delete/list 的同步等待方案（Atomics.wait / SharedArrayBuffer / 预创建所有可能路径），并评估可行性；或与 SPEC 组讨论 Vfs trait 是否需为 wasm 提供异步变体。

---

## 重要（I）

### I-1 M2-08：文件路径错误 — `segment/scalars.rs` 不存在

**计划**：`M2-08-stored-zstd.md` §2 涉及文件
**证据**：
- 计划写「Modify `crates/vane-core/src/segment/scalars.rs`（`ScalarReader::decode_scalars`）：`FORMAT_VERSION` → `SCALARS_FORMAT_V1`」。
- 实际 `find crates/vane-core/src -name "*scalar*"` 无结果；`ScalarReader` 与 `decode_scalars` 定义在 `crates/vane-core/src/segment/mod.rs:569+`（`pub struct ScalarReader` 在 mod.rs:583，`impl ScalarReader` 在 mod.rs:587，`decode_scalars` 在 mod.rs:600 附近）。
**影响**：执行时找不到文件，需改为 `segment/mod.rs`。属文件目标不真实。

### I-2 M2-09：SQ8 距离计算未覆盖 L2/dot 两种 metric

**计划**：`M2-09-sq8.md` §3 接口契约 / §4 TDD 测试清单
**证据**：
- 计划定义 `sq8_distance(sq8_a: &[u8], sq8_b: &[u8], dim: u32) -> f32`——单一函数，无 metric 参数。
- `brute_search_sq8(sq8, dim, query, topk, filter)` 签名也无 metric 参数。
- 但 collection schema 支持 cosine/L2/dot 三种 metric（SPEC §3.1/§8），`brute_search` 原函数通过 metric 参数区分距离计算。
- 测试 3 只断言「sq8_distance vs f32 **cosine** 距离误差 <1e-3」，L2 和 dot 两种 metric 的 SQ8 距离适配无测试、无实现说明。
**影响**：启用 sq8 feature 后，L2/dot metric 的暴力回退路径距离计算错误或无法编译。需补充 metric 参数或提供三个距离函数。

### I-3 M2-12：export 快照恢复路径未定义，P0-3 数据主权承诺未完整满足

**计划**：`M2-12-export-snapshot.md` §4 测试 4 / §5 验收标准
**证据**：
- 测试 4「快照恢复（**若实装** `read_snapshot`）... 若不实装恢复，文档明示 export 仅作备份，恢复用 `Db::open` 直接打开快照目录（若快照是目录拷贝）或解包脚本」——条件性测试，TDD 不明确。
- 但快照格式是**单文件**（`magic|version|num_files|{path|file}...`），`Db::open` 期望**目录路径**，不能直接打开单文件快照。测试 4 所述「Db::open 直接打开快照目录」与单文件格式矛盾。
- REQUIREMENTS §1 P0-3「浏览器数据主权：OPFS 被驱逐构成产品事故，必须提供快照导出」——导出后需可恢复才满足主权承诺。
- §5 验收标准只说「export 成功」「三侧可用」，无「恢复可用」验收点。
**影响**：export 落地但恢复路径空缺，P0-3 数据主权承诺半成品。需明确恢复方案（`read_snapshot` 实装或提供解包工具/脚本并测试）。

### I-4 M2-04：wasm-bindgen-futures 体积未在 README 体积评估表登记

**计划**：`M2-04-worker-shell.md` §2 / README §「新依赖体积评估」表
**证据**：
- M2-04 §2 引入 `wasm-bindgen-futures`（`[features] worker = ["dep:web-sys", "dep:js-sys", "dep:wasm-bindgen-futures"]`）。
- README §「新依赖体积评估」表（line 378-385）列了 wasm-bindgen / web-sys / js-sys / ruzstd / zstd / rayon / cbindgen，**未列 wasm-bindgen-futures**。
- wasm-bindgen-futures 引入 Promise/Future 桥接，体积不可忽略（预估 +5~15KB gzip）。
**影响**：800KB 门禁预算缺一项，体积评估不完整。

### I-5 体积预算全局累计管理缺失

**计划**：README §「新依赖体积评估」+ 各模块验收「≤800KB」
**证据**：
- 各模块（M2-01/02/04/05/08）独立声称「启用 feature 后 ≤800KB」，但无全局预算分配表（如 vane-wasm baseline + web-sys[opfs+idb+worker] + wasm-bindgen-futures + ruzstd 累计）。
- web-sys 多 feature（FileSystemSyncAccessHandle + IdbDatabase + Worker 等）叠加体积非线性；README 估「+30~80KB 按启用 feature」但未区分 opfs/idb/worker 三 feature 同时启用时的累计值。
- ruzstd（+30~60KB）+ web-sys（+30~80KB）+ wasm-bindgen-futures（未估）+ vane-core 代码（jieba feature 代码虽不启但 zstd-decode 启）累加后 800KB 余量紧张，无预算追踪机制。
**影响**：后期某模块引入后可能整体超 800KB 才发现，回溯成本高。建议建立累计预算表（baseline + 各 feature 增量实测）并在每模块落地时更新。

### I-6 M2-05：LLVM 自动向量化有效性未验证，双产物可能名义存在

**计划**：`M2-05-simd128-variants.md` §2 首选方案 / §4 测试 2
**证据**：
- 首选方案「core 不引入手写 SIMD intrinsics，依赖 LLVM 自动向量化（`-Ctarget-feature=+simd128` 时编译器自动向量化 f32 距离循环）」。
- 测试 2「`wasm-objdump -x` 显示 `features: simd128`」——只验证 wasm 模块的 feature flag 开启，**不验证**实际生成 simd128 指令。
- 若 LLVM 未对 brute_search/HNSW 距离循环实际向量化（循环结构/数据布局不满足向量化条件），simd 产物与 scalar 产物二进制等价，双产物构建失去性能意义，M2-06 召回回归也必然通过（无数值分歧）。
- 计划无「反汇编验证 simd128 指令存在」或「simd 产物性能优于 scalar」的测试。
**影响**：可能交付一个名义 simd128 但无实际 SIMD 收益的产物，浪费构建复杂度。建议增测试：`wasm-objdump -d` 反汇编 grep simd128 指令（如 `f32x4`），或 simd 产物 distance 循环 bench 优于 scalar。

---

## 次要（M）

### M-1 M2-08：`header.rs` 行号「21,40」不准确
**证据**：计划 §2 写 `header.rs:21,40`；实际 `grep -n FORMAT_VERSION header.rs` 命中 2/21/46/49。行号 40 应为 46。

### M-2 M2-10：Db 持有 `executor: Arc<dyn Executor>` 字段，cfg 选择位置未明确，I-5 风险
**证据**：M2-10 §2「`Db` 增 `executor: Arc<dyn Executor>` 字段（open 时构造，cfg 决定 native/串行）」。若 `cfg(target_arch="wasm32")` 出现在 `api/db.rs` 选择 executor 类型，则违反 I-5（cfg(target) 仅限 executor/mod.rs + vfs）。建议在 `executor/mod.rs` 提供 `pub fn default_executor() -> Arc<dyn Executor>` 工厂函数，cfg 集中在该文件。计划未明确此设计。

### M-3 M2-10：I-5 守护测试 4 grep 表述有误
**证据**：测试 4 写「vfs（M0 memory/std-fs 非 target_arch，是 trait impl）」——实际 `vfs/mod.rs:18` 是 `#[cfg(not(target_arch = "wasm32"))] pub mod std_fs;`，**是 target_arch 分支**。grep `cfg(target_arch` 会命中 std_fs，测试描述与实际不符，可能误导执行者误判为违规。

### M-4 M2-11：wazero 形态实现路径未详述
**证据**：M2-11 §2 只列 `bindings/go/wazero/` 目录，未说明其内容。wazero 形态需 vane-core 编译为 wasm + Go wazero host 代码，与 cgo staticlib 形态完全不同。计划「build tag 切换」未列具体步骤（wasm 产物来源、wazero host 封装、Go API 对齐）。建议引用 M1 README §09 wazero 契约或补充实现步骤。

### M-5 M2-09：`sq8_query_distance` 中 query（f32）与 sq8_vectors（u8）混合距离计算方式未说明
**证据**：`sq8_query_distance(sq8_vectors: &[u8], query: &[f32], dim, topk)`——query 是 f32，sq8_vectors 是 u8。距离计算需先将 query 量化为 SQ8（需 min/max），或用混合距离（f32 query × u8 解码）。计划未说明。属实现细节缺失，非阻塞。

### M-6 M2-13：fixture 体积评估偏乐观
**证据**：计划「500 篇 × 平均 1KB = ~500KB，总体积 ≤1MB」。但 fixture 要求「200~2000 字」，中文 UTF-8 每 3 字节/字，2000 字 ≈ 6KB，500 篇平均 3KB ≈ 1.5MB。体积评估偏乐观，可能超 1MB 门禁。

### M-7 M2-13：测试 8「领域覆盖：人工抽检，非自动测试」——TDD 不可执行
**证据**：测试 8 标注「非自动测试」。TDD 要求可自动化执行，人工抽检不进 CI。建议改为自动统计（如 fixture 含科技/历史/地理关键词分布的断言）或移出测试清单列为验收检查项。

### M-8 M2-14：测试清单「行为测试，非 unit test」——TDD 顺序不清晰
**证据**：M2-14 §4 标注「行为测试，非 unit test」，10 个测试无自动化框架（Demo 是前端页面）。TDD 先写失败测试→实现→通过的流程不适用。建议明确 Demo 验收用 e2e 脚本（如 Playwright/headless browser）或降级为人工验收清单。

### M-9 M2-06：测试 4「数值分歧检测」是诊断步骤非测试断言
**证据**：测试 4「若 Jaccard <0.99，记录分歧查询 + 评估是否 SIMD 路径 bug」——这是诊断流程不是可断言的测试。TDD 要求明确 pass/fail 条件。建议改为「Jaccard ≥0.99 硬断言，失败时附诊断信息」。

### M-10 M2-12：递归 list segments/seg_*/ 的逻辑未说明
**证据**：M2-12 §3「遍历 manifest + 全部段文件 + wal.log → 打包写 dest」。`Vfs::list(dir)` 只列单层目录，段文件在 `segments/seg_<ulid>/` 子目录下。计划未说明如何递归遍历段目录（需 list("segments") 后对每个 seg_ list 再读文件）。属实现细节缺失。

---

## 已核查通过项

- **文件目标真实性**（除 I-1 外）：M2-07 引用的 `segment/mod.rs:315-325/344-394/417-419/442-456` 与实际 SegmentReader struct/open/vectors/stored_json 行号匹配；M2-08 `types.rs:15` FORMAT_VERSION 匹配；M2-08 `hnsw/mod.rs:533` 匹配；M2-09 `collection.rs:765,776` brute_search 调用点匹配；M2-11 vane-ffi stub（`lib.rs:1`）匹配；M2-01 vane-wasm 骨架（`lib.rs` vane_version）匹配；M2-12 `db.rs:164-166` export 占位匹配；M2-12 vane-node `db.rs:110` ExportTask 匹配；M2-10 `reindex.rs:142`/`merge/mod.rs:199`/`filter/mod.rs` compile_filter 引用大致匹配。
- **依赖黑名单**：计划未引入 regex/tokio/prost/tonic/openssl/lindera/ndarray/wee_alloc/dashmap/parking_lot。M2-11 明确用 `std::sync::RwLock`（非 dashmap）。rayon 仅 M2-10 executor native impl；wasm-bindgen/web-sys/js-sys 仅 vane-wasm binding。位置合法。
- **M2-07 懒加载**：OnceLock 字段改造不改 `&self` 签名，`vectors()`/`dim()`/`stored_json()`/`text()` 签名不变——与 `segment/mod.rs:417/420/442/450` 实际签名一致，兼容性 OK。
- **M2-08 双模读取**：v1 原路径 + v2 ruzstd 解压逻辑完整；zstd-encode（C 库）不进 wasm、ruzstd（纯 Rust）进 wasm 的分工合理；体积 +30~60KB 预估合理（待实测）。
- **M2-10 rayon 引入**：`executor-native` feature + cfg 仅 executor/mod.rs 两个 impl 块——不违反 I-5（前提 M-2 的 Db 字段选择位置处理得当）。
- **M2-11 cgo 步骤完整性**：cbindgen + zig cc 交叉 + wazero build tag 步骤可执行（除 M-4 wazero 细节外）；句柄注册表 RwLock + arena 分配/释放设计完整。

---

## 结论

M2 计划整体结构清晰、TDD 测试清单多数具体、文件引用基本真实。但 M2-02 OPFS Vfs 的 SyncAccessHandle 同步覆盖性问题是核心阻塞——4 个 Vfs 方法无法用纯同步 SyncAccessHandle 实现，计划未给出同步桥方案。其余 6 项重要发现（文件路径错误、SQ8 metric 覆盖、export 恢复路径、体积评估缺项、SIMD 有效性验证）需在落地前修订。建议阻塞项先行澄清，重要项修订计划后推进。
