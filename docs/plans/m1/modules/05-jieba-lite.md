# 05-jieba-lite：jieba 算法内核 + 精简词典 DAT+zstd + 中英混排

> SPEC 引用：§5.1（jieba 管线）、§5.2（词典格式）、§5.3（自定义词表优先级）、§5.4（TokenizerId 含词典版本）、§13.2-2（中文分词四项验收）、§13.1（词典冷加载 <150ms）。
> 前置依赖：M0 `tokenizer`（已核查 git HEAD：`Tokenizer` trait、`build_tokenizer`、`compute_tokenizer_id`、`id.rs`）。
> M1 README 契约：`vane_core::tokenizer::jieba`（feature `jieba`）。

## Goal

实现 jieba 算法内核（前缀 DAG 最大概率切分 + HMM 未登录词识别），算法与 jieba-rs 完全一致，仅裁词典（红线）。精简词典 ~20 万词，双数组 Trie（DAT）序列化 + zstd 压缩 ≤1.5MB。中英混排按 script 边界切 run。词典永不进 wasm（红线）。

## Architecture

- **算法层**（`tokenizer/jieba/seg.rs`）：
  - 前缀 DAG：对 CJK run 构建前缀词典 DAG，节点 = 字符位置，边 = 词典命中词，权重 = -log(freq/total)。最大概率路径 = 最短路径（Dijkstra 或动态规划）。
  - HMM 未登录词：对 DAG 中未命中的连续单字，用 HMM（B/M/E/S 四状态，Viterbi 解码）识别新词。转移矩阵 + 发射矩阵随 dict.bin 同包。
  - **算法与 jieba-rs 逐字一致**——不发明切分规则。
- **TokenizerId（R-3，推翻方案 A）**：`builtin_dict_version(Jieba)` = 编译期格式常量 `b"jieba-fmt-v1"`（SPEC v1.1 §5.4）。仅当 DAT 结构 / HMM 参数**格式**变更时递增；词典**内容**升级（增删词条、日历版本变）**不改变** `builtin_dict_version`，故不改变 TokenizerId（满足 REQUIREMENTS §3.3「词典升级仅警告不强制重建」）。`JiebaTokenizer::id()` **直接用 `compute_tokenizer_id(Jieba, user_dict)`，无二次哈希**。词典日历版本 + sha256_prefix 仍存 dict.bin 头 + CollectionMeta，供 §12.3 三渠道一致性 + §3.3 升级警告，**不进 TokenizerId**。
- **词典层**（`tokenizer/jieba/dict.rs`）：
  - `JiebaDict`：DAT（双数组 Trie，base/check 数组）+ 词频表 + HMM 参数 + 词典版本 + sha256 前 8 字节。
  - `load(bytes)`：解析 dict.bin（已解压），零拷贝引用 `bytes` 切片（<150ms 冷加载）。
  - 用户词表合并：用户词 > 内置词；同用户词 freq 高者优先；歧义消解保持 jieba 原版（DAG 优先命中用户词）。
- **混排层**（`tokenizer/jieba/mod.rs`）：
  - 复用 M0 `cjk_bigram.rs` 的 `is_cjk` + run 切分逻辑。CJK run 进 DAG+HMM；非 CJK run 进 standard 管线（lowercase + Porter stem）。position 跨 run 连续递增（不变量 I-4）。
- **feature 隔离**：`vane-core/Cargo.toml` 增 `[features] jieba = ["ruzstd"]`。`tokenizer/mod.rs` 的 `build_tokenizer` 在 `Jieba` 分支 `cfg(feature="jieba")` 实装，否则仍返回 `DictUnavailable`（wasm32 永不启用 jieba feature）。
- **dict.bin 生成**（`crates/vane-dict-zh/build.rs` 或离线脚本）：从 jieba 开源词表剪枝 ~20 万词 → 构建 DAT → zstd 压缩 → 写 `data/dict.bin`。生成脚本不在 core，在 07 计划。

## 涉及文件

- **Create**：
  - `crates/vane-core/src/tokenizer/jieba/mod.rs`（JiebaTokenizer + 混排）
  - `crates/vane-core/src/tokenizer/jieba/dict.rs`（JiebaDict + DAT load）
  - `crates/vane-core/src/tokenizer/jieba/seg.rs`（DAG + HMM Viterbi）
  - `crates/vane-core/src/tokenizer/jieba/hmm.rs`（HMM 参数 + Viterbi）
  - `crates/vane-core/src/tokenizer/jieba/tests.rs`
  - `crates/vane-core/src/tokenizer/jieba/test_fixture.dict.bin`（小规模测试词典，手工生成 ~100 词）
