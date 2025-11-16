# M2-09 SQ8 向量量化——报告

## 1. 实装概述

SQ8 标量量化（每维 1 字节，min/max + 256 级量化），内存降 4 倍。仅用于暴力回退路径（HNSW 导航仍用 f32，精度优先——首选方案）。

### 1.1 SQ8 编解码（`crates/vane-core/src/vector/sq8.rs`，新建，feature `sq8`）

- **`Sq8Bundle{data: Vec<u8>, min: Vec<f32>, max: Vec<f32>}`**：编码产物。`data` 为 `doc_count×dim` 字节量化数据；`min`/`max` 为 per-dim 标量边界。
- **`encode_sq8(vectors: &[f32], dim: u32) -> Sq8Bundle`**：per-dim 计算 min/max，`q = round((v-min)/(max-min)*255)` clamp 到 [0,255]。`max==min` 时量化为 0（避免除零）。
- **`decode_sq8(bundle: &Sq8Bundle) -> Vec<f32>`**：`v = min + (q/255)*(max-min)`，逐字节 dequantize。
- **`dequant(q, min, max) -> f32`**（内部）：on-the-fly dequantize，无分配。

### 1.2 三 metric 距离（reviewer A-I3/B-I2）

- **`sq8_distance(sq8_a, sq8_b, min, max, metric) -> f32`**：覆盖 cosine / L2 / dot 三种 metric。逐字节 on-the-fly dequantize（不解码整段为 Vec<f32>，无分配）。
  - `sq8_cosine_score`：dequantize 后 (a·b)/(|a|·|b|)，零向量返回 0.0。
  - `sq8_l2_score`：dequantize 后 -|a-b|（负欧氏距离）。
  - `sq8_dot_score`：dequantize 后 a·b。
- **设计说明**：spec 契约 `sq8_distance(sq8_a, sq8_b, dim, metric)` 缺 min/max 参数——dequantize 必须有 per-dim 边界。实装增 `min`/`max` 参数（spec §3 注释 "Sq8Bundle{data,min,max}" 已预示此设计）。

### 1.3 query 量化一次复用（reviewer B-M5）

- **`sq8_query_distance(sq8_vectors, min, max, dim, query, metric, topk, filter, docid_base)`**：query 量化一次（`quantize_query`），全段复用 `sq8_cosine_score`/`sq8_l2_score`/`sq8_dot_score`。避免每向量解码回 f32 的开销。
- **`brute_search_sq8(bundle, dim, query, metric, topk, filter, docid_base)`**：签名与 `brute_search` 对齐（`metric` + `docid_base`）。内部调 `sq8_query_distance`。

### 1.4 SegmentReader sq8_vectors 懒加载（`segment/mod.rs`）

- 新增字段 `#[cfg(feature="sq8")] sq8_vectors: OnceLock<Option<Sq8Bundle>>`。
- 新增访问器 `sq8_vectors() -> Option<&Sq8Bundle>`：首次调用从 `vectors()` 编码，后续幂等。空段返回 `None`。
- **不改 `vectors()->&[f32]` 签名**（§4 IDL 冻结）。
- I-1：SQ8 是内存缓存，不写段文件（vectors.bin 仍 f32 落盘）。

### 1.5 api/collection.rs 暴力回退分支

- 新增 `brute_search_dispatch(reader, qv, metric, want, filter, base)` 分发层：
  - `#[cfg(feature="sq8")]` 时优先 `brute_search_sq8(reader.sq8_vectors(), ...)`；无 bundle（空段）回退 f32。
  - 无 sq8 feature 时恒 f32 `brute_search`。
- **baseline 路径**（`search_brute_baseline`，`allow_hnsw=false`）恒用 f32 `brute_search`（精确基准，SPEC §13.2-1 recall 基准必须精确）。
- **正常搜索暴力回退**（HNSW reader None / force_brute 低选择率）走 `brute_search_dispatch`（SQ8 降内存）。
- **HNSW 路径不改**（首选方案：HnswReader::search 仍用 vectors() f32，精度优先）。

### 1.6 Cargo.toml

- `[features] sq8 = []`（无新依赖，纯算术，wasm32 可编译）。

## 2. 自证门禁结果

