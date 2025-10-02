# Workspace 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development 执行本计划。步骤用 checkbox `- [ ]` 标记。

**Goal:** 从零搭建 Cargo workspace + git 仓库 + vane-core 基础类型（VaneError/Schema/常量），为所有后续计划提供类型地基。
**Architecture:** 单 workspace 含 `vane-core`（lib）、`vane-ffi`（cdylib 骨架）、`vane-node`（napi 骨架）三个 crate。`vane_core::types` 定义全部跨模块共享的数据结构与错误码，纯数据无 IO 逻辑。
**Tech Stack:** Rust 2021 edition、Cargo workspace、roaring crate、sha2 crate。
**SPEC 引用:** §3.1 Schema、§3.2 文档约束、§3.3 规模红线、§6.2 magic/version、§10 错误码、§13.3 工程纪律门禁、§14 I-5 核心零平台分支。
**前置依赖:** 无（M0 起点）。
**验收标准:**
- [ ] `cargo build --workspace` 通过
- [ ] `cargo test -p vane-core` 通过
- [ ] VaneError 11 个变体的 code() 与 SPEC §10 完全一致
- [ ] Schema::validate 拒绝 0 个或 ≥2 个 vector 字段；拒绝 dim>4096
- [ ] core crate 无 `std::fs`/`std::net`/mmap（grep 验证）
- [ ] deny.toml 列出全部黑名单依赖

## Global Constraints
- core crate 禁止 `std::fs`/`std::net`/mmap（SPEC §6.1/§13.3，CI 门禁 M0 第一天）。
- `cfg(target)` 只允许在 VFS/Executor 实现处；core 算法代码零 cfg（§11/不变量 I-5）。
- 依赖黑名单：regex / tokio 全套 / prost / tonic / openssl / lindera / ndarray / wee_alloc（§4.1/§13.3）。
- dim 上限 4096；单文档 ≤16MB；topK 上限 1000；段数上限 10；BM25 k1=1.2 b=0.75；RRF k=60（§3.1/§3.2/§3.3/§6.3/§8.2）。
- 段文件头：4 字节 magic(b"VANE") + 4 字节 format_version(1)（§6.2）。
- 协议 Apache-2.0（REQUIREMENTS §7）。

## File Structure
- `Cargo.toml` — workspace 根，声明 members + 共享依赖
- `.gitignore` — target/、*.node、node_modules/
- `rustfmt.toml` — edition 2021、max_width 100
- `deny.toml` — cargo-deny 配置，bans 黑名单
- `LICENSE` — Apache-2.0
- `crates/vane-core/Cargo.toml` — lib crate，依赖 roaring/sha2
- `crates/vane-core/src/lib.rs` — pub mod types/vfs/tokenizer/fusion/vector/segment/bm25/persistence/api;（一次性预声明全部 9 模块）
- `crates/vane-core/src/types.rs` — VaneError/Result/ScoredDoc/Metric/TokenizerId/Schema/FieldDef/常量
- `crates/vane-ffi/Cargo.toml` — cdylib 骨架（M0 仅占位）
- `crates/vane-ffi/src/lib.rs` — 空骨架
- `crates/vane-node/Cargo.toml` — napi 骨架（M0 仅占位，09 计划填充）
- `crates/vane-node/src/lib.rs` — 空骨架

## 任务清单（bite-sized TDD）

### Task 1: git init + workspace 脚手架
**Files:**
- Create: `Cargo.toml`, `.gitignore`, `rustfmt.toml`, `LICENSE`
- Create: `crates/vane-core/Cargo.toml`, `crates/vane-core/src/lib.rs`
- Create: `crates/vane-ffi/Cargo.toml`, `crates/vane-ffi/src/lib.rs`
- Create: `crates/vane-node/Cargo.toml`, `crates/vane-node/src/lib.rs`

**Interfaces:**
- Consumes from: 无
- Produces: Cargo workspace（后续所有计划的编译基础）

