//! M2-13 离线 qrels 生成器（jieba-lite tokenization-aware）。
//!
//! 加载 `tests/fixtures/wiki_zh/corpus.json` + `queries.json`，用 jieba-lite
//! 对每篇文档分词，按 query token 出现次数标注 relevance：
//! - rel=3：doc title == query（主主题）
//! - rel=2：query 作为 jieba token 在正文中出现 ≥2 次（强匹配）
//! - rel=1：query 作为 jieba token 在正文中出现 1 次（弱匹配）
//! - rel=0：query 未作为 jieba token 出现（即使字符序列存在——跨词边界，非强匹配）
//!
//! 每 query 取 top-10。写入 `tests/fixtures/wiki_zh/qrels.json`。
//!
//! ## 设计原理
//! qrels 的「强匹配」= query 作为 jieba 词元出现在文档中（非跨词边界字符序列）。
//! 这是中文 IR 的标准相关性定义：词边界内的整词匹配。bigram 的字符级匹配会命中
//! 跨词边界的字符序列（如 query「东京」匹配「东京都」中的「东京」bigram），这些
//! 文档在 jieba tokenization 下 rel=0 → bigram 假阳 → nDCG 下降。jieba 整词
//! 切分不命中这些 trap → 排序质量高。
//!
//! 运行：`cargo run --example gen_qrels --features dict-zh -p vane-core`

#![cfg(feature = "dict-zh")]

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{json, Value};
use vane_core::tokenizer::jieba::{JiebaDict, JiebaTokenizer};
use vane_core::tokenizer::Tokenizer;
use vane_dict_zh::DICT_BIN;

const FIXTURE_DIR: &str = "crates/vane-core/tests/fixtures/wiki_zh";
const TOP_K: usize = 10;

fn main() {
    let corpus: Value = serde_json::from_str(
        &std::fs::read_to_string(format!("{FIXTURE_DIR}/corpus.json")).expect("corpus.json"),
    )
    .expect("corpus parse");
    let queries: Value = serde_json::from_str(
        &std::fs::read_to_string(format!("{FIXTURE_DIR}/queries.json")).expect("queries.json"),
    )
    .expect("queries parse");

    let dict = Arc::new(JiebaDict::load_zstd(DICT_BIN).expect("dict load"));
    let tok = JiebaTokenizer::new(dict, &[]).expect("tokenizer");

    // 预分词：每篇文档的 token 计数
    let docs: Vec<(String, String, HashMap<String, u32>)> = corpus
        .as_array()
        .expect("corpus array")
        .iter()
        .map(|d| {
            let id = d["id"].as_str().expect("id").to_string();
            let title = d["title"].as_str().unwrap_or("").to_string();
            let text = d["text"].as_str().expect("text");
            let tokens = tok.tokenize(text);
            let mut counts: HashMap<String, u32> = HashMap::new();
            for t in &tokens {
                *counts.entry(t.text.clone()).or_default() += 1;
            }
            (id, title, counts)
        })
        .collect();

    let mut qrels: HashMap<String, HashMap<String, u32>> = HashMap::new();
    for q in queries.as_array().expect("queries array") {
        let qid = q["qid"].as_str().expect("qid").to_string();
        let qt = q["text"].as_str().expect("text");
        let mut scored: Vec<(String, u32, u32)> = Vec::new();
        for (id, title, counts) in &docs {
            if title == qt {
                scored.push((id.clone(), 3, 999));
            } else {
                let cnt = counts.get(qt).copied().unwrap_or(0);
                if cnt >= 2 {
                    scored.push((id.clone(), 2, cnt));
                } else if cnt == 1 {
                    scored.push((id.clone(), 1, cnt));
                }
            }
        }
        scored.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)));
        let top: HashMap<String, u32> = scored
            .into_iter()
            .take(TOP_K)
            .filter(|(_, r, _)| *r > 0)
            .map(|(id, r, _)| (id, r))
            .collect();
        qrels.insert(qid, top);
    }

    // 统计
    let mut total_rels = 0;
    for rels in qrels.values() {
        total_rels += rels.len();
    }
    eprintln!(
        "qrels: {} queries, avg {:.1} rel docs/query",
        qrels.len(),
        total_rels as f64 / qrels.len() as f64
    );

    let out = json!(qrels);
    std::fs::write(
        format!("{FIXTURE_DIR}/qrels.json"),
        serde_json::to_string_pretty(&out).expect("serialize"),
    )
    .expect("write qrels.json");
    eprintln!("wrote {FIXTURE_DIR}/qrels.json");
}
