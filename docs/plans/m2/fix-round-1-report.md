# M2 Phase One Fix Round 1 报告

> 角色：plan-splitter（fix 轮）
> 范围：修正双视角 reviewer（A 契约 + B 可行性）发现的全部阻塞/重要项 + OPFS VFS 设计重写。只改 `docs/plans/m2/` 下文件，不改代码/Cargo/SPEC。
> 输入：`review-A-contracts.md`（0 阻塞 / 5 重要 / 4 次要）、`review-B-feasibility.md`（1 阻塞 / 6 重要 / 10 次要）、`opfs-vfs-design.md`（路径 A，已评审）。
> 日期：2026-08-09

## 修订文件清单（11 份）

1. `modules/M2-02-opfs-vfs.md`（**重写**）
2. `modules/M2-03-idb-vfs.md`（重写）
3. `modules/M2-04-worker-shell.md`
4. `modules/M2-05-simd128-variants.md`
5. `modules/M2-06-simd-recall-regression.md`
6. `modules/M2-07-lazy-load.md`
7. `modules/M2-08-stored-zstd.md`
8. `modules/M2-09-sq8.md`
9. `modules/M2-10-million-executor.md`
10. `modules/M2-11-go-cgo-binding.md`
11. `modules/M2-12-export-snapshot.md`
12. `modules/M2-13-wiki-ndcg-corpus.md`
13. `modules/M2-14-demo.md`
14. `modules/M2-01-wasm-cdylib-size.md`
15. `README.md`

---

## 阻塞项（1）

### B-1 M2-02 OPFS VFS：SyncAccessHandle 不覆盖目录操作 → 单容器 + overlay 重写
**改动**：`modules/M2-02-opfs-vfs.md` 完全重写，依据 `opfs-vfs-design.md` 路径 A。
- 物理模型：整个 Db 是单 OPFS 容器文件 `vane.db`，Worker init 异步获取唯一 `FileSystemSyncAccessHandle`（一次性 await `createSyncAccessHandle`），此后全部 Vfs 方法基于该同步句柄操作容器内字节区间。
- 内存虚拟 FS overlay（`MemOverlay`）：虚拟路径 `<db>/segments/seg_<ulid>/...` 映射到容器 `(offset,size)` 区间；文件表 + free list + 双 meta_slot + CRC + generation。
- `OverlayBackend` trait 抽象（read/write/flush/size/truncate），OpfsVfs impl 一次，IdbVfs 复用。
- 8 方法全同步：create（内存表登记）/ read_at/write_at/append（区间 IO）/ sync（统一 flush）/ rename（表项改挂 + 元数据落非活跃槽 + 翻转）/ delete（区间进 free list）/ list（表 keys 前缀过滤，与 `MemoryVfs::list` `vfs/memory.rs:99` 语义一致）。
- manifest 原子性（I-6 等价，对 core 透明）：双 meta_slot + CRC，崩溃恢复三时点测试（步骤 2 后 / 元数据写一半 / flush 后）。
- compaction：初版 append-only + 阈值触发全量 rewrite。
- **core / Vfs trait 零改动**（验收「`crates/vane-core/` git diff 为空」）。
- TDD 18 测试（含 Vfs 套件复用、容器 round-trip、双 meta_slot 翻转、3 个崩溃恢复、free list 复用、compaction、core 调用面兼容集成）。
- 涉及文件：Create `vfs/{mod,overlay,container,opfs}.rs`；web-sys feature `FileSystemSyncAccessHandle`/`FileSystemFileHandle`/`FileSystemDirectoryHandle`/`Storage`（不启完整 `FileSystemAccess`）。

---

## 重要项（11）

### A-I1/I4 + B：Db::open 签名漏 vfs 参数
- `README.md` M2-11 节 Consumes：补 `Db::open(vfs: Arc<dyn Vfs>, path, opts)` `api/db.rs:35` + export `db.rs:164` + close `db.rs:168` + Collection 方法列表（无 export/close）。
- `modules/M2-01-wasm-cdylib-size.md` §3 Consumes：补 `vfs: Arc<dyn Vfs>` 首参 + export/close 归 Db + Collection 仅 add/flush/search/delete/compact/reindex。
- `modules/M2-11-go-cgo-binding.md` §3 Consumes：补 `Db::open(vfs: Arc<dyn Vfs>, path, opts)` + 注明 `vane_open` 内部构造 StdFsVfs。