- **Modify**：
  - `crates/vane-core/Cargo.toml`（增 `ruzstd` 可选依赖 + `[features] jieba`）
  - `crates/vane-core/src/tokenizer/mod.rs`（`build_tokenizer` Jieba 分支 cfg；re-export jieba 模块）
  - `crates/vane-core/src/tokenizer/id.rs`（`builtin_dict_version(Jieba)` 从 M0 占位 `b""` 改为编译期格式常量 `b"jieba-fmt-v1"`；修正第 23 行注释「日历版本」→「格式版本」+ 模块文档注释第 4-5 行「jieba 词典版本」→「jieba 词典**格式**版本」——R-3，实装时改）

## Interfaces

### Consumes from M0（已核查 git HEAD）

```rust
// crates/vane-core/src/tokenizer/mod.rs
pub struct Token { pub text: String, pub position: u32 }
pub trait Tokenizer: Send + Sync {
    fn tokenize(&self, text: &str) -> Vec<Token>;
    fn id(&self) -> &TokenizerId;
}
pub enum BuiltinTokenizer { Standard, CjkBigram, Jieba }
pub enum UserDictEntry { Word(String), WordWithFreq { term: String, freq: u32 } }
pub const MAX_USER_DICT_ENTRIES: usize = 100_000;
pub fn build_tokenizer(kind: BuiltinTokenizer, user_dict: &[UserDictEntry]) -> Result<Box<dyn Tokenizer>>;

// crates/vane-core/src/tokenizer/id.rs
pub fn compute_tokenizer_id(kind: BuiltinTokenizer, user_dict: &[UserDictEntry]) -> TokenizerId;
// 内部 fn builtin_dict_version(kind) -> &'static [u8]  当前 Jieba 返回 b""（M0 占位）

// crates/vane-core/src/tokenizer/cjk_bigram.rs
fn is_cjk(c: char) -> bool;  // 复用 run 切分
```

### Produces（见 README § 05-jieba-lite 契约）

**TokenizerId 签名说明**（R-3，推翻方案 A）：
- `compute_tokenizer_id(kind, user_dict)` 公开签名不变（M0 冻结）。
- `id.rs::builtin_dict_version(Jieba)` 从 M0 占位 `b""` 改为编译期格式常量 `b"jieba-fmt-v1"`（返回类型仍是 `&'static [u8]`，无需改签名）。仅当 DAT/HMM **格式**变更时递增；词典内容升级不变。
- `JiebaTokenizer::id()` **直接用 `compute_tokenizer_id(Jieba, user_dict)`，无二次哈希**。旧方案 A（`sha256(compute_tokenizer_id(...).as_bytes() || dict.version() || sha256_prefix)`）使 TokenizerId 依赖词典内容，词典升级→TokenizerId 变→E_TOKENIZER_MISMATCH→实质强制重建，违反 REQUIREMENTS §3.3，**否决**。
- 词典日历版本 `dict.version()` + `sha256_prefix()` 存 dict.bin 头 + CollectionMeta，供 §12.3 三渠道一致性校验 + §3.3 升级警告，**不进 TokenizerId**。

## TDD 任务清单

### Task 1：feature 门控 + JiebaDict 骨架（无词典降级）

**测试**（`crates/vane-core/src/tokenizer/jieba/tests.rs`）：
```rust
#![cfg(feature = "jieba")]
use super::*;
use crate::tokenizer::{BuiltinTokenizer, Tokenizer};

#[test]
fn jieba_dict_load_empty_fails() {
    // 空 bytes 解析失败
    let r = JiebaDict::load(&[]);
    assert!(r.is_err());
}

#[test]
fn jieba_dict_load_valid_header() {
    // 手工构造最小 dict.bin（magic+version+sha256+version_str+0 words+空 dat+空 hmm）
    let bytes = minimal_dict_bin();
    let d = JiebaDict::load(&bytes).expect("load");
    assert_eq!(d.version(), "2026.08");
}
```
验证失败：`cargo test -p vane-core --features jieba jieba` 编译错误。
最小实现：
- `Cargo.toml`：`ruzstd = { version = "0.5", optional = true }` + `[features] jieba = ["ruzstd"]`。
- `tokenizer/mod.rs`：`#[cfg(feature = "jieba")] pub mod jieba;`
- `dict.rs`：`JiebaDict::load` 解析头（magic/version/sha256_prefix/dict_version/num_words/dat_blob/hmm_blob）。`minimal_dict_bin()` 测试辅助构造合法空词典。
commit：`jieba: add feature-gated dict loader skeleton`。

