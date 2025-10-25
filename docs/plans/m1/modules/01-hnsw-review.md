# 01-hnsw 模块代码审查

> 审查对象：BASE=19c03d1 → HEAD=9c372f7（提交 aa252ca + 0c1cb26）
> 审查日期：2026-08-09
> 契约：SPEC v1.1 §3.1/§3.3/§6.2/§8.1/§7.2/§13.1 + M1 README §01-hnsw

## 维度逐条结论

### 1. R-hnsw-vec（hnsw.bin 嵌入向量）—— ❌ 确认必须修

**证据（ hnsw.bin 确实存/读向量）**：

- `write_hnsw` 在每节点记录末尾追加 `dim*4 LE f32` 向量：`crates/vane-core/src/hnsw/mod.rs:483-493`（注释明确标 `// 向量嵌入（R-hnsw-vec）`）。
- `HnswReader::open` 反序列化时读取向量入 `Node.vector`：`mod.rs:591-601`。
- `HnswReader::search` 与 `search_layer` 用 `self.nodes[e].vector` 计算导航距离：`mod.rs:372,397,679,719,740`。
- `HnswWriter::insert` 亦将 `vector.to_vec()` 存入 `Node`：`mod.rs:247`。

**违反契约**：
- SPEC §6.2 明确 `hnsw.bin = 段内 HNSW 图`、`vectors.bin = f32 定长连续排布`——向量规范存储是 vectors.bin，hnsw.bin 应 graph-only。
- M1 README §01-hnsw 契约格式仅列 `{ local_docid | level | num_neighbors | neighbors }`，**无 vector 字段**。
- 报告自称是对 README 格式的「扩展」，但实质是双存。

**内存影响核算**：
- 10 万×384×f32 = 154MB/份。双存：vectors.bin 154MB（SegmentReader 已全加载）+ hnsw.bin 内向量 154MB（HnswReader::open 全读入 `Vec<Node>`）+ 图结构 ~13MB ≈ **321MB 常驻**，加 inverted/stored 接近 §13.1 <500MB 上限。
- 50 万×384 → vectors.bin 770MB + hnsw.bin 向量 770MB + 图 ~65MB ≈ **1.6GB**，违反 §3.3「50 万不塌红线」（OOM 风险）。
- `HnswReader::open` 还先把整个 hnsw.bin 读入 `Vec<u8>`（`read_all_vfs`）再解析为 `Vec<Node>`，峰值瞬时内存再翻倍（文件 buffer + 解析后 Node 向量）。

**修复方向（编排者已判定必须修）**：
- hnsw.bin 改 graph-only：`write_hnsw`/`HnswReader::open` 删除每节点 vector 字段。
- `HnswReader` 改为从 vectors.bin 取向量算距离。两种可行路径：
  - (A) `HnswReader::open` 额外加载 vectors.bin（独立 `Vec<f32>`）——简单但仍有 vectors.bin + hnsw 内向量副本问题（除非 HnswReader 不再持有向量，改为持 SegmentReader 引用）。
  - (B) **推荐**：`HnswReader` 持有 `Arc<dyn Vfs>` + 段目录，`open` 时加载 vectors.bin 一次（或直接复用 `SegmentReader.vectors()`），`search` 按 `local_docid` 索引 vectors 切片算距离。api 层 flush 已有 `reader.vectors()` 可传给 HnswReader 构造。
- README 契约 `HnswReader::open(vfs, segment_dir)` 签名可保持不变（内部读 vectors.bin）。

**受影响的测试**（需改为同时写 vectors.bin 或在测试里提供向量源）：
- `hnsw_search_returns_topk_nearest`（tests.rs:38）
- `hnsw_search_with_filter_skips_excluded`（tests.rs:56）
- `hnsw_graph_bytes_stable_after_write`（tests.rs:77）——此测试断言的是 hnsw.bin 字节稳定，改 graph-only 后断言仍成立，但测试体不再依赖向量嵌入。
- `hnsw_open_missing_file_returns_err`（tests.rs:113）——不变。
- `hnsw_search_cosine_metric`（tests.rs:120）
- `hnsw_search_docid_base_offset`（tests.rs:141）
- `hnsw_recall_vs_brute_small_scale`（tests.rs:159）
- 集成测试 `api_hnsw_*`（走 api flush，自动写 vectors.bin，无需改测试体，但需验证 HnswReader 改造后仍走 HNSW 路径）。

