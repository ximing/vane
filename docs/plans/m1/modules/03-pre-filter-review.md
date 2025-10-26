# 03-pre-filter 代码审查

> 审查基线：BASE=e9e9016（02 完成后）.. HEAD=6d2223f。
> 审查方式：只读 diff + 代码审查，不运行 cargo（编排者集成门禁已确认全绿）。
> 审查日期：2026-08-09。

## 逐维度结论

### 1. scalars.col 写读 ✅

- **`SegmentWriter::set_scalar`**（`segment/mod.rs:138-172`）：校验字段在 schema 且为 Scalar（经 `schema_scalar_kind`，`segment/mod.rs:175-189`），校验 value 变体与 ScalarKind 匹配（`segment/mod.rs:147-159`）。add_doc 前调用报 `Err(Schema)`（`segment/mod.rs:139-141`）。重复调用覆盖当前 docid 该字段值。add_doc 冻结签名不变（`segment/mod.rs:84-119`）。
- **finalize 写 scalars.col**（`segment/mod.rs:250-280`）：格式 `magic(4) | version(4 LE) | num_fields(4 LE) | { name_len(4 LE) | name | kind(1) | count(4 LE) | per-doc: present(1) + [value] }`。字段名排序保证写盘确定性（`segment/mod.rs:256-257`）。count = doc_count（dense per-docid 槽），未设值 present=0。
- **`ScalarReader::open/get/has_field`**（`segment/mod.rs:583-619`）：解码正确，present=0 → None。docid 越界 → None（`v.get(i)` 返回 None）。M0 空段（num_fields=0）向后兼容（`segment/mod.rs:648-665`）。
- **`read_all_optional`**（`segment/mod.rs:622-645`）：文件不存在返回 `Ok(None)`，兼容 M0 空段及 scalars.col 缺失场景。

### 2. 稀疏格式偏离（裁决项）✅ 合理

- **偏离内容**：`ScalarColumn` 用 `Vec<Option<T>>`（`segment/mod.rs:571-576`）替代 README 契约的 dense `Vec<T>`；磁盘格式 per-doc present(1 byte) + value。
- **合理性**：需支持「部分 docid 未调 set_scalar」场景。若用 dense + 默认值（如 Int=0），则 `Eq(Int(0))` 会误匹配未设值文档。Option 语义正确区分「未设值」与「设为默认值」。这是正确的设计抉择。
- **format_version 保持 1**：M0 stub 写 `magic+version+0u32`（num_fields=0，`e9e9016:segment/mod.rs:179-189`），新格式 num_fields=0 时字节完全相同。M0 从未写过字段级列数据，不存在旧格式字段数据需迁移的情况。bump version 无必要。`corpus_compat.rs:220-279` 测试已校验 scalars.col 的 magic+version=1 头。**结论：无需 bump version，偏离已文档化，可接受。**

### 3. compile_filter ✅

- **eq/in/gte/lte**（`filter/mod.rs:116-123`）：eq 用 `scalar_eq`（同类型才等）；in 用 `any(scalar_eq)`；gte/lte 用 `scalar_cmp().is_some_and(|o| o.is_ge()/is_le())`。跨类型比较返回 None（不命中），安全。
- **多字段 AND**（`filter/mod.rs:68-72`）：`acc = Some(prev & field_bm)`，正确取交集。
- **tombstone AND NOT**（`filter/mod.rs:77-79`）：`bm -= tb.as_ref()`，绝对 docid 空间一致。
- **local→绝对 docid**（`filter/mod.rs:61-64`）：`base + local as u64`，base 取自 `reader.meta().docid_base`。超 u32::MAX 防御性跳过（与 search 一致）。
- **Float 比较**用 `total_cmp`（`filter/mod.rs:141`），NaN 确定性。

### 4. should_fallback_brute ✅

- `bitmap.len() < 2 * topk as u64`（`filter/mod.rs:150`）。roaring 0.10 的 `len()` 即 cardinality（元素计数）。边界测试 `should_fallback_brute_boundary`（`filter/tests.rs:383-391`）验证 `==2*topk` 不回退。

### 5. api search 接入 ✅

- **M0 filter reject 移除**：search 方法不再对 `query.filter.is_some()` 返回 InvalidArg。原 M0 测试更名更新（`api/tests.rs:441-471`）。
- **compile_filter 产位图透传**：`collection.rs:540-563`，位图传给 vector 路（HnswReader::search / brute_search）和 text 路（InvertedIndexReader::search）。
- **低选择率回退**：`force_brute = should_fallback_brute(bm, topk)`（`collection.rs:570-573`），true → brute_search，false → HnswReader::search。
- **HnswReader::search 实际签名**：调用 `hr.search(qv, want, ef, merged_filter, base, reader.vectors())`（`collection.rs:596`），与实际签名 `search(&self, query, topk, ef_search, filter, docid_base, vectors: &[f32])`（`hnsw/mod.rs:624-632`）完全匹配。vectors 参数由 `reader.vectors()` 传入。✅

### 6. flush set_scalar ✅

- `collection.rs:254-264`：flush 遍历 `BufferedDoc.meta`，仅 schema 中声明为 Scalar 的字段经 `set_scalar` 写入。非标量 meta 仍走 stored_json。类型不匹配时 `set_scalar` 返回 `Err(Schema)`，flush 失败（严格校验，合理）。

### 7. Q-7 MergeTask 标量重写 ✅

