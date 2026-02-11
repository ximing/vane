# M4 阶段二 b：crash_recovery 审查报告

> 审查者：task reviewer SubAgent（opus，只读）
> 审查对象：`crates/vane-core/tests/crash_recovery.rs`（+591 行，commit c7e3cdf）
> 审查范围：spec 合规 + 代码质量（重点：断言正确性 / vacuous assert 检测）
> 日期：2026-08-11

## A. Spec 合规：✅

| 检查项 | 结果 | 说明 |
|---|---|---|
| 5 场景覆盖 | ✅ | meta_slot 翻转 / WAL flush / merge 中断 / ENOSPC / 部分写，全 5 场景对应 M4-PLAN 阶段二 + §3.1 注入点映射表 |
| FaultVfs 规则匹配 §3.1 映射表 | ✅ | 每 场景的 (op, path_pattern, one_shot, trigger_on_nth) 对应映射表行（见下逐一） |
| `#![cfg(feature="fault-injection")]` 门控 | ✅ | 默认 features 编译为空、0 测试不报错（report §3.5 验证） |
| 测试安全铁律 | ✅ | 全用 `FaultVfs::wrap_memory()`（MemoryVfs），无 tempdir/StdFsVfs，无真破坏宿主机 |

### 逐场景 FaultVfs 规则与 §3.1 映射表对照

| 场景 | 映射表行 | 实际 FaultVfs 规则 | 匹配 |
|---|---|---|---|
| 1 meta_slot | "manifest 翻转前（sync tmp 失败）" → `IoError{op:Sync, path:"*.json.tmp"}` | `IoError{op:Sync, path:"*/manifest.json.tmp", one_shot:true, trigger_on_nth:0}` | ✅ |
| 2 WAL flush | "WAL flush 前（append 失败）" → `IoError{op:Append, path:"*/wal.log"}` | `IoError{op:Append, path:"*/wal.log", one_shot:true, trigger_on_nth:0}` | ✅ |
| 3 merge 中断 | "merge persist 前（write_inverted 失败）" → `IoError{op:WriteAt, path:"*/segments/seg_*/inverted.bin"}` | `IoError{op:WriteAt, path:"*/segments/seg_*/inverted.bin", one_shot:true, trigger_on_nth:0}` | ✅ |
| 4 ENOSPC | "ENOSPC（磁盘满）" → `Enospc{op:WriteAt, path:"*"}` | `Enospc{op:WriteAt, path:"*", one_shot:true, trigger_on_nth:0}` | ✅ |
| 5 部分写 | "部分写" → `PartialWrite{op:WriteAt, path:"*/header.bin", bytes_before_fail:8}` | `PartialWrite{op:WriteAt, path:"*/header.bin", bytes_before_fail:8, one_shot:true, trigger_on_nth:0}` | ✅ |

## B. 代码质量——断言正确性

### 逐场景断言验证（与实现源码交叉核对）

**场景 1（meta_slot 翻转）**——断言正确、非 vacuous：
- `flush_err.is_err()` + `contains("manifest.json.tmp")`：✅ FaultVfs 的 `VaneError::Io("simulated sync failure on manifest.json.tmp")` 经 `save_atomic` → `add_segment` → `flush` 的 `?` 传播（无 wrapping），`VaneError::Io` Display = `"E_IO: {msg}"`（types.rs:90），含 "manifest.json.tmp"。非 vacuous——验证了故障注入 + 错误传播链。
- `segment_count == 1`（崩溃前 + 重开后）：✅ `flush()` 的 snapshot 更新在 `manifest_store.add_segment` 之后（collection.rs:440-451），save_atomic 失败 → `?` 传播 → snapshot 未更新 → segment_count 仍 1。验证 "manifest 未切换"。
- d0-d2 可见 / d3-d5 不可见（重开后）：✅ WAL 有 `AddSegment(B)`（flush 的 wal.append 在 save_atomic 前成功，collection.rs:422-426），manifest 不含 B → recover `AddSegment` 分支清理孤儿 B（wal/mod.rs:175-181）。旧 manifest 完好 → 旧段数据可见。验证 "旧完好 + 孤儿清理"。

**场景 2（WAL flush）**——断言正确、非 vacuous，**关键路径已源码验证**：
- `del_err.is_err()`（delete d1 失败）：✅
- `contains_id(&hits, "d1")`（d1 内存仍可见——位图未更新）：✅ **已源码验证** `delete`（collection.rs:1019-1071）顺序为：WAL append（line 1052-1058，`?` 传播）→ 内存位图更新（line 1060-1069）。WAL append 失败 → `?` 提前返回 → 位图更新代码不执行 → d1 未 tombstone → search 可见。断言与实现完全一致。
- 重开后 d0 不可见（已确认事务重放）/ d1 可见（未确认不重放）/ `hits.len() == 4`：✅ recover（wal/mod.rs:144-194）读 WAL 仅含 `AddTombstone(d0)`（d1 的 append 失败 → 未入 WAL），ULID 仍在 manifest → 聚合到 tombstone map → 注入 CollectionInner → d0 被删除。d1 无 WAL 记录 → 不重放 → 可见。4 个活文档（d1-d4）。

