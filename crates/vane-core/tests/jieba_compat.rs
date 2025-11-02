//! SPEC §13.2-2 ①：jieba 200 句兼容性验收测试。
//!
//! jieba-rs 因依赖 regex（黑名单）不能作 dev-dep。fixture 由
//! `scripts/gen_jieba_fixture.rs` 离线生成（jieba-rs 0.7 原版切分），
//! 固化到 `tests/fixtures/jieba_200.txt`。本测试用 jieba-lite（内置 20 万词
//! dict.bin via vane-dict-zh）切分 200 句 vs fixture 比对，断言 100% 一致。
//!
//! fixture 格式：每行 = `句子\t词1/词2/词3`

#![cfg(feature = "dict-zh")]

use std::sync::Arc;

use vane_core::tokenizer::jieba::{JiebaDict, JiebaTokenizer};
use vane_core::tokenizer::Tokenizer;
use vane_dict_zh::DICT_BIN;

fn load_fixture() -> Vec<(String, Vec<String>)> {
    let raw = include_str!("fixtures/jieba_200.txt");
    raw.lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            let mut parts = line.splitn(2, '\t');
            let sentence = parts.next().unwrap().to_string();
            let words: Vec<String> = parts
                .next()
                .unwrap_or("")
                .split('/')
                .map(|w| w.to_string())
                .collect();
            (sentence, words)
        })
        .collect()
}

#[test]
fn jieba_lite_matches_jieba_rs_200_sentences() {
    let dict = Arc::new(JiebaDict::load_zstd(DICT_BIN).expect("dict.bin load"));
    let tok = JiebaTokenizer::new(dict, &[]).expect("tokenizer build");

    let fixture = load_fixture();
    assert_eq!(
        fixture.len(),
        200,
        "fixture must contain exactly 200 sentences"
    );

    let mut mismatches = Vec::new();

    for (sentence, expected_words) in &fixture {
        let tokens = tok.tokenize(sentence);
        let actual_words: Vec<String> = tokens.iter().map(|t| t.text.clone()).collect();

        if &actual_words != expected_words {
            mismatches.push(format!(
                "  「{}」\n    expected: {:?}\n    actual:   {:?}",
                sentence, expected_words, actual_words
            ));
        }
    }

    if !mismatches.is_empty() {
        eprintln!(
            "jieba-lite vs jieba-rs: {}/{} sentences mismatched:",
            mismatches.len(),
            fixture.len()
        );
        for m in &mismatches {
            eprintln!("{}", m);
        }
        panic!(
            "jieba-lite 200 句兼容性测试失败：{} 处不一致（SPEC §13.2-2 ① 要求 100% 一致）",
            mismatches.len()
        );
    }

    println!(
        "jieba-lite 200 句兼容性测试通过：{}/{} 一致",
        fixture.len(),
        fixture.len()
    );
}
