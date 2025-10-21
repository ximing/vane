# 阶段零-B：housekeeping 清理 — 代码审查报告

> 审查者：read-only reviewer
> 基线：BASE=c287458（阶段零-A），HEAD=0a0ce5e
> 审查日期：2026-08-09
> 契约：SPEC v1.0 §6.4/§7.1/§13.3/§14、M0 README Global Interface Contracts

## 审查结论

**APPROVED_WITH_MINOR** — 无阻塞项。07-api-core refactor 经逐项核查语义等价（无回归），FF5 回退门禁在主用例（workflow_dispatch on feature branch）下真正生效。2 项低风险观察项与 2 项待编排者裁决项见末节。

## 维度 1：07-api-core refactor 语义安全性（最高优先）

### 1.1 `vector_field()` hoist 到循环外 — ✅ 语义等价

**证据**：`api/collection.rs:276-291`（HEAD）

- `vector_field()` 签名 `Result<(&str, u32, Metric)>`（types.rs:184，BASE 与 HEAD 零 diff）。
- 原代码循环外取 `.1`（dim），循环内每段取 `.2`（metric），共 N+1 次调用。
- 新代码循环前一次性解构 `let (_, dim, metric) = self.inner.schema.vector_field()?;`，存入 `vf: Option<Metric>`。
- `Metric` 派生 `Copy`（types.rs:98），`vf` 在循环内按值匹配不消耗。
- `vector_field()` 是 schema 纯读取（遍历 `self.fields`），schema 在 search 期间不可变（持有读锁），多次调用结果恒定。
- **结论**：无语义变化。dim 校验错误路径（`query.vector` 有值但 dim 不匹配）行为不变。

### 1.2 `wrapping_sub` → `checked_sub` — ✅ 语义等价且更安全

**证据**：`api/collection.rs:408-413`（HEAD）

- 原代码：`let local = sd.docid.wrapping_sub(base);` → 产生巨大 local（u64::MAX 量级）→ `reader.external_id(local)` 越界返回 None → `if let Some(eid)` 不匹配 → 继续下一段。
- 新代码：`checked_sub` 返回 None 时 `continue` 跳过该段。
- 两者结果相同：该段未命中，继续下一段。新代码不依赖 `external_id` 的越界检查，更稳健。
- **下溢路径测试覆盖**：无专门针对 `sd.docid < base` 的单测，但行为等价（external_id 对越界 local 必返回 None，因 doc_count 远小于 u64::MAX）。已有 `multi_segment_flush_and_search` + `restore_multi_segment_uses_stored_docid_base` 覆盖多段回填路径。可接受。

### 1.3 `inv_readers[i]` → `zip` 迭代 — ✅ 语义等价

**证据**：`api/collection.rs:301`（HEAD）

- `snap`（readers）与 `inv_readers` 在 `flush`（collection.rs:226-228）与 `restore_from_manifest`（collection.rs:113-117）中成对 push，长度恒等、顺序一致。
- `zip` 迭代不会越界或错配。原 `enumerate + inv_readers[i]` 在长度不一致时会 panic 或越界，新写法更安全。
- **结论**：无语义变化。

### 1.4 auto-commit flush 吞错改 eprintln — ✅ 未改 pub API

**证据**：`api/collection.rs:158-167`（HEAD）

- `AddReport` 结构（types.rs:109-112）BASE 与 HEAD 零 diff（字段 `accepted: u64` + `visible_after_flush: bool` 未变）。
- `types.rs` 全文件零 diff。
- `eprintln!` 是 std 宏；vane-core 已依赖 std（如 `std::collections::HashMap`）；`cargo check --target wasm32-unknown-unknown -p vane-core` 通过。wasm32 下 `eprintln!` 输出到 console，无 std 依赖问题。
- 未引入 `log` crate（Cargo.toml 零 diff）。
- **结论**：不改 pub API，eprintln 在 wasm32 安全。过渡方案可接受。

### 1.5 restore 累加 base → 读段头 docid_base — ✅ 语义等价且更稳健

