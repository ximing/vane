# Tokenizer 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development 执行本计划。步骤用 checkbox `- [ ]` 标记。

**Goal:** 实现 `vane_core::tokenizer` 模块——`Token` 结构、`Tokenizer` trait、`BuiltinTokenizer`/`UserDictEntry` 枚举、`compute_tokenizer_id`（SPEC §5.4 sha256 身份计算）、`build_tokenizer` 工厂，以及 M0 两个内置分词器 `standard`（unicode 分词→lowercase→Porter stemmer）与 `cjk_bigram`（CJK run 切二元组 + 非 CJK 走 standard 管线，position 跨 run 连续递增）。

**Architecture:**
- `tokenizer/mod.rs`：对外类型与工厂。定义 `Token`、`Tokenizer` trait、`BuiltinTokenizer`、`UserDictEntry`、`MAX_USER_DICT_ENTRIES` 常量、`build_tokenizer`、`compute_tokenizer_id`（自 `id` 模块 re-export）。
- `tokenizer/id.rs`：TokenizerId 哈希计算。`algorithm_version(kind)` / `builtin_dict_version(kind)` / `serialize_user_dict(entries)` 三个内部函数 + `compute_tokenizer_id` 公开函数。无平台分支、无 IO。
- `tokenizer/standard.rs`：`StandardTokenizer`（持 `TokenizerId` + `rust_stemmers::Stemmer`）。`unicode_words` 切词 → `to_lowercase` → Porter stem。
- `tokenizer/cjk_bigram.rs`：`CjkBigramTokenizer`。逐 char 分类 CJK/非 CJK → 分 run；CJK run 切重叠二元组（单字 run 退化为 unigram）；非 CJK run 复用 standard 管线（unicode_words→lowercase→stem）；`position: u32` 全程跨 run 单调递增。
- `Tokenizer` trait 对象安全（`Send + Sync`）；`build_tokenizer` 返回 `Box<dyn Tokenizer>`。两个具体分词器零 `cfg`，wasm32 友好。

**Tech Stack:**
- `unicode-segmentation = "1.11"`（`UnicodeSegmentation::unicode_words` 按 Unicode word boundary 切词，纯 Rust、no_std 兼容、wasm32 可用）
- `rust-stemmers = "1.2"`（`Stemmer::create(Algorithm::English)` Porter 英语词干，纯 Rust、`Send + Sync`、wasm32 可用）
- `sha2 = "0.10"`（`Sha256` 摘要，纯 Rust、no_std 兼容、wasm32 可用）
- 依赖黑名单（禁用）：regex / lindera / ndarray（SPEC §13.3）——本模块不引入。

**SPEC 引用:** §5.1（内置分词器表 + 中英混排规则）、§5.3（用户词表上限 10 万、超限 E_DICT_TOO_LARGE）、§5.4（TokenizerId = sha256(algorithm_version ‖ builtin_dict_version ‖ user_dict_bytes)）、§10（E_DICT_UNAVAILABLE=-8 / E_DICT_TOO_LARGE=-7）、§14 不变量 I-4（单一分词身份：相同输入得相同 id；不同 kind/dict 得不同 id）。

**前置依赖:** 00-workspace（`vane_core::types::{TokenizerId, VaneError, Result}`，`TokenizerId(pub [u8;32])` 含 `as_bytes/to_hex/from_hex`；`VaneError::{DictTooLarge, DictUnavailable}`；`type Result<T> = std::result::Result<T, VaneError>`）。

**验收标准:**
- [ ] `compute_tokenizer_id` 对相同 `(kind, user_dict)` 返回字节相同的 `TokenizerId`（确定性，不变量 I-4）。
- [ ] 不同 `kind`（同 user_dict）返回不同 id；不同 `user_dict`（同 kind）返回不同 id（不变量 I-4）。
- [ ] `StandardTokenizer`：`unicode_words` 切分 → lowercase → Porter stem；`position` 从 0 起每 token +1 连续递增；空串返回 `[]`。
- [ ] `CjkBigramTokenizer`：CJK run 切重叠二元组；单字 CJK run 退化为 unigram；非 CJK run 走 standard 管线；`position` 跨 run 连续递增。
- [ ] `build_tokenizer(Standard|CjkBigram, _)` 在 `user_dict.len() <= 100_000` 时返回 `Ok`，且 `tokenizer.id() == &compute_tokenizer_id(kind, user_dict)`。
- [ ] `build_tokenizer(Jieba, _)` 返回 `Err(VaneError::DictUnavailable)`（M0 占位）。
- [ ] `build_tokenizer(_, user_dict)` 当 `user_dict.len() > 100_000` 返回 `Err(VaneError::DictTooLarge)`（SPEC §5.3）。
- [ ] `cargo test -p vane-core` 全绿；`cargo check --target wasm32-unknown-unknown -p vane-core` 通过（无 `std::fs`/`cfg(target)`）。

## Global Constraints

（从 SPEC §13.3 / README 全局约束表复制相关条目）