### Task 2：DAT 查询 + 词频

**测试**：
```rust
#[test]
fn dict_lookup_word_freq() {
    let bytes = dict_bin_with_words(&[("机器学习", 100), ("学习", 200), ("机器", 50)]);
    let d = JiebaDict::load(&bytes).unwrap();
    assert_eq!(d.freq("机器学习"), Some(100));
    assert_eq!(d.freq("学习"), Some(200));
    assert_eq!(d.freq("不存在词"), None);
}

#[test]
fn dict_prefix_match() {
    let bytes = dict_bin_with_words(&[("机器学习", 100), ("机器", 50)]);
    let d = JiebaDict::load(&bytes).unwrap();
    // "机器学习" 前缀含 "机器"
    let prefixes = d.common_prefix_search("机器学习");
    assert!(prefixes.contains(&"机器".to_string()));
    assert!(prefixes.contains(&"机器学习".to_string()));
}
```
最小实现：DAT（双数组 base/check）`common_prefix_search` + `freq`。测试辅助 `dict_bin_with_words` 构造小 DAT（可直接用有序数组二分替代真 DAT，但 SPEC §5.2 要求 DAT 格式——本计划实装真 DAT，~150 行）。
commit：`jieba: implement DAT prefix search and freq lookup`。

### Task 3：DAG 最大概率切分

**测试**：
```rust
#[test]
fn dag_segment_known_words() {
    let bytes = dict_bin_with_words(&[("我", 100), ("爱", 100), ("北京", 200), ("天安门", 300)]);
    let d = std::sync::Arc::new(JiebaDict::load(&bytes).unwrap());
    let tok = JiebaTokenizer::new(d, &[]).unwrap();
    let toks = tok.tokenize("我爱北京天安门");
    let texts: Vec<_> = toks.iter().map(|t| t.text.as_str()).collect();
    assert_eq!(texts, vec!["我", "爱", "北京", "天安门"]);
}

#[test]
fn dag_picks_higher_freq_path() {
    // "研究生命" -> "研究/生命" (freq 高) vs "研究生/命"
    let bytes = dict_bin_with_words(&[
        ("研究", 100), ("研究生", 50), ("生命", 200), ("命", 10),
    ]);
    let d = std::sync::Arc::new(JiebaDict::load(&bytes).unwrap());
    let tok = JiebaTokenizer::new(d, &[]).unwrap();
    let toks = tok.tokenize("研究生命");
    let texts: Vec<_> = toks.iter().map(|t| t.text.as_str()).collect();
    // DAG 最大概率：研究(100) + 生命(200) 总权重优于 研究生(50)+命(10)
    assert!(texts.contains(&"研究".to_string()));
    assert!(texts.contains(&"生命".to_string()));
}
```
最小实现：`seg.rs::build_dag(text, dict) -> DAG`；`max_prob_path(dag, dict) -> Vec<&str>`（动态规划，权重 = -log(freq/total)，total = Σfreq）。未命中单字 fallback 为单字 token。
commit：`jieba: implement DAG max-probability segmentation`。

### Task 4：HMM 未登录词识别

**测试**：
```testing
#[test]
fn hmm_recognizes_unknown_word() {
    // 词典无 "蓝瘦香菇" 但 HMM 应识别为词（基于训练参数）
    // 注：HMM 参数来自 jieba 原版，对特定输入有确定输出。
    // 用 jieba-rs 原版同输入作对照（200 句验收在 10-ci-m1）
    let bytes = dict_bin_with_words(&[("我", 100), ("的", 100)]);
    let d = std::sync::Arc::new(JiebaDict::load(&bytes).unwrap());
    let tok = JiebaTokenizer::new(d, &[]).unwrap();
    // "蓝瘦香菇" 不在词典，HMM 应切为一个词或按单字
    let toks = tok.tokenize("蓝瘦香菇");
    // 与 jieba-rs 原版行为一致（验收锚点①在 CI 全量测）
    assert!(!toks.is_empty());
}
```
最小实现：`hmm.rs`：4 状态（B/M/E/S）Viterbi，转移矩阵 + 发射矩阵（从 dict.bin 的 hmm_blob 反序列化）。对 DAG 中未命中的连续单字 segment 跑 HMM。
commit：`jieba: implement HMM viterbi for unknown words`。

### Task 5：中英混排 + position 连续

