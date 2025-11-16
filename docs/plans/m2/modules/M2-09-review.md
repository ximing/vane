# M2-09 SQ8 量化——评审报告

**评审对象**：M2-09 SQ8 标量量化（encode/decode/distance 三 metric + brute_search_sq8 + segment 懒加载 + api dispatch）
**评审者**：task reviewer（只读，未跑 cargo）
**diff 范围**：BASE 68ee620..HEAD a844ee1（vane-core，7 文件 +957/-11）
**评审日期**：2026-08-10

---

## 状态：BLOCKED

**Blocker: 1（I-5 违规）｜Issue: 0｜Minor: 5**

---

## B-1（Blocker / I-5 违规）：api search 路径存在 `#[cfg(feature = "sq8")]` 属性

**证据**：
- `crates/vane-core/src/api/collection.rs:39`：
  ```rust
  fn brute_search_dispatch(
      reader: &SegmentReader,
      ...
  ) -> Vec<crate::types::ScoredDoc> {
      let dim = reader.dim();
      #[cfg(feature = "sq8")]          // ← 属性在 api search 路径函数体内
      if let Some(bundle) = reader.sq8_vectors() {
          return crate::vector::sq8::brute_search_sq8(...);
      }
      brute_search(reader.vectors(), dim, qv, metric, want, merged_filter, base)
  }
  ```
- 该函数被 search 路径直接调用：`api/collection.rs:802`（HNSW reader None 回退）、`api/collection.rs:806`（force_brute 低选择率回退）。

**判定依据**：
- SPEC v1.2 释义："`cfg(feature=sq8)` 是存储编解码能力开关，允许在 segment/vector 编解码处"。
- 评审任务约束明确："若在 api search 路径有实际 cfg 属性 = I-5 违规，需移 dispatch 入 vector 模块"。
- `brute_search_dispatch` 位于 `api/collection.rs`（API 搜索调度层），**不是 segment/vector 编解码处**。它选择 SQ8 vs f32 编码路径，属于搜索算法调度逻辑，非编解码本身。
- 实装报告 §6 遗留 #1 已主动承认此边界争议，并提供修复方向："若 reviewer 判定此处违反 I-5，需将 dispatch 逻辑移入 vector 模块"。

**修复建议**：将 `brute_search_dispatch` 移入 `vector/mod.rs`（或 `vector/sq8.rs`），api 层只调用统一签名的 dispatch 函数，cfg 属性下沉到 vector 模块内部。需注意依赖方向：dispatch 若接 `&SegmentReader` 会引入 vector→segment 依赖；可改为接 `vectors: &[f32]` + `sq8_bundle: Option<&Sq8Bundle>` 两个参数，由 api 层分别取 `reader.vectors()` 和 `reader.sq8_vectors()`（后者本身 cfg-gated 访问器在 segment，属编解码处允许）。这样 api search 路径零 cfg 属性。

---

## SQ8 编解码正确性核查（通过）

### encode_sq8（`vector/sq8.rs:60`）
- per-dim min/max 计算（`sq8.rs:79-92`），`q = round((v-min)/(max-min)*255)` clamp [0,255]（`sq8.rs:99-108`）。
- `max==min` / `range` 非有限时量化为 0（`sq8.rs:101-103`），避免除零。✓
- 非法输入（dim=0 / dim>DIM_MAX / len%dim!=0）返回空 bundle（`sq8.rs:61-78`）。✓

### decode_sq8（`vector/sq8.rs:123`）
- `v = min + (q/255)*(max-min)`（`dequant` `sq8.rs:107-115`），与 encode 互逆。✓
- 误差 < range/255（测试 `encode_decode_roundtrip_small_error` `sq8.rs:363`、`encode_decode_roundtrip_dim_384` `sq8.rs:387`）。✓
- **M-4**：签名从 spec 契约 `decode_sq8(sq8, dim)` 改为 `decode_sq8(bundle: &Sq8Bundle)`（用 `bundle.min.len()` 推 dim）。合理偏离（min/max 在 bundle 内），但未在 spec 变更记录文档化。

### sq8_distance（`vector/sq8.rs:201`）
- 覆盖 cosine / L2 / dot 三 metric（`sq8.rs:208-212`）。✓
- 逐字节 on-the-fly dequantize，无分配（`sq8_cosine_score` `sq8.rs:160` / `sq8_l2_score` `sq8.rs:179` / `sq8_dot_score` `sq8.rs:191`）。✓
- 零向量 cosine 返回 0.0（`sq8.rs:173-175`）。✓
- **M-5**：签名增 `min`/`max` 参数（spec 契约 `sq8_distance(sq8_a, sq8_b, dim, metric)` 缺）。dequantize 必须有 per-dim 边界，合理偏离，报告 §6 #2 已说明，spec §3 注释 "Sq8Bundle{data,min,max}" 已预示。

