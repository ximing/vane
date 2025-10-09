# M0 执行笔记（编排者维护）

> 记录执行阶段发现的计划偏离、裁决、跨计划副作用，防上下文压缩丢失。
> 每条标注来源计划、裁决依据、影响范围。

---

## L0 · 00-workspace（进行中）

### N1. ulid 禁用默认 feature（wasm32 门禁）
- **现象**：`ulid = "1"` 默认拉 rand→getrandom，getrandom 拒绝在 wasm32-unknown-unknown 编译，§13.3 门禁失败。
- **裁决**：根 Cargo.toml `ulid = { version = "1", default-features = false }`（默认 feature 仅 "rand"，禁用后编码 API 仍可用）。
- **依据**：SPEC §13.3 wasm32 check 为 M0 第一天硬门禁；ULID 仅用于段目录命名，不需密码学随机。
- **副作用 → 04-segment-format**：`gen_ulid()` 不能用 `Ulid::new()`（需 rand feature），改用 `Ulid::from_parts(timestamp_ms, counter_as_u128)`，计数器替代随机位。**L2 派发 04 时必须告知 implementer。**

### N2. text_fields() 返回 Vec<&str>
- **现象**：00 计划正文与 README 全局契约不一致（计划 Vec<String>，契约 Vec<&str>）。
- **裁决**：统一为 `Vec<&str>`，对齐 README 单一事实源 + vector_field() 的借用风格。
- **影响**：下游 05/07 按 Vec<&str> 消费。

### N3. DIM_MAX 定义顺序
- 00 计划 Task 4 引用 DIM_MAX 但 Task 5 才定义，implementer 提前到 Task 4。最终常量集一致，非问题。

---

## L1 · 串行执行（worktree 隔离不可用，回退串行 + 审查/实现重叠流水线）

worktree 隔离在该环境失败（`Failed to resolve base branch "HEAD": git rev-parse failed`，无 remote + 隔离子进程 git 上下文异常）。回退串行，但利用"L1 各模块独立、写不同目录"的特性，让前一路的审查与下一路的实现重叠（reviewer 只读已提交代码，implementer 写新目录，无 git 冲突）。

### 01-vfs — ✅ 完成 + 审查通过（commit 185bacb..cf4969d）
- 18 测试通过，五项自证全绿（test/clippy/fmt/wasm32/check-no-std-fs）。
- 审查 9 项全绿，无阻塞/重要。5 处偏离计划均合理。
- **Parked 次要（交最终评审 triage）**：
  - P1: memory.rs:117 注释"I11"应为有效不变量编号（I-1..I-8 无 I11）或描述性文字。
  - P2: StdFsVfs::list 未排序 vs MemoryVfs::list 排序，建议统一（conformance 用 .contains() 不受影响，低危）。
  - P3: PageCache::put 可加同 key 去重防御（当前调用流无此路径）。
  - P4: StdFsVfs::resolve 每次 create_dir_all，生产化时考虑缓存。
- **M1 接线备忘**：PageCache 签名 `&mut self`（对齐 README 契约），07/Db 接线时需 `Mutex<PageCache>` 包裹。

### 02-tokenizer — ✅ 完成 + 审查通过（commit 71e4506..f734433）
- 46 测试通过（tokenizer 31 + 既有 15 无回归），四项自证全绿。
- 审查 10 项全绿，无阻塞/重要。契约逐字一致，I-4 九维覆盖，position 连续性有专项测试。
- 偏离 2 处合理（`.err().unwrap()` 绕开 Debug bound、standard 计数器 zip 改写）。
- **Parked 次要**：cjk_bigram.rs 仍用 `let mut position`（与 standard zip 写法不一致，clippy 未告警，无功能问题）。

### 03-fusion — ✅ 完成 + 审查通过（commit 4cf5e8b）
- 73 测试通过（fusion 27 + 既有 46 无回归），四项自证全绿。
- 审查 8 项全绿，RRF/linear 公式手算验证正确，数值健壮性（max==min/NaN/排序确定性）到位。
- 偏离 2 处合理（`#[cfg(test)]` 测试门控非 I-5 平台分支、crate 内用 `crate::types`）。
- **Parked 次要**：P3 NaN 测试命名"rejected"略误导（NaN 非首元素仍 NaN，但调用方契约不含 NaN，可接受）；P4 模块文档措辞 `vane_core::types` 与代码 `crate::types` 不一致。