- [ ] **Step 1: 写失败验证** — 预期 `cargo build --workspace` 在空仓库下失败。先创建文件：
```toml
# Cargo.toml
[workspace]
members = ["crates/vane-core", "crates/vane-ffi", "crates/vane-node"]
resolver = "2"

[workspace.package]
version = "0.1.0"
edition = "2021"
license = "Apache-2.0"
authors = ["Vane Contributors"]

[workspace.dependencies]
vane-core = { path = "crates/vane-core" }
roaring = "0.10"
sha2 = "0.10"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
unicode-segmentation = "1.11"
rust-stemmers = "1.2"
ulid = "1"
```
```toml
# crates/vane-core/Cargo.toml
[package]
name = "vane-core"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
roaring = { workspace = true }
sha2 = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
unicode-segmentation = { workspace = true }
rust-stemmers = { workspace = true }
ulid = { workspace = true }
```
**不引入 dashmap、不引入 parking_lot**（B2 裁决：并发原语统一 std::sync，wasm32 绝对安全）。后续 02/04 等计划确认 00 已加入所需依赖，不重复添加（重复键会导致 Cargo 解析失败）。
```rust
// crates/vane-core/src/lib.rs
// 一次性预声明全部模块（B1 裁决：避免 L1/L2 各计划并行改 lib.rs 冲突）
pub mod types;
pub mod vfs;
pub mod tokenizer;
pub mod fusion;
pub mod vector;
pub mod segment;
pub mod bm25;
pub mod persistence;
pub mod api;
```
每个模块建空占位文件（`crates/vane-core/src/vfs/mod.rs`、`tokenizer/mod.rs`、`fusion/mod.rs`、`vector/mod.rs`、`segment/mod.rs`、`bm25.rs`、`persistence/mod.rs`、`api/mod.rs`），内容仅 `// 由 NN-xxx 计划填充`。types.rs 由本计划 Task 2-5 填充。
```toml
# crates/vane-ffi/Cargo.toml
[package]
name = "vane-ffi"
version.workspace = true
edition.workspace = true
license.workspace = true

[lib]
crate-type = ["cdylib", "staticlib", "rlib"]

[dependencies]
vane-core = { workspace = true }
```
```rust
// crates/vane-ffi/src/lib.rs
// M0 占位；FFI 实现见 M1 计划。
```
```toml
# crates/vane-node/Cargo.toml
[package]
name = "vane-node"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
vane-core = { workspace = true }
```
```rust
// crates/vane-node/src/lib.rs
// M0 占位；napi 绑定见 09-node-binding 计划。
```
```gitignore
# .gitignore
/target
*.node
node_modules/
```

- [ ] **Step 2: 跑构建确认** — `cargo build --workspace`。因 `types` 模块尚未创建，预期 `error[E0433]`（未找到模块 types）。
```bash
cargo build --workspace 2>&1 | head -20
```

- [ ] **Step 3: 最小实现** — 创建 `crates/vane-core/src/types.rs` 空文件：
```rust
// crates/vane-core/src/types.rs
// 基础类型定义；Task 2-5 逐步填充。
```
- [ ] **Step 4: 跑构建确认通过** — `cargo build --workspace` 成功，无错误。
- [ ] **Step 5: Commit**
```bash
git init
git add -A
git commit -m "chore: scaffold cargo workspace with vane-core/ffi/node crates

"
```

### Task 2: VaneError + Result + code() 映射
**Files:**
- Modify: `crates/vane-core/src/types.rs`
- Test: `crates/vane-core/src/types.rs`（#[cfg(test)] mod tests）

**Interfaces:**
- Consumes from: 无
- Produces: `VaneError`（11 变体）、`VaneError::code() -> i32`、`VaneError::name() -> &'static str`、`type Result<T>`
- 后续计划全部消费此类型

