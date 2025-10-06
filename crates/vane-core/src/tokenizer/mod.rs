//! 分词器模块（SPEC §5）。

mod cjk_bigram;
mod id;
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