- `merge/mod.rs:134-148`：加载源段 `ScalarReader`，枚举 schema 标量字段（有列的）。
- `merge/mod.rs:207-212`：逐 non-tombstoned doc 从源段 `scalar_reader.get(field, local)` 读值 → `writer.set_scalar(field, sv)` 写入新段（new_local 由 add_doc 返回，set_scalar 写入当前 docid 槽）。源 doc 未设值（None）→ 跳过，保留稀疏语义。
- **标量 roundtrip 测试**：`compact_preserves_scalars_for_filter`（`pre_filter.rs:307-344`，验证 Keyword + Int 列重写）+ `compact_then_filter_after_delete`（`pre_filter.rs:347-373`，验证 compact 物理清除后 filter 正常）。

### 8. tombstone 并入（无 filter 场景）✅

- `collection.rs:539-563`：无 filter 但有 tombstone → `alive_bitmap`（全量减 tombstone）；无 filter 无 tombstone → None（M0 行为，最高效）。
- `alive_bitmap`（`filter/mod.rs:85-113`）：遍历段 insert_range(base..base+count)，逐段 `bm -= tombstones[i]`。测试 `alive_bitmap_excludes_tombstone`（`filter/tests.rs:394-405`）+ `no_filter_still_excludes_tombstone`（`pre_filter.rs:262-279`）。

### 9. 不变量/M0 签名 ✅

- **I-5 零 cfg**：filter/segment 仅 `#[cfg(test)]`，无 feature cfg。
- **M0 冻结签名零破坏**：`add_doc`、`brute_search`、`InvertedIndexReader::search` 签名均不变。仅新增 `set_scalar` / `ScalarReader` / `ScalarColumn` / `compile_filter` / `should_fallback_brute` / `alive_bitmap`。
- **`ScalarValue` 增 `PartialEq`**（`api/types.rs:58`）：additive derive，非签名变更。Float 的 `==` 与 `scalar_eq` 行为一致（NaN != NaN），`scalar_cmp` 用 `total_cmp` 保证 Gte/Lte 确定性。

### 10. 范围合规 ✅

- 只做 03 + Q-7，无 WAL(04)/reindex(06) 越界。
- 无黑名单依赖。core 禁 std::fs：filter/segment 无 std::fs（仅 vfs/std_fs.rs 内部使用，属 VFS 实现层）。

### 11. 测试质量 ✅

- **11 pre_filter 集成测试**（`tests/pre_filter.rs`）：filter eq/gte/in、多字段 AND、低选择率回退（验证 d9 唯一命中）、text 模式 filter、tombstone 排除（有/无 filter 两条路径）、跨段 filter、Q-7 compact 标量保留（Keyword+Int）、reopen ScalarReader。真实断言命中/排除/数量。
- **16 filter 单元测试**（`src/filter/tests.rs`）：roundtrip（Int/Keyword/Float/Bool）、sparse missing doc、set_scalar 错误（before add_doc / wrong field / kind mismatch）、compile_filter 全条件 + AND 空集 + 字段缺失 + tombstone 排除、should_fallback_brute 边界、alive_bitmap。覆盖充分。

### 12. compile_filter schema 参数未校验（裁决项）⚠️ 可接受

- `_schema` 参数未用于校验（`filter/mod.rs:29`）。未知字段 → `sr.has_field(field)` 全段 false → `field_bm` 为空 → 结果空匹配。非崩溃，非误命中。
- **评估**：SPEC §8.3 未要求 filter 编译期校验字段名。空匹配语义安全（等价 SQL NULL 不匹配）。补 `E_SCHEMA` 校验可提升用户体验（早报错而非空结果），但非阻塞项。**结论：可接受，建议后续补 schema 校验以改善 DX，不阻塞合并。**

## 额外发现（非阻塞）

### F-1：compile_filter 文档注释与行为不一致（minor）

`filter/mod.rs:25-26` 文档注释称「无 filter 字段时返回全量 alive 位图」，但实际 `filter.fields` 为空时 `acc` 为 None → `bm = acc.unwrap_or_default()` = 空位图。api 层从未以空 fields 调 compile_filter（无 filter 走 `alive_bitmap` 路径），故无功能影响。建议修正注释为「空 fields 返回空位图（仅含 tombstone 排除）」或实装注释所述行为。

### F-2：`merged_filter` 命名遗留（cosmetic）

`collection.rs:581` `let merged_filter: Option<&RoaringBitmap> = filter_bm;` 仅是 `filter_bm` 的别名（非合并），命名遗留自 02 手动 alive_bm 并入。功能正确，建议改名 `filter_bm` 直接使用。

### F-3：read_all_optional 错误遮蔽（minor）

`segment/mod.rs:630` 首次读失败时 `Err(Io(_)) if !started` → `Ok(None)`。将文件不存在与首次读 IO 错误（如权限）等同处理。实践中 VFS 后端（Memory/StdFs）对不存在文件返回 Io 错误，权限错误罕见，可接受。如需精确区分可引入 `VaneError::NotFound`，非阻塞。

## 最终结论

**verdict: APPROVED_WITH_MINOR**

阻塞项：无。

稀疏格式偏离结论：`Vec<Option<T>>` + per-doc present byte 是合理设计抉择，正确区分「未设值」与「设为默认值」。format_version 保持 1 成立（M0 stub num_fields=0 字节级兼容，无旧字段数据需迁移）。无需 bump version，偏离已文档化。

需编排者裁决疑点：
1. **compile_filter schema 校验**（维度 12）：未知字段当前返回空匹配。是否需补 `E_SCHEMA` 早报错？建议非阻塞，后续 DX 优化。
2. **compile_filter 文档注释**（F-1）：空 fields 行为与注释不符。是否修正注释或实装「空 fields → 全量 alive」？当前无功能影响（api 不以空 fields 调用）。
