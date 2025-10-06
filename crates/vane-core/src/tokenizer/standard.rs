//! Standard 分词器（SPEC §5.1）：unicode 分词 → lowercase → Porter stemmer。

use rust_stemmers::{Algorithm, Stemmer};
use unicode_segmentation::UnicodeSegmentation;

use crate::tokenizer::{compute_tokenizer_id, BuiltinTokenizer, Token, Tokenizer, UserDictEntry};
use crate::types::TokenizerId;

pub(crate) struct StandardTokenizer {
    id: TokenizerId,
    stemmer: Stemmer,
}

impl StandardTokenizer {
    /// `user_dict` 在 M0 不参与 standard 的切分逻辑，仅影响 TokenizerId（SPEC §5.3/§5.4）。
    pub(crate) fn new(user_dict: &[UserDictEntry]) -> Self {
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