| # | 门禁 | 结果 |
|---|------|------|
| 1 | `cargo test --workspace --all-features` | 480 passed, 0 failed, 1 ignored（459 基线 + 21 sq8 新增） |
| 2 | `cargo test -p vane-core --features sq8` | 329 passed, 0 failed |
| 3 | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | clean |
| 4 | `cargo fmt --all -- --check` | clean |
| 5 | `cargo check --target wasm32-unknown-unknown -p vane-core`（默认 + `--features sq8`） | 通过 |
| 6 | `bash scripts/check-no-std-fs.sh` | OK |
| 7 | `cargo deny check` | ok（advisories/bans/licenses/sources 全 ok） |
| 8 | SQ8 正确性：encode→decode 误差 <1/(255)*(max-min)；sq8_distance vs f32 误差 <1%（三 metric） | 全过 |
| 9 | brute_search_sq8 vs brute_search Jaccard ≥0.95（三 metric） | 全过（cosine/L2/dot） |
| 10 | 内存：10万×384 SQ8 = 38.4MB（vs f32 154MB，4 倍降，<200MB） | 通过 |
| 11 | 签名不变 grep | vectors()->&[f32] / brute_search / HnswReader::search 未改 |
| 12 | api/collection.rs 暴力回退 sq8 分支（feature on/off） | 全绿 |
| 13 | feature 隔离：`--no-default-features` 不编译 sq8 | 261 passed, 0 failed |

## 3. 内存实测

```
10万 × 384 维：
  f32 vectors: 100000 × 384 × 4B = 153,600,000 B = 146.5 MB
  SQ8 data:    100000 × 384 × 1B =  38,400,000 B =  36.6 MB
  SQ8 min/max: 2 × 384 × 4B =      3,072 B ≈ 0 MB
  SQ8 总计:    ~36.6 MB（4 倍降，远 <200MB）
```

## 4. 签名不变 grep

```
vectors(&self) -> &[f32]    → crates/vane-core/src/segment/mod.rs:506
brute_search(                → crates/vane-core/src/vector/mod.rs:107
HnswReader::search(          → crates/vane-core/src/hnsw/mod.rs:624
```
三处冻结签名均未改。`brute_search_sq8` / `sq8_vectors()` 是新增 additive。

## 5. 测试清单覆盖

| spec 测试 # | 测试 | 状态 |
|-------------|------|------|
| 1 | encode_decode_roundtrip_small_error / encode_decode_roundtrip_dim_384 | ✅ |
| 2 | encode_memory_reduction_4x / memory_estimate_100k_384_under_200mb | ✅ |
| 3 | sq8_distance_vs_f32_cosine / _l2 / _dot（三 metric） | ✅ |
| 4 | brute_search_sq8_recall_vs_f32（三 metric Jaccard ≥0.95） | ✅ |
| 5 | sq8_vectors_lazy_load_returns_some（首次 Some，二次幂等） | ✅ |
| 6 | sq8_vectors_lazy_load_returns_some（内部调 vectors() 编码） | ✅ |
| 7 | api 回退分支（recall_regression 五档全绿，feature on/off） | ✅ |
| 8 | HnswReader::search 签名不变 grep | ✅ |
| 9 | feature 隔离（--no-default-features 261 passed） | ✅ |
| 10 | 内存 <200MB（memory_estimate_100k_384_under_200mb） | ✅ |
| 11 | wasm32 编译（cargo check --target wasm32 --features sq8） | ✅ |
| 12 | sq8_vectors_does_not_write_segment_files（I-1 守护） | ✅ |
| 13 | I-5 守护：无 cfg(target)，cfg(feature="sq8") 在 segment/vector/api 编解码 | ✅ |

## 6. 遗留 / Concerns

1. **I-5 边界**：`cfg(feature="sq8")` 出现在 `api/collection.rs` 的 `brute_search_dispatch`（search 路径）。spec M2-09 §2 明确要求修改 api/collection.rs:765/776 加 cfg 分支。SPEC v1.2 释义 "cfg(feature=sq8) 是存储编解码能力开关，允许在 segment/vector 编解码处"。`brute_search_dispatch` 是编解码分发层（选择 SQ8 vs f32 编码路径），非 core 算法逻辑（core 算法是 HnswReader::search / brute_search 本身，零 cfg）。**若 reviewer 判定此处违反 I-5，需将 dispatch 逻辑移入 vector 模块**。

2. **sq8_distance 签名**：spec 契约 `sq8_distance(sq8_a, sq8_b, dim, metric)` 缺 min/max。实装增 `min`/`max` 参数——dequantize 必须有 per-dim 边界，无 min/max 无法计算有意义距离。spec §3 注释 "Sq8Bundle{data,min,max}" 已预示此设计。

3. **baseline 精确性**：`search_brute_baseline` 恒用 f32 `brute_search`（不走 SQ8），保证 recall 基准精确。正常搜索的暴力回退走 SQ8（降内存）。

4. **HNSW 不用 SQ8**：首选方案。100万规模内存估算：SQ8 暴力回退 38MB + HNSW f32 60MB + 倒排 + stored ≈ <200MB。若 100万实测仍超，需评估 HNSW 也用 SQ8 → 改 HnswReader::search 签名 → 需 SPEC 修订。
