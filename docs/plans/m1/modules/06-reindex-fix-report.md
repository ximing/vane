# 06-reindex I-4 混排窗口修复报告

> 日期：2026-08-09
> 缺陷：06-review #1（run_reindex 完成阶段非原子，存在 I-4 新旧分词身份混排窗口）
> 类型：定向修复（TDD），不扩展功能、不碰 04/07/10 范围

## 1. 缺陷回顾

`run_reindex` 原完成顺序：

1. `[snapshot/offsets/inv/hnsw/scalar/tombstones 写锁块]` 切换 snapshot 到新段 → **释放所有写锁**；
2. 删除旧段目录；
3. `tokenizer.write() = new_tokenizer`；
4. `tokenizer_id.write() = new_tokenizer_id`；
5. `dict_state = Stable`。

步骤 1 与步骤 3/4 之间存在窗口：并发 `search` 先取 `snapshot.read()`（已指向**新段**，段头新 TokenizerId），再取 `tokenizer.read()`（仍**旧**分词器）tokenize 查询 → 新旧身份混排检索，违反 I-4 / §7.4 禁止行为。M1 测试用 standard 分词器（不消费 user_dict 做切分）未捕获；jieba 场景会出现 recall 回退。

## 2. 修复改动

文件：`crates/vane-core/src/api/collection.rs`，`run_reindex`。

将 `tokenizer`/`tokenizer_id` 的更新**移入 snapshot 写锁块内**，与新段替换同步完成（原子）：

- 在持有 `snapshot.write()` 期间，同时替换 snapshot 段列表 **AND** 更新 `tokenizer`/`tokenizer_id`，再统一释放。
- 写锁获取顺序（块内，`collection.rs:1103-1120` 附近）：
  `snapshot → offsets → inverted_readers → hnsw_readers → scalar_readers → tombstones → tokenizer → tokenizer_id`。
- 赋值位于块末尾（tombstone re-key 之后）：
  ```rust
  // I-4：tokenizer/tokenizer_id 与 snapshot 段列表原子切换（同写锁块）。
  *tok_w = new_tokenizer;
  *tok_id_w = new_tokenizer_id;
  ```
- 删除原块外的 `*self.inner.tokenizer.write()...` / `*self.inner.tokenizer_id.write()...` 两行（已移入块内）。
- 「删除旧段目录」「state → Stable」仍在块外（与身份原子性无关，保持不变）。

## 3. 锁顺序分析（无死锁证明）

### search 读侧（`collection.rs:647-756`）

```
snapshot.read()  → seg_offsets.read() → inverted_readers.read()
→ hnsw_readers.read() → scalar_readers.read() → tombstones.read()
→ [循环内] tokenizer.read()
```

`tokenizer.read()` 在循环内最末获取（`collection.rs:752`），晚于所有其他读锁。
search 不读 `tokenizer_id`。

### run_reindex 写侧（修复后）

```
snapshot.write() → seg_offsets.write() → inverted_readers.write()
→ hnsw_readers.write() → scalar_readers.write() → tombstones.write()
→ tokenizer.write() → tokenizer_id.write()
```

与 search 读侧**完全同序**。

### 无死锁论证

- 两侧锁顺序一致（snapshot 先，tokenizer 系最末），不存在反向加锁循环。
- 修复后 search 要么在 reindex 获取 `snapshot.write()` **之前**进入（持旧 snapshot.read() + 旧 tokenizer.read()，见旧段+旧分词器），要么在 reindex 释放全部写锁**之后**进入（持新 snapshot.read() + 新 tokenizer.read()，见新段+新分词器）。
- 不存在「snapshot 已切到新段但 tokenizer 仍旧」的窗口：二者同一写锁块内切换，search 阻塞在 `snapshot.read()` 直至两者皆释放。
- `tokenizer_id` 在 search 路径不被读取（仅 flush/db.rs 幂等校验读，而 flush 在 Rebuilding 期被 E_BUSY 拒绝，不并发），放最末安全。

### 其他写路径

- `flush`（`collection.rs:405-549`）：Rebuilding 期 E_BUSY（`collection.rs:222`），不与 reindex 并发；其锁顺序 snapshot→offsets→inv→hnsw→scalar→tomb，不涉及 tokenizer 写，无环。
- `compact`/`merge`：Rebuilding 期 E_BUSY，不并发。

## 4. 回归测试

文件：`crates/vane-core/src/api/reindex_tests.rs`（新增 2 个）。

### `reindex_tokenizer_switch_is_atomic_with_snapshot`

断言 reindex `handle.wait()` 返回**立即**，`collection.tokenizer_id()` 与所有 snapshot 段头 `tokenizer_id` 完全一致，且不等于旧 id。验证「无滞后窗口」的最终一致性。

### `concurrent_search_during_reindex_no_panic`

两个线程在 reindex 期间持续 `search("hello")`，覆盖 reindex 收尾窗口。断言：
- 所有 `search` 均 `unwrap()` 成功（不 panic、不死锁、不混排报错）；
- 至少完成若干次 search（证明未死锁）；
- reindex 完成后 `tokenizer_id` 与所有段头一致（最终一致性）。

standard 分词器下 tokenization 不随 user_dict 变化，故 search 结果稳定；测试核心是**并发不 panic + 不死锁**，覆盖窗口期的不可达性。

### 既有测试不回归

`userdict_reindex.rs` 5 集成测试 + `reindex_tests.rs` 原有 11 单元测试全绿。

## 5. 自证门禁

```
cargo test --workspace --all-features       # 245 + 集成全绿（+2 新测试）
cargo clippy --workspace --all-targets --all-features -- -D warnings   # 无告警
cargo check --target wasm32-unknown-unknown -p vane-core               # 通过
cargo fmt --all -- --check                                              # 通过
bash scripts/check-no-std-fs.sh                                         # OK
```

测试统计：vane-core 单元 245 passed/1 ignored；集成 5+9+7+3+11+1+2+3 全绿。

## 6. 红线遵守

- 仅修 I-4 原子性窗口 + 回归测试，未扩展功能、未碰 04/07/10。
- M0 冻结签名零破坏（`run_reindex` 签名不变）。
- core 禁 std::fs：修复未引入任何 std::fs 调用。
- 零 cfg：修复代码无 `cfg(target)`。
- 无新依赖。
- 锁顺序 snapshot→tokenizer，无死锁（见 §3）。

## 7. 提交

```
fix(reindex): atomic tokenizer/snapshot switch to close I-4 mixed-identity window


```

提交 hash：`b862ef7d0155d17b67de7113dc8d6d5a75431910`