| 约束 | 值 | 来源 |
|---|---|---|
| 依赖黑名单 | regex / tokio / prost / tonic / openssl / lindera / ndarray / wee_alloc | §13.3 |
| 用户词表上限 | 10 万词条；超限 `E_DICT_TOO_LARGE` | §5.3 |
| M0 jieba | `build_tokenizer(Jieba, _)` 返回 `Err(VaneError::DictUnavailable)` | M0 范围 / §10 |
| core 禁 `std::fs`/`std::net`/mmap | CI 门禁，M0 第一天起 | §6.1/§13.3 |
| `cfg` 只允许在 VFS/Executor 实现 | 核心算法零 `cfg`（不变量 I-5） | §11 |
| BM25 k1/b、RRF k 等 | 本模块不涉及 | — |
| TokenizerId | `sha256(algorithm_version ‖ builtin_dict_version ‖ user_dict_bytes)`，`[u8;32]` | §5.4 |
| Token position | u32，全程连续递增（跨语言 phrase query 依赖，不变量 I-4） | §5.1 |

## File Structure

新建：
- `crates/vane-core/src/tokenizer/mod.rs` —— 对外类型、trait、工厂、常量、re-export。
- `crates/vane-core/src/tokenizer/id.rs` —— `compute_tokenizer_id` 与三个内部辅助函数 + 内联测试。
- `crates/vane-core/src/tokenizer/standard.rs` —— `StandardTokenizer` + 内联测试。
- `crates/vane-core/src/tokenizer/cjk_bigram.rs` —— `CjkBigramTokenizer` + `is_cjk` + 内联测试。

修改：
- `crates/vane-core/src/lib.rs` —— 确认 00-workspace 已在 lib.rs 一次性预声明全部 9 模块（含 `pub mod tokenizer;`），本计划不修改 lib.rs（B1 裁决）。
- `crates/vane-core/Cargo.toml` —— 确认 00-workspace 已在 vane-core Cargo.toml 一次性加入 `unicode-segmentation`、`rust-stemmers`、`sha2`（B1 裁决：00 一次性加全部依赖，后续计划不重复添加，重复键会导致 Cargo 解析失败）。本计划不修改 Cargo.toml。

---

## 任务清单（bite-sized TDD）

### Task 1: Token 结构 + Tokenizer trait + TokenizerId 计算（compute_tokenizer_id + sha256）

**Files:**
- Create: `crates/vane-core/src/tokenizer/mod.rs`
- Create: `crates/vane-core/src/tokenizer/id.rs`
- Modify: 无（确认 00-workspace 已在 vane-core Cargo.toml 加入 `sha2`（B1 裁决，无需重复添加）；00-workspace 已在 lib.rs 一次性预声明全部 9 模块（含 `pub mod tokenizer;`），本计划不修改 lib.rs）

**Interfaces:**
- Consumes from 00-workspace: `TokenizerId(pub [u8;32])`（含 `as_bytes/to_hex/from_hex`）、`VaneError`、`Result`。
- Produces: `Token`、`Tokenizer` trait、`BuiltinTokenizer`、`UserDictEntry`、`MAX_USER_DICT_ENTRIES`、`compute_tokenizer_id`、`serialize_user_dict`（内部）、`algorithm_version`/`builtin_dict_version`（内部）。

- [ ] **Step 1: 写失败测试**