### A-I2 / B-I1：M2-08 引用不存在的 segment/scalars.rs
- `modules/M2-08-stored-zstd.md` §2：`segment/scalars.rs` → `segment/mod.rs:652`（`decode_scalars` 在 mod.rs:652，`ScalarReader` 在 mod.rs:583；`segment/` 目录无 scalars.rs，已 grep 确认）。
- §2 header.rs 行号 `21,40` → `21,46`（行 46 是 decode `version != FORMAT_VERSION` 校验，行 40 是 `buf.len() < 8` 长度检查；reviewer A-M2/B-M1）。
- §2 hnsw 行号 `533` → `534`（decode 校验在 534，encode 在 461）。

### A-I3 / B-I2：M2-09 brute_search_sq8 签名缺 metric + docid_base
- `modules/M2-09-sq8.md` §3 Produces：`brute_search_sq8` 补 `metric: Metric` + `docid_base: u64`（与 `brute_search` `vector/mod.rs:101` 对齐）；`sq8_distance` 补 `metric: Metric`；`sq8_query_distance` 补 `metric` + `docid_base`。
- §2 调用点行号说明：`api/collection.rs:765,776`（grep 实际命中，reviewer A M-3 称 ~769/782 为误判，不改）。
- §4 测试 3：`sq8_distance` vs f32 误差 <1e-3 **覆盖 cosine/L2/dot 三种 metric**（原仅测 cosine）。
- §3 补 `sq8_query_distance` 混合距离说明（reviewer B-M5）：query f32 先量化为 SQ8（用 sq8_vectors 的 min/max），再调 sq8_distance，避免每向量解码。
- `README.md` M2-09 节：同步签名。

### A-I5：README jieba 约束放宽
- `README.md` 全局约束表：原「wasm32 永不启用 jieba feature」改为「**永不启用 dict-zh**（dict-zh 捆绑 vane-dict-zh 词典数据，红线）；jieba feature（仅算法代码 DAT/HMM/seg，无词典数据）可在 vane-wasm 非 default 启用，须通过 800KB 门禁实测」。注明放宽 M1 约束理由（M2 Prompt「含 jieba 代码、不含词典数据」）。
- `modules/M2-01`/`M2-04` 验收/Consumes：同步措辞（default 不启 dict-zh；jieba 可启用须过门禁）。

### B-I3：M2-12 export 恢复路径
- `modules/M2-12-export-snapshot.md` §3 Produces：`read_snapshot` 从「可选」改为「M2 实装」；定义恢复路径（解包单文件快照到 db_path 目录 → `Db::open(vfs, db_path, opts)`，消解与单文件格式矛盾）；三侧恢复（wasm/Node/Go）。
- §3 补递归 list segments/seg_*/ 逻辑（reviewer B-M10）：`list("segments")` → 每个 `seg_<ulid>` `list("segments/seg_<ulid>")` → read_at 打包，固定 2 层。
- §4 测试 4：从条件性测试改为硬测试（export → read_snapshot → Db::open → search 一致）。
- §5 验收：增「恢复路径可用」（P0-3 数据主权闭环）。
- `README.md` M2-12 节：同步 read_snapshot 实装。

### B-I4：M2-04 wasm-bindgen-futures 体积未登记
- `modules/M2-04-worker-shell.md` §5 验收：补 wasm-bindgen-futures 体积实测登记（预估 +5~15KB gzip）。
- `README.md` 体积评估表：增 wasm-bindgen-futures 行。

### B-I5：体积预算全局累计管理
- `README.md` 全局约束表：增「wasm 体积预算累计管理」项（vane-core + wasm-bindgen + web-sys + jieba 算法 + ruzstd + overlay 总和 ≤800KB gzip，每模块贡献登记）。
- `README.md` 体积评估表：改版为「累计管理」，增累计预算说明 + jieba 算法代码行 + overlay 内核行 + 三 feature 同启实测要求。

