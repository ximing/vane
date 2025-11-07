# M2 plan-splitter 派发简报

> 编排者→plan-splitter SubAgent。本文件是 Phase One 计划拆分的输入约束。
> 镜像 M1 的 `docs/plans/m1/plan-splitter-brief.md` 模式。

## 你的任务
把 SPEC v1.2 §15 的 M2 范围拆成独立可执行的 TDD 计划，写入 `docs/plans/m2/`：
- `docs/plans/m2/README.md`：计划索引 + **M2 Global Interface Contracts（单一事实源）** + 依赖图（mermaid）+ 拓扑批次 + 范围边界 + 全局约束表 + 不变量覆盖矩阵（I-1~I-8 M2 负责部分）+ 阶段性偏离节 + 降级顺序。
- `docs/plans/m2/modules/M2-NN-<name>.md`：每模块一份计划。

## 必读输入（以 git 实际代码为准，不得臆测）
- `docs/SPEC.md` **v1.2**（M2 scoping 已落实三处修订：§13.1 冷启动懒加载承诺、§6.2 per-file format_version + stored v1/v2 双模 + 懒加载注释、§14 I-5 释义澄清）。
- `docs/REQUIREMENTS.md` v1.1 §7（M2 范围）+ §13（非功能）。
- `docs/plans/m1/README.md`：**M1 Global Interface Contracts**——M2 计划消费 M0/M1 既有 pub API 须引用其精确签名（HnswReader/SegmentReader/MergeTask/compile_filter/Wal/JiebaTokenizer/ReindexHandle/vane-ffi C ABI 等）。
- `docs/plans/m1/M1-SUMMARY.md` §3 遗留 + §4 M2 建议。
- `docs/plans/m2/00-scoping-report.md`：**M2 模块分解预览（§3，M2-00~14）+ 懒加载设计（§A）+ stored-zstd 设计（§B）+ Phase Zero 安全清理清单（§2）**。本简报的模块编号与依赖拓扑以此为基，可细化但不得丢项。

## M2 模块清单（基线，可细化编号）
- **M2-00 Phase Zero 安全清理**：parked minors + wiki nDCG corpus + vane-wasm 骨架。**注：M2-00 由编排者在 Phase Zero 直接执行（不经 plan-splitter 拆计划），plan-splitter 仅在 README 标注其状态/产出契约，不重复拆计划。** vane-wasm 骨架已产出（见 m2-00 报告）。
- M2-01 vane-wasm cdylib + 体积门禁（SIMD 探针占位）——消费 M2-00 骨架。
- M2-02 OPFS VFS 后端（OpfsVfs 实现 Vfs trait，SyncAccessHandle，Worker 内同步）。
- M2-03 IndexedDB 降级 VFS（IdbVfs 适配层，binding crate，OPFS 不可用降级，不抛错）。
- M2-04 Dedicated Worker 壳（postMessage Promise 边界 + init 探针）。
- M2-05 SIMD128 双变体（simd128 默认 / scalar fallback 两产物 + init WebAssembly.validate 探针）。
- M2-06 SIMD 双变体召回回归（两变体各跑 recall@10≥0.95 五档，§8.4）。
- M2-07 冷启动懒加载（SegmentReader OnceLock 按需加载 vectors/stored，open<1s；SPEC v1.2 修订 A 已批准；设计见 scoping §A）。
- M2-08 stored.bin zstd + per-file format_version（v1/v2 双模 + per-file 常量 + zstd-encode feature；SPEC v1.2 修订 B + I-5 释义已批准；设计见 scoping §B）。
- M2-09 SQ8 向量量化（f32→SQ8 编解码 + 距离适配，内存降 4 倍 <200MB；依赖 M2-07 vectors 访问点）。
- M2-10 100 万规模承诺恢复（段合并调优 + Executor trait + rayon 并行搜索，cfg 仅在 Executor impl；依赖 M2-09 降内存）。
- M2-11 Go cgo 绑定（vane-ffi C ABI 实装 + cbindgen + staticlib + zig cc 交叉 + wazero build tag；vane-ffi 当前为 M0 占位 stub，须实装；消费 M1 README §09 契约）。
- M2-12 export 快照导出（Db::export() 单文件快照实装，M0/M1 占位 E_UNSUPPORTED；依赖 M2-02 OPFS 写快照）。
- M2-13 真实维基 nDCG corpus（500 篇+50 查询 fixture + 验收②；方案见 scoping §2.2，M2-00 已备方案）。
- M2-14 Demo（纯前端拖入 markdown 文件夹本地混合搜索含中文；依赖 M2-04/M2-05）。