### 2. HNSW 算法正确性 —— ✅

- **分层插入**：`level = floor(-ln(u) * mL)`，`mL = 1/ln(M)`，`M.max(2)` 防退化（`mod.rs:238-240`）。✓ 标准 HNSW。
- **贪婪层降找 ef_construction 近邻**：先从 `max_level` 降到 `level+1`（ef=1 导航），再从 `min(level, max_level)` 降到 0（ef_construction 搜索 + 选 M 邻居 + 双向连接 + 修剪）（`mod.rs:261-320`）。✓
- **层级指数分布**：xorshift64 固定种子，`next_unit` 返回 (0,1]，避免 ln(0)（`mod.rs:187-202`）。✓ 确定性可复现，无 rand 依赖。
- **entry_point = 最高层节点**：新节点 level > max_level 时更新（`mod.rs:324-328`）。✓
- **修剪**：lc==0 上限 2M，其余 M（`mod.rs:299`）。select_neighbors 用简单「按距离升序取前 M」（`mod.rs:423-429`，与计划「简单取 M 最近」一致）。
- **借用安全**：新节点先入 `self.nodes`（neighbors 暂空）再连接，保证修剪时 `self.nodes[new_idx]` 与 `self.nodes[nb]` 合法（`mod.rs:259-260`）。
- **search**：层降 `max_level..=1` ef=1 无 filter 导航，第 0 层 ef_search 搜索带 filter（`mod.rs:662-672`）。`ef = ef_search.max(topk)` 保证 ef≥topk（`mod.rs:659`）。✓
- **距离转换**：导航用 `metric_distance`（cosine=1-cos / L2=|a-b|² / dot=-dot，单调等价 -score）；结果用 `metric_score`（cosine / -sqrt(L2) / dot）。L2 用平方距离导航单调等价真实 L2，正确。✓
- **score 语义与 brute_search 一致**：`metric_score` L2=`-sqrt(s)` 与 `vector::l2_score` 完全相同；cosine/dot 亦对齐（`vector/mod.rs:48-83`）。排序 score 降序、同分 docid 升序（`mod.rs:687-693`），与 brute_search 一致。✓
- **NaN 处理**：`DistNode::Ord` 把 NaN 视为最远（`mod.rs:42-53`），用 `total_cmp` 避免排序 UB。✓

**注**：`HnswWriter::search_layer` 与 `HnswReader::search_layer` 逻辑完全相同（双份代码）。非阻塞，可后续提取共享函数，但当前可读性可接受。

### 3. filter 参数（§8.3） —— ✅

- `search_layer` 增 `filter: Option<&RoaringBitmap>` + `docid_base`（`mod.rs:350-357`）。
- 结果堆 W 仅入 `passes_filter` 命中节点（`filter.contains(local + base)`，`mod.rs:374-377,408-413,721-723,751-755`）。
- 邻居均可作导航点：candidates 堆不检查 filter，无论是否命中都入 candidates（`mod.rs:406-407,749-750`）。✓ 符合 §8.3「位图进 HNSW 遍历」。
- 上层导航（layer>0）传 `filter=None`（`mod.rs:664`），仅第 0 层应用 filter。✓ 合理。
- `passes_filter` 防御性处理 abs > u32::MAX（`mod.rs:443-446`）。✓
- filter 测试 `hnsw_search_with_filter_skips_excluded` 真实断言：仅返回 5,6,7 且 docid=6 最近（tests.rs:56-74）。✓

### 4. Q-5 缺失 fallback —— ✅

- `HnswReader::open` 缺失 hnsw.bin → `Err`（`read_all_vfs` 失败或 magic 校验失败，`mod.rs:531-547`）。测试 `hnsw_open_missing_file_returns_err`（tests.rs:113）。
- `hnsw_readers: RwLock<Vec<Option<Arc<HnswReader>>>>`（`collection.rs:51`）。✓ Option 因 M0 段无 hnsw.bin。
- restore_from_manifest：`HnswReader::open` Ok→Some，Err→None（`collection.rs:118-121`）。
- flush：写失败/open 失败 → None（`collection.rs:269-280`）。✓
- search：`Some(hr)` → HNSW，`None` → brute_search（`collection.rs:378-403`）。✓
- 集成测试 `m0_corpus_without_hnsw_bin_falls_back_to_brute` 真实删 hnsw.bin + reopen + 搜索命中（hnsw_recall.rs:49-92）。✓ M0 corpus 可被 M1 打开暴力检索。