### sq8_query_distance（`vector/sq8.rs:221`）
- query 量化一次（`quantize_query` `sq8.rs:141`），全段复用 `score_fn`（`sq8.rs:240-244`）。✓
- query 超出 bundle [min,max] 范围时 clamp 到 [0,255]（`sq8.rs:148`）——SQ8 近似本质，可接受。
- topK 堆 + 同分 docid 升序排序（`sq8.rs:284-295`），与 brute_search 一致。✓

### brute_search_sq8 签名对齐（通过）
- `brute_search_sq8(bundle: &Sq8Bundle, dim, query, metric, topk, filter, docid_base)`（`sq8.rs:323`）。
- 与 `brute_search(vectors, dim, query, metric, topk, filter, docid_base)`（`vector/mod.rs:107`）对齐（metric + docid_base）。第一参数 `&Sq8Bundle` vs `&[f32]` 合理差异。✓
- `brute_search` 原签名不变（`vector/mod.rs:107`）。✓
- `HnswReader::search` 签名不变（`hnsw/mod.rs:624`：`(query, topk, ef_search, filter, docid_base, vectors)`）。✓ 首选方案落实。
- `vectors()` 签名不变（`segment/mod.rs:506`：`&self -> &[f32]`）。✓

---

## recall 核查（通过，附 Minor）

- 测试 `brute_search_sq8_recall_vs_f32`（`sq8.rs:494`）：dim=128, doc_count=1000，三 metric Jaccard@10 ≥0.95。
- baseline 用 f32 `brute_search`（`sq8.rs:513`），精确基准。✓
- **M-2**：测试规模偏小（1000 docs × dim 128）；query = data[0] + 0.001 扰动（`sq8.rs:507-510`），doc0 是明确最近邻，设计偏宽松（确保 top1 不翻转，但 top2-10 仍由随机向量竞争）。缺 100k 规模 recall 实测。SPEC §13.2-1 口径在 100k 规模未实证。

---

## 内存核查（通过，附 Minor）

- 10万×384 SQ8 data = 38.4MB（估算测试 `memory_estimate_100k_384_under_200mb` `sq8.rs:640`）。✓
- f32 154MB → SQ8 38MB，4 倍降。✓
- **M-3**：仅估算 SQ8 data 单项。SPEC §13.1 "全加载 <200MB" 应含 HNSW f32 + 倒排 + stored。报告 §3 也只估算 SQ8 部分，未实测全加载 RSS。100万 carry-forward（报告 §6 #4）交接清晰：HNSW f32 在 100万约 154MB，加 SQ8 38MB + 倒排 + stored 可能超 200MB → M2-10 评估 HNSW 也用 SQ8 → 改签名 → SPEC 修订。

---

## segment sq8_vectors 懒加载（通过）

- 字段 `#[cfg(feature="sq8")] sq8_vectors: OnceLock<Option<Sq8Bundle>>`（`segment/mod.rs:343`）。✓ cfg 在 segment 编解码处（I-5 允许）。
- 访问器 `sq8_vectors()`（`segment/mod.rs:522`）：`get_or_init` 从 `self.vectors()` 编码，空段（dim==0 或 doc_count==0）返回 None。✓
- 幂等测试 `sq8_vectors_lazy_load_returns_some`（`segment/tests.rs:1061`）：验证 min/max 正确 + 二次调用同引用。✓
- 空段测试 `sq8_vectors_empty_segment_returns_none`（`segment/tests.rs:1103`）。✓
- I-1 守护 `sq8_vectors_does_not_write_segment_files`（`segment/tests.rs:1125`）：触发编码后段文件列表不变，无 sq8 文件。✓

---

## api dispatch（功能通过，I-5 违规见 B-1）

- feature on 走 `brute_search_sq8`，off 走 `brute_search`（`collection.rs:39-51`）。✓
- baseline 路径（`allow_hnsw=false`）恒 f32 `brute_search`（`collection.rs:810`），保证 recall 基准精确。✓
- **M-1**：api 层无专门 dispatch 双路径测试。`brute_search_dispatch` 是私有函数，依赖既有 `recall_regression` 间接覆盖。dispatch 逻辑简单（if let Some(bundle) 走 sq8 else f32），且 sq8_vectors() 在 segment 已测、brute_search_sq8 在 vector 已测，但 feature on/off 双路径在 api 层的端到端回归缺直接断言。

---

## I-5 cfg 位置全局核查

| 文件 | 行 | cfg 类型 | 判定 |
|---|---|---|---|
| `Cargo.toml:30` | `sq8 = []` | feature 定义 | ✓ 允许 |
| `vector/mod.rs:11` | `#[cfg(feature="sq8")] pub mod sq8;` | 模块声明 | ✓ 编解码处 |
| `segment/mod.rs:343,445,521` | 字段/构造/访问器 | segment 编解码 | ✓ 允许 |
| `segment/tests.rs:1059,1101,1123` | 测试 | 测试处 | ✓ 允许 |
| **`api/collection.rs:39`** | **`#[cfg(feature="sq8")]` 在 `brute_search_dispatch` 函数体内** | **api search 路径** | **✗ I-5 违规（B-1）** |
| `api/collection.rs:28` | 注释提及 I-5 | 注释 | 非属性 |