在 `crates/vane-core/src/tokenizer/id.rs` 底部写内联测试（此时尚无实现，编译失败即红）：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokenizer::{BuiltinTokenizer, UserDictEntry};

    #[test]
    fn same_input_same_id() {
        let a = compute_tokenizer_id(BuiltinTokenizer::Standard, &[]);
        let b = compute_tokenizer_id(BuiltinTokenizer::Standard, &[]);
        assert_eq!(a.as_bytes(), b.as_bytes(), "相同输入必须得相同 id");
    }

    #[test]
    fn different_kind_different_id() {
        let std_id = compute_tokenizer_id(BuiltinTokenizer::Standard, &[]);
        let cjk_id = compute_tokenizer_id(BuiltinTokenizer::CjkBigram, &[]);
        let jieba_id = compute_tokenizer_id(BuiltinTokenizer::Jieba, &[]);
        assert_ne!(std_id.as_bytes(), cjk_id.as_bytes());
        assert_ne!(std_id.as_bytes(), jieba_id.as_bytes());
        assert_ne!(cjk_id.as_bytes(), jieba_id.as_bytes());
    }

    #[test]
    fn different_user_dict_different_id() {
        let empty = compute_tokenizer_id(BuiltinTokenizer::Standard, &[]);
        let with_word = compute_tokenizer_id(
            BuiltinTokenizer::Standard,
            &[UserDictEntry::Word("机器学习".to_string())],
        );
        let with_freq = compute_tokenizer_id(
            BuiltinTokenizer::Standard,
            &[UserDictEntry::WordWithFreq { term: "机器学习".to_string(), freq: 100 }],
        );
        assert_ne!(empty.as_bytes(), with_word.as_bytes());
        assert_ne!(with_word.as_bytes(), with_freq.as_bytes());
    }

    #[test]
    fn id_is_32_bytes_and_hex_roundtrip() {
        let id = compute_tokenizer_id(BuiltinTokenizer::CjkBigram, &[]);
        assert_eq!(id.as_bytes().len(), 32);
        let hex = id.to_hex();
        assert_eq!(hex.len(), 64);
        let parsed = TokenizerId::from_hex(&hex).expect("from_hex 必须成功");
        assert_eq!(parsed.as_bytes(), id.as_bytes());
    }

    #[test]
    fn user_dict_order_matters() {
        // 顺序不同 → 序列化不同 → id 不同（Vec 语义）
        let a = compute_tokenizer_id(
            BuiltinTokenizer::Standard,
            &[
                UserDictEntry::Word("a".to_string()),
                UserDictEntry::Word("b".to_string()),
            ],
        );
        let b = compute_tokenizer_id(
            BuiltinTokenizer::Standard,
            &[
                UserDictEntry::Word("b".to_string()),
                UserDictEntry::Word("a".to_string()),
            ],
        );
        assert_ne!(a.as_bytes(), b.as_bytes());
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

```bash
cargo test -p vane-core --lib tokenizer::id
```

预期：编译失败（`compute_tokenizer_id` / `serialize_user_dict` / 引用的符号未定义；`mod tokenizer` 未在 `lib.rs` 声明；`sha2` 依赖未加）。错误信息形如 `cannot find function compute_tokenizer_id in this scope` / `unresolved import crate::tokenizer`。

- [ ] **Step 3: 最小实现**

确认 00-workspace 已在 vane-core Cargo.toml 加入 `sha2`（B1 裁决，无需重复添加）。00-workspace 已在 lib.rs 一次性预声明全部 9 模块（含 `pub mod tokenizer;`），本计划不修改 lib.rs（B1 裁决）。

创建 `crates/vane-core/src/tokenizer/id.rs`：

```rust
//! TokenizerId 计算（SPEC §5.4）。
//! TokenizerId = sha256( algorithm_version ‖ builtin_dict_version ‖ user_dict_bytes )
//!
//! 任何分词算法变更（unicode 边界规则、stemmer 版本、bigram 策略、jieba 词典版本）
//! 必须递增对应 version 标签，从而产生新 TokenizerId 触发 reindex。

use sha2::{Digest, Sha256};

use crate::types::TokenizerId;
use crate::tokenizer::{BuiltinTokenizer, UserDictEntry};

/// 算法版本标签（参与 sha256）。变更分词算法 → 递增此版本 → id 改变。
fn algorithm_version(kind: BuiltinTokenizer) -> &'static [u8] {
    match kind {
        BuiltinTokenizer::Standard => b"std-v1",
        BuiltinTokenizer::CjkBigram => b"cjk-bigram-v1",
        BuiltinTokenizer::Jieba => b"jieba-v1",
    }
}

/// 内置词典版本标签（参与 sha256）。
/// - standard / cjk_bigram：无内置词典，用空串。
/// - jieba：M0 占位空串；M1 接入 jieba-lite 后填词典日历版本（如 b"jieba-lite-2026.08"）。
fn builtin_dict_version(kind: BuiltinTokenizer) -> &'static [u8] {
    match kind {
        BuiltinTokenizer::Standard => b"",
        BuiltinTokenizer::CjkBigram => b"",
        BuiltinTokenizer::Jieba => b"",
    }
}

/// 用户词表的确定性二进制序列化（参与 sha256）。
/// 格式：逐条拼接 ——
///   Word(term)         => 0x00 || u32_le(term.len()) || term_bytes
///   WordWithFreq{..}   => 0x01 || u32_le(term.len()) || term_bytes || u32_le(freq)
pub(crate) fn serialize_user_dict(entries: &[UserDictEntry]) -> Vec<u8> {
    let mut out = Vec::new();
    for e in entries {
        match e {
            UserDictEntry::Word(term) => {
                out.push(0x00);
                let bytes = term.as_bytes();
                out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
                out.extend_from_slice(bytes);
            }
            UserDictEntry::WordWithFreq { term, freq } => {
                out.push(0x01);
                let bytes = term.as_bytes();
                out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
                out.extend_from_slice(bytes);
                out.extend_from_slice(&freq.to_le_bytes());
            }
        }
    }
    out
}

/// 计算 TokenizerId（SPEC §5.4）。
/// sha256( algorithm_version ‖ builtin_dict_version ‖ user_dict_bytes )
pub fn compute_tokenizer_id(
    kind: BuiltinTokenizer,
    user_dict: &[UserDictEntry],
) -> TokenizerId {
    let mut hasher = Sha256::new();
    hasher.update(algorithm_version(kind));
    hasher.update(builtin_dict_version(kind));
    hasher.update(serialize_user_dict(user_dict));
    let hash = hasher.finalize();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&hash);
    TokenizerId(arr)
}
```

创建 `crates/vane-core/src/tokenizer/mod.rs`（本任务只放类型骨架 + id re-export，`build_tokenizer` 留到 Task 4，但为编译通过先放一个最小 `build_tokenizer` 占位会破坏 TDD——此处改为 Task 1 不定义 `build_tokenizer`，仅定义类型与 `compute_tokenizer_id`；Task 4 再补工厂）：

```rust
//! 分词器模块（SPEC §5）。

mod id;

pub use id::compute_tokenizer_id;

use crate::types::TokenizerId;

/// 一个分词结果 token。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub text: String,
    pub position: u32,
}

/// 分词器 trait（对象安全，`Send + Sync`）。
pub trait Tokenizer: Send + Sync {
    /// 对文本分词，返回 token 列表（position 从 0 起单调递增）。
    fn tokenize(&self, text: &str) -> Vec<Token>;
    /// 返回此分词器的身份标识（SPEC §5.4）。
    fn id(&self) -> &TokenizerId;
}

/// 内置分词器种类（SPEC §5.1）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum BuiltinTokenizer {
    Standard,
    CjkBigram,
    Jieba,
}

/// 用户词表条目（SPEC §5.3）。
/// - `Word(term)`：缺省 freq（M0 仅参与 id 计算；M1 jieba 用）。
/// - `WordWithFreq { term, freq }`：显式词频。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum UserDictEntry {
    Word(String),
    WordWithFreq { term: String, freq: u32 },
}

/// 用户词表上限（SPEC §5.3：10 万词条）。
pub const MAX_USER_DICT_ENTRIES: usize = 100_000;
```

- [ ] **Step 4: 跑测试确认通过**

```bash
cargo test -p vane-core --lib tokenizer::id
```

预期：5 个测试全绿（`same_input_same_id` / `different_kind_different_id` / `different_user_dict_different_id` / `id_is_32_bytes_and_hex_roundtrip` / `user_dict_order_matters`）。

附带验证 wasm32 通过（不变量 I-5）：

```bash
cargo check --target wasm32-unknown-unknown -p vane-core
```

- [ ] **Step 5: Commit**

```bash
git add crates/vane-core/src/tokenizer/mod.rs crates/vane-core/src/tokenizer/id.rs
git commit -m "feat(tokenizer): Token/Tokenizer trait + compute_tokenizer_id (sha256)

- 定义 Token、Tokenizer trait、BuiltinTokenizer、UserDictEntry、MAX_USER_DICT_ENTRIES
- 实现 compute_tokenizer_id = sha256(algorithm_version ‖ builtin_dict_version ‖ user_dict_bytes)
- serialize_user_dict 确定性二进制序列化（tag + len + bytes + freq）
- 覆盖不变量 I-4：相同输入同 id；不同 kind/dict 不同 id

"
```

---

### Task 2: Standard 分词器（unicode 分词 → lowercase → Porter stemmer，position 连续）

**Files:**
- Create: `crates/vane-core/src/tokenizer/standard.rs`
- Modify: `crates/vane-core/src/tokenizer/mod.rs`（加 `mod standard;` + `pub(crate) use` 给内部）
- Modify: 无（确认 00-workspace 已在 vane-core Cargo.toml 加入 `unicode-segmentation`、`rust-stemmers`（B1 裁决，无需重复添加））

**Interfaces:**
- Consumes from 00-workspace: `TokenizerId`。
- Consumes from Task 1: `Token`、`Tokenizer` trait、`BuiltinTokenizer`、`compute_tokenizer_id`。
- Produces: `StandardTokenizer`（`pub(crate)`，由 Task 4 工厂装配）。

- [ ] **Step 1: 写失败测试**

在 `crates/vane-core/src/tokenizer/standard.rs` 底部写内联测试：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokenizer::{BuiltinTokenizer, Tokenizer, compute_tokenizer_id};

    fn tok() -> StandardTokenizer {
        StandardTokenizer::new(&[])
    }

    #[test]
    fn empty_text_returns_empty() {
        let t = tok();
        assert!(t.tokenize("").is_empty());
    }

    #[test]
    fn lowercase_and_stem() {
        // "Running" -> lower "running" -> Porter stem "run"
        // "RUNNERS" -> lower "runners" -> Porter stem "runner"
        let t = tok();
        let toks = t.tokenize("Running RUNNERS");
        assert_eq!(toks.len(), 2);
        assert_eq!(toks[0].text, "run");
        assert_eq!(toks[1].text, "runner");
    }

    #[test]
    fn positions_continuous_from_zero() {
        let t = tok();
        let toks = t.tokenize("the quick brown fox");
        assert_eq!(toks.len(), 4);
        for (i, tk) in toks.iter().enumerate() {
            assert_eq!(tk.position, i as u32, "position 必须从 0 连续递增");
        }
    }

    #[test]
    fn punctuation_and_whitespace_dropped() {
        let t = tok();
        let toks = t.tokenize("hello, world!  \t foo-bar");
        // unicode_words 把 "foo-bar" 当作一个词（连字符不切），stem 后 "foo-bar" 不被 Porter 规则收缩
        // 这里只断言 token 数与关键 stem 结果，避免对 stemmer 边界过度耦合
        assert!(toks.len() >= 3);
        assert_eq!(toks[0].text, "hello");
        assert_eq!(toks[1].text, "world");
    }

    #[test]
    fn digits_preserved_as_token() {
        let t = tok();
        let toks = t.tokenize("vane 2026 release");
        assert_eq!(toks.len(), 3);
        assert_eq!(toks[1].text, "2026"); // 数字不被 stemmer 改写
    }

    #[test]
    fn id_matches_compute() {
        let t = StandardTokenizer::new(&[]);
        let expected = compute_tokenizer_id(BuiltinTokenizer::Standard, &[]);
        assert_eq!(t.id().as_bytes(), expected.as_bytes());
    }

    #[test]
    fn id_reflects_user_dict() {
        use crate::tokenizer::UserDictEntry;
        let t_empty = StandardTokenizer::new(&[]);
        let t_with = StandardTokenizer::new(&[UserDictEntry::Word("xyz".to_string())]);
        assert_ne!(t_empty.id().as_bytes(), t_with.id().as_bytes());
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

```bash
cargo test -p vane-core --lib tokenizer::standard
```

预期：编译失败——`StandardTokenizer` 未定义；`unicode-segmentation` / `rust-stemmers` 依赖未加。错误形如 `cannot find type StandardTokenizer in this scope`。

- [ ] **Step 3: 最小实现**

确认 00-workspace 已在 vane-core Cargo.toml 加入 `unicode-segmentation`、`rust-stemmers`（B1 裁决，无需重复添加）。

修改 `crates/vane-core/src/tokenizer/mod.rs`，在 `mod id;` 之后加：

```rust
mod standard;
```

创建 `crates/vane-core/src/tokenizer/standard.rs`：

```rust
//! Standard 分词器（SPEC §5.1）：unicode 分词 → lowercase → Porter stemmer。

use unicode_segmentation::UnicodeSegmentation;
use rust_stemmers::{Algorithm, Stemmer};

use crate::types::TokenizerId;
use crate::tokenizer::{compute_tokenizer_id, BuiltinTokenizer, Token, Tokenizer};

pub(crate) struct StandardTokenizer {
    id: TokenizerId,
    stemmer: Stemmer,
}

impl StandardTokenizer {
    /// `user_dict` 在 M0 不参与 standard 的切分逻辑，仅影响 TokenizerId（SPEC §5.3/§5.4）。
    pub(crate) fn new(user_dict: &[crate::tokenizer::UserDictEntry]) -> Self {
        Self {
            id: compute_tokenizer_id(BuiltinTokenizer::Standard, user_dict),
            stemmer: Stemmer::create(Algorithm::English),
        }
    }
}

impl Tokenizer for StandardTokenizer {
    fn tokenize(&self, text: &str) -> Vec<Token> {
        let mut tokens = Vec::new();
        let mut position: u32 = 0;
        for word in text.unicode_words() {
            let lower = word.to_lowercase();
            let stemmed = self.stemmer.stem(&lower);
            tokens.push(Token {
                text: stemmed.into_owned(),
                position,
            });
            position += 1;
        }
        tokens
    }

    fn id(&self) -> &TokenizerId {
        &self.id
    }
}
```

- [ ] **Step 4: 跑测试确认通过**

```bash
cargo test -p vane-core --lib tokenizer::standard
```

预期：7 个测试全绿。再跑全模块与 wasm32 check：

```bash
cargo test -p vane-core --lib tokenizer
cargo check --target wasm32-unknown-unknown -p vane-core
```

- [ ] **Step 5: Commit**

```bash
git add crates/vane-core/src/tokenizer/standard.rs crates/vane-core/src/tokenizer/mod.rs
git commit -m "feat(tokenizer): StandardTokenizer (unicode_words + lowercase + Porter stem)

- unicode_segmentation 按 Unicode word boundary 切词
- to_lowercase 后喂 rust_stemmers Porter 英语词干
- position 从 0 起每 token +1 连续递增
- TokenizerId 经 compute_tokenizer_id 反映 user_dict

"
```

---

### Task 3: CJK bigram 分词器（CJK run 二元组 + 非 CJK 走 standard + position 跨 run 连续）

**Files:**
- Create: `crates/vane-core/src/tokenizer/cjk_bigram.rs`
- Modify: `crates/vane-core/src/tokenizer/mod.rs`（加 `mod cjk_bigram;`）

**Interfaces:**
- Consumes from Task 1/2: `Token`、`Tokenizer` trait、`compute_tokenizer_id`、`BuiltinTokenizer`、`rust_stemmers::Stemmer`。
- Produces: `CjkBigramTokenizer`（`pub(crate)`）、`is_cjk`（内部）。

- [ ] **Step 1: 写失败测试**

在 `crates/vane-core/src/tokenizer/cjk_bigram.rs` 底部写内联测试：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokenizer::{BuiltinTokenizer, Tokenizer, compute_tokenizer_id};

    fn tok() -> CjkBigramTokenizer {
        CjkBigramTokenizer::new(&[])
    }

    #[test]
    fn pure_cjk_bigrams() {
        // "机器学习" (4 字) → 重叠二元组: 机器 / 器学 / 学习
        let t = tok();
        let toks = t.tokenize("机器学习");
        assert_eq!(toks.len(), 3);
        assert_eq!(toks[0].text, "机器");
        assert_eq!(toks[1].text, "器学");
        assert_eq!(toks[2].text, "学习");
        assert_eq!(toks[0].position, 0);
        assert_eq!(toks[1].position, 1);
        assert_eq!(toks[2].position, 2);
    }

    #[test]
    fn single_cjk_char_is_unigram() {
        // 单字 CJK run 退化为 unigram（无二元组可切）
        let t = tok();
        let toks = t.tokenize("中");
        assert_eq!(toks.len(), 1);
        assert_eq!(toks[0].text, "中");
        assert_eq!(toks[0].position, 0);
    }

    #[test]
    fn two_cjk_chars_one_bigram() {
        let t = tok();
        let toks = t.tokenize("世界");
        assert_eq!(toks.len(), 1);
        assert_eq!(toks[0].text, "世界");
    }

    #[test]
    fn non_cjk_run_uses_standard_pipeline() {
        // "Running" 是 Latin run → lowercase + Porter stem → "run"
        let t = tok();
        let toks = t.tokenize("Running");
        assert_eq!(toks.len(), 1);
        assert_eq!(toks[0].text, "run");
    }

    #[test]
    fn mixed_cjk_and_latin_positions_continuous() {
        // "hello 世界 ok" →
        //   非 CJK run "hello " → standard: "hello" (stem "hello", position 0)
        //   CJK run "世界" → bigram "世界" (position 1)
        //   非 CJK run " ok" → standard: "ok" (position 2)
        let t = tok();
        let toks = t.tokenize("hello 世界 ok");
        assert_eq!(toks.len(), 3);
        assert_eq!(toks[0].text, "hello");
        assert_eq!(toks[0].position, 0);
        assert_eq!(toks[1].text, "世界");
        assert_eq!(toks[1].position, 1);
        assert_eq!(toks[2].text, "ok");
        assert_eq!(toks[2].position, 2);
    }

    #[test]
    fn multiple_cjk_runs_keep_positions_continuous() {
        // "中a文" → CJK run "中"(unigram, pos0) + 非CJK "a"(pos1) + CJK run "文"(unigram, pos2)
        let t = tok();
        let toks = t.tokenize("中a文");
        assert_eq!(toks.len(), 3);
        assert_eq!(toks[0].text, "中");
        assert_eq!(toks[0].position, 0);
        assert_eq!(toks[1].text, "a");
        assert_eq!(toks[1].position, 1);
        assert_eq!(toks[2].text, "文");
        assert_eq!(toks[2].position, 2);
    }

    #[test]
    fn empty_text_returns_empty() {
        let t = tok();
        assert!(t.tokenize("").is_empty());
    }

    #[test]
    fn id_matches_compute() {
        let t = CjkBigramTokenizer::new(&[]);
        let expected = compute_tokenizer_id(BuiltinTokenizer::CjkBigram, &[]);
        assert_eq!(t.id().as_bytes(), expected.as_bytes());
    }

    #[test]
    fn is_cjk_covers_common_ranges() {
        assert!(is_cjk('汉'));      // U+6C49 CJK 统一
        assert!(is_cjk('あ'));      // U+3042 平假名
        assert!(is_cjk('カ'));      // U+30AB 片假名
        assert!(!is_cjk('a'));
        assert!(!is_cjk(' '));
        assert!(!is_cjk('1'));
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

```bash
cargo test -p vane-core --lib tokenizer::cjk_bigram
```

预期：编译失败——`CjkBigramTokenizer` / `is_cjk` 未定义；`mod cjk_bigram` 未声明。

- [ ] **Step 3: 最小实现**

修改 `crates/vane-core/src/tokenizer/mod.rs`，在 `mod standard;` 之后加：

```rust
mod cjk_bigram;
```

创建 `crates/vane-core/src/tokenizer/cjk_bigram.rs`：

```rust
//! CJK bigram 分词器（SPEC §5.1）。
//! 先按 unicode script 边界切 run：CJK run 切重叠二元组（单字 run 退化为 unigram）；
//! 非 CJK run 走 standard 管线（unicode_words → lowercase → Porter stem）。
//! token position 全程跨 run 连续递增（不变量 I-4，跨语言 phrase query 正确性依赖）。

use unicode_segmentation::UnicodeSegmentation;
use rust_stemmers::{Algorithm, Stemmer};

use crate::types::TokenizerId;
use crate::tokenizer::{compute_tokenizer_id, BuiltinTokenizer, Token, Tokenizer};

pub(crate) struct CjkBigramTokenizer {
    id: TokenizerId,
    stemmer: Stemmer,
}

impl CjkBigramTokenizer {
    pub(crate) fn new(user_dict: &[crate::tokenizer::UserDictEntry]) -> Self {
        Self {
            id: compute_tokenizer_id(BuiltinTokenizer::CjkBigram, user_dict),
            stemmer: Stemmer::create(Algorithm::English),
        }
    }
}

impl Tokenizer for CjkBigramTokenizer {
    fn tokenize(&self, text: &str) -> Vec<Token> {
        let mut tokens = Vec::new();
        let mut position: u32 = 0;

        let chars: Vec<char> = text.chars().collect();
        let n = chars.len();
        let mut i = 0usize;
        while i < n {
            if is_cjk(chars[i]) {
                // 收集连续 CJK run
                let start = i;
                while i < n && is_cjk(chars[i]) {
                    i += 1;
                }
                let run: String = chars[start..i].iter().collect();
                emit_cjk_run(&run, &mut tokens, &mut position);
            } else {
                // 收集连续非 CJK run
                let start = i;
                while i < n && !is_cjk(chars[i]) {
                    i += 1;
                }
                let run: String = chars[start..i].iter().collect();
                // 非 CJK 走 standard 管线
                for word in run.unicode_words() {
                    let lower = word.to_lowercase();
                    let stemmed = self.stemmer.stem(&lower);
                    tokens.push(Token {
                        text: stemmed.into_owned(),
                        position,
                    });
                    position += 1;
                }
            }
        }
        tokens
    }

    fn id(&self) -> &TokenizerId {
        &self.id
    }
}

/// 对一个 CJK run 切重叠二元组；单字 run 退化为 unigram。
fn emit_cjk_run(run: &str, tokens: &mut Vec<Token>, position: &mut u32) {
    let cjk_chars: Vec<char> = run.chars().collect();
    if cjk_chars.is_empty() {
        return;
    }
    if cjk_chars.len() == 1 {
        tokens.push(Token {
            text: cjk_chars[0].to_string(),
            position: *position,
        });
        *position += 1;
        return;
    }
    for w in cjk_chars.windows(2) {
        let bigram: String = w.iter().collect();
        tokens.push(Token {
            text: bigram,
            position: *position,
        });
        *position += 1;
    }
}

/// 判断一个字符是否属于 CJK 表意文字/假名范围（用于 run 切分）。
fn is_cjk(c: char) -> bool {
    let cp = c as u32;
    matches!(cp,
        0x3000..=0x303F   // CJK 符号和标点
        | 0x3040..=0x309F // 平假名
        | 0x30A0..=0x30FF // 片假名
        | 0x3400..=0x4DBF // CJK Ext A
        | 0x4E00..=0x9FFF // CJK 统一表意文字
        | 0xF900..=0xFAFF // CJK 兼容表意文字
        | 0x20000..=0x2A6DF // CJK Ext B
        | 0x2A700..=0x2B73F // CJK Ext C
        | 0x2B740..=0x2B81F // CJK Ext D
        | 0x2B820..=0x2CEAF // CJK Ext E
    )
}
```

- [ ] **Step 4: 跑测试确认通过**

```bash
cargo test -p vane-core --lib tokenizer::cjk_bigram
```

预期：9 个测试全绿。再跑全模块 + wasm32：

```bash
cargo test -p vane-core --lib tokenizer
cargo check --target wasm32-unknown-unknown -p vane-core
```

- [ ] **Step 5: Commit**

```bash
git add crates/vane-core/src/tokenizer/cjk_bigram.rs crates/vane-core/src/tokenizer/mod.rs
git commit -m "feat(tokenizer): CjkBigramTokenizer (CJK run bigram + non-CJK standard pipeline)

- 按 is_cjk 把文本切成 CJK / 非 CJK 交替 run
- CJK run 切重叠二元组（单字 run 退化为 unigram）
- 非 CJK run 复用 standard 管线（unicode_words + lowercase + Porter stem）
- position 跨 run 连续递增（不变量 I-4，跨语言 phrase query 依赖）

"
```

---

### Task 4: build_tokenizer 工厂 + Jieba 返回 DictUnavailable + user_dict 超限 E_DICT_TOO_LARGE

**Files:**
- Modify: `crates/vane-core/src/tokenizer/mod.rs`（补 `build_tokenizer` 工厂 + 工厂内联测试）

**Interfaces:**
- Consumes from Task 1/2/3: `StandardTokenizer`、`CjkBigramTokenizer`、`MAX_USER_DICT_ENTRIES`、`VaneError`、`Result`。
- Produces: `build_tokenizer`（公开工厂，契约签名冻结）。

- [ ] **Step 1: 写失败测试**

在 `crates/vane-core/src/tokenizer/mod.rs` 底部追加内联测试（`build_tokenizer` 尚未定义 → 编译失败）：

```rust
#[cfg(test)]
mod factory_tests {
    use super::*;
    use crate::types::VaneError;

    #[test]
    fn build_standard_ok_and_id_matches() {
        let t = build_tokenizer(BuiltinTokenizer::Standard, &[]).expect("standard 必须成功");
        let expected = compute_tokenizer_id(BuiltinTokenizer::Standard, &[]);
        assert_eq!(t.id().as_bytes(), expected.as_bytes());
        // 可调用 tokenize
        let toks = t.tokenize("Running");
        assert_eq!(toks.len(), 1);
        assert_eq!(toks[0].text, "run");
    }

    #[test]
    fn build_cjk_bigram_ok_and_id_matches() {
        let t = build_tokenizer(BuiltinTokenizer::CjkBigram, &[]).expect("cjk_bigram 必须成功");
        let expected = compute_tokenizer_id(BuiltinTokenizer::CjkBigram, &[]);
        assert_eq!(t.id().as_bytes(), expected.as_bytes());
        let toks = t.tokenize("机器学习");
        assert_eq!(toks.len(), 3);
    }

    #[test]
    fn build_jieba_returns_dict_unavailable() {
        let err = build_tokenizer(BuiltinTokenizer::Jieba, &[]).unwrap_err();
        assert!(matches!(err, VaneError::DictUnavailable), "M0 jieba 必须返回 DictUnavailable，实际: {:?}", err);
        assert_eq!(err.code(), -8); // SPEC §10: E_DICT_UNAVAILABLE = -8
    }

    #[test]
    fn build_with_user_dict_ok_when_under_limit() {
        let dict: Vec<UserDictEntry> = (0..100_000)
            .map(|i| UserDictEntry::Word(format!("w{}", i)))
            .collect();
        let t = build_tokenizer(BuiltinTokenizer::Standard, &dict).expect("10 万词条必须通过");
        let expected = compute_tokenizer_id(BuiltinTokenizer::Standard, &dict);
        assert_eq!(t.id().as_bytes(), expected.as_bytes());
    }

    #[test]
    fn build_rejects_dict_over_limit() {
        let dict: Vec<UserDictEntry> = (0..=100_000)
            .map(|i| UserDictEntry::Word(format!("w{}", i)))
            .collect();
        assert_eq!(dict.len(), 100_001);
        let err = build_tokenizer(BuiltinTokenizer::Standard, &dict).unwrap_err();
        assert!(matches!(err, VaneError::DictTooLarge), "超限必须返回 DictTooLarge，实际: {:?}", err);
        assert_eq!(err.code(), -7); // SPEC §10: E_DICT_TOO_LARGE = -7
    }

    #[test]
    fn build_jieba_with_over_limit_dict_returns_dict_too_large_first() {
        // 词表上限检查优先于 jieba 可用性检查（输入校验先于资源校验）
        let dict: Vec<UserDictEntry> = (0..=100_000)
            .map(|i| UserDictEntry::Word(format!("w{}", i)))
            .collect();
        let err = build_tokenizer(BuiltinTokenizer::Jieba, &dict).unwrap_err();
        assert!(matches!(err, VaneError::DictTooLarge));
    }

    #[test]
    fn built_tokenizer_is_send_sync() {
        // Box<dyn Tokenizer> 必须是 Send + Sync（trait 约束已保证，此处编译期断言）
        fn assert_send_sync<T: Send + Sync>(_: &T) {}
        let t = build_tokenizer(BuiltinTokenizer::Standard, &[]).unwrap();
        assert_send_sync(&t);
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

```bash
cargo test -p vane-core --lib tokenizer::factory_tests
```

预期：编译失败——`build_tokenizer` 未定义。错误形如 `cannot find function build_tokenizer in this scope`。

- [ ] **Step 3: 最小实现**

修改 `crates/vane-core/src/tokenizer/mod.rs`，在 `MAX_USER_DICT_ENTRIES` 常量定义之后追加工厂函数：

```rust
/// 工厂：构建内置分词器（SPEC §5.1 / §5.3 / §10）。
///
/// - `Standard` / `CjkBigram`：M0 完整实现。
/// - `Jieba`：M0 返回 `Err(VaneError::DictUnavailable)`（M1 实现）。
/// - `user_dict.len() > 100_000`：返回 `Err(VaneError::DictTooLarge)`（SPEC §5.3），优先于 jieba 可用性。
pub fn build_tokenizer(
    kind: BuiltinTokenizer,
    user_dict: &[UserDictEntry],
) -> crate::types::Result<Box<dyn Tokenizer>> {
    use crate::types::VaneError;

    if user_dict.len() > MAX_USER_DICT_ENTRIES {
        return Err(VaneError::DictTooLarge);
    }

    match kind {
        BuiltinTokenizer::Standard => Ok(Box::new(standard::StandardTokenizer::new(user_dict))),
        BuiltinTokenizer::CjkBigram => Ok(Box::new(cjk_bigram::CjkBigramTokenizer::new(user_dict))),
        BuiltinTokenizer::Jieba => Err(VaneError::DictUnavailable),
    }
}
```

注意：`standard` 与 `cjk_bigram` 模块需对 `mod.rs` 可见——它们已声明为 `mod standard;` / `mod cjk_bigram;`（私有模块），`StandardTokenizer::new` / `CjkBigramTokenizer::new` 标记为 `pub(crate)`，因此 `mod.rs` 内可直接调用。无需改可见性。

- [ ] **Step 4: 跑测试确认通过**

```bash
cargo test -p vane-core --lib tokenizer
```

预期：`factory_tests` 7 个测试 + 前 3 个任务测试全绿。再跑 wasm32 与 clippy：

```bash
cargo check --target wasm32-unknown-unknown -p vane-core
cargo clippy -p vane-core -- -D warnings
```

- [ ] **Step 5: Commit**

```bash
git add crates/vane-core/src/tokenizer/mod.rs
git commit -m "feat(tokenizer): build_tokenizer factory + M0 boundary checks

- build_tokenizer(Standard|CjkBigram, _) 返回 Ok(Box<dyn Tokenizer>)
- build_tokenizer(Jieba, _) 返回 Err(DictUnavailable)（M0 占位，§10）
- user_dict.len() > 100_000 返回 Err(DictTooLarge)（§5.3），优先于 jieba 检查
- 词表上限检查与 jieba 可用性检查的优先级由测试固化

"
```

---

## 完成后整体验收

执行以下命令，全部通过即本计划交付完成：

```bash
# 1. 全量测试
cargo test -p vane-core

# 2. wasm32 编译门禁（不变量 I-5：core 零平台分支）
cargo check --target wasm32-unknown-unknown -p vane-core

# 3. clippy 零告警
cargo clippy -p vane-core -- -D warnings

# 4. 确认 core 无 std::fs / cfg(target)（手动抽查）
! grep -rnE 'std::fs|cfg\(target' crates/vane-core/src/
```

不变量 I-4 覆盖汇总（由上述测试覆盖）：
- 相同 `(kind, user_dict)` → 相同 id：`id::tests::same_input_same_id`、`factory_tests::build_standard_ok_and_id_matches`。
- 不同 `kind` → 不同 id：`id::tests::different_kind_different_id`。
- 不同 `user_dict` → 不同 id：`id::tests::different_user_dict_different_id`、`id::tests::user_dict_order_matters`、`standard::tests::id_reflects_user_dict`。
- `tokenizer.id()` 与 `compute_tokenizer_id(kind, user_dict)` 一致：`standard::tests::id_matches_compute`、`cjk_bigram::tests::id_matches_compute`、`factory_tests::build_*_ok_and_id_matches`。
