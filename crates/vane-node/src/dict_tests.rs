//! 07-dict-distribution-node 测试（Task 2 + Task 3）。
//!
//! - Task 2：loadDict 返回 DICT_BIN Buffer；collection tokenizer=jieba 自动加载词典。
//! - Task 3：tokenizer=jieba 且 dict 不可用 → build_tokenizer 返回 DictUnavailable
//!   （绑定层 convert 降级 CjkBigram + warn）。

#![cfg(test)]

use vane_core::tokenizer::jieba::JiebaDict;
use vane_core::tokenizer::{build_jieba_tokenizer, build_tokenizer, BuiltinTokenizer};
use vane_core::types::VaneError;

/// Task 2：loadDict 返回的 DICT_BIN 可被 core 加载为 JiebaDict。
#[test]
fn load_dict_buffer_loadable_by_core() {
    let bin = vane_dict_zh::DICT_BIN;
    assert!(!bin.is_empty());
    let dict = JiebaDict::load_zstd(bin).expect("DICT_BIN must load as JiebaDict");
    assert_eq!(dict.version(), vane_dict_zh::DICT_VERSION);
}

/// Task 2：用 DICT_BIN 构建 jieba 分词器，中文切分非空。
#[test]
fn jieba_tokenizer_from_bundled_dict_segments_chinese() {
    let dict = std::sync::Arc::new(JiebaDict::load_zstd(vane_dict_zh::DICT_BIN).unwrap());
    let tok = build_jieba_tokenizer(dict, &[]).unwrap();
    // fixture 含「我」「爱」「北京」「天安门」等词
    let toks = tok.tokenize("我爱北京天安门");
    assert!(!toks.is_empty(), "jieba 必须对中文产生非空切分");
}

/// Task 2：DICT_BIN 的 sha256_prefix 与 vane_dict_zh::sha256_prefix() 一致。
#[test]
fn bundled_dict_sha256_consistent() {
    let dict = JiebaDict::load_zstd(vane_dict_zh::DICT_BIN).unwrap();
    assert_eq!(dict.sha256_prefix(), vane_dict_zh::sha256_prefix());
}

/// Task 3：无词典注入时 build_tokenizer(Jieba) 返回 DictUnavailable（降级前触发点）。
#[test]
fn jieba_dict_missing_returns_dict_unavailable() {
    let r = build_tokenizer(BuiltinTokenizer::Jieba, &[]);
    assert!(
        matches!(r, Err(VaneError::DictUnavailable)),
        "无词典时 build_tokenizer(Jieba) 必须返回 DictUnavailable"
    );
}

/// Task 3：CjkBigram 始终可用（降级目标）。
#[test]
fn cjk_bigram_fallback_available() {
    let tok = build_tokenizer(BuiltinTokenizer::CjkBigram, &[]).unwrap();
    assert!(!tok.tokenize("机器学习").is_empty());
}
