//! 分词器模块（SPEC §5）。

mod cjk_bigram;
mod id;
#[cfg(feature = "jieba")]
pub mod jieba;
mod standard;

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

/// 工厂：构建内置分词器（SPEC §5.1 / §5.3 / §10）。
///
/// - `Standard` / `CjkBigram`：M0 完整实现。
/// - `Jieba`：需先加载词典（`JiebaDict::load`）再调 `build_jieba_tokenizer`；
///   无词典实例时返回 `Err(VaneError::DictUnavailable)`（M0 行为不变，wasm32 永不启用 jieba feature）。
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

/// 从已加载的 `JiebaDict` 构建 jieba 分词器（SPEC §5.1 / §5.4，R-3）。
///
/// `build_tokenizer(Jieba, ..)` 无词典实例时返回 `DictUnavailable`；绑定层（Node/Go）
/// 加载 dict.bin 后调本函数获取分词器。`JiebaTokenizer::id()` 直接用
/// `compute_tokenizer_id(Jieba, user_dict)`，无二次哈希（R-3：词典内容升级不改变 TokenizerId）。
#[cfg(feature = "jieba")]
pub fn build_jieba_tokenizer(
    dict: std::sync::Arc<jieba::JiebaDict>,
    user_dict: &[UserDictEntry],
) -> crate::types::Result<Box<dyn Tokenizer>> {
    use crate::types::VaneError;
    if user_dict.len() > MAX_USER_DICT_ENTRIES {
        return Err(VaneError::DictTooLarge);
    }
    Ok(Box::new(jieba::JiebaTokenizer::new(dict, user_dict)?))
}

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
        let err = build_tokenizer(BuiltinTokenizer::Jieba, &[]).err().unwrap();
        assert!(
            matches!(err, VaneError::DictUnavailable),
            "M0 jieba 必须返回 DictUnavailable，实际: {:?}",
            err
        );
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
        let err = build_tokenizer(BuiltinTokenizer::Standard, &dict)
            .err()
            .unwrap();
        assert!(
            matches!(err, VaneError::DictTooLarge),
            "超限必须返回 DictTooLarge，实际: {:?}",
            err
        );
        assert_eq!(err.code(), -7); // SPEC §10: E_DICT_TOO_LARGE = -7
    }

    #[test]
    fn build_jieba_with_over_limit_dict_returns_dict_too_large_first() {
        // 词表上限检查优先于 jieba 可用性检查（输入校验先于资源校验）
        let dict: Vec<UserDictEntry> = (0..=100_000)
            .map(|i| UserDictEntry::Word(format!("w{}", i)))
            .collect();
        let err = build_tokenizer(BuiltinTokenizer::Jieba, &dict)
            .err()
            .unwrap();
        assert!(matches!(err, VaneError::DictTooLarge));
    }

    #[test]
    fn built_tokenizer_is_send_sync() {
        // Box<dyn Tokenizer> 必须是 Send + Sync（trait 约束已保证，此处编译期断言）
        fn assert_send_sync<T: Send + Sync>(_: &T) {}
        let t = build_tokenizer(BuiltinTokenizer::Standard, &[]).unwrap();
        assert_send_sync(&t);
    }

    // ---- Task 7: build_jieba_tokenizer 接入（feature = "jieba"）----

    #[cfg(feature = "jieba")]
    #[test]
    fn build_jieba_with_dict_succeeds() {
        let bytes = jieba::tests::test_fixture_dict_bin();
        let dict = std::sync::Arc::new(jieba::JiebaDict::load(&bytes).unwrap());
        let t = build_jieba_tokenizer(dict, &[]).unwrap();
        assert!(!t.tokenize("测试").is_empty());
    }

    // ---- Task 8: 缺词典降级（无 jieba feature 时 build_tokenizer(Jieba) 返回 DictUnavailable）----

    #[test]
    fn jieba_without_feature_returns_dict_unavailable() {
        // build_tokenizer(Jieba) 无词典实例时返回 DictUnavailable（M0 行为不变）。
        // wasm32 永不启用 jieba feature → 此分支是 wasm 侧降级前的最后防线。
        let r = build_tokenizer(BuiltinTokenizer::Jieba, &[]);
        assert!(matches!(r, Err(VaneError::DictUnavailable)));
    }
}
