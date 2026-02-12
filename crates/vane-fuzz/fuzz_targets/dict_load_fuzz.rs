//! Fuzz target：畸形词典字节 → 降级 bigram 不抛错（M2-04 铁律）。
//!
//! 输入 decode：ByteCursor → n_entries（0..=16）+ 各 entry 的 word 字符串 + freq。
//! 不变量（M2-04）：分词器构造路径不 panic——
//!   - build_tokenizer(Jieba, ..) 无 dict 实例 → Err(DictUnavailable / DictTooLarge)，不 panic；
//!   - build_tokenizer(CjkBigram, ..) 降级路径 → Ok，不 panic；
//!   - build_tokenizer(Standard, ..) → Ok，不 panic；
//!   - Collection::set_user_dict(fuzzer entries) → Ok 或 Err（DictTooLarge / Busy），不 panic。
//!
//! 设计 §3.2 target 表第 5 行。
//! 取舍：JiebaDict::load/load_zstd 的畸形字节→Err 路径需 jieba feature
//! （ruzstd）。设计 §3.2 Cargo.toml 按"字面采用"不启 jieba（避 workspace
//! feature unification 触 wasm32-check / 其他门禁）。本 target 验 M2-04 的
//! API 层降级不变量（Jieba→Err→CjkBigram→Ok 不 panic）；JiebaDict::load 的
//! 畸形字节 fuzz defer Phase 6（如需，vane-fuzz 加 optional `jieba` feature
//! + cfg-gated JiebaDict::load 调用）。

#![no_main]

mod common;

use std::sync::Arc;

use libfuzzer::fuzz_target;

use common::{build_schema, ByteCursor};
use vane_core::api::{CollectionOptions, Db, OpenOptions};
use vane_core::tokenizer::{build_tokenizer, BuiltinTokenizer, UserDictEntry};
use vane_core::vfs::memory::MemoryVfs;
use vane_core::vfs::Vfs;

fuzz_target!(|data: &[u8]| {
    let mut c = ByteCursor::new(data);

    // 畸形词典字节 → 结构化 UserDictEntry（lossy UTF-8，不 panic）。
    let n_entries = (c.u8() as usize).min(16);
    let mut user_dict: Vec<UserDictEntry> = Vec::with_capacity(n_entries);
    for _ in 0..n_entries {
        let word = c.small_string();
        if word.is_empty() {
            continue;
        }
        if c.bool() {
            user_dict.push(UserDictEntry::Word(word));
        } else {
            user_dict.push(UserDictEntry::WordWithFreq {
                term: word,
                freq: c.u32_le(),
            });
        }
    }

    // M2-04 不变量 1：Jieba 路径无 dict → Err（DictUnavailable 或 DictTooLarge），不 panic。
    //    （user_dict.len() ≤ 16 < 100k → 不会 DictTooLarge；应返 DictUnavailable。）
    let jieba_tok = build_tokenizer(BuiltinTokenizer::Jieba, &user_dict);
    drop(jieba_tok); // 接受 Ok 或 Err——不 panic 即满足。

    // M2-04 不变量 2：CjkBigram 降级路径 → Ok，不 panic。
    let bigram_tok = build_tokenizer(BuiltinTokenizer::CjkBigram, &user_dict);
    assert!(bigram_tok.is_ok(), "CjkBigram fallback must succeed");
    drop(bigram_tok);

    // M2-04 不变量 3：Standard 路径 → Ok，不 panic。
    let std_tok = build_tokenizer(BuiltinTokenizer::Standard, &user_dict);
    assert!(std_tok.is_ok(), "Standard tokenizer must succeed");
    drop(std_tok);

    // 端到端：Collection::set_user_dict 不 panic（可能 Err，不 panic 即满足）。
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    let db = Db::open(vfs, "db", OpenOptions::default()).expect("Db::open");
    let schema = build_schema(true, 4);
    let col = db
        .collection("c", schema, CollectionOptions::default())
        .expect("collection create");
    // set_user_dict：>100k → DictTooLarge；Rebuilding → Busy。此处 ≤16，应 Ok。
    let _ = col.set_user_dict(&user_dict);
    // 不 panic 即满足 M2-04。
});
