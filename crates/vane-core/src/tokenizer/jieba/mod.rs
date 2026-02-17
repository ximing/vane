//! jieba-lite 分词器（SPEC §5.1/§5.2/§5.3/§5.4）。
//!
//! 前缀 DAG 最大概率切分 + HMM 未登录词识别，算法与 jieba-rs 完全一致（仅裁词典）。
//! 中英混排复用 M0 `cjk_bigram::is_cjk` 切 run：CJK run 进 DAG+HMM；非 CJK run 进
//! standard 管线（unicode_words → lowercase → Porter stem）。position 跨 run 连续递增（不变量 I-4）。
//!
//! TokenizerId（R-3）：`JiebaTokenizer::id()` 直接用 `compute_tokenizer_id(Jieba, user_dict)`，
//! 无二次哈希。词典内容升级不改变 TokenizerId（满足 REQUIREMENTS §3.3）。

mod dict;
mod hmm;
mod seg;

pub use dict::JiebaDict;

use rust_stemmers::{Algorithm, Stemmer};
use seg::UserTrie;
use std::sync::Arc;
use unicode_segmentation::UnicodeSegmentation;

use crate::tokenizer::{
    compute_tokenizer_id, is_cjk, BuiltinTokenizer, Token, Tokenizer, UserDictEntry,
    MAX_USER_DICT_ENTRIES,
};
use crate::types::{Result as VaneResult, TokenizerId, VaneError};

/// jieba 分词器（持有 `JiebaDict` + 用户词表 + Porter stemmer）。
pub struct JiebaTokenizer {
    dict: Arc<JiebaDict>,
    user: UserTrie,
    id: TokenizerId,
    stemmer: Stemmer,
}

impl JiebaTokenizer {
    /// 从已加载词典 + 用户词表构建。校验用户词表上限（SPEC §5.3）。
    ///
    /// 缺省 freq（`UserDictEntry::Word(term)`）= `dict.max_freq()`（词典最高频值），
    /// 与 jieba-rs 原版一致——保证用户词在 DAG 最大概率路径中优先命中（覆盖内置
    /// 同词的低 freq 切分路径，SPEC §5.3：缺省 freq = 内置词典最高频值）。
    pub fn new(dict: Arc<JiebaDict>, user_dict: &[UserDictEntry]) -> VaneResult<Self> {
        if user_dict.len() > MAX_USER_DICT_ENTRIES {
            return Err(VaneError::DictTooLarge(
                "user dict exceeds 100000 entries".into(),
            ));
        }
        // max_freq = 词典最高频值；UserDictEntry::Word 缺省 freq 用此值（SPEC §5.3）。
        let max_freq = dict.max_freq();
        let mut user = UserTrie::new();
        for entry in user_dict {
            let (term, freq) = match entry {
                UserDictEntry::Word(t) => (t.as_str(), max_freq),
                UserDictEntry::WordWithFreq { term, freq } => (term.as_str(), *freq),
            };
            if !term.is_empty() {
                user.insert(term, freq);
            }
        }
        Ok(Self {
            dict,
            user,
            id: compute_tokenizer_id(BuiltinTokenizer::Jieba, user_dict),
            stemmer: Stemmer::create(Algorithm::English),
        })
    }

    /// 暴露给测试/绑定层构造词典实例。
    pub fn dict(&self) -> &JiebaDict {
        &self.dict
    }
}

impl Tokenizer for JiebaTokenizer {
    fn tokenize(&self, text: &str) -> Vec<Token> {
        let mut tokens = Vec::new();
        let mut position: u32 = 0;

        let chars: Vec<char> = text.chars().collect();
        let n = chars.len();
        let mut i = 0usize;
        while i < n {
            if is_cjk(chars[i]) {
                let start = i;
                while i < n && is_cjk(chars[i]) {
                    i += 1;
                }
                let run: Vec<char> = chars[start..i].to_vec();
                for word in seg::cut(&run, &self.dict, &self.user) {
                    tokens.push(Token {
                        text: word,
                        position,
                    });
                    position += 1;
                }
            } else {
                let start = i;
                while i < n && !is_cjk(chars[i]) {
                    i += 1;
                }
                let run: String = chars[start..i].iter().collect();
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

#[cfg(all(test, feature = "jieba"))]
pub(crate) mod tests;