### 5. 自适应回退（§8.1） —— ✅（骨架，03 前不触发）

- `force_brute = filter_bm.len() < 2*topk`（`collection.rs:358-361`）。✓ 符合 §8.1「过滤候选 <2×topK 暴力回退」。
- M1 `filter_bm=None` → `force_brute=false`，分支不触发（03 接入位图后生效）。✓ 与计划一致。
- 强制暴力时走 `brute_search`（`collection.rs:393-403`）。✓

### 6. api 接入 —— ✅

- flush：add_doc 完成后从 `reader.vectors()` 构建 HnswWriter（M=16/ef_construction=200）→ build → write_hnsw → open HnswReader 入缓存（`collection.rs:250-281`）。✓
- restore_from_manifest：每段 try open，Some/None 入缓存（`collection.rs:118-128`）。✓
- search vector 路：Some→HnswReader::search、None→brute（`collection.rs:378-403`）。✓
- 00 的 set_text 接入未破坏：`writer.set_text(...)` 仍在 flush 主路径（`collection.rs:231`）。✓
- 新增 `Collection::segment_ulids()` 公共访问器（`collection.rs:529-537`），additive，非破坏。
- ef_search = `max(ef_construction, want*4)`（`collection.rs:380`）。✓

### 7. R-4/R-6 全串行 —— ✅

- `grep` 确认 hnsw/mod.rs + api/collection.rs **零 `cfg(target)`**（仅注释提及「零 cfg」）。
- 无 `thread::scope`、无 `rayon`。
- 多段归并：串行 `for ... zip(...)` 循环（`collection.rs:364-423`）。✓
- wasm32 check 通过（报告自证）。✓

### 8. I-3 图不删 —— ✅

- hnsw.bin 写一次（flush），读期只读：`HnswReader` 无任何写方法。✓
- Task 6 字节稳定测试（tests.rs:77）：写后读两次字节一致。弱断言（报告已承认），强断言（delete 后不变）由 02 Task 7 补。✓ 占位合理。
- 图重建仅段合并（02 MergeTask 调 HnswWriter，契约已就绪）。✓

### 9. M0 冻结签名零破坏 —— ✅

- `brute_search` / `SegmentReader::vectors`/`dim`/`add_doc` 签名均未变（grep 确认）。
- 仅新增 `pub mod hnsw;`（lib.rs:5）+ api 内部 `hnsw_readers: pub(crate)`（collection.rs:51）+ `segment_ulids()` 公共访问器（additive）。
- `SearchQuery.filter` 从 M0 reject(InvalidArg) 改为透传 None（`collection.rs:312`）——这是计划要求的行为变更，非签名变更。测试更名 `search_filter_accepted_but_not_compiled_in_m1`（tests.rs:444）。✓

### 10. hnsw.bin 格式（除向量问题） —— ✅

- 头：`magic(4)="VANE" | format_version(4 LE)=1 | dim(4 LE) | metric(1) | m(4 LE) | ef_construction(4 LE) | entry_point(4 LE) | max_level(4 LE) | num_nodes(4 LE)`（`mod.rs:457-465`）。✓ 符合 README 契约 + SPEC §6.2「magic+version 开头」。
- entry_point 用 `u32::MAX` 哨兵表示 None（`mod.rs:463,553-557`）。✓
- level 以 u8 存储（`mod.rs:469-475`，>255 报 Corrupt）。✓ HNSW 实际层数远 <255。
- 反序列化有完整性校验（truncated → Corrupt，`mod.rs:564-595`）。✓
- magic/version 校验（`mod.rs:538-547`）。✓

### 11. 测试质量 —— ✅（小瑕疵）

- 10 hnsw 单元 + 3 集成，均有真实断言（非 tautological）。
- `hnsw_recall_vs_brute_small_scale`：300×8 L2，20 query，对比 brute_search 求 recall@10≥0.95，实测 1.0。小规模但非平凡——验证导航正确性（HNSW 在低维易 recall 1.0，有意义但偏弱，大规模五档由 12-regression 负责）。✓
- `api_hnsw_recall_vs_brute_at_least_95pct`：**仅断言返回 10 条，未实际对比 brute recall**（hnsw_recall.rs:44-45）。报告已承认 recall 检查交 12。⚠️ 测试名与断言强度不匹配，建议改名 `api_hnsw_returns_topk` 或补真实 recall 断言。非阻塞。
- 多段串行归并测试 `api_hnsw_multi_segment_merge_serial` 真实断言跨段 topK 顺序（hnsw_recall.rs:95-149）。✓