- [ ] **Step 1: 写失败测试** — 在 `types.rs` 末尾追加：
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_code_matches_spec_section_10() {
        assert_eq!(VaneError::Io("x".into()).code(), -1);
        assert_eq!(VaneError::Schema("x".into()).code(), -2);
        assert_eq!(VaneError::NotFound("x".into()).code(), -3);
        assert_eq!(VaneError::Corrupt("x".into()).code(), -4);
        assert_eq!(VaneError::Version("x".into()).code(), -5);
        assert_eq!(VaneError::TokenizerMismatch("x".into()).code(), -6);
        assert_eq!(VaneError::DictTooLarge.code(), -7);
        assert_eq!(VaneError::DictUnavailable.code(), -8);
        assert_eq!(VaneError::Busy.code(), -9);
        assert_eq!(VaneError::Unsupported.code(), -10);
        assert_eq!(VaneError::InvalidArg("x".into()).code(), -11);
    }

    #[test]
    fn error_name_matches_spec() {
        assert_eq!(VaneError::Io("x".into()).name(), "E_IO");
        assert_eq!(VaneError::Schema("x".into()).name(), "E_SCHEMA");
        assert_eq!(VaneError::NotFound("x".into()).name(), "E_NOT_FOUND");
        assert_eq!(VaneError::Corrupt("x".into()).name(), "E_CORRUPT");
        assert_eq!(VaneError::Version("x".into()).name(), "E_VERSION");
        assert_eq!(VaneError::TokenizerMismatch("x".into()).name(), "E_TOKENIZER_MISMATCH");
        assert_eq!(VaneError::DictTooLarge.name(), "E_DICT_TOO_LARGE");
        assert_eq!(VaneError::DictUnavailable.name(), "E_DICT_UNAVAILABLE");
        assert_eq!(VaneError::Busy.name(), "E_BUSY");
        assert_eq!(VaneError::Unsupported.name(), "E_UNSUPPORTED");
        assert_eq!(VaneError::InvalidArg("x".into()).name(), "E_INVALID_ARG");
    }

    #[test]
    fn error_is_display_and_std_error() {
        let e = VaneError::InvalidArg("topK exceeds 1000".into());
        assert!(format!("{}", e).contains("topK exceeds 1000"));
        // std::error::Error trait 可调用 source()
        assert!(std::error::Error::source(&e).is_none());
    }
}
```

- [ ] **Step 2: 跑测试确认失败** —
```bash
cargo test -p vane-core 2>&1 | head -20
```
预期编译失败（VaneError 未定义）。

- [ ] **Step 3: 最小实现** — 在 `types.rs` 顶部（测试之前）写入：
```rust
use std::fmt;

/// SPEC §10 错误码。code() 返回值与 SPEC §10 表一一对应。
#[derive(Debug, Clone)]
pub enum VaneError {
    Io(String),
    Schema(String),
    NotFound(String),
    Corrupt(String),
    Version(String),
    TokenizerMismatch(String),
    DictTooLarge,
    DictUnavailable,
    Busy,
    Unsupported,
    InvalidArg(String),
}

impl VaneError {
    pub fn code(&self) -> i32 {
        match self {
            Self::Io(_) => -1,
            Self::Schema(_) => -2,
            Self::NotFound(_) => -3,
            Self::Corrupt(_) => -4,
            Self::Version(_) => -5,
            Self::TokenizerMismatch(_) => -6,
            Self::DictTooLarge => -7,
            Self::DictUnavailable => -8,
            Self::Busy => -9,
            Self::Unsupported => -10,
            Self::InvalidArg(_) => -11,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Io(_) => "E_IO",
            Self::Schema(_) => "E_SCHEMA",
            Self::NotFound(_) => "E_NOT_FOUND",
            Self::Corrupt(_) => "E_CORRUPT",
            Self::Version(_) => "E_VERSION",
            Self::TokenizerMismatch(_) => "E_TOKENIZER_MISMATCH",
            Self::DictTooLarge => "E_DICT_TOO_LARGE",
            Self::DictUnavailable => "E_DICT_UNAVAILABLE",
            Self::Busy => "E_BUSY",
            Self::Unsupported => "E_UNSUPPORTED",
            Self::InvalidArg(_) => "E_INVALID_ARG",
        }
    }
}

impl fmt::Display for VaneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(m) => write!(f, "E_IO: {}", m),
            Self::Schema(m) => write!(f, "E_SCHEMA: {}", m),
            Self::NotFound(m) => write!(f, "E_NOT_FOUND: {}", m),
            Self::Corrupt(m) => write!(f, "E_CORRUPT: {}", m),
            Self::Version(m) => write!(f, "E_VERSION: {}", m),
            Self::TokenizerMismatch(m) => write!(f, "E_TOKENIZER_MISMATCH: {}", m),
            Self::DictTooLarge => write!(f, "E_DICT_TOO_LARGE"),
            Self::DictUnavailable => write!(f, "E_DICT_UNAVAILABLE"),
            Self::Busy => write!(f, "E_BUSY"),
            Self::Unsupported => write!(f, "E_UNSUPPORTED"),
            Self::InvalidArg(m) => write!(f, "E_INVALID_ARG: {}", m),
        }
    }
}

impl std::error::Error for VaneError {}

pub type Result<T> = std::result::Result<T, VaneError>;
```

- [ ] **Step 4: 跑测试确认通过** — `cargo test -p vane-core`，3 个测试全绿。
- [ ] **Step 5: Commit**
```bash
git add -A
git commit -m "feat(core/types): VaneError with spec-§10 code mapping + Result alias