## 每份计划必须含
1. **目标**（一句话 + SPEC 节号）。
2. **涉及文件**（Create/Modify 精确路径，Modify 标注 file:line 区间）。
3. **接口契约**：Consumes from（M0/M1 既有 pub API 精确签名，引用 M1 README 对应节）+ Produces for（M2 新增/扩展签名，下游模块消费）。**M2 Global Interface Contracts 节是跨计划唯一沟通渠道**。
4. **TDD 测试清单**（逐项测试名 + 断言）。
5. **验收标准**（门禁阈值）。
6. **前置依赖**（模块编号）。
7. **不变量覆盖**（触及 I-1~I-8 哪几条 + 测试要求）。

## 关键约束（每份计划必须遵守）
- **不得改 M0/M1 已冻结 pub API 签名**（SPEC §4 IDL M0 冻结）。M2-07 懒加载用 OnceLock 保 `vectors(&self)->&[f32]` 不变（已批准）。若发现必须改冻结签名，**停下上报编排者**（走 SPEC 修订，不得绕行）。
- **vane-ffi C ABI**：M2-11 实装 M1 README §09 契约（句柄注册表 std::sync::RwLock<HashMap>，非 dashmap；vane_export 保留）。
- **Db::export()**：M0/M1 占位 E_UNSUPPORTED，M2-12 实装为 SPEC §4.1 `export(destPath)->Result<()>`。
- **词典永不进 wasm**（红线 800KB gzip，含 jieba 代码不含词典数据）；vane-wasm default features 不启 jieba/dict-zh。
- **core 禁 std::fs/std::net/mmap**；**cfg 只允许 VFS/Executor/vane-wasm binding**（I-5，SPEC v1.2 已澄清：cfg(feature) 能力开关如 zstd-encode 允许在 segment 编解码，cfg(target) 仅限 VFS/Executor）。
- **依赖黑名单**：regex/tokio/prost/tonic/openssl/lindera/ndarray/wee_alloc/dashmap/parking_lot。M2 引入 wasm-bindgen/web-sys/js-sys/rayon 须评估体积 + 在 README 全局约束表登记。rayon 仅在 Executor impl（M2-10），不进 core 算法。
- **浏览器目标 wasm32-unknown-unknown**（不引 wasi）；core 保持同步 IO（SyncAccessHandle 在 Worker 内同步）；OPFS 主 + IDB 降级（适配层在 vane-wasm binding，不污染 core）；`persistence` 映射 navigator.storage.persist()；文档声明浏览器存储非可靠，关键数据用 export() 快照。
- **SIMD 双变体**：simd128 默认 + scalar fallback，init 时 WebAssembly.validate 探针选产物，用户只下载其一。
- **WASM 词典**（§12.3）：CDN URL fetch → sha256 校验 → OPFS 缓存；dictData 内联注入；fetch 失败降级 bigram + console.warn 不抛错（E_DICT_UNAVAILABLE 禁止到达）。
- **SQ8**：feature 可选，10万×384 全加载 <200MB。
- **100 万规模**：恢复承诺（M0/M1 50万不塌红线 → M2 100万）。
- **MoSCoW 即合同**：超范围需求拒绝并记录；Won't-have（内置 embedding/GPU/SQL/分布式）不得触碰。

## 依赖拓扑（高层，scoping §3）
- M2-00（Phase Zero，已部分执行）无前置。
- M2-07/M2-08：SPEC v1.2 已批准，可早做。M2-09 依赖 M2-07；M2-10 依赖 M2-09。
- M2-02/03/04 浏览器三件套依赖 M2-01；M2-06 依赖 M2-05；M2-12 依赖 M2-02；M2-14 依赖 M2-04/M2-05。
- M2-11（Go cgo）独立链，可与浏览器链并行。
- M2-13（wiki nDCG）独立测试增强。

## 产出后
写完 README + 全部 modules/ 计划后，**只返回**：模块数 + 依赖图 mermaid 文本 + 一段≤400字摘要（哪些模块触及 M0/M1 冻结签名需编排者复核、有无 SPEC 进一步矛盾、有无阻塞）。不要贴计划全文。计划文件即产出。
