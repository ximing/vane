# 阶段零-B：清理（FF5 benchmark 回退门禁 + parked 次要项）实现报告

> 实现者：housekeeping SubAgent
> 基线：HEAD c287458（阶段零-A 已落地）
> 完成时间：2026-08-09

## 状态

**DONE_WITH_CONCERNS** — 全部 parked 项已处理，自证门禁全绿；2 项需编排者裁决（见末节）。

## 提交列表

| hash | 说明 |
|------|------|
| ae52beb | fix(ci): FF5 benchmark 回退门禁真正生效 |
| 958f6c8 | refactor(vfs): 01-vfs parked P1-P4 清理 |
| 584d326 | docs(tokenizer): 02 cjk_bigram position 写法说明 |
| 6c6c43f | refactor(fusion): 03 P3/P4 清理 |
| e0201c8 | test(segment): 04 corpus_compat 补 inverted.bin 头校验 |
| 0d0b249 | refactor(api): 07-api-core parked 项清理 |
| ac99236 | chore(node): 09 check-thin.sh 注释精简 |
| 33102a2 | chore(ci): 10-ci-gates 清理 |
| c6ed5c1 | style: cargo fmt 应用（顺带纳入编排者预置 plan 文档） |

## 逐项实际改动

### A. FF5 benchmark 回退门禁修复（commit ae52beb）

**问题根因**：`.github/workflows/benchmark.yml` 用 `git worktree add ../vane-main main` 在对侧目录跑 main baseline，criterion baseline 存在各自 worktree 的 `target/criterion`，repo 根的 `critcmp main current` 读不到对侧 baseline → 回退门禁失效。同时 `check-bench-regression.py` 的 regex 要求行内含 `current` 字面词，critcmp 表格数据行无此词 → 解析永远返回空 → `exit 0` 兜底。

**改动**：
- `.github/workflows/benchmark.yml`：改为同一 checkout 内顺序切分支（`git checkout main` → `--save-baseline main` → `git checkout 触发分支` → `--save-baseline current`），两者共享 `target/criterion`。保留 `critcmp ... || true`（critcmp 退出码不可靠），真正判定交给 python 脚本。
- `scripts/check-bench-regression.py`：重写解析器，按行抓取所有 `<数值 单位>` token（正则 `([\d.]+)\s*(ms|µs|us|ns|s)`），取前两个作 main/current，归一化到 ms。不再依赖行内 `current` 字面词。
- `scripts/examples/compare_regression.txt` / `compare_ok.txt`：新增样例 fixture，固化脚本解析行为。

**本地验证**：
- 回退样例（hybrid +13.8%）→ `exit 1`，输出 `FAIL: 1 benchmark(s) regressed > 10%`。
- 无回退样例 → `exit 0`，输出 `OK`。
- 空文件 → `exit 0`（容错）。

> 注：benchmark.yml 是 schedule + workflow_dispatch 触发，本地不实跑（耗时长）。本地仅验证脚本解析行为。

### B. 01-vfs（commit 958f6c8）

- **P1** `vfs/memory.rs:100`：注释 "I11" 改为描述性文字（I-1~I-8 无 I11 编号）。
- **P2** `vfs/std_fs.rs::list`：加 `out.sort()`，与 `MemoryVfs::list` 一致返回有序结果。新增 `std_fs_vfs_list_returns_sorted` 测试（创建 c/a/b 三文件，断言返回 `["a.bin","b.bin","c.bin"]`）。
- **P3** `vfs/page_cache.rs::Inner::put`：加同 key 去重防御——`pages.insert` 返回旧值时，从 `order` 移除旧条目并 `saturating_sub` 旧 `used_bytes`，避免重复条目与重复累加。新增 `put_same_key_dedup_no_double_accounting` 单元测试（在 page_cache.rs 内部，访问私有 `inner`）。
- **P4** `vfs/std_fs.rs::StdFsVfs`：加 `created_dirs: Mutex<HashSet<PathBuf>>` 字段，`resolve` 命中缓存则跳过 `create_dir_all`。不改 pub 签名（字段私有）。`new()` / `with_root()` 初始化缓存。

### 02-tokenizer（commit 584d326）

- `tokenizer/cjk_bigram.rs:29`：`let mut position` 加注释说明为何不能改用 standard 的 `zip(0_u32..)` 写法——position 跨 run 连续递增（不变量 I-4），需可变状态在 run 间累积。无功能改动。

### 03-fusion（commit 6c6c43f）

- **P3** `fusion/tests.rs`：`minmax_nan_safe_input_rejected` 改名 `minmax_nan_input_does_not_panic` + 注释澄清（NaN 非首元素仍 NaN，调用方契约不含 NaN，测试仅验证不 panic）。
- **P4** `fusion/mod.rs:3`：模块文档 `vane_core::types::ScoredDoc` 改为 `crate::types::ScoredDoc`，与代码引用一致。

### 04-segment（commit e0201c8）

- `tests/corpus_compat.rs::corpus_segment_files_have_magic_version_headers`：校验清单补 `inverted.bin`。已 Read `bm25.rs:229-236`（write_inverted 写 magic+version）与 `:338-358`（open 校验）确认格式合规。测试通过。

### 07-api-core（commit 0d0b249）