"
```

### Task 3: 基础数据类型（ScoredDoc / Metric / TokenizerId）
**Files:**
- Modify: `crates/vane-core/src/types.rs`（追加类型 + 测试）

**Interfaces:**
- Consumes from: Task 2（VaneError, Result）
- Produces: `ScoredDoc`、`Metric`、`TokenizerId`（含 as_bytes/to_hex/from_hex）
- 后续 02-tokenizer 消费 TokenizerId；06-vector-brute 消费 Metric/ScoredDoc；05-bm25 消费 ScoredDoc

- [ ] **Step 1: 写失败测试** — 追加到 tests mod：
```rust
    #[test]
    fn tokenizer_id_hex_roundtrip() {
        let raw = [0u8; 32];
        let id = TokenizerId(raw);
        let hex = id.to_hex();
        assert_eq!(hex.len(), 64);
        let back = TokenizerId::from_hex(&hex).unwrap();
        assert_eq!(back.as_bytes(), &raw);
    }

    #[test]
    fn tokenizer_id_from_hex_rejects_bad_input() {
        assert!(TokenizerId::from_hex("short").is_err());
        assert!(TokenizerId::from_hex("zz").is_err());
    }

    #[test]
    fn metric_variants() {
        let m = Metric::Cosine;
        assert_eq!(format!("{:?}", m), "Cosine");
    }
```

- [ ] **Step 2: 跑测试确认失败** — `cargo test -p vane-core`，编译失败（TokenizerId/Metric 未定义）。
- [ ] **Step 3: 最小实现** — 在 VaneError 定义之后追加：
```rust
/// 检索结果文档（跨 bm25/vector-brute/fusion 模块）。
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScoredDoc {
    pub docid: u64,
    pub score: f32,
}

/// SPEC §3.1 向量距离度量。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Metric {
    Cosine,
    L2,
    Dot,
}

/// SPEC §5.4 分词器身份标识（sha256 产物）。
/// 结构定义在此（workspace），计算逻辑在 02-tokenizer 的 compute_tokenizer_id。
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct TokenizerId(pub [u8; 32]);

impl TokenizerId {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        let mut s = String::with_capacity(64);
        for b in &self.0 {
            s.push_str(&format!("{:02x}", b));
        }
        s
    }

    pub fn from_hex(s: &str) -> Result<Self> {
        if s.len() != 64 {
            return Err(VaneError::InvalidArg(format!(
                "TokenizerId hex must be 64 chars, got {}",
                s.len()
            )));
        }
        let mut out = [0u8; 32];
        for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
            let hi = hex_val(chunk[0])?;
            let lo = hex_val(chunk[1])?;
            out[i] = hi * 16 + lo;
        }
        Ok(TokenizerId(out))
    }
}

fn hex_val(c: u8) -> Result<u8> {
    match c {
        b'0'..=b'9' => Ok(c - b'0'),
        b'a'..=b'f' => Ok(c - b'a' + 10),
        b'A'..=b'F' => Ok(c - b'A' + 10),
        _ => Err(VaneError::InvalidArg(format!("invalid hex char: {:?}", c as char))),
    }
}
```
- [ ] **Step 4: 跑测试确认通过** — `cargo test -p vane-core`，新增 3 测试全绿。
- [ ] **Step 5: Commit**
```bash
git add -A
git commit -m "feat(core/types): ScoredDoc, Metric, TokenizerId with hex roundtrip

"
```

### Task 4: Schema + FieldDef + validate()
**Files:**
- Modify: `crates/vane-core/src/types.rs`

**Interfaces:**
- Consumes from: Task 2-3（VaneError, Result, Metric）
- Produces: `ScalarKind`、`FieldDef`、`Schema`（含 new/vector_field/text_fields/validate）
- 后续 04-segment-format, 07-api-core, 08-persistence 消费 Schema

- [ ] **Step 1: 写失败测试** — 追加到 tests mod：
```rust
    #[test]
    fn schema_with_single_vector_field_is_valid() {
        let s = Schema::new(vec![
            ("title".into(), FieldDef::Text),
            ("vec".into(), FieldDef::Vector { dim: 384, metric: Metric::Cosine }),
        ]).unwrap();
        assert_eq!(s.vector_field().unwrap().0, "vec");
        assert_eq!(s.vector_field().unwrap().1, 384);
        assert_eq!(s.text_fields(), vec!["title".to_string()]);
    }

    #[test]
    fn schema_with_zero_vector_fields_is_invalid() {
        // SPEC §3.1：恰好一个 vector 字段（M0–M2 限制）
        let r = Schema::new(vec![
            ("body".into(), FieldDef::Text),
        ]);
        assert!(matches!(r, Err(VaneError::Schema(_))));
    }

    #[test]
    fn schema_with_two_vector_fields_is_invalid() {
        let r = Schema::new(vec![
            ("v1".into(), FieldDef::Vector { dim: 128, metric: Metric::Dot }),
            ("v2".into(), FieldDef::Vector { dim: 256, metric: Metric::Cosine }),
        ]);
        assert!(matches!(r, Err(VaneError::Schema(_))));
    }

    #[test]
    fn schema_dim_over_4096_rejected() {
        let r = Schema::new(vec![
            ("v".into(), FieldDef::Vector { dim: 4097, metric: Metric::Cosine }),
        ]);
        assert!(matches!(r, Err(VaneError::Schema(_))));
    }