**证据**：`api/collection.rs:104-119`（HEAD）vs BASE `:95-112`

- `SegmentMeta.docid_base`（segment/mod.rs:15）由 `SegmentWriter::new(..., base_docid)`（segment/mod.rs:43-53）传入，`finalize()` 写入 header.bin（segment/mod.rs:168，header.rs:21,64）。
- flush 时 `base_docid = docs.first().map(|d| d.docid)`（collection.rs:181），即 buffer 首文档的全局 docid = 当前 `next_docid`。
- M0 连续追加场景：段 1 base=0，段 2 base=count1，... 累加结果与段头一致。`offsets` HashMap 填充值相同。`next_docid`：原 `= base（累加总和）`，新 `= max_end = max(base+count)`，连续场景下 `max_end = 最后段 base+count = 总和`。等价。
- M1 compaction（非连续段）场景：新代码读段头更正确（实现者报告已正确判定为防御性改进非 bug 修复）。
- **测试**：新增 `restore_multi_segment_uses_stored_docid_base`（api/tests.rs:682-781），两段 reopen 后查 c 向量命中 c（验证第二段 offset=2），再灌 e 验证 next_docid=4。测试通过。
- **结论**：M0 无语义变化，M1 更稳健。不改 pub API。

### 1.6 核心问题结论

**refactor 无隐藏语义变化或回归。** 测试覆盖充分：20 个 api 测试全绿（含新增 restore 多段测试），185 lib 测试全绿。

## 维度 2：FF5 回退门禁

### 2.1 同一 target/criterion 比较 — ✅ 生效

**证据**：`.github/workflows/benchmark.yml:29-37`

- 原方案 `git worktree add ../vane-main main` 导致 main baseline 存在于 `../vane-main/target/criterion`，repo 根 `critcmp` 读不到。
- 新方案：同一 checkout 内 `git checkout main` → `--save-baseline main` → `git checkout 触发分支` → `--save-baseline current`。两者共享 `target/criterion`（`/target` 在 .gitignore，Cargo.lock 未跟踪，`git checkout` 不会因工作树脏失败）。
- critcmp 默认读 `target/criterion`，能同时读到两个 baseline。

### 2.2 `|| true` 保留 — ✅ 有理

- critcmp 退出码在 baseline 缺失等情况下不可靠。真正门禁交给 `check-bench-regression.py`。符合简报"保留 `|| true` 但确保 python 脚本解析正确并在回退时 exit 1"的裁准。

### 2.3 regex 匹配 — ✅ 已验证

**证据**：`scripts/check-bench-regression.py:14`，`_TIME_RE = re.compile(r'([\d.]+)\s*(ms|µs|us|ns|s)\b', re.IGNORECASE)`

- 支持 ms/µs/us/ns/s 五种单位，归一化到 ms。
- 本地验证三组样例：
  - `compare_regression.txt`（hybrid +13.8%）→ exit 1，输出 `FAIL: 1 benchmark(s) regressed > 10%`。✅
  - `compare_ok.txt`（最大 +4.5%）→ exit 0，输出 `OK`。✅
  - 空文件 → exit 0（容错）。✅
- 旧 regex 要求行内含 `current` 字面词，critcmp 数据行无此词 → 永远解析空 → exit 0 兜底（门禁失效）。新 regex 按 token 抓取，已修复。

### 2.4 观察项（非阻塞）

⚠️ **schedule 触发在 main 上无意义**：`on: schedule: cron '0 3 * * *'` 在默认分支（main）上跑，`github.ref_name = "main"`，步骤会 `git checkout main` 跑 main baseline，再 `git checkout "main"` 跑 current baseline——两者是同一份代码，critcmp 显示 ~0% 差异，门禁永远通过。这是预存设计限制（原 worktree 方案同样如此），非本次回归。FF5 修复使 workflow_dispatch on feature branch 用例真正生效，这才是回退检测的主用例。若编排者希望 schedule 也有意义，需改为对比 main 的历史 baseline（不同机制，超出本任务范围）。