**场景 3（merge 中断）**——断言正确、非 vacuous，覆盖最完整：
- `compact_err.is_err()` + `contains("inverted.bin")`：✅ `write_inverted`（bm25.rs:289）用 `vfs.write_at(&path, &buf, 0)?`，FaultVfs `IoError{op:WriteAt, path:"*/segments/seg_*/inverted.bin"}` 命中 → `VaneError::Io("simulated write failure on inverted.bin during merge")` 经 `write_inverted` → `finalize_merge` → `merge_segments` → `compact` 的 `?` 传播。Display 含 "inverted.bin"。
- `segment_count == 2`（崩溃前 + 重开后）：✅ `merge_segments` 的 snapshot 更新在 `save_atomic` 之后（collection.rs:594-633），`finalize_merge` 失败 → `?` 在 line 539 传播 → save_atomic / WAL append / snapshot 更新均未执行。旧段在 manifest 中保留。recover 目录扫描清理孤儿新段。
- d0 不可见（tombstone WAL 重放）/ d1-d5 可见（5 活文档）：✅ WAL 仅含 `AddTombstone(d0)`（delete 的 append 成功）；failed compact 未追加任何 WAL 记录（merge_segments 的 wal.append 在 finalize_merge 之后，未执行）。recover 重放 d0 tombstone。
- compact 可重试成功 + `segment_count == 1` + 5 活文档不变：✅ one_shot 故障已消费 → 重试无故障 → finalize_merge 成功 → WAL append + save_atomic + snapshot 更新 + WAL truncate（run_compact line 1119-1120）。d0 物理清除（merged 段无 tombstone），d1-d5 在 merged 段。

**场景 4（ENOSPC）**——断言正确、非 vacuous：
- `flush_err.is_err()` + `contains("ENOSPC")`：✅ FaultVfs `Enospc` 返 `VaneError::Io("ENOSPC: write_at ... (simulated, no bytes written)")`，Display 含 "ENOSPC"。
- d0/d2 可见（已有数据不损）/ d3 不可见：✅ 第二批 flush 的 `SegmentWriter::finalize` 首 `write_at` 命中 ENOSPC → flush 失败 → snapshot 未更新 → 基线段完好。
- 重开后 1 段 + 3 文档 + `seg_dirs.len() == 1`（孤儿清理）：✅ manifest 未切换（save_atomic 未到达）→ 基线 1 段。孤儿段目录（finalize 的 create 可能已建空目录）被 recover `cleanup_orphan_segment_dirs` 清理。

**场景 5（部分写）**——断言实质性、非 vacuous，但**有 spec 覆盖缺口**（见 Important-1）：
- `flush_err.is_err()` + `contains("partial write")`：✅ FaultVfs `PartialWrite` 写 8 字节后返 `VaneError::Io("partial write at ... (8 bytes written before failure)")`，Display 含 "partial write"。
- `segment_count == 1`：✅ finalize 失败 → snapshot 未更新。
- `n == 8` + `buf[..4] == b"VANE"` + `ver == 1`：✅ 直接 Vfs 读 header.bin 验证部分写字节内容。实质性断言。
- 重开后 1 段 + 3 文档 + `seg_dirs.len() == 1`（孤儿清理）：✅ 孤儿段目录被 recover 清理。
- **缺口**：spec §3.1 映射表明确要求验证 "decode_header 校验失败 → Corrupt"，但测试未直接调 `SegmentReader::open` / `decode_header` 验证拒绝。因 `decode_header` latent bug（见 Important-2），直接测会 panic 而非返 Corrupt，implementer 改用间接验证。

## C. Findings

### Critical

无。所有 5 场景的断言均实质性、非 vacuous、非 trivially-true。每场景验证了文档数 / external_id 回填（`contains_id` 检 `Hit.id == "d{N}"`）/ search 结果集 / 段集合（`segment_count` / `seg_dirs.len()`）。未发现 `assert!(true)` / `assert!(result.is_ok())` 但不检内容 / 断言与场景目标无关等 vacuous 模式。

### Important

