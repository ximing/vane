# 02-merge-fix 报告：partial auto-merge target_docid_base 碰撞修复

> 修复：02-review B-2（partial auto-merge `target_docid_base=0` 硬编码 → docid 空间重叠）
> 编排者裁决：Option A（partial merge `target_docid_base = max(保留段 base+count)`）

## 缺陷复现

`crates/vane-core/src/api/collection.rs` 的 `merge_segments` 原硬编码 `target_docid_base=0`
（collection.rs:393）。`auto_merge_two_smallest` 合并 2/N 段时，若保留段含 base=0 段
（首段 doc_count 大不入选"最小两段"），新段 [0, new_count) 与保留段 [0, old_count)
**docid 空间重叠** → search 回填（collection.rs:706-730）按 `sd.docid - base` 遍历段、
首个命中即 break，误命中保留段；fusion rrf 去重按 docid 折叠，丢失新段文档。

既有测试 `flush_auto_merges_when_exceeding_segment_max` 因 11 等量段 stable sort 恰好
选中 base=0 段而未触发。

## 修复改动

文件：`crates/vane-core/src/api/collection.rs`，`merge_segments` 函数。

1. **target_docid_base 选择**（collection.rs:390-413）：
   - 读快照 `snap`，判断 `is_full_merge = snap.iter().all(|r| source_ulids.contains(&r.meta().ulid))`。
   - **compact 全合并**（source 覆盖全部段，无保留段）：`target_docid_base = 0`（不变）。
   - **partial auto-merge**（合并 2/N 段）：`target_docid_base = max(保留段 base + count)`，
     新段 docid 从所有保留段最大 docid 之后开始，避免与任何保留段重叠。

2. **next_docid 推进**（collection.rs:416-424）：
   - partial merge 后 `next_docid = max(next_docid, target_docid_base + new_meta.doc_count)`，
     避免后续 flush 分配的 docid 与新段 [target_docid_base, +new_count) 重叠。
   - 02-review 维度 8a 的"不要动 next_docid"仅适用 compact 全合并（base=0、stale-high 无害）；
     partial merge 必须推进，否则引入新的 future-flush 碰撞（新段占据 [max_retained_end,
     +new_count)，而 next_docid == max_retained_end 时下次 flush 会落到同区间）。
   - compact 全合并不调整 next_docid（保持 stale-high）。

`MergeTask::new` / `finalize_merge` / `MergeTask` 签名零变更；M0 冻结签名未触碰。

## 回归测试

文件：`crates/vane-core/tests/tombstone_merge.rs`，新增
`partial_auto_merge_does_not_overlap_docid_with_retained_segments`。

- 构造大段（base=0, 5 docs d0..d4）+ 10 个小段（各 1 doc s0..s9）。
- 第 11 次 flush 触发 auto_merge，选最小两段（s0、s1）合并，保留 base=0 大段。
- 缺陷下：新段 base=0 → [0,2) 与大段 [0,5) 重叠 → search 回填把新段 docid 0,1
  误命中大段 d0,d1 → unique ids 仅 13（s0、s1 丢失）。**测试先失败**（13 != 15）。
- 修复后：新段 base = max(保留段 base+count) = 15 → [15,17) 无重叠 → 15 条全唯一。**通过**。

## 自证门禁

```
cargo test --workspace --all-features       # 234 passed, 1 ignored, 0 failed
cargo clippy --workspace --all-targets --all-features -- -D warnings   # 无警告
cargo check --target wasm32-unknown-unknown -p vane-core                # 通过
cargo fmt --all -- --check                                             # 无差异
bash scripts/check-no-std-fs.sh                                        # OK
```

重点测试：`tombstone_merge`（9 passed，含新回归）、`pre_filter`、`filter`、`merge::tests`。

## 提交

- hash: `72bb641`
- message: `fix(merge): partial auto-merge target_docid_base avoids docid overlap with retained segments`

## 范围合规

- 仅修 partial-merge base 碰撞 + 回归测试；未碰 03/04/06 范围。
- core 禁 std::fs（未引入）；零 cfg；无新依赖。
- compact 全合并 base=0 不变。
- `next_docid` 推进仅 partial merge 路径，compact 不调整。