⚠️ **regex `s` 替代项边缘误匹配**：bench 名称若含"数字+s"（如 `test_10s`）会被误识别为 10 秒时间 token。当前 bench 命名（`hybrid_search_10k_topk10`、`batch_add_10k`、`vector_only_10k`、`text_only_10k`）均无此问题。低风险。

## 维度 3：pub API 零改动 — ✅

- `git diff c287458..0a0ce5e -- crates/ | grep -E '^[+-]\s*pub (fn|struct|enum|trait|type|const) '` → **空**。无任何 pub 签名增删。
- `AddReport`（types.rs:109-112）：BASE 与 HEAD 零 diff。
- `types.rs` 全文件零 diff。
- `vector_field(&self) -> Result<(&str, u32, Metric)>`：未变。
- `StdFsVfs`：仅新增私有字段 `created_dirs: Mutex<HashSet<PathBuf>>`（std_fs.rs:9），`new()`/`with_root()` 签名不变。
- `PageCache` pub API（new/read/invalidate）：未变。
- `Vfs` trait：未变。
- `Schema`：未变。

## 维度 4：其余 parked 项质量

| 项 | 结论 | 证据 |
|----|------|------|
| 01-vfs P1 I11 注释 | ✅ | memory.rs:100 "I11" → 描述性文字（I-1~I-8 无 I11） |
| 01-vfs P2 list 排序统一 | ✅ | std_fs.rs:137 `out.sort()` + `std_fs_vfs_list_returns_sorted` 测试通过 |
| 01-vfs P3 PageCache 去重 | ✅ | page_cache.rs:107-113 先 insert 捕获旧值、移除旧 order 条目、saturating_sub 旧字节；`put_same_key_dedup_no_double_accounting` 测试通过。正常路径（无重复 key）行为与原一致 |
| 01-vfs P4 resolve 缓存 | ✅ | std_fs.rs:41-50 `Mutex<HashSet<PathBuf>>` 缓存，命中跳过 `create_dir_all`。不改 pub 签名。`create_dir_all` 失败仍吞错（与原一致），后续 write 会报错。无回归 |
| 02 cjk_bigram | ✅ | cjk_bigram.rs:29 仅加注释说明 position 跨 run 累积（I-4），无功能改动 |
| 03 P3 NaN 命名 | ✅ | fusion/tests.rs:153 `minmax_nan_safe_input_rejected` → `minmax_nan_input_does_not_panic` + 注释澄清 |
| 03 P4 文档措辞 | ✅ | fusion/mod.rs:4 `vane_core::types` → `crate::types`，与代码引用一致 |
| 04 inverted.bin 头校验 | ✅ | corpus_compat.rs:247 补 `inverted.bin`；bm25.rs:235-236 `write_inverted` 确写 `MAGIC`+`FORMAT_VERSION`；测试通过 |
| 09 check-thin | ✅ | 注释精简，grep 命令不变，I-8 门禁通过 |
| 10 install-matrix version | ✅ | install-matrix.yml:39-49 `Resolve version` 步骤从 package.json 动态读取，修复 workflow_run 时 `github.event.inputs` 为空回退 `'0.1.0'` |
| 10 hybrid_search 死代码 | ✅ | benches/hybrid_search.rs:67-81 删除 `db.collections()` + `_name` 未用行；schema 提取为变量（仍需作 `collection()` 参数）；bench 编译通过 |

## 维度 5：不变量 I-1~I-8

| 不变量 | 结论 | 证据 |
|--------|------|------|
| I-1 段不可变 | ✅ 未触 | 无 segment 写路径改动 |
| I-2 双索引原子可见 | ✅ 守住 | api refactor 未改 flush 的 manifest 切换逻辑；search 循环仅改迭代方式，不改快照加载 |
| I-3 图不原地删 | ✅ 未触 | 无 HNSW/tombstone 代码 |
| I-4 单一分词身份 | ✅ 守住 | cjk_bigram 注释明确 I-4；tokenizer_id 未变 |
| I-5 核心零平台分支 | ✅ 守住 | 无新 cfg；StdFsVfs 的 `#[cfg(not(target_arch="wasm32"))]` 是原有的 |
| I-6 manifest 原子性 | ✅ 守住 | restore_from_manifest 是读路径（open 段 + 读 meta），未改 manifest 写路径；api refactor 未碰 manifest 切换 |
| I-7 FFI 内存铁律 | ✅ 未触 | 无 FFI 改动 |
| I-8 binding 薄壳 | ✅ 守住 | check-thin.sh grep 不变，门禁通过 |