- core 零 `cfg(target)`（grep 仅命中注释 `hnsw/mod.rs:5`、`sq8.rs:13`）。✓

---

## TDD 覆盖核查

- 新增 21 测试：sq8.rs 18 + segment/tests.rs 3。匹配报告。✓
- 覆盖：encode/decode roundtrip（小规模 + dim384）、三 metric distance vs f32、brute_search_sq8 recall（三 metric Jaccard）、懒加载（幂等 + 空段 + I-1 不写文件）、边界（dim0/dim超限/空/topk0/filter/docid_base/排序）、内存估算。✓
- 缺口：M-1（api dispatch 端到端双路径）、M-2（100k 规模 recall）。

---

## 不变量覆盖

| 不变量 | 状态 | 证据 |
|---|---|---|
| I-1 段不可变 | ✓ | sq8 不写段文件（测试 `segment/tests.rs:1125`）；vectors.bin 仍 f32 落盘 |
| I-5 cfg(feature) 编解码处 | **✗** | B-1：api/collection.rs:39 search 路径有 cfg 属性 |
| I-8 | n/a | 不涉 |
| 签名冻结 | ✓ | vectors()/brute_search/HnswReader::search 三处未改 |

---

## 100万 carry-forward

- HNSW 不用 SQ8（首选方案），签名不变。✓
- 100万若 >200MB → HNSW SQ8 → 签名变更 → SPEC 修订（M2-10 评估）。报告 §6 #4 交接清晰。✓

---

## 发现汇总

| 级别 | 编号 | 内容 | 位置 |
|---|---|---|---|
| **Blocker** | **B-1** | **I-5 违规：api search 路径 `brute_search_dispatch` 函数体内有 `#[cfg(feature="sq8")]` 属性** | `api/collection.rs:39` |
| Minor | M-1 | api dispatch feature on/off 双路径无端到端测试 | `api/collection.rs:30` |
| Minor | M-2 | recall 测试规模偏小（1000 docs），query 设计偏宽松，缺 100k 实测 | `vector/sq8.rs:494` |
| Minor | M-3 | 内存仅估算 SQ8 data 单项，未实测全加载 RSS | 报告 §3 |
| Minor | M-4 | `decode_sq8` 签名偏离 spec 契约（`decode_sq8(sq8, dim)` → `decode_sq8(bundle)`） | `vector/sq8.rs:123` |
| Minor | M-5 | `sq8_distance` 签名增 min/max 参数（spec 契约缺），合理但需文档化 | `vector/sq8.rs:201` |

---

## 修复建议（B-1 解阻塞路径）

将 `brute_search_dispatch` 移入 `vector/mod.rs`，签名改为不依赖 SegmentReader：

```rust
// vector/mod.rs
pub fn brute_search_dispatch(
    vectors: &[f32],
    sq8_bundle: Option<&sq8::Sq8Bundle>,  // None 时走 f32（feature off 或空段）
    dim: u32,
    query: &[f32],
    metric: Metric,
    topk: usize,
    filter: Option<&roaring::RoaringBitmap>,
    docid_base: u64,
) -> Vec<ScoredDoc> {
    #[cfg(feature = "sq8")]
    if let Some(bundle) = sq8_bundle {
        return sq8::brute_search_sq8(bundle, dim, query, metric, topk, filter, docid_base);
    }
    let _ = sq8_bundle; // feature off 时消解未使用参数
    brute_search(vectors, dim, query, metric, topk, filter, docid_base)
}
```

api 层调用：
```rust
// api/collection.rs（零 cfg 属性）
let sq8_bundle = reader.sq8_vectors(); // segment 访问器 cfg-gated（编解码处允许）
crate::vector::brute_search_dispatch(
    reader.vectors(), sq8_bundle, reader.dim(), qv, metric, want, merged_filter, base,
)
```

这样 api search 路径零 `cfg(feature)` 属性，cfg 下沉到 vector 模块（编解码处）。`reader.sq8_vectors()` 在 feature off 时不存在（segment cfg-gated 访问器），api 层需用 `#[cfg(feature="sq8")]` 取 bundle 或用 trait 抽象——**这一步仍需谨慎，避免 cfg 泄漏到 api**。可行方案：在 segment 定义 `sq8_vectors()` 在 feature off 时返回恒 `None`（不 cfg-gate 方法，cfg-gate 内部实现），api 层始终可调用且拿 `Option`。

---

## 结论

SQ8 编解码、三 metric 距离、懒加载、签名冻结、recall 测试均正确且覆盖充分。**唯一阻塞项为 B-1（I-5 违规）**：api search 路径 `brute_search_dispatch` 函数体内存在 `#[cfg(feature="sq8")]` 属性，违反 SPEC v1.2 "cfg(feature=sq8) 仅在 segment/vector 编解码处" 释义。实装报告已主动标注此争议并给出修复方向。建议按上述修复建议将 dispatch 下沉到 vector 模块，使 api search 路径零 cfg 属性后重新评审。
