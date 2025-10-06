//! CJK bigram 分词器（SPEC §5.1）。
//! 先按 unicode script 边界切 run：CJK run 切重叠二元组（单字 run 退化为 unigram）；
//! 非 CJK run 走 standard 管线（unicode_words → lowercase → Porter stem）。
//! token position 全程跨 run 连续递增（不变量 I-4，跨语言 phrase query 正确性依赖）。

use rust_stemmers::{Algorithm, Stemmer};
use unicode_segmentation::UnicodeSegmentation;

use crate::tokenizer::{compute_tokenizer_id, BuiltinTokenizer, Token, Tokenizer, UserDictEntry};
use crate::types::TokenizerId;

pub(crate) struct CjkBigramTokenizer {
    id: TokenizerId,
    stemmer: Stemmer,
}

impl CjkBigramTokenizer {
    pub(crate) fn new(user_dict: &[UserDictEntry]) -> Self {
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
        assert!(is_cjk('汉')); // U+6C49 CJK 统一
        assert!(is_cjk('あ')); // U+3042 平假名
        assert!(is_cjk('カ')); // U+30AB 片假名
        assert!(!is_cjk('a'));
        assert!(!is_cjk(' '));
        assert!(!is_cjk('1'));
    }
}