```

- [ ] **Step 2: 跑测试确认失败** — `cargo test -p vane-core`，编译失败（Schema/FieldDef 未定义）。
- [ ] **Step 3: 最小实现** — 追加到 types.rs（hex_val 之后、tests 之前）：
```rust
/// SPEC §3.1 标量字段类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ScalarKind {
    Int,
    Float,
    Bool,
    Keyword,
}

/// SPEC §3.1 字段定义。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum FieldDef {
    Text,
    Vector { dim: u32, metric: Metric },
    Scalar { kind: ScalarKind },
}

/// SPEC §3.1 Collection schema。创建后仅允许附录式扩展（M0 不实现扩展）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Schema {
    pub fields: Vec<(String, FieldDef)>,
}

impl Schema {
    pub fn new(fields: Vec<(String, FieldDef)>) -> Result<Self> {
        let schema = Self { fields };
        schema.validate()?;
        Ok(schema)
    }

    /// 返回 (name, dim, metric)。§3.1 恰好一个 vector 字段。
    pub fn vector_field(&self) -> Result<(&str, u32, Metric)> {
        let mut found: Option<(&str, u32, Metric)> = None;
        for (name, def) in &self.fields {
            if let FieldDef::Vector { dim, metric } = def {
                if found.is_some() {
                    return Err(VaneError::Schema("multiple vector fields".into()));
                }
                found = Some((name.as_str(), *dim, *metric));
            }
        }
        found.ok_or_else(|| VaneError::Schema("no vector field".into()))
    }

    pub fn text_fields(&self) -> Vec<String> {
        self.fields
            .iter()
            .filter(|(_, d)| matches!(d, FieldDef::Text))
            .map(|(n, _)| n.clone())
            .collect()
    }

    /// §3.1 约束：恰好 1 个 vector 字段；dim ≤ 4096。
    pub fn validate(&self) -> Result<()> {
        let mut vec_count = 0;
        for (_, def) in &self.fields {
            if let FieldDef::Vector { dim, .. } = def {
                vec_count += 1;
                if *dim > DIM_MAX {
                    return Err(VaneError::Schema(format!(
                        "dim {} exceeds max {}", dim, DIM_MAX
                    )));
                }
            }
        }
        // SPEC §3.1：恰好一个 vector 字段（M0–M2 限制）
        if vec_count != 1 {
            return Err(VaneError::Schema(format!(
                "expected exactly 1 vector field, got {}", vec_count
            )));
        }
        Ok(())
    }
}
```
- [ ] **Step 4: 跑测试确认通过** — `cargo test -p vane-core`，4 个新测试全绿。
- [ ] **Step 5: Commit**
```bash
git add -A
git commit -m "feat(core/types): Schema/FieldDef/ScalarKind with §3.1 validation

"
```

### Task 5: 冻结常量
**Files:**
- Modify: `crates/vane-core/src/types.rs`

**Interfaces:**
- Consumes from: 无
- Produces: 全部 SPEC 冻结常量（DIM_MAX/TOPK_MAX/SEGMENT_MAX/DOC_MAX_BYTES/BM25_K1/BM25_B/RRF_K/PAGE_CACHE_DEFAULT_MB/PAGE_SIZE/MAGIC/FORMAT_VERSION/MAX_SEGMENT_DOCS_SMALL）

- [ ] **Step 1: 写失败测试** — 追加到 tests mod：
```rust
    #[test]
    fn frozen_constants_match_spec() {
        assert_eq!(DIM_MAX, 4096);
        assert_eq!(TOPK_MAX, 1000);
        assert_eq!(SEGMENT_MAX, 10);
        assert_eq!(DOC_MAX_BYTES, 16 * 1024 * 1024);
        assert_eq!(BM25_K1, 1.2);
        assert_eq!(BM25_B, 0.75);
        assert_eq!(RRF_K, 60);
        assert_eq!(PAGE_CACHE_DEFAULT_MB, 32);
        assert_eq!(PAGE_SIZE, 64 * 1024);
        assert_eq!(MAGIC, b"VANE");
        assert_eq!(FORMAT_VERSION, 1);
        assert_eq!(MAX_SEGMENT_DOCS_SMALL, 10_000);
    }
