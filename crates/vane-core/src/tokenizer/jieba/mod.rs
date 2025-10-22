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
    compute_tokenizer_id, BuiltinTokenizer, Token, Tokenizer, UserDictEntry, MAX_USER_DICT_ENTRIES,
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
    pub fn new(dict: Arc<JiebaDict>, user_dict: &[UserDictEntry]) -> VaneResult<Self> {
        if user_dict.len() > MAX_USER_DICT_ENTRIES {
            return Err(VaneError::DictTooLarge);
        }
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

/// 判断字符是否属于 CJK 表意文字/假名范围（与 `cjk_bigram::is_cjk` 一致，复用 run 切分）。
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

#[cfg(all(test, feature = "jieba"))]
pub(crate) mod tests;