### B-I6：M2-05 LLVM 自动向量化验证
- `modules/M2-05-simd128-variants.md` §4 测试 4（新增）：`wasm-objdump -d vane_wasm_simd.wasm | grep -E 'f32x4|i32x4|v128'` 命中 simd128 指令；若 grep 无命中 → 回退 trait Distance 方案（cfg 在 impl 处，停下标注 ⚠️ 需 SPEC 修订）。
- §4 测试 5（新增）：召回回归 gate（与 M2-06 协同），自动向量化致 recall 退步则回退。
- §4 测试 8（原 6）：core 零 cfg grep 描述修正（`vfs/mod.rs:18` std_fs 是 target_arch 分支）。

### M2-07/08 dim 协同 stub-then-regress 显式标注
- `modules/M2-07-lazy-load.md` §3 dim 来源设计：显式标注 stub-then-regress 策略（M2-07 测试用 stub v2 header 手工构造 12 字节头；M2-08 落实 finalize 写 v2 后回归；version 字段判别；两计划可独立落地无循环依赖）。
- `modules/M2-08-stored-zstd.md` §3 Consumes：同步 stub-then-regress + 版本对齐策略（v2 头固定 12 字节，v1 头 8 字节，M2-08 写 v2 时 dim 必填 schema.dim）。

### M2-03 IDB 降级复用 overlay 内核
- `modules/M2-03-idb-vfs.md` 重写：复用 M2-02 `MemOverlay` + `OverlayBackend`（后端无关），底层换内存 `Vec<u8>` + 异步 checkpoint；sync best-effort（标 dirty，JS 壳层异步 tick put 回 IDB）；I-6 语义降级为「尽力持久化」，关键数据走 export()。工作量下修（无重复实现）。
- `README.md` M2-03 节 + 计划清单摘要 + 阶段性偏离 §1：同步。

### M2-04 Worker init 异步序列 + Safari 探测
- `modules/M2-04-worker-shell.md` §3 Consumes：补 Worker init 异步序列（OPFS: getDirectory→getFileHandle→createSyncAccessHandle→OpfsVfs::from_handle→Db::open；IDB 降级: open_idb→get blob→IdbVfs::from_blob→Db::open）。
- §4 测试 3/4：补 init 异步序列 + Safari OPFS bug 能力探测降级（getDirectory 存在性 + createSyncAccessHandle 可用性 + 小写 round-trip 探针）。

---

## 次要项（11）

### A-M1 / B：M2-10 compile_filter 行号 43→32
- `modules/M2-10-million-executor.md` §3 Consumes：`filter/mod.rs:43` → `filter/mod.rs:32`（grep 确认）。
- `README.md` M2-10 节 Consumes：同步。

### A-M2 / B-M1：M2-08 header.rs:40→46
- 已随 A-I2 处理（`modules/M2-08` §2）。

### A-M3：M2-09 调用点行号 765,776 实为 ~769/782
- **未修**。grep `crates/vane-core/src/api/collection.rs` 实际命中 `brute_search(` 在行 765 与 776（与 M2-09 计划原行号一致）。reviewer A M-3 称 ~769/782 为误判。在 `modules/M2-09` §2 补注「实际行号 765/776，grep 确认」明示。

### A-M4：M2-10 §2 mod.rs:740-790 → api/collection.rs:740-790
- `modules/M2-10-million-executor.md` §2：文件名笔误修正（`mod.rs` → `api/collection.rs`）。

### B-M2：M2-10 Db executor cfg 选择位置
- `modules/M2-10-million-executor.md` §2：补 `executor::default_executor() -> Arc<dyn Executor>` 工厂函数在 `executor/mod.rs` 内 cfg 选择；`api/db.rs` 仅调工厂无 `cfg(target)`，避免 I-5 风险。
- `README.md` M2-10 节 I-5 守护：同步。

### B-M3：M2-10 测试 4 grep 表述
- `modules/M2-10-million-executor.md` §4 测试 4：修正描述——`vfs/mod.rs:18` `cfg(not(target_arch="wasm32")) pub mod std_fs;` **是 target_arch 分支**（原测试描述误称 std_fs 非 target_arch）；grep 命中 std_fs 不算违规。

### B-M4：M2-11 wazero 形态实现路径
- `modules/M2-11-go-cgo-binding.md` §2：补 wazero 形态 5 步实现路径（vane-core 编译 wasm32-wasi → wazero host 封装 → Go API 对齐 → build tag 切换 → 参考 M1 README §09）。

### B-M5：M2-09 sq8_query_distance 混合距离说明
- 已随 A-I3 处理（`modules/M2-09` §3 Produces 补说明）。