- **vector_field hoist** `api/collection.rs::search`：dim 校验 + metric 解析合并为一次性 `vf: Option<Metric>`，hoist 出循环。循环内用 `if let (Some(qv), Some(metric)) = (&query.vector, vf)`。无语义变化（仅当 query.vector 有值时才调用 vector_field，保持原行为）。
- **inv_readers zip**：`for (i, reader) in snap.iter().enumerate()` + `inv_readers[i]` 改为 `for (reader, inv_reader) in snap.iter().zip(inv_readers.iter())`。对齐更稳健（不会越界）。
- **checked_sub**：Hit 回填循环 `sd.docid.wrapping_sub(base)` 改为 `sd.docid.checked_sub(base)`，`None` 时 `continue` 跳过该段（避免 wrapping 产生巨大 local 误命中脆弱 external_id 查找）。
- **auto-commit flush 吞错**：`let _ = self.flush();` 改为 `if let Err(e) = self.flush() { eprintln!(...) }`。**不改 AddReport pub API**（加失败标志属 pub API 变更，标记交编排者）。M1 可引入 log crate 做结构化日志。
- **restore base**：`restore_from_manifest` 从段头读 `reader.meta().docid_base`（而非累加 `doc_count` 推断），`next_docid` 取 `max(base + count)`。新增 `restore_multi_segment_uses_stored_docid_base` 测试：两段（a/b + c/d）reopen 后查 c 向量命中 c（验证第二段 offset=2 正确），再灌 e 验证 next_docid=4 不冲突。

> 裁决说明：restore base 在 M0 不是真实 bug（段总是从 0 连续追加，累加结果与段头一致），但累加推断在 M1 compaction（非连续段）场景会出错。改为读段头是更稳健的防御性改进，非 bug 修复。不改 pub API。

### 09-node-binding（commit ac99236）

- `crates/vane-node/scripts/check-thin.sh`：精简冗余注释（合并重复说明行），保留 I-8 门禁语义与管道解释。门禁仍通过。
- **`[profile.release]` LTO 评估**：根 Cargo.toml 历史无 `[profile.release]`。napi cdylib + LTO 有边缘案例（napi 符号注册），本地无法完整验证 napi build（需 napi CLI + node-gyp 全流程）。按"不确定就停下"原则**不擅自加**，标记交编排者作 M1 可选优化。

### 10-ci-gates（commit 33102a2）

- `.github/workflows/install-matrix.yml`：新增 `Resolve version` 步骤，`workflow_dispatch` 传参优先，否则 `node -p "require('./crates/vane-node/package.json').version"` 动态读取。此前 `workflow_run` 触发时 `github.event.inputs` 为空，硬编码回退 `'0.1.0'` 会与已发布版本脱节。
- `crates/vane-core/benches/hybrid_search.rs:65-85`：删除冗余死代码（`db.collections()` 未用结果 `_name` + 重复 schema 构造），`collection()` 幂等返回已有句柄。bench 编译通过。

## 自证门禁结果

| 门禁 | 结果 |
|------|------|
| `cargo test --workspace --all-features` | ✅ 185 lib + 2 corpus_compat + 19 node + 4 ffi + 1 = 全绿（1 ignored 为原有） |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | ✅ 无告警 |
| `cargo check --target wasm32-unknown-unknown -p vane-core` | ✅ 通过 |
| `cargo fmt --all -- --check` | ✅ 通过 |
| `bash scripts/check-no-std-fs.sh` | ✅ OK |
| `bash crates/vane-node/scripts/check-thin.sh` | ✅ OK (I-8 clean) |
| `cargo test --test corpus_compat -p vane-core` | ✅ 2 passed |
| `check-bench-regression.py` 样例验证 | ✅ 回退>10% exit 1，否则 exit 0 |

## 需编排者裁决的项

### 1. auto-commit flush 失败标志（pub API 变更）

**现状**：auto-commit flush 失败改为 `eprintln!` 记录，不改 AddReport。

**待裁决**：是否在 AddReport 加 `auto_commit_flush_error: Option<VaneError>` 字段以暴露失败给调用方？这属 pub API 变更（`AddReport` 是 pub struct with pub fields）。若同意，需走 pub API 变更流程并同步 FFI/Node 绑定。当前 eprintln 仅是过渡，M1 建议引入 `log` crate 做结构化日志（core 目前无 log 依赖，引入需评估是否在允许依赖清单内）。

### 2. `[profile.release]` LTO

**待裁决**：是否在工作区根 Cargo.toml 加 `[profile.release] lto = "thin"`（或 `true`）+ `codegen-units = 1`？对 napi cdylib 二进制体积/性能有益，但 napi build 边缘案例需远程 CI 实测验证。建议在 M1 首次发布前于 CI 上试跑 `napi build --platform --release` 确认无符号问题后再加。

## 偏离与裁决记录

- **cjk_bigram zip 统一**：评估后 cjk_bigram 的 `let mut position` 是必要的（跨 run 累积），无法改 standard 的 zip 写法，改为加注释说明，未做功能性改动。符合简报"无功能问题"的判定。
- **restore base**：判定为 M0 非真实 bug（连续追加场景累加与段头一致），按防御性改进处理（读段头），未按 TDD bug 修复流程走，但补了多段 restore 测试。若编排者认为应严格按 bug 修复流程标注，可调整。
- **plan 文档纳入**：最后 fmt 提交 `git add -A` 意外纳入编排者预置的 `docs/plans/m1/00-cleanup-review.md`、`01-housekeeping.md`、`EXECUTION-NOTES.md`。这些是 plan 文档，纳入 repo 无害，保留。
