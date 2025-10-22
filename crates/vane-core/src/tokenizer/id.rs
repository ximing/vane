//! TokenizerId 计算（SPEC §5.4）。
//! TokenizerId = sha256( algorithm_version ‖ builtin_dict_version ‖ user_dict_bytes )
//!
//! 任何分词算法变更（unicode 边界规则、stemmer 版本、bigram 策略、jieba 词典**格式**版本）
//! 必须递增对应 version 标签，从而产生新 TokenizerId 触发 reindex。

use sha2::{Digest, Sha256};

use crate::tokenizer::{BuiltinTokenizer, UserDictEntry};
use crate::types::TokenizerId;

/// 算法版本标签（参与 sha256）。变更分词算法 → 递增此版本 → id 改变。
fn algorithm_version(kind: BuiltinTokenizer) -> &'static [u8] {
    match kind {
        BuiltinTokenizer::Standard => b"std-v1",
        BuiltinTokenizer::CjkBigram => b"cjk-bigram-v1",
        BuiltinTokenizer::Jieba => b"jieba-v1",
    }
}

/// 内置词典**格式**版本标签（参与 sha256，SPEC §5.4 v1.1）。
/// - standard / cjk_bigram：无内置词典，用空串。
/// - jieba：编译期**格式**常量 `b"jieba-fmt-v1"`（R-3）。仅当 DAT 结构 / HMM 参数**格式**
///   变更时递增；词典**内容**升级（增删词条、日历版本变化）**不改变**此值，故不改变
///   TokenizerId（满足 REQUIREMENTS §3.3「词典升级仅警告不强制重建」）。词典运行时
///   日历版本 + sha256 前缀存 dict.bin 头 + CollectionMeta，不进 TokenizerId。
fn builtin_dict_version(kind: BuiltinTokenizer) -> &'static [u8] {
    match kind {
        BuiltinTokenizer::Standard => b"",
        BuiltinTokenizer::CjkBigram => b"",
        BuiltinTokenizer::Jieba => b"jieba-fmt-v1",
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
pub fn compute_tokenizer_id(kind: BuiltinTokenizer, user_dict: &[UserDictEntry]) -> TokenizerId {
    let mut hasher = Sha256::new();
    hasher.update(algorithm_version(kind));
    hasher.update(builtin_dict_version(kind));
    hasher.update(serialize_user_dict(user_dict));
    let hash = hasher.finalize();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&hash);
    TokenizerId(arr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokenizer::{BuiltinTokenizer, UserDictEntry};
    use crate::types::TokenizerId;

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
            &[UserDictEntry::WordWithFreq {
                term: "机器学习".to_string(),
                freq: 100,
            }],
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