### 12. ef_search 公式 —— ✅（编排者已接受）

- `max(ef_construction, want*4)`，want=cand（hybrid）或 topk（vector）（`collection.rs:380`）。
- hybrid 模式 want=cand > topk，略大于计划表述的 `topk*4`，对 recall 有利。编排者已接受，12-regression 验证。仅记录。✓

---

## 阻塞项

### B-1（必须修）：R-hnsw-vec — hnsw.bin 嵌入向量

**问题**：`write_hnsw`/`HnswReader::open`/`search` 在 hnsw.bin 存/读向量（`mod.rs:483-493,591-601,372,397,679,719,740`），违反 SPEC §6.2 + README 契约（hnsw.bin = graph-only）。

**影响**：向量双存（vectors.bin + hnsw.bin），10 万×384≈321MB 常驻（逼近 §13.1 <500MB），50 万×384≈1.6GB 违反 §3.3「50 万不塌红线」会 OOM。`HnswReader::open` 还先把整文件读入 `Vec<u8>` 再解析，峰值内存再翻倍。

**修复**：
1. `write_hnsw` 删除每节点 vector 字段，hnsw.bin 改 graph-only。
2. `HnswReader` 改为从 vectors.bin 取向量（推荐：`open` 时加载 vectors.bin 一次，或 api 层把 `SegmentReader.vectors()` 传入 HnswReader 构造；README 签名 `open(vfs, segment_dir)` 可保持不变，内部读 `vectors.bin`）。
3. 调整 Task 3/4/6 单元测试：写 hnsw.bin 前需先写 vectors.bin（或测试辅助函数提供向量切片给 HnswReader）。集成测试走 api flush 自动写 vectors.bin，测试体基本不变。
4. 修复后重跑全部门禁。

**verdict 关联**：此项阻塞 → CHANGES_REQUESTED。

## 需编排者裁决疑点

1. **HnswReader 向量源方案选择**：修复 R-hnsw-vec 时，HnswReader 应 (A) 内部独立加载 vectors.bin（持有自己的 `Vec<f32>`，与 SegmentReader.vectors() 形成第二份副本——仍冗余但合规），还是 (B) api 层把 `Arc<SegmentReader>` 或 `&[f32]` 传给 HnswReader 共享向量（零冗余，但 HnswReader 需持有外部引用或 Arc，签名/生命周期变化）？方案 B 更省内存但耦合 api 层；方案 A 简单但 10 万规模仍有 ~154MB×2 副本（SegmentReader + HnswReader 各一份）。建议编排者定方案 B（api 层 flush/restore 已有 SegmentReader，传 `&[f32]` 或 Arc 共享），避免双副本。
2. **`api_hnsw_recall_vs_brute_at_least_95pct` 测试名 vs 断言**（维度 11 瑕疵）：测试名承诺 ≥95% recall 但只断言返回 10 条。建议改名或补真实 recall 断言（对比同段 brute）。非阻塞，可并入 R-hnsw-vec 修复一并处理。

## 非阻塞观察（记录，不要求本模块修）

- `HnswWriter::search_layer` 与 `HnswReader::search_layer` 逻辑重复，可后续提取共享函数。
- `select_neighbors` 用简单策略（非 heuristic 2），与计划一致；若大规模 recall 不足可在 12-regression 后升级。
- `read_all_vfs` 整文件读入 `Vec<u8>`：R-hnsw-vec 修复后 hnsw.bin 仅 ~13-65MB，可接受；无需改 mmap（core 禁 mmap）。

---

## verdict

**CHANGES_REQUESTED**

- 阻塞项：B-1（R-hnsw-vec）。hnsw.bin 必须改 graph-only，HnswReader 从 vectors.bin 取向量。修复后重跑门禁。
- HNSW 算法正确性、filter、Q-5 fallback、自适应回退骨架、api 接入、全串行、I-3、M0 签名零破坏、hnsw.bin 头格式、测试质量均 ✅ 通过。
- 编排者需裁决：HnswReader 向量源方案（A/B）+ 测试名瑕疵处理方式。