**I-1. 场景 5 缺失 spec 表 "decode_header 校验失败 → Corrupt" 断言**
- 位置：`crates/vane-core/tests/crash_recovery.rs:549-575`（场景 5）
- 缺陷：spec §3.1 注入点映射表"部分写"行的验证列明确为 "header.bin 写 8 字节（magic+version）后失败 → **decode_header 校验失败 → Corrupt**"。测试仅用 Vfs 直读验证 8 字节内容 + recover 孤儿清理间接验证，**从未调 `SegmentReader::open` / `decode_header` 验证损坏段被拒绝**。
- 失败场景：生产中段 header.bin 被截断为 8 字节（magic+version，缺 ulid_len 及后续）→ `SegmentReader::open`（segment/mod.rs:374 调 `decode_header`）在 `buf[8]` 处 **panic**（非 `VaneError::Corrupt`）→ 崩溃恢复路径本应优雅降级却 panic → 假绿（测试通过但生产会 panic）。
- 根因：implementer 因 `decode_header` latent bug（I-2）避开直接测 `SegmentReader::open`，改用间接验证。间接验证覆盖 "recover 孤儿清理" 但不覆盖 "损坏段被拒绝"。recover 的 `cleanup_orphan_segment_dirs`（wal/mod.rs:201-225）仅按 manifest 成员关系清理，**不尝试 open 段**——故 decode_header 拒绝路径完全未被任何场景 exercised。

**I-2. `decode_header` off-by-one：`buf.len()==8` 过 `< 8` 门但 `buf[8]` panic**
- 位置：`crates/vane-core/src/segment/header.rs:39,52-53`
- 缺陷：line 39 `if buf.len() < 8` 门，line 52 `let mut pos = 8;`，line 53 `let ulid_len = buf[pos]`（即 `buf[8]`）。`buf.len()==8` 过 `< 8` 门但 `buf[8]` 越界 panic（非 `VaneError::Corrupt`）。应 `< 9`。
- 失败场景：任何路径产出的恰好 8 字节 header.bin（如场景 5 的 PartialWrite{bytes_before_fail:8}，或真实磁盘只写了 8 字节就崩溃）→ `SegmentReader::open` → `decode_header` → `buf[8]` panic（index-out-of-bounds）→ **崩溃恢复本应优雅返 Corrupt 却 panic**。生产数据安全范畴的最差失败模式。
- 预存 latent bug，被场景 5 撞出。implementer 自报确认（report §5.1）。

### Minor

**M-1. 场景 1 I16 tmp 清理未断言**
- 位置：`crates/vane-core/tests/crash_recovery.rs:209-210`
- 缺陷：§3.1 映射表 "manifest 翻转前" 行验证列含 "tmp 残留下次清理（I16）"。测试仅注释 "验证 manifest.json.tmp 不存在...或即使存在也不影响数据一致性"，无实际断言。
- 影响：低。I16 是防御性清理（save_atomic 的 `delete(tmp)` 处理残留），非场景 1 核心验证点。核心验证（manifest 未切换 + 旧数据完好）已满足。但 spec 表提及的验证项应显式断言或注明不覆盖。

**M-2. 场景 5 注释掩盖 panic**
- 位置：`crates/vane-core/tests/crash_recovery.rs:572-574`
- 缺陷：注释 "8 字节恰好过长度门但缺 ulid_len → 无效段" 未说明 `buf[8]` 访问会 **panic**（非返 Corrupt）。对未来维护者误导——暗示 decode_header 会优雅拒绝，实际会 panic。
- 影响：低。文档/注释问题，不影响测试正确性。修 I-2 后此注释应同步更新。

## D. 场景 5 间接验证定性：Important

implementer 用 Vfs 直读 + recover 孤儿清理间接覆盖场景 5。

**间接覆盖的属性**：
- ✅ 部分写产出的 8 字节内容正确（magic + version）
- ✅ recover 孤儿清理路径正确（段目录扫描 + manifest 成员关系过滤）
- ✅ 旧数据不丢（基线段完好）

**未覆盖的属性**（spec 表明确要求）：
- ✗ `decode_header` / `SegmentReader::open` 对 8 字节损坏 header 返 `VaneError::Corrupt`

**定性理由**：
1. spec §3.1 映射表 "部分写" 行的验证列**明确**写 "decode_header 校验失败 → Corrupt"——这是 spec 合同的一部分，不是可选验证。
2. 间接验证覆盖的是 "recover 清理孤儿"（manifest 成员关系驱动，不 open 段）和 "部分写字节正确"——两者都不是 "decode_header 拒绝损坏段"。
3. `decode_header` 的拒绝路径（magic / version / truncation guard）对生产数据安全至关重要——崩溃恢复系统必须优雅拒绝损坏段而非 panic。
4. 当前测试通过但生产中同样的 8 字节 header 会 panic——这是 "假绿" 的一种形式（测试覆盖了 recover 清理，但未覆盖用户真正会撞到的 `SegmentReader::open` 拒绝路径）。
5. 非 Critical（不进 fix 循环则阻塞合并）是因为现有断言本身是实质性的（非 vacuous），且 recover 清理路径确实被验证了——缺口是 "spec 表要求的特定验证缺失" + "latent bug 未暴露"。