### 06-vector-brute — ✅ 完成 + 审查通过（commit cad7cec）
- 111 测试通过（vector 38 + 既有 73 无回归），四项自证全绿。
- 审查 9 项全绿，cosine/l2/dot 数值手算正确，filter/topK/NaN 边界全覆盖。
- 5 处偏离均为修正计划测试数据 bug（dim_mismatch broken test、bitmap 用绝对 docid、tie-break 同分向量），未引入语义错误。

### L1 集成节点 — ✅ 通过（HEAD 344ba7c）
- `cargo test -p vane-core`：111 passed / 0 failed / 1 ignored
- `cargo clippy -p vane-core --all-targets -- -D warnings`：clean（集成节点发现 03 测试代码 approx_constant 回归，已修 344ba7c）
- `cargo check --target wasm32-unknown-unknown -p vane-core`：pass
- `cargo fmt --check`：pass
- `bash scripts/check-no-std-fs.sh`：OK
- **过程教训**：今后所有 implementer 自证 clippy 必须含 `--all-targets`（覆盖测试代码），已固化到 L2+ 派发模板。

---

## L2 · 串行 + 审查/实现重叠（04-segment-format / 05-bm25 / 08-persistence，彼此独立）

### 04-segment-format — ✅ 完成 + 审查通过（commit b96dfb1..1e15085）
- 122 测试通过（segment 11 + 既有 111 无回归），四项自证全绿（含 --all-targets clippy）。
- 审查 13 项达标，可合并。N1 gen_ulid（Ulid::from_parts）、I4 docid_base、I10 stored.bin 裸 JSON、F1 stored_json、S2 scalars 空 stub、S4 vector=None 填零 均落实。
- **遗留→格式冻结前清理 pass**（不阻塞 05/07/08）：
  - FF1（重要）：vectors.bin 缺 magic+version 头，违反 §6.2"所有文件以 magic+version 开头"。裁决：加 8 字节头合规，SegmentReader 加载时跳过，不影响 brute_search 拿纯 f32。
  - FF2（重要）：add_doc 返回局部 docid（从 0 起，全局=base+local）——设计正确（§3.2），但 README §04 注释误导 + base>0 测试缺失。修 README 注释 + 补测试。**07 派发时必须明确：add_doc 返回局部 docid。**
  - FF3（次要）：format_version 字节序混合（magic/version BE，payload LE）。统一全 LE。
  - FF4（次要）：E1 vectors.bin dim 推导无校验、E2 stored/idmap 解码静默截断。M1 加严。

### 05-bm25 — ✅ 完成 + 审查通过（commit 3df0ab7）
- 149 测试通过（bm25 27 + 既有 122 无回归），四项自证全绿（含 --all-targets）。
- 审查 11 项全绿，WAND 与暴力基线 100% 一致（tiebreak 修复正确），posting/vbyte/BM25 公式/错误码全符合 §6.3/§8.1。
- B4 InvertedIndexReader::open(&Arc) 落实。3 个次要观察（未用 TermEntry.idf、num_docs u32、filter 测试未比 score）记 triage。

### 08-persistence — ✅ 完成 + 审查通过（commit a246904..6263253）
- 161 测试通过（persistence 12 + 既有 149 无回归），四项自证全绿。
- 审查 9 项全绿，I-6 原子性（crash-before-rename + 残留 tmp 清理）+ I16 + AutoCommitter 双触发均有测试。
- B8 仅自有类型加 serde derive。orphans.contains 类型修正合理。

### L2 集成节点 — ✅ 通过（HEAD c1c34b0）
- `cargo test -p vane-core`：161 passed / 0 failed / 1 ignored
- `cargo clippy -p vane-core --all-targets -- -D warnings`：clean
- `cargo check --target wasm32-unknown-unknown -p vane-core`：pass
- `cargo fmt --check`：pass
- `bash scripts/check-no-std-fs.sh`：OK（集成节点发现脚本对 segment/tests.rs + persistence 注释误报，已由 01-vfs 稳健化修复 c1c34b0：排除所有 tests.rs + 匹配 std::fs:: 实际用法）

---

## L3 · 07-api-core（单独，集成全部 L1+L2，opus）
### 07-api-core — 🔄 派发中

---

## M0 格式冻结前清理 pass（DoD 前执行）
收集执行期发现的格式/文档遗留项，在 M0 格式冻结前统一清理：
- [来自 01] P1-P4（I11 注释笔误、list 排序统一、PageCache put 去重、resolve 缓存）
- [来自 02] cjk_bigram position 写法不一致
- [来自 03] P3 NaN 测试命名、P4 文档路径措辞
- [来自 04] FF1 vectors.bin magic 头、FF2 add_doc 注释+测试、FF3 字节序统一、FF4 解码健壮性
- 格式冻结后跑 §13.3 冻结 corpus 兼容测试骨架