**测试**：
```rust
#[test]
fn mixed_script_positions_continuous() {
    let bytes = dict_bin_with_words(&[("机器学习", 100)]);
    let d = std::sync::Arc::new(JiebaDict::load(&bytes).unwrap());
    let tok = JiebaTokenizer::new(d, &[]).unwrap();
    // "机器学习 running" -> CJK run "机器学习"(pos0) + Latin "running"(stem "run", pos1)
    let toks = tok.tokenize("机器学习 running");
    assert_eq!(toks[0].text, "机器学习");
    assert_eq!(toks[0].position, 0);
    assert_eq!(toks[1].text, "run");
    assert_eq!(toks[1].position, 1);
}

#[test]
fn latin_run_uses_standard_pipeline() {
    let bytes = dict_bin_with_words(&[]);
    let d = std::sync::Arc::new(JiebaDict::load(&bytes).unwrap());
    let tok = JiebaTokenizer::new(d, &[]).unwrap();
    let toks = tok.tokenize("Running RUNNERS");
    assert_eq!(toks[0].text, "run");
    assert_eq!(toks[1].text, "runner");
}
```
最小实现：`mod.rs` 复用 `is_cjk` 切 run；CJK run 进 DAG+HMM；非 CJK run 进 `unicode_words` + lowercase + Porter stem（复用 `rust_stemmers`）。position 跨 run 累积。
commit：`jieba: integrate mixed-script run segmentation`。

### Task 6：用户词表优先级（§5.3）

**测试**：
```rust
#[test]
fn user_dict_overrides_builtin() {
    let bytes = dict_bin_with_words(&[("机器学习", 100)]);
    let d = std::sync::Arc::new(JiebaDict::load(&bytes).unwrap());
    let user = vec![UserDictEntry::WordWithFreq { term: "机器学习".into(), freq: 999 }];
    let tok = JiebaTokenizer::new(d, &user).unwrap();
    // 用户词 freq 高 → DAG 优先命中（这里同词，验证不 panic + 仍切出）
    let toks = tok.tokenize("机器学习");
    assert_eq!(toks[0].text, "机器学习");
}

#[test]
fn user_dict_new_word_single_token() {
    // 验收锚点③：生造词注入后单 token 入索引
    let bytes = dict_bin_with_words(&[]);
    let d = std::sync::Arc::new(JiebaDict::load(&bytes).unwrap());
    let user = vec![UserDictEntry::Word("布地奈德".into())];
    let tok = JiebaTokenizer::new(d, &user).unwrap();
    let toks = tok.tokenize("布地奈德治疗效果");
    assert!(toks.iter().any(|t| t.text == "布地奈德"));
}
```
最小实现：用户词表在 DAG 构建时注入前缀词典（freq = 用户指定或内置最高频值）。`JiebaTokenizer::new` 校验 `user_dict.len() <= MAX_USER_DICT_ENTRIES`（复用 M0 build_tokenizer 的校验，但本构造器直接调也需校验）。
commit：`jieba: support user dict priority (§5.3)`。

### Task 7：build_tokenizer 接入 + TokenizerId（R-3：无二次哈希）

**测试**（`crates/vane-core/src/tokenizer/mod.rs` factory_tests 扩展）：
```rust
#[cfg(feature = "jieba")]
#[test]
fn build_jieba_with_dict_succeeds() {
    let bytes = test_fixture_dict_bin();
    let dict = std::sync::Arc::new(vane_core::tokenizer::jieba::JiebaDict::load(&bytes).unwrap());
    let t = vane_core::tokenizer::build_jieba_tokenizer(dict, &[]).unwrap();
    assert!(t.tokenize("测试").len() >= 1);
}

#[cfg(feature = "jieba")]
#[test]
fn jieba_tokenizer_id_independent_of_dict_calendar_version() {
    // R-3：词典日历版本/内容变化不改变 TokenizerId（仅格式变化才改）。
    // 两个不同日历版本的词典（同格式）→ 同一 TokenizerId（user_dict 相同）。
    let dict_v1 = std::sync::Arc::new(JiebaDict::load(&dict_bin_with_words(&[("测试", 100)])).unwrap());
    let dict_v2 = std::sync::Arc::new(JiebaDict::load(&dict_bin_with_words(&[("测试", 100), ("新词", 50)])).unwrap());
    let t1 = JiebaTokenizer::new(dict_v1, &[]).unwrap();
    let t2 = JiebaTokenizer::new(dict_v2, &[]).unwrap();
    assert_eq!(t1.id(), t2.id(), "dict content change must not change TokenizerId (R-3, REQUIREMENTS §3.3)");
}
```
最小实现：
- 新增 `pub fn build_jieba_tokenizer(dict: Arc<JiebaDict>, user_dict: &[UserDictEntry]) -> Result<Box<dyn Tokenizer>>`（扩展，不改 M0 `build_tokenizer` 签名）。
- `build_tokenizer` 的 `Jieba` 分支保持 `Err(DictUnavailable)`（无词典实例时无法构建）——**M0 行为不变**，wasm32 永不启用 jieba feature 故永不走此分支。
- `id.rs::builtin_dict_version(Jieba)` 改为 `b"jieba-fmt-v1"`（编译期常量，R-3）。
- `JiebaTokenizer::id()`：**直接用 `compute_tokenizer_id(Jieba, user_dict)`**，无二次哈希。
commit：`jieba: wire build_jieba_tokenizer with format-constant dict version (R-3, no double-hash)`。