```

- [ ] **Step 2: 跑测试确认失败** — `cargo test -p vane-core`，编译失败（常量未定义）。
- [ ] **Step 3: 最小实现** — 在 types.rs 顶部（use 之后）追加：
```rust
// SPEC §3.1/§3.2/§3.3/§4.2/§6.1/§6.2/§6.3/§8.2 冻结常量
pub const DIM_MAX: u32 = 4096;
pub const TOPK_MAX: u32 = 1000;
pub const SEGMENT_MAX: usize = 10;
pub const DOC_MAX_BYTES: usize = 16 * 1024 * 1024;
pub const BM25_K1: f32 = 1.2;
pub const BM25_B: f32 = 0.75;
pub const RRF_K: u32 = 60;
pub const PAGE_CACHE_DEFAULT_MB: u32 = 32;
pub const PAGE_SIZE: usize = 64 * 1024;
pub const MAGIC: &[u8; 4] = b"VANE";
pub const FORMAT_VERSION: u32 = 1;
pub const MAX_SEGMENT_DOCS_SMALL: u32 = 10_000;
```
- [ ] **Step 4: 跑测试确认通过** — `cargo test -p vane-core`，全绿。
- [ ] **Step 5: Commit**
```bash
git add -A
git commit -m "feat(core/types): frozen spec constants (dim/topk/segment/bm25/rrf/etc)

"
```

### Task 6: deny.toml + 依赖黑名单 + 无 std::fs 门禁脚本
**Files:**
- Create: `deny.toml`

**Interfaces:**
- Consumes from: Task 1（workspace 存在）
- Produces: cargo-deny 配置（10-ci-gates 消费）。`scripts/check-no-std-fs.sh` 由 **01-vfs** 计划创建（单一事实源，含 `grep -v 'crates/vane-core/src/vfs/std_fs.rs'` 排除合法文件）。本计划不创建该脚本。

- [ ] **Step 1: 写失败验证** — 创建 `deny.toml`，运行 `cargo deny check bans` 预期黑名单未被检测（因 cargo-deny 未安装则跳过，记录手动审查）。
```toml
# deny.toml
[advisories]
db-path = "~/.cargo/advisory-db"
db-urls = ["https://github.com/rustsec/advisory-db"]
vulnerability = "deny"
unmaintained = "warn"
yanked = "deny"
notice = "warn"

[bans]
multiple-versions = "warn"
deny = [
    { name = "regex" },
    { name = "tokio" },
    { name = "prost" },
    { name = "tonic" },
    { name = "openssl" },
    { name = "lindera" },
    { name = "ndarray" },
    { name = "wee_alloc" },
]

[licenses]
allow = [
    "Apache-2.0",
    "MIT",
    "MIT-0",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "Unicode-DFS-2016",
    "Zlib",
    "CC0-1.0",
]
confidence-threshold = 0.8

[sources]
unknown-registry = "deny"
unknown-git = "deny"
allow-registry = ["https://github.com/rust-lang/crates.io-index"]
```
deny.toml 的唯一事实源在 00-workspace。10-ci-gates 不再重复创建 deny.toml，仅引用本计划产出。如需补 licenses.allow 增量，在 00 维护。

- [ ] **Step 2: 跑脚本确认通过** — 确认 01-vfs 已产出 `scripts/check-no-std-fs.sh`，若尚未产出则跳过此步。
- [ ] **Step 3: 若已安装 cargo-deny，跑 `cargo deny check`** —
```bash
cargo install cargo-deny 2>/dev/null || true
cargo deny check 2>&1 | tail -20
```
- [ ] **Step 4: 验证最终状态** —
```bash
cargo test -p vane-core
cargo build --workspace
```
确认 01-vfs 已产出 `scripts/check-no-std-fs.sh`，若尚未产出则跳过此步。
全绿。
- [ ] **Step 5: Commit**
```bash
git add -A
git commit -m "chore: cargo-deny config (§13.3)

"
```
