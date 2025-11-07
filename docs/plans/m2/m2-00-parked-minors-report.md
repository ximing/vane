# M2 Phase Zero — Parked Minors 清理报告

> 产出：M2 Phase Zero developer SubAgent（2026-08-09）
> 起点：HEAD `6548a3`（M1 完成，340 测试绿）
> 终点：HEAD `ff1d527`（6 项 parked minors 全部清理，347 测试绿）

---

## 逐项改动

### 2.1.1 is_cjk 代码复制

- `crates/vane-core/src/tokenizer/mod.rs`：新增 `pub(crate) fn is_cjk(c: char) -> bool`（9 个 unicode range `matches!`，从 cjk_bigram.rs 提取）。新增 `factory_tests::is_cjk_covers_common_ranges` 单测（6 断言：汉/あ/カ 命中，a/空格/1 不命中）。
- `crates/vane-core/src/tokenizer/cjk_bigram.rs`：删除本地 `fn is_cjk`，`use` 改 `is_cjk`；旧测试 `is_cjk_covers_common_ranges` 改调 `super::is_cjk`（通过 `use` 引入）。
- `crates/vane-core/src/tokenizer/jieba/mod.rs`：删除本地 `fn is_cjk`，`use` 改 `is_cjk`。
- commit `f81be11`

### 2.1.2 UserTrie max(freq) 缺省值

- `crates/vane-core/src/tokenizer/jieba/mod.rs:36-44`：`JiebaTokenizer::new` 补文档注释明确"缺省 freq = `dict.max_freq()`，与 jieba-rs 原版一致，保证 DAG 优先命中（SPEC §5.3）"；`max_freq` 赋值行补注释。
- `crates/vane-core/src/tokenizer/jieba/tests.rs`：新增 `user_dict_default_freq_overrides_builtin_in_dag` 单测——内置词典含"机器"(50)+"学习"(200) 但不含整词"机器学习"；无用户词时切分为"机器"+"学习"；注入 `UserDictEntry::Word("机器学习")`（缺省 freq=max_freq=200）后切分为整词"机器学习"（单 token），验证 DAG 优先命中。
- commit `70622b2`

### 2.1.3 compile_filter schema 校验

- `crates/vane-core/src/filter/mod.rs:27-30`：`_schema` 参数去 `_` 改名 `schema`。入口新增 schema 校验循环：每个 filter 字段必须在 `schema.fields` 且为 `FieldDef::Scalar`，否则 `Err(VaneError::InvalidArg(...))`。补文档注释说明校验语义与行为变更。
- `crates/vane-core/src/filter/mod.rs:15`：`use` 新增 `FieldDef`。
- commit `d490b43`

**受影响测试清单及处理**：

| 测试 | 文件 | 原行为 | 处理 |
|---|---|---|---|
| `compile_filter_field_missing_in_segment` | `filter/tests.rs:336` | filter 字段 `"nonexistent"` 不在 schema，断言空位图 | 改名 `compile_filter_field_not_in_schema_errors`，断言 `Err(InvalidArg)` |
| `search_filter_compiled_returns_empty_when_no_match` | `api/tests.rs:443` | schema 仅含 `v`(Vector)，filter 用 `lang` 不在 schema，断言 `Ok(空 Vec)` | schema 补 `lang`(Scalar Keyword) 字段，断言不变（`Ok(空 Vec)`） |
| `compile_filter_non_scalar_field_errors` | `filter/tests.rs`（新增） | — | 新增：filter 字段为 Vector → `Err(InvalidArg)` |
| `search_filter_field_not_in_schema_errors` | `api/tests.rs`（新增） | — | 新增：filter 字段不在 schema → search 返回 `Err(InvalidArg)` |
| `tests/pre_filter.rs` 全部集成测试 | — | 全用 schema-valid Scalar 字段（`lang`/`year`） | 无需改 |

### 2.1.4 recover 目录扫描

- `crates/vane-core/src/wal/mod.rs:184-243`：`recover` 末尾调 `cleanup_orphan_segment_dirs`。新增 `cleanup_orphan_segment_dirs(vfs, db_path, manifest)`：`Vfs::list("segments")` 扫描每个 `seg_<ulid>` 子目录，若 ulid 不在 manifest 任何 collection 的 `segment_ulids` 中，调 `merge::delete_segment_dir` 递归删除。`segments/` 目录不存在（`Err(Io)`）时无操作。新增 `ulid_in_any_collection` 辅助函数。`recover` 文档注释补"目录扫描"语义说明。
- `crates/vane-core/src/wal/tests.rs`：新增 `recover_cleans_orphan_segment_dir_not_in_wal`（孤儿段清理+合法段保留+非 seg_ 目录不触碰）、`recover_empty_segments_dir_no_error`（新库无异常）。
- commit `4ff9203`