### Task 8：缺词典降级（WASM 侧禁止 E_DICT_UNAVAILABLE 到达）

**测试**：
```rust
#[test]
fn jieba_without_feature_returns_dict_unavailable() {
    // 非 jieba feature 构建时，build_tokenizer(Jieba) 仍返回 DictUnavailable
    // （wasm32 永不启用 jieba feature → 此分支是 wasm 侧降级前的最后防线）
    let r = crate::tokenizer::build_tokenizer(BuiltinTokenizer::Jieba, &[]);
    assert!(matches!(r, Err(crate::types::VaneError::DictUnavailable)));
}
```
说明：WASM 侧降级（bigram + console.warn）在 M2 浏览器交付层实装；M1 core 保证 `build_tokenizer(Jieba)` 无词典时返回 `DictUnavailable`，绑定层（Node/Go）在加载词典失败时 fallback `CjkBigram` + 警告（07/08 计划）。
commit：`jieba: assert dict unavailable fallback path`。

## 验收标准

- **SPEC §13.2-2 ①**：200 句与 jieba-rs 原版切分 100% 一致——需 jieba-rs 作为 dev-dependency 对照（`[dev-dependencies] jieba-rs = "0.7"`，仅测试，非 core 运行时依赖；cargo-deny 允许）。测试在 `tests/jieba_compat.rs`，200 句 fixture 存 `tests/fixtures/jieba_200.txt`。**10-ci-m1 跑此 job**。
- **SPEC §13.2-2 ②**：中文维基 500 篇 + 50 查询，jieba-lite 相对完整版 nDCG@10 差 <2%、相对 bigram 提升 ≥15%——10-ci-m1 job，fixture 离线生成。
- **SPEC §13.2-2 ③**：20 生造词注入 userDict 单 token 入索引、短语命中 100%——Task 6 + `tests/jieba_userdict.rs`。
- **SPEC §13.2-2 ④**：缺词典自动降级 bigram + warn 不抛错——Task 8 + 07/08 绑定层。
- **SPEC §13.1**：词典冷加载 <150ms——`benches/dict_load.rs` criterion bench（10-ci-m1 跑）。
- **SPEC §5.4/不变量 I-4**（R-3，v1.1）：`builtin_dict_version(Jieba)` = 编译期格式常量 `b"jieba-fmt-v1"`；`JiebaTokenizer::id()` 直接用 `compute_tokenizer_id(Jieba, user_dict)`，无二次哈希。词典内容升级不改变 TokenizerId（满足 REQUIREMENTS §3.3）。
- **红线**：算法与 jieba-rs 完全一致只裁词典；词典永不进 wasm（`jieba` feature 默认关，wasm32 check 不启用）。
- **jieba-rs dev-dependency**：须验证 `jieba-rs` 传递依赖不含黑名单 crate（regex/ndarray 等）。`cargo tree -p vane-core --features jieba -e dev` 核查；若含黑名单，改用预先固化的 jieba-rs 切分结果 fixture（`tests/fixtures/jieba_200.txt`）而非 jieba-rs 运行时对照。

## 前置依赖

- M0 `tokenizer`（已合并）。
- 无 M1 内部前置（L0 批次，与 01/09 并行）。

## Global Constraints

词典永不进 wasm（`jieba` feature 默认关；wasm32 check `cargo check --target wasm32-unknown-unknown -p vane-core` 不带 `--features jieba`）；jieba 算法不动只裁词典（验收①对照 jieba-rs）；`ruzstd` 非黑名单依赖（纯 Rust，wasm32 安全，但 core 默认不启用故不影响 wasm32 体积）；core 禁 std::fs；cfg 只在 feature 门（`cfg(feature="jieba")` 是 feature 非 target cfg，不违反 I-5）。
