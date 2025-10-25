# 00-text-persistence 代码审查

> 审查基线：BASE=ba67d51 → HEAD=97823d0（`git diff ba67d51..HEAD -- crates/`）
> 审查对象：原文持久化（L0 前置模块，解锁 02-merge / 06-reindex）
> 审查日期：2026-08-09
> 审查者：code-reviewer

## 维度逐条核查

### 1. stored.bin 新布局正确性 ✅

**布局**（`segment/mod.rs:142-161` finalize 写入）：
`magic(4) | format_version(4 LE) | count(4 LE) | { docid(8 LE) | text_len(4 LE) | text_bytes | meta_json_len(4 LE) | meta_json_bytes }...`

- 头部 magic+version+count 保留（`MAGIC` + `FORMAT_VERSION.to_le_bytes()` + `count.to_le_bytes()`），与 M0 一致。
- encode/decode 对称：`decode_stored`（`mod.rs:404-460`）逐字段按 docid(8)→text_len(4)→text_bytes→meta_len(4)→meta_bytes 读取，与写入顺序完全一致。
- text_len=0 边界：`entry.text.as_deref().unwrap_or("")` 落空串，`text_bytes.len()==0`；解码侧 `pos+0<=buf.len()` 通过、`from_utf8(&buf[pos..pos])` 返回空串。`stored_text_roundtrip` 测试 d1 未调 set_text → `r.text(1)==Some("")` 验证通过。
- 截断校验比 M0 的 `decode_kv_map` 更严格（每字段 `pos+N>buf.len()` 返回 `Corrupt` 而非 `break`），合理。
- UTF-8 校验：text 与 meta 均经 `from_utf8` 校验，错误返回 `Corrupt`。

### 2. M0 冻结签名零破坏 ✅

对照 `docs/plans/m0/README.md:283-316` 契约与 `crates/vane-core/src/segment/mod.rs` 实际 pub fn：

| 方法 | M0 契约 | 当前实现 | 结论 |
|---|---|---|---|
| `SegmentWriter::new` | `(vfs, segments_dir, schema, tokenizer_id, docid_base) -> Result<Self>` | 同 | 未变 |
| `add_doc` | `(&mut self, external_id, vector, stored_json) -> Result<u64>` | 同 | 未变 |
| `finalize` | `(self) -> Result<SegmentMeta>` | 同 | 未变 |
| `docid_base` | `(&self) -> u64` | 同 | 未变 |
| `SegmentReader::open` | `(vfs: &Arc<dyn Vfs>, segment_dir: &str) -> Result<Self>` | 同 | 未变 |
| `meta/vectors/dim/doc_count/external_id/stored_json/segment_dir/vfs` | 同 M0 | 同 | 未变 |
| `set_text` | — | 新增 `((&mut self, text: &str) -> Result<()>` | 仅新增 |
| `text` | — | 新增 `(&self, local_docid: u64) -> Option<&str>` | 仅新增 |

内部字段 `stored` 由 `Vec<(u64, String)>` 改为 `Vec<StoredEntry>`、Reader 侧 `HashMap<u64, String>` 改为 `HashMap<u64, StoredReadEntry>`——均为私有字段，不破坏 pub API。

### 3. set_text 语义 ✅

- `set_text`（`mod.rs:116-122`）取 `stored.last_mut()`，`None` 时 `Err(VaneError::Schema("set_text called before add_doc"))`。`set_text_before_add_doc_errors` 测试断言 `matches!(err, VaneError::Schema(_))` 通过。
- 仅修改写期 buffer 的 `entry.text`，不触盘。finalize 一次性序列化全部 stored。I-1 保持。
- 重复调用覆盖（`entry.text = Some(...)` 直接赋值），符合计划。
- 仅绑定最近一次 add_doc——api flush 在每次 `add_doc` 之后立即调用，绑定正确。

### 4. text() 返回 ⚠️（轻微，需裁决）

- 实现：`stored.get(&id).map(|e| e.text.as_str())`。未调 set_text 的文档 → `Some("")`；docid 不存在 → `None`。`stored_text_roundtrip` 三路断言（`Some("机器学习检索")` / `Some("")` / `None`）通过。
- **与计划 Produces 契约不一致**：`00-text-persistence.md:86` 写「无原文或 docid 不存在返回 None」，但 Architecture 段（:126）与实现一致为 `Some("")`。报告已显式记录此偏离并选择遵循 Architecture。
- 影响下游：02-merge / 06-reindex 调用方需明确「无原文」是 `Some("")` 还是 `None`。当前实现更便于 reindex（始终拿 `&str` 喂分词器，空串 tokenize 为空）。

### 5. stored_json 语义不变 ✅

`stored_json`（`mod.rs:336-338`）返回 `e.meta_json.as_str()`，仍为 meta JSON。api search 回填 `Hit.fields` 路径未受影响。`stored_text_roundtrip` 断言 `r.stored_json(0)==Some("{}")` 通过。

### 6. api flush 接入 ✅

`api/collection.rs:216-219`：`add_doc` 之后新增 `writer.set_text(doc.text.as_deref().unwrap_or(""))?`，text=None 落空串。`?` 错误传播正确。其余 flush 逻辑（stored_json 构造、tokens tokenize、inv_builder.add_document、finalize、manifest 切换、段快照替换）均未改动。`flush_persists_empty_text_when_none` 验证 None 不报错且向量搜索仍命中。

