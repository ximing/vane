# 06-reindex I-4 原子性 fix 复审

> 基线 BASE=b28b1f4，fix commit=578dfc4（`fix(reindex): atomic tokenizer/snapshot switch`）
> 日期：2026-08-09
> 范围：仅复审 I-4 原子性 fix（commit 578dfc4）；b28b1f4..HEAD 全量 diff 含 07-dict-distribution-node 改动，07 部分已有独立审查报告，不在本复审范围。

## 逐维度结论

### 1. 原子性闭环 — ✅

**证据** `crates/vane-core/src/api/collection.rs:1108-1166`

- `tokenizer.write()` / `tokenizer_id.write()` 的获取已移入 snapshot 写锁块（:1115-1116），紧随 `tombstones.write()` 之后。
- 赋值 `*tok_w = new_tokenizer; *tok_id_w = new_tokenizer_id;` 位于块末尾（:1164-1165），tombstone re-key 之后、块释放之前。
- 原块外两行赋值已删除（diff 确认 `- *self.inner.tokenizer.write()...` / `- *self.inner.tokenizer_id.write()...`）。
- 块释放在 `:1166`，此后才执行「删除旧段目录」（:1168-1172）和「state→Stable」（:1175）。

**结论**：snapshot 段列表切换与 tokenizer/tokenizer_id 更新在同一 `snapshot.write()` 持有期内完成。search 要么在 reindex 获取 `snapshot.write()` 前进入（旧段+旧分词器），要么在释放后进入（新段+新分词器）。不存在「snapshot 新段但 tokenizer 旧」窗口。

### 2. 无死锁 — ✅

**写侧锁顺序**（`collection.rs:1109-1116`）：
```
snapshot → offsets → inverted_readers → hnsw_readers
→ scalar_readers → tombstones → tokenizer → tokenizer_id
```

**读侧（search）锁顺序**（`collection.rs:647-752`）：
```
snapshot.read → seg_offsets.read → inverted_readers.read
→ hnsw_readers.read → scalar_readers.read → tombstones.read
→ [循环内 :752] tokenizer.read
```

两侧完全同序，无锁序反转。`tokenizer_id` 不被 search 读取（仅 flush/db.rs 幂等校验读，而 flush 在 Rebuilding 期被 E_BUSY 拒绝，不并发），放最末安全。

**其他写路径无环**：
- `flush`（:405-409）：Rebuilding 期 E_BUSY（:222），不与 reindex 并发；且 flush 在 :291/:340 获取 `tokenizer.read()`/`tokenizer_id.read()` 后**显式 drop / clone 释放**（:346 `drop(tok)`），再于 :405 获取 `snapshot.write()`——不构成 tokenizer→snapshot 嵌套持有，无锁序反转。
- `compact`/`merge`（:487-488）：`tokenizer.read()`/`tokenizer_id.read()` 仅 clone 后即释放，:544 才获取 `snapshot.write()`，同理无嵌套。
- `dict_state.write()`（:1175）：在 snapshot 块外、块释放后获取；search 不读 dict_state，独立无环。

**结论**：无死锁风险。

### 3. 回归测试质量 — ✅（含 minor 说明）

#### `reindex_tokenizer_switch_is_atomic_with_snapshot`（:231-263）

- 断言 `current_id != old_id`（身份推进）、所有段头 `tokenizer_id == current_id`（段头与 collection 一致）、`dict_state == Stable`。
- **非 tautological**：若代码存在「tokenizer_id 更新但段头用旧 id」或「段头新 id 但 collection 未更新」缺陷，此测试会失败。
- **minor**：与既有 `single_tokenizer_identity_throughout_reindex`（:199-221）核心断言重叠（均验证「全库段头 id == collection tokenizer_id」）。新增测试多了 `dict_state` 和 `!readers.is_empty()` 断言，且注释明确标注 I-4 原子性语义。作为回归标记可接受，非冗余问题。

#### `concurrent_search_during_reindex_no_panic`（:265-323）

- 两线程在 reindex 期间循环 `search("hello").unwrap()`，断言：所有 search 成功（不 panic）、`searches > 0`（未死锁）、最终段头 id 一致。
- **非 tautological**：`unwrap()` + `join().unwrap()` 真实断言无 panic / 无死锁。若新锁序（tokenizer.write 移入 snapshot 块）引发死锁，`join()` 会挂起或 panic。
- **atomicity 覆盖说明**：standard 分词器下 tokenization 不随 user_dict 变化，故即使存在混排窗口，search 结果也不变、不会 panic。此测试**证明无死锁/无 panic**，不直接证明无混排。无混排由代码结构（同锁块）+ 上一个测试的最终一致性共同保证。报告 §4 已明确承认此局限。M1 standard 场景可接受；jieba 场景的真正混排检测留待 07 集成测试。

**结论**：两测试均有真实断言、非 tautological。atomicity 由「代码结构正确性（维度 1/2）+ 最终一致性测试」共同闭环。

### 4. 既有 06 测试不回归 — ✅

- fix commit 仅移动 2 行赋值入锁块 + 新增 2 测试，未改动任何既有测试逻辑。
- 既有 11 单元测试（reindex_tests.rs）+ 5 集成测试（userdict_reindex.rs）的断言对象（最终状态：段头 id 一致、tombstone 保留、E_BUSY、ULID 替换）均不受「tokenizer 更新位置变更」影响——最终状态不变，仅到达路径变短。
- 锁块新增 2 把写锁（tokenizer/tokenizer_id），但既有测试均为单线程，无并发死锁风险。

### 5. 无新回归 — ✅

**fix commit（578dfc4）隔离 diff** 仅触及 2 文件：
- `collection.rs`：移动 2 行 + 注释（无新 cfg、无 std::fs、无新依赖）
- `reindex_tests.rs`：+2 测试（用 `std::thread`，测试态 host 构建，wasm32 不受影响）

**M0 签名零破坏**：`run_reindex` 签名不变；`reindex() -> Result<ReindexHandle>` 不变；无 pub API 变更。

**零 cfg / 禁 std::fs**：fix 区域（:1100-1180）无 `cfg(target)`、无 `std::fs`。

**门禁声明**（报告 §5，未独立验证——只读审查不运行 cargo）：
- `cargo test --workspace --all-features`：245 unit + 集成全绿（243+2=245 一致）
- clippy / wasm32 / fmt / no-std-fs：声明全过

**范围澄清**：`git diff b28b1f4..HEAD -- crates/vane-core/src/api/` 含 07-dict-distribution-node 改动（jieba_dict 字段、build_collection_tokenizer、load_default_jieba_dict 等），这些来自 11b477c..b82fcb4 系列 07 提交，**非本 fix 引入**，已有独立 07 审查报告。fix commit 578dfc4 本身干净，未碰 07 范围。

## 阻塞/裁决项

无。

## 结论

- **verdict：APPROVED**
- **闭环**：是。I-4 混排窗口已闭环——tokenizer/tokenizer_id 与 snapshot 段列表在同一写锁块内原子切换，锁顺序与 search 读侧一致无死锁，回归测试覆盖最终一致性 + 并发无 panic/无死锁。
- **未闭环项**：无。minor 提示（非阻塞）：
  1. `reindex_tokenizer_switch_is_atomic_with_snapshot` 与 `single_tokenizer_identity_throughout_reindex` 断言重叠，可合并但不影响正确性。
  2. 并发测试用 standard 分词器无法触发真实混排效果（报告已承认）；jieba 场景的混排检测留待 07 集成测试。