## 维度 6：范围合规 — ✅

- 无 HNSW/jieba/tombstone/WAL/Go/FF4 严格化/stored zsd/recall 硬编码改动（diff 中唯一 `hnsw` 命中是 check-thin.sh 的 grep 模式注释）。
- 无新依赖（所有 Cargo.toml 零 diff）。
- 无 dashmap/parking_lot/log/tracing/env_logger 引入。
- `StdFsVfs` 新增 `std::sync::Mutex` + `std::collections::HashSet` 均为 std 内置，非外部依赖；且在 `#[cfg(not(target_arch="wasm32"))]` 下，wasm32 不编译。

## 维度 7：implementer 裁决项判断

### 7.1 auto-commit 用 eprintln 不改 pub API — ✅ 可接受延后

- `AddReport` 加 `auto_commit_flush_error` 字段属 pub struct 字段变更，需走 pub API 变更流程 + FFI/Node 绑定同步。当前 eprintln 是合理过渡。
- 引入 `log` crate **不必要**：当前仅一处 auto-commit flush 错误记录，eprintln 满足过渡需求。M1 若需结构化日志再统一评估（注意 `log` 不在 M0 冻结依赖清单，引入需编排者批准）。

### 7.2 LTO 延后远程 CI — ✅ 合理

- 根 Cargo.toml 历史无 `[profile.release]`。napi cdylib + LTO 有符号注册边缘案例，本地无法完整验证 napi build。按"不确定就停下"原则延后 M1 远程 CI 实测，合理。

## 自证门禁复核

| 门禁 | 复核结果 |
|------|----------|
| `cargo test -p vane-core --lib` | ✅ 185 passed, 0 failed, 1 ignored |
| `cargo test -p vane-core --test corpus_compat` | ✅ 2 passed |
| `cargo test -p vane-core --lib api::` | ✅ 20 passed（含新增 restore 多段测试） |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | ✅ 无告警 |
| `cargo check --target wasm32-unknown-unknown -p vane-core` | ✅ 通过 |
| `cargo fmt --all -- --check` | ✅ 通过 |
| `bash scripts/check-no-std-fs.sh` | ✅ OK |
| `bash crates/vane-node/scripts/check-thin.sh` | ✅ OK (I-8 clean) |
| `check-bench-regression.py` 样例 | ✅ 回退>10% exit 1，无回退 exit 0，空文件 exit 0 |

## 观察项（非阻塞，按严重度排序）

1. ⚠️ **FF5 schedule 触发在 main 上无意义**（预存设计限制，非回归）：schedule 跑在 main 上时 main baseline 与 current baseline 是同一份代码。workflow_dispatch on feature branch 是主用例，门禁已生效。若需 schedule 有意义，需改为对比 main 历史 baseline（超出本任务范围）。
2. ⚠️ **bench regression regex `s` 替代项边缘误匹配**：bench 名称含"数字+s"时可能误识别。当前命名无此问题，低风险。

## 需编排者裁决的疑点

1. **auto-commit flush 失败标志（pub API 变更）**：是否在 `AddReport` 加 `auto_commit_flush_error: Option<VaneError>` 字段？当前 eprintln 过渡，M1 可引入 `log` crate（需评估是否在允许依赖清单）。implementer 判断"不改 pub API、延后"——审查者认可。
2. **`[profile.release]` LTO**：是否在根 Cargo.toml 加 `lto = "thin"` + `codegen-units = 1`？implementer 建议远程 CI 实测 napi build 后再加——审查者认可。