### 7. corpus_compat 更新 ✅

`tests/corpus_compat.rs:282-325` 扩展 `corpus_segment_files_have_magic_version_headers`：
- 读 stored.bin 全字节。
- 校验 `buf.len() >= 12+8+4`（头 + 首条 docid + text_len）。
- `text_len = u32::LE(buf[20..24])`（偏移 12 头 + 8 docid = 20，正确）。
- 断言 `text_len > 0`、`text_len == corpus_docs()[0].text 字节数`、`buf[24..24+text_len] == expected_text`。
- `meta_len = u32::LE(buf[24+text_len..])`，断言 `meta_len > 0`。
- 顶部注释文档化 v1.1 起 stored.bin 含原文+meta 分离布局。

字节级与段级断言俱备，真实 roundtrip 验证（非 tautological）。

### 8. 不变量 I-1 ✅

stored.bin 仍在 `finalize`（`mod.rs:139-161`）一次性 `create`+`write_at`+`sync`。`set_text` 仅修改 `self.stored` Vec 内 `StoredEntry.text`，无任何写盘操作。段不可变保持。

### 9. 范围合规 ✅

- 仅实现原文持久化（set_text/text/stored.bin 布局），未触及 merge/reindex/HNSW/jieba。
- 无黑名单依赖引入（diff 仅触及 vane-core 内部）。
- `scripts/check-no-std-fs.sh` → OK。业务代码无 `std::fs`；`std::fs` 仅出现在 `vfs/std_fs.rs`（cfg 隔离例外）与测试清理代码。

### 10. 测试质量 ✅

5 个新测试 + 1 个扩展，均为真实断言：

| 测试 | 断言内容 | 评价 |
|---|---|---|
| `stored_text_roundtrip`（unit） | 中文原文 roundtrip + `Some("")` + `None` + `stored_json` 语义不变 | 字节级 + 语义双覆盖 |
| `set_text_before_add_doc_errors` | `VaneError::Schema` | 边界正确 |
| `flush_persists_text_readable_after_reopen` | reopen 后 text 搜索命中 d0 | 端到端数据流 |
| `flush_persists_empty_text_when_none` | None 不报错 + 向量搜索命中 | 边界 |
| `reindex_prerequisite_text_readable_for_retokenize` | text="机器学习" 搜索命中 | reindex 前置可用性（间接，api 不暴露 text()） |
| `corpus_segment_files_have_magic_version_headers`（扩展） | stored.bin 字节级 text_len/text_bytes/meta_len | 段级 + 字节级 |

reindex 前置测试按计划降级为「搜索命中证明原文进倒排」（api 不暴露 SegmentReader::text，02/06 经 CollectionInner 内部访问），报告已说明。06 实装真实 reindex 时可直接用 `SegmentReader::text`（已由 unit 字节级验证）。

## 门禁复跑

- `cargo test -p vane-core --test text_persistence --test corpus_compat` → 5/5 通过。
- `bash scripts/check-no-std-fs.sh` → OK。
- 报告自证 clippy/fmt/wasm32 check 全绿，未复跑（测试已编译通过，可信）。

## 需编排者裁决的疑点

### 疑点 A（轻微）：text() 返回 Some("") vs Produces 契约 None

计划 Produces 段（`00-text-persistence.md:86`）写「无原文或 docid 不存在返回 None」，实现与 Architecture 段为 `Some("")`（无原文）/ `None`（docid 不存在）。报告已显式记录并选择后者。

- 实现侧更便于 06 reindex（始终 `&str` 喂分词器）。
- 但 02-merge / 06-reindex 计划若按 Produces 段写「None」分支，会出现契约漂移。
- **建议**：编排者裁决以实现为准，回写计划 Produces 段为「无原文返回 Some("")，docid 不存在返回 None」，并确认 02/06 计划对此对齐。非阻塞。

### 疑点 B（轻微）：format_version 未递增

SPEC §6.2:214 规定「格式变更必须：① version 递增；② 提供迁移器或双模读取」。本次 stored.bin 字节布局相对 M0 实际产物（`docid(8)|len(4)|json`）已实质变更（插入 text_len+text_bytes），但 `format_version` 保持 1。

- 计划/报告的论证：SPEC §6.2:211 对 stored.bin 的描述始终是「原文/JSON meta」，M0 实现是占位不完整；仓库无历史发布产物，`corpus_compat` 已重新生成，无迁移负担。
- 严格按 §6.2:214 字面应 bump version；但 §13.3「旧版本写出的库必须被新版本打开」在「无旧版本发布」前提下 vacuously 成立。
- **建议**：编排者显式背书此「M0 stored.bin 视为未发布占位、补全 spec'd 字段不构成格式变更」的定性，避免后续模块引用此先例放宽 version 纪律。非阻塞。

## 阻塞项

无。

## Verdict

**APPROVED_WITH_MINOR**

实现正确、对称、覆盖完整，M0 冻结签名零破坏，I-1 保持，测试真实有效。两处轻微疑点（text() 返回值契约措辞、format_version 不 bump）均已在报告中显式记录并给出合理论证，需编排者一次性背书即可，不阻塞合并。
