# 03-pre-filter 实装报告

> SPEC §8.3（过滤 pre-filter）/§3.1（scalar）/§6.2（scalars.col）/§8.1（低选择率暴力回退）。
> 前置：M0 segment/bm25/vector/api + 01-hnsw + 02-tombstone-merge。

## 各 Task 改动

### Task 1：scalars.col 写读（SegmentWriter::set_scalar + ScalarReader）

- `crates/vane-core/src/segment/mod.rs`：
  - `SegmentWriter` 增 `scalars: HashMap<String, (ScalarKind, Vec<Option<ScalarValue>>)>` + `schema_snapshot: Option<Schema>`（new 时 clone，不改 M0 new 签名）。
  - `set_scalar(field, value)`：校验字段在 schema 且为 Scalar + kind 匹配，写入当前 docid 槽位。add_doc 前调用报 Schema 错。
  - `finalize` 写 scalars.col 真实数据，替换 M0 空 stub。格式：`magic(4) | version(4 LE) | num_fields(4 LE) | { name_len(4 LE) | name | kind(1) | count(4 LE) | per-doc: present(1) + [value] }`。count = doc_count（dense，per-docid 槽）。未设值 present=0。
  - 新增 `ScalarReader::open/get/has_field` + `ScalarColumn` 枚举。M0 段（num_fields=0）向后兼容。
- `crates/vane-core/src/api/types.rs`：`ScalarValue` 增 `derive(PartialEq)`（additive，非 M0 签名变更；测试 assert_eq 用）。

### Task 2：Filter 编译（compile_filter）

- `crates/vane-core/src/filter/mod.rs`（新建）：
  - `compile_filter(filter, schema, segments, scalars, tombstones) -> RoaringBitmap`：遍历 filter.fields，每字段扫 ScalarReader 列式块，按 eq/in/gte/lte 匹配 docid 入位图（绝对 docid）；多字段 AND（交集）。末尾 `and_not` 排除各段 tombstone。
  - 跨类型比较返回 None（不命中）；Float 用 total_cmp 保证 NaN 确定性。

### Task 3：低选择率暴力回退（should_fallback_brute）

- `should_fallback_brute(bm, topk) = bm.len() < 2 * topk`（roaring 0.10.12 用 `len()` 非 `cardinality()`）。

### Task 4：api search 接入 filter + 回退

- `crates/vane-core/src/api/collection.rs`：
  - `CollectionInner` 增 `scalar_readers: RwLock<Vec<Arc<ScalarReader>>>` 缓存。
  - `search`：移除 02 手动 alive_bm 并入。改为：有用户 filter → `compile_filter`（含 tombstone 排除）；无 filter 但有 tombstone → `alive_bitmap`；否则 None。`force_brute = should_fallback_brute(bm, topk)`。位图透传各段 HnswReader::search/brute_search/InvertedIndexReader::search。
  - `flush`：BufferedDoc.meta 中 schema 标量字段经 `set_scalar` 写入。
  - `restore_from_manifest` / `merge_segments`：加载并缓存 ScalarReader。
- `crates/vane-core/src/api/tests.rs`：M0 `search_filter_accepted_but_not_compiled_in_m1` 更名为 `search_filter_compiled_returns_empty_when_no_match`（语义更新）。
- `crates/vane-core/tests/pre_filter.rs`（新建）：11 集成测试覆盖 filter eq/gte/in、多字段 AND、低选择率回退、text 模式 filter、tombstone 排除、跨段 filter、reopen。

### Task 5：tombstone 并入 filter

- `compile_filter` 末尾对每段 `bm -= tombstone`（绝对 docid 空间）。
- `alive_bitmap(segments, tombstones)`：无 filter 时构造全量 alive 位图（所有段 docid 减 tombstone），供无 filter 的 search 路径统一排除 tombstone。

### Q-7：MergeTask 标量重写

- `crates/vane-core/src/merge/mod.rs`：
  - `MergeTask::step` 加载源段 `ScalarReader`，枚举 schema 标量字段（有列的），合并时按新 docid 调 `new_writer.set_scalar` 写入新段。补 `compact_preserves_scalars_for_filter` / `compact_then_filter_after_delete` 集成测试。

## 偏离与裁决

- **HnswReader::search 实际签名**：确认与编排者提示一致——`search(&self, query, topk, ef_search, filter, docid_base, vectors: &[f32])`。api 层传 `reader.vectors()`。计划文档 stale 签名（无 vectors 参数）未采用。
- **ScalarColumn 偏离 README 契约**：README 列 `Vec<i64>` 等（dense，无 Option）。实装改为 `Vec<Option<T>>` 以表达「该 docid 未设值」（filter 不命中）。属新增类型，非 M0 冻结签名变更；`get` 返回 `Option<ScalarValue>` 契约不变。
- **scalars.col 磁盘格式**：采用 per-doc present(1 byte) + value（若 present），而非 README 暗示的纯 dense 值块。原因：需支持稀疏（部分 docid 未调 set_scalar）。format_version 仍为 1（M0 空 stub num_fields=0 向后兼容）。
- **roaring API**：0.10.12 用 `len()` 非 `cardinality()`；`insert_range` / `-= ` 运算符均可用。
- **无 filter 无 tombstone 路径**：search 传 None（M0 行为，最高效），避免无谓位图构建。
- **compile_filter 的 schema 参数**：当前未用于校验（`_schema`），filter 字段不在 schema 时按「该段无列」处理 → 空匹配。保留参数满足契约。

## 自证门禁结果（全绿）

```
cargo test --workspace --all-features        → 234 lib + 11 pre_filter + 全部集成 pass（1 ignored 性能测试）
cargo clippy --workspace --all-targets --all-features -- -D warnings  → clean
cargo check --target wasm32-unknown-unknown -p vane-core  → clean（零 cfg）
cargo fmt --all -- --check                   → clean
bash scripts/check-no-std-fs.sh              → OK
bash crates/vane-node/scripts/check-thin.sh  → OK
cargo bench --no-run -p vane-core            → build OK
```

## 提交 hash

- `57785ce` segment: 实装 scalars.col 列式块写读（Task 1）
- `15dfade` filter: compile_filter + should_fallback_brute + alive_bitmap（Task 2/3/5）
- `5260c49` api: 接入 pre-filter 编译 + 自适应暴力回退 + MergeTask 标量重写（Task 4/Q-7）

实际 3 commit（Task 2/3/5 因 filter/mod.rs 单文件交织合并为一 commit；Task 4+Q-7 因 api/merge 交织合并为一 commit）。

## 遗留/疑问

- 无。全部 5 Task + Q-7 实装完成，门禁全绿。