### B-M6：M2-13 fixture 体积评估偏乐观
- `modules/M2-13-wiki-ndcg-corpus.md` §4 测试 7 + §3 Produces + §5 验收：~500KB → ~1.5MB（中文 UTF-8 每 3 字节/字，500 篇 × 平均 3KB），门禁 ≤1MB → ≤1.5MB。

### B-M7：M2-13 测试 8 人工抽检 → 自动化
- `modules/M2-13-wiki-ndcg-corpus.md` §4 测试 8：改为自动断言 fixture 关键词分布（科技/历史/地理三领域关键词各 ≥30 篇命中）。

### B-M8：M2-14 行为测试 TDD 顺序
- `modules/M2-14-demo.md` §4：明确 e2e 脚本自动化（Playwright/headless browser，`demo/e2e/` 目录），无法自动化步骤降级为人工验收清单（标注）。

### B-M9：M2-06 测试 4 诊断步骤 → 硬断言
- `modules/M2-06-simd-recall-regression.md` §4 测试 4：Jaccard ≥0.99 改为**硬断言**（失败即 CI 阻断），附诊断信息。

### B-M10：M2-12 递归 list 逻辑
- 已随 B-I3 处理（`modules/M2-12` §3 补递归 list 说明）。

---

## 校验

### 1. 未引入新冻结签名破坏
- Vfs trait 8 方法签名：零改动（M2-02 重写后仍 `impl Vfs for OpfsVfs`，8 方法委托 MemOverlay）。
- §4 IDL：`Db::open/export/close`、`Collection::{add,flush,search,delete,compact,reindex}`、`SegmentReader::open/vectors/dim/stored_json/text`、`HnswReader::search`、`brute_search`、`compile_filter` 签名均未改。
- M2-05 首选方案（LLVM 自动向量化）core 零 cfg；M2-09 首选方案（SQ8 仅暴力回退）HnswReader::search 不改。两处 ⚠️ 修订点首选方案保持，回退方案标注「需 SPEC 修订」。

### 2. M2-02/03/04 三计划契约一致
- overlay 内核共享：M2-02 产出 `MemOverlay` + `OverlayBackend`（`overlay.rs`），M2-03 复用（`idb.rs` impl `OverlayBackend` 内存 Vec），M2-04 Worker init 按统一异步序列注入两者。
- README M2-02/03 节 Produces for 与各模块 §3 一致。

### 3. README M2 Global Interface Contracts 与各 modules Produces for 一致
- M2-02：README OpfsVfs 签名（`from_handle` + `impl Vfs`）与 M2-02 §3 一致。
- M2-03：README IdbVfs（`from_blob` + `impl Vfs` + overlay 复用）与 M2-03 §3 一致。
- M2-09：README `brute_search_sq8` 签名（补 metric+docid_base）与 M2-09 §3 一致。
- M2-11：README Consumes 补 Db::open vfs + export/close 归 Db 与 M2-11 §3 一致。
- M2-12：README read_snapshot 实装与 M2-12 §3 一致。
- M2-10：README compile_filter `filter/mod.rs:32` + I-5 守护（default_executor 工厂）与 M2-10 §3/§4 一致。

### 4. 无新 SPEC 修订需求
- OPFS VFS 路径 A：Vfs trait 零改动，§6.1/§6.2/§6.4 语义全部保持（`opfs-vfs-design.md` §8 已判定）。可在 §6.1 后追加非规范性注记（非修订），但非必须。
- jieba 约束放宽：仅改 README 全局约束表（M2 约束），不触及 SPEC §13.2-3（「核心 wasm ≤800KB 含 jieba 代码、不含词典数据」措辞本就支持放宽）。
- M2-05 回退 trait Distance / M2-09 回退 HNSW 用 SQ8：仍标注「⚠️ 需 SPEC 修订」，未触发。

---

## 残留

- **无残留阻塞**。B-1（OPFS VFS）已由路径 A 重写解除。
- **无残留重要项**。A 的 5 项 + B 的 6 项（合并去重后 11 项）全部落实。
- **未修次要项 1 条**：A-M3（M2-09 调用点行号 765,776）—— grep 实际确认行号正确，reviewer A M-3 误判，已在计划中补注「grep 确认」明示，不改行号。
- **无新 SPEC 修订需求**。