**修复后应加的断言**（修 I-2 后）：
```rust
// 损坏 header 被 SegmentReader::open 拒绝（非 panic）
let open_err = SegmentReader::open(&vfs, &orphan_seg_dir);
assert!(matches!(open_err, Err(VaneError::Corrupt(_))), "corrupt 8-byte header must be rejected, got: {:?}", open_err);
```
可选：加 `decode_header` 8 字节单元测试（返 Corrupt 非 panic）。

## E. decode_header bug 处置建议：M4 2b fix 循环

**建议**：在 M4 阶段二 b 的 fix 循环中修（非独立 fix 任务 / 非 defer）。

**理由**：
1. **M4 核心范畴**：M4 = 生产门槛 + 数据安全。`decode_header` panic 是崩溃恢复系统的最差失败模式——本应优雅返 Corrupt 却 panic。与 M4 目标直接矛盾。
2. **bug 被本任务暴露**：场景 5 的 PartialWrite{bytes_before_fail:8} 恰好撞出此 bug。自然在本任务的 fix 循环中修。
3. **修复 trivial**：1 行（`< 8` → `< 9`），低风险。
4. **修复后解锁 spec 合规**：修 I-2 后，场景 5 可加直接 `SegmentReader::open` Corrupt 断言（I-1），完成 spec 表要求的验证。
5. **非 defer**：若 defer 到 M4 后续，场景 5 的 spec 覆盖缺口持续存在，且生产 panic bug 持续潜伏——M4 的数据安全门槛未真正达成。

**fix 循环应包含**：
1. `segment/header.rs:39`：`< 8` → `< 9`（或更健壮：在 `buf[8]` 前加 bounds check）
2. `tests/crash_recovery.rs` 场景 5：加 `SegmentReader::open` 返 `VaneError::Corrupt` 的直接断言
3. 可选：`segment/header.rs` 单元测试加 `buf.len()==8` case（返 Corrupt 非 panic）
4. 更新场景 5 注释（M-2）说明 decode_header 返 Corrupt

**非独立 fix 任务的理由**：bug fix + test enhancement 紧耦合于 crash_recovery 任务目标，2b fix 循环是 M4 标准流程，开销最小。虽然 fix 跨越 `segment/header.rs`（非 crash_recovery 任务原始范围），但 bug 是本任务测试暴露的，orchestrator 可授权在 2b fix 循环中一并修。

## F. ⚠️ 无法从 diff 验证项

| 项 | 说明 |
|---|---|
| 测试执行结果 | 未重跑门禁（按指令）。report 声明 5 passed；逻辑经源码交叉验证一致，但执行层面无法从 diff 确认。 |
| `decode_header` 实际 panic 行为 | 验证了代码逻辑（`buf.len()==8` 过 `< 8` 门 + `buf[8]` 越界），但未执行。Rust 的 `[u8]` 索引越界是确定的 panic（非 UB），逻辑确定性高。 |
| `write_inverted` 用 `write_at` | 已读源码（bm25.rs:289 `vfs.write_at(&path, &buf, 0)?`）确认——非 diff 推断。 |
| `SegmentReader::open` 调 `decode_header` | 已读源码（segment/mod.rs:374 `header::decode_header(&hbuf)?`）确认——非 diff 推断。 |
| `delete` 顺序（WAL→bitmap） | 已读源码（collection.rs:1052-1069）确认——WAL append 在前（`?` 传播），bitmap 更新在后。非 diff 推断。 |
| `VaneError::Io` Display 含原始 msg | 已读源码（types.rs:90 `Self::Io(m) => write!(f, "E_IO: {}", m)`）确认。 |
| `flush` 的 save_atomic 在 wal.append 之后 | 已读源码（collection.rs:422-430）确认——wal.append(AddSegment) 在 manifest_store.add_segment (→ save_atomic) 之前。 |

## G. 总体

**进 fix 循环**：是（2 条 Important findings）。

- **I-1**（场景 5 spec 覆盖缺口）+ **I-2**（decode_header panic bug）是紧耦合对：修 I-2 解锁 I-1 的直接断言。同 2b fix 循环一次修完。
- 修完后场景 5 达到 spec 合规（"decode_header 校验失败 → Corrupt" 被直接验证），且消除一个生产数据安全 panic。
- 无 Critical（无 vacuous assert），无阻塞合并的架构/安全/正确性缺陷。

**Spec 合规**：✅（5 场景 + FaultVfs 规则 + 门控 + 测试安全全满足；场景 5 的 spec 验证缺口 I-1 是 Important 而非 Critical，fix 后完全合规）
