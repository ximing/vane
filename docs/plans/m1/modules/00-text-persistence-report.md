# 00-text-persistence 实装报告

> 模块：00-text-persistence（原文持久化，L0 前置）
> 计划：`docs/plans/m1/modules/00-text-persistence.md`
> 基线 commit：24b446a → 实装完成 52089c1

## Task 1：stored.bin 原文 roundtrip

**改动文件**：
- `crates/vane-core/src/segment/mod.rs`
  - 新增 `StoredEntry { docid, text: Option<String>, meta_json }`（写期单条记录）。
  - `SegmentWriter.stored` 由 `Vec<(u64, String)>` 改为 `Vec<StoredEntry>`。
  - `add_doc` push `StoredEntry { docid, text: None, meta_json }`（签名不变）。
  - 新增 `SegmentWriter::set_text(&mut self, text: &str) -> Result<()>`：取 `stored.last_mut()` 设 text，空则 `Err(Schema)`。
  - `finalize` 写 stored.bin 新布局：`docid(8 LE) | text_len(4 LE) | text_bytes | meta_json_len(4 LE) | meta_json_bytes`。
  - 新增 `StoredReadEntry { text, meta_json }`（读期单条记录）；`SegmentReader.stored` 改为 `HashMap<u64, StoredReadEntry>`。
  - `load_stored` 改调 `decode_stored`（新增解码函数，逐字段校验截断/UTF-8）。
  - 新增 `SegmentReader::text(local_docid) -> Option<&str>`；`stored_json` 语义不变（返回 `e.meta_json`）。
  - `decode_kv_map` 保留服务 idmap.bin（未改）。
- `crates/vane-core/src/segment/tests.rs`
  - 新增 `stored_text_roundtrip`（含中文原文 roundtrip + 未调 set_text 返回空串 + docid 不存在返回 None + meta JSON 语义不变）。
  - 新增 `set_text_before_add_doc_errors`（边界：add_doc 前调 set_text 报 Schema）。

**偏离与裁决**：
- 计划 Task 1 测试写 `TokenizerId::from_bytes([0u8; 32])`，M0 无此方法。按复审 minor 改用 `TokenizerId([0u8; 32])`（M0 公开元组字段）。
- 计划描述「text 为 None 时返回 None」，但 finalize 写入时 Option→空串，读期 text 始终是 `Some(String)`。实现遵循计划 Architecture 段落：`text()` 返回 `stored.get(&id).map(|e| e.text.as_str())`，故未调 set_text 的文档返回 `Some("")`，docid 不存在返回 `None`。测试断言与之一致。

**commit**：91c8d7d

## Task 2：api flush 接入 set_text

**改动文件**：
- `crates/vane-core/src/api/collection.rs`（flush 循环，约 215-220 行）：`add_doc` 之后新增 `writer.set_text(doc.text.as_deref().unwrap_or(""))?`。不改 `add_doc` 调用方式。
- `crates/vane-core/tests/text_persistence.rs`（新建）：
  - `flush_persists_text_readable_after_reopen`：reopen 后搜索 text 命中验证原文数据流完整。
  - `flush_persists_empty_text_when_none`：text=None 落空串不报错，向量搜索仍命中。

**commit**：b693391

## Task 3：corpus 兼容测试更新

**改动文件**：
- `crates/vane-core/tests/corpus_compat.rs`：
  - 顶部注释文档化 stored.bin v1.1 起含原文+meta 分离布局（format_version 保持 1，无迁移）。
  - `corpus_segment_files_have_magic_version_headers` 扩展：读 stored.bin 字节，校验首条记录 `text_len > 0` 且 `text_bytes == corpus_docs()[0].text` UTF-8 字节，`meta_json_len > 0`。

**偏离与裁决**：
- 计划提到「corpus_compat 增加 stored.bin 含原文的段级断言需访问 SegmentReader」，裁决为直接读 stored.bin 字节校验（不依赖 api 暴露 SegmentReader::text），与计划最小实现一致。

**commit**：5ef9f9c

## Task 4：reindex 前置可用性测试

**改动文件**：
- `crates/vane-core/tests/text_persistence.rs`：
  - `reindex_prerequisite_text_readable_for_retokenize`：flush 后搜索 text="机器学习" 命中，证明原文进了倒排数据流完整。不实装 reindex（06 负责），仅验证前置管线不缺料。

**commit**：955ff77

## 样式修正

- `cargo fmt` 应用于 segment/mod.rs、segment/tests.rs、corpus_compat.rs（行宽换行）。
- **commit**：52089c1

## 自证门禁结果

| 门禁 | 结果 |
|---|---|
| `cargo test --workspace --all-features` | PASS（187 lib + 2 corpus + 1 recall + 3 text_persistence + 其余 crate 全绿） |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | PASS（零告警） |
| `cargo check --target wasm32-unknown-unknown -p vane-core` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `bash scripts/check-no-std-fs.sh` | OK |
| `bash crates/vane-node/scripts/check-thin.sh` | OK |
| `cargo test --test corpus_compat -p vane-core` | PASS（2/2） |
| `cargo bench --no-run -p vane-core` | PASS（编译通过） |

## 提交 hash

1. 91c8d7d — segment: persist original text in stored.bin (SPEC §6.2)
2. b693391 — api: persist doc.text into stored.bin on flush
3. 5ef9f9c — test: update corpus_compat for stored.bin text+meta layout
4. 955ff77 — test: assert reindex prerequisite (original text persisted and indexable)
5. 52089c1 — style: cargo fmt 应用于 00-text-persistence 改动

## 红线核查

- M0 冻结 pub API 不破坏：`SegmentWriter::new/add_doc/finalize`、`SegmentReader::open/stored_json/external_id/vectors/dim/meta/segment_dir/vfs` 签名不变。仅新增 `SegmentWriter::set_text` + `SegmentReader::text`。✅
- core 禁 std::fs：业务代码无 std::fs，stored.bin 经 Vfs。✅
- 段不可变（I-1）：stored.bin 仍在 finalize 一次性写入，set_text 仅修改写期 buffer。✅
- format_version 保持 1（补全 spec'd 格式，无发布数据故无迁移）。✅
- 不引入黑名单依赖。✅
- MoSCoM 边界：仅做 00 范围（原文持久化），未实现 merge/reindex/HNSW。✅

## 遗留/疑问

- 无遗留阻塞。Task 4 按计划降级为「搜索命中证明原文数据流完整」（api 不暴露 SegmentReader::text，02/06 经 CollectionInner 内部访问）；06 实装真实 reindex 时可直接用 `SegmentReader::text`（已由 Task 1 字节级验证）。