### 2.1.5 并发测试 jieba 场景

- `crates/vane-core/src/api/reindex_tests.rs`：新增 `#[cfg(all(test, feature="jieba"))] jieba_concurrent_search_during_reindex_no_panic`——jieba collection + 2 线程 search + 主线程 `setUserDict`→`reindex`，断言不 panic、search 次数 >0、reindex 完成后所有 reader `tokenizer_id` 一致（I-4 不混排）。用 `std::thread` + `std::sync`（AtomicBool/AtomicUsize/Arc），不引 dashmap/parking_lot。
- `crates/vane-core/src/api/db.rs:216-230`：新增 `#[cfg(all(test, feature="jieba"))] Db::set_jieba_dict_for_test`——`pub(crate)` 测试专用方法，`Arc::get_mut` 注入 jieba 词典（绕过 dict-zh 自动加载），供并发测试构造 jieba collection。
- commit `f964864`

### 2.1.6 header.bin tombstone abs/local 语义文档化

- `crates/vane-core/src/segment/header.rs:4-11`：顶部布局注释补"tombstone_data 存绝对 docid（u32 空间，与 WAL/run-time 一致，M-minor-2）"。
- `crates/vane-core/src/segment/mod.rs:17-22`：`SegmentMeta.tombstones` 字段注释补"存绝对 docid，与 WAL `WalRecord::AddTombstone.docids` 及运行期 `CollectionInner.tombstones` 一致"。
- 纯文档化，无新测试，无行为变更。
- commit `ff1d527`

---

## 自证门禁结果

| # | 门禁 | 结果 |
|---|---|---|
| 1 | `cargo test --workspace --all-features` | 347 passed, 0 failed（基线 340 + 新增 7） |
| 2 | `cargo test -p vane-core --features jieba` | 307 passed, 0 failed |
| 3 | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | clean |
| 4 | `cargo fmt --all -- --check` | clean |
| 5 | `cargo check --target wasm32-unknown-unknown -p vane-core` | 通过（core 零 cfg(target) 未破） |
| 6 | `bash scripts/check-no-std-fs.sh` | OK |
| 7 | `cargo deny check` | advisories ok, bans ok, licenses ok, sources ok |

**新增测试清单（+7）**：
- `tokenizer::factory_tests::is_cjk_covers_common_ranges`（2.1.1）
- `tokenizer::jieba::tests::user_dict_default_freq_overrides_builtin_in_dag`（2.1.2）
- `filter::tests::compile_filter_field_not_in_schema_errors`（2.1.3，改名）
- `filter::tests::compile_filter_non_scalar_field_errors`（2.1.3，新增）
- `api::tests::search_filter_field_not_in_schema_errors`（2.1.3，新增）
- `wal::tests::recover_cleans_orphan_segment_dir_not_in_wal`（2.1.4）
- `wal::tests::recover_empty_segments_dir_no_error`（2.1.4）
- `api::reindex_tests::jieba_concurrent_search_during_reindex_no_panic`（2.1.5）

---

## 遗留/疑问

- **2.1.5 jieba 并发测试 feature 门控**：测试用 `#[cfg(all(test, feature="jieba"))]`，但构造 jieba collection 需注入词典。为不依赖 `dict-zh` feature（使 `--features jieba` 即可跑），新增了 `Db::set_jieba_dict_for_test`（`#[cfg(test)]`）测试专用方法，用 `Arc::get_mut` 注入测试夹具词典。该方法是 `pub(crate)` + `#[cfg(test)]`，不进生产产物、不暴露 pub API。若未来需在 `--features jieba`（无 dict-zh）下正式支持 jieba collection 创建，需考虑公开注入接口。
- **2.1.3 行为变更**：filter 字段不存在/非标量从静默空位图改为 `Err(InvalidArg)`。这是 SPEC §10 预期行为（E_INVALID_ARG），但绑定层（vane-node/vane-ffi）若有调用方依赖旧的"静默空结果"行为，需适配错误处理。M2 后续模块（M2-01~）应留意。
- 无阻塞项。
