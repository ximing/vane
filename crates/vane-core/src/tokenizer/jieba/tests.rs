//! jieba-lite 测试 + 测试夹具构建器（DAT 构建 / dict.bin 序列化 / HMM 参数）。
//!
//! 测试夹具 `dict_bin_with_words` 在运行期构建小规模 dict.bin（含真 DAT + 真 HMM 参数），
//! 供本模块测试及 `tokenizer::factory_tests`（Task 7）共用。

#![cfg(feature = "jieba")]
#![allow(clippy::excessive_precision)] // jieba 原版 HMM 参数，须逐字保留

use std::collections::VecDeque;

use super::dict::JiebaDict;
use super::JiebaTokenizer;
use crate::tokenizer::{BuiltinTokenizer, Tokenizer, UserDictEntry};

// ============================================================
// 测试夹具构建器
// ============================================================

/// 构建最小 dict.bin（合法头 + 空词典 + 空 HMM）。
pub(crate) fn minimal_dict_bin() -> Vec<u8> {
    dict_bin_with_words(&[])
}

/// 从词表构建 dict.bin（解压后字节）：真 DAT + 真 HMM 参数。
///
/// HMM 发射矩阵：为词表中所有单字添加 S 态发射（prob=-3.0），
/// 确保 DAG 路径中的已知单字在 HMM 中保持单字切分（与 jieba 行为一致）。
pub(crate) fn dict_bin_with_words(words: &[(&str, u32)]) -> Vec<u8> {
    let word_pairs: Vec<(String, u32)> = words.iter().map(|(w, f)| (w.to_string(), *f)).collect();
    let (base, check, values) = build_dat(&word_pairs);
    let total_freq: u64 = word_pairs.iter().map(|(_, f)| *f as u64).sum();

    // 收集词表中所有单字 → S 态发射
    let mut s_emit: Vec<(u32, f64)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (w, _) in &word_pairs {
        for c in w.chars() {
            let cc = c as u32;
            if seen.insert(cc) {
                s_emit.push((cc, -3.0));
            }
        }
    }

    let hmm_blob = build_hmm_blob(&s_emit);

    serialize_dict_bin(
        "2026.08", &[0u8; 8], // sha256_prefix（测试夹具无需真实哈希）
        total_freq, &base, &check, &values, &hmm_blob,
    )
}

/// 构建测试用 dict.bin fixture（~20 词，供 factory_tests 使用）。
pub(crate) fn test_fixture_dict_bin() -> Vec<u8> {
    dict_bin_with_words(&[
        ("测试", 100),
        ("我", 100),
        ("爱", 100),
        ("北京", 200),
        ("天安门", 300),
        ("机器学习", 100),
        ("学习", 200),
        ("机器", 50),
        ("研究", 100),
        ("研究生", 50),
        ("生命", 200),
        ("命", 10),
        ("的", 100),
    ])
}

// ---- DAT 构建（双数组 Trie，Aoe BFS 算法）----

fn build_dat(words: &[(String, u32)]) -> (Vec<i32>, Vec<i32>, Vec<i32>) {
    // 1. 构建 trie
    struct TrieNode {
        children: Vec<(u32, usize)>, // (char_code, child_id)
        terminal_freq: i32,
    }
    let mut trie: Vec<TrieNode> = vec![TrieNode {
        children: vec![],
        terminal_freq: -1,
    }];
    for (word, freq) in words {
        let mut node = 0usize;
        for c in word.chars() {
            let cc = c as u32;
            if let Some(pos) = trie[node].children.iter().position(|(ch, _)| *ch == cc) {
                node = trie[node].children[pos].1;
            } else {
                let new_id = trie.len();
                trie.push(TrieNode {
                    children: vec![],
                    terminal_freq: -1,
                });
                trie[node].children.push((cc, new_id));
                node = new_id;
            }
        }
        trie[node].terminal_freq = *freq as i32;
    }
    for node in &mut trie {
        node.children.sort_by_key(|(c, _)| *c);
    }

    // 2. BFS 转换为 DAT
    let mut base = vec![0i32];
    let mut check = vec![-1i32];
    let mut values = vec![-1i32];

    let mut queue: VecDeque<(usize, usize)> = VecDeque::new(); // (trie_id, dat_id)
    queue.push_back((0, 0));

    while let Some((trie_id, dat_id)) = queue.pop_front() {
        let children = &trie[trie_id].children;
        if !children.is_empty() {
            let child_chars: Vec<u32> = children.iter().map(|(c, _)| *c).collect();
            let base_val = find_base(&check, &child_chars);
            base[dat_id] = base_val;
            for &(cc, child_trie_id) in children {
                let t = (base_val + cc as i32) as usize;
                while t >= check.len() {
                    check.push(-1);
                    base.push(0);
                    values.push(-1);
                }
                check[t] = dat_id as i32;
                queue.push_back((child_trie_id, t));
            }
        }
        if trie[trie_id].terminal_freq >= 0 {
            values[dat_id] = trie[trie_id].terminal_freq;
        }
    }

    (base, check, values)
}

fn find_base(check: &[i32], child_chars: &[u32]) -> i32 {
    if child_chars.is_empty() {
        return 0;
    }
    let mut base = 1i32;
    loop {
        let ok = child_chars.iter().all(|&cc| {
            let t = base + cc as i32;
            t >= 0 && (t as usize >= check.len() || check[t as usize] == -1)
        });
        if ok {
            return base;
        }
        base += 1;
    }
}

// ---- HMM blob 构建（jieba 原版 START_P / TRANS_P + 测试发射矩阵）----

fn build_hmm_blob(s_emit: &[(u32, f64)]) -> Vec<u8> {
    // jieba 原版参数（B=0, M=1, E=2, S=3）
    let start_p: [f64; 4] = [
        -0.26268660809250016, // B
        -3.14e100,            // M
        -3.14e100,            // E
        -1.4652633398537698,  // S
    ];
    let trans: [[f64; 4]; 4] = [
        [
            -3.14e100,
            -0.916290731874155,
            -0.5133150735914917,
            -3.14e100,
        ], // B->
        [
            -3.14e100,
            -1.2603623831137852,
            -0.3330136237024262,
            -3.14e100,
        ], // M->
        [
            -0.7432080337278319,
            -3.14e100,
            -3.14e100,
            -0.6378454977269382,
        ], // E->
        [
            -0.6931061394328019,
            -3.14e100,
            -3.14e100,
            -0.3146855793925807,
        ], // S->
    ];

    let mut buf = Vec::with_capacity(32 + 128 + 16 + s_emit.len() * 12);
    // start_p
    for v in &start_p {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    // trans (row-major)
    for row in &trans {
        for v in row {
            buf.extend_from_slice(&v.to_le_bytes());
        }
    }
    // emit_counts: [B=0, M=0, E=0, S=s_emit.len()]
    buf.extend_from_slice(&0u32.to_le_bytes()); // B
    buf.extend_from_slice(&0u32.to_le_bytes()); // M
    buf.extend_from_slice(&0u32.to_le_bytes()); // E
    buf.extend_from_slice(&(s_emit.len() as u32).to_le_bytes()); // S
                                                                 // S 态发射条目
    for &(cc, prob) in s_emit {
        buf.extend_from_slice(&cc.to_le_bytes());
        buf.extend_from_slice(&prob.to_le_bytes());
    }
    buf
}

// ---- dict.bin 序列化 ----

#[allow(clippy::too_many_arguments)]
fn serialize_dict_bin(
    dict_version: &str,
    sha256_prefix: &[u8; 8],
    total_freq: u64,
    base: &[i32],
    check: &[i32],
    values: &[i32],
    hmm_blob: &[u8],
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(64 + base.len() * 12 + hmm_blob.len());
    buf.extend_from_slice(b"VNDT"); // magic
    buf.extend_from_slice(&1u32.to_le_bytes()); // format_version
    buf.extend_from_slice(sha256_prefix);
    let ver_bytes = dict_version.as_bytes();
    buf.extend_from_slice(&(ver_bytes.len() as u16).to_le_bytes());
    buf.extend_from_slice(ver_bytes);
    buf.extend_from_slice(&total_freq.to_le_bytes());
    let dat_len = base.len() as u32;
    buf.extend_from_slice(&dat_len.to_le_bytes());
    for &v in base {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    for &v in check {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    for &v in values {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    buf.extend_from_slice(&(hmm_blob.len() as u32).to_le_bytes());
    buf.extend_from_slice(hmm_blob);
    buf
}

// ============================================================
// Task 1: feature 门控 + JiebaDict 骨架
// ============================================================

#[test]
fn jieba_dict_load_empty_fails() {
    let r = JiebaDict::load(&[]);
    assert!(r.is_err());
}

#[test]
fn jieba_dict_load_valid_header() {
    let bytes = minimal_dict_bin();
    let d = JiebaDict::load(&bytes).expect("load");
    assert_eq!(d.version(), "2026.08");
    assert_eq!(d.sha256_prefix(), [0u8; 8]);
}

// ============================================================
// Task 2: DAT 查询 + 词频
// ============================================================

#[test]
fn dict_lookup_word_freq() {
    let bytes = dict_bin_with_words(&[("机器学习", 100), ("学习", 200), ("机器", 50)]);
    let d = JiebaDict::load(&bytes).unwrap();
    assert_eq!(d.freq("机器学习"), Some(100));
    assert_eq!(d.freq("学习"), Some(200));
    assert_eq!(d.freq("机器"), Some(50));
    assert_eq!(d.freq("不存在词"), None);
}

#[test]
fn dict_prefix_match() {
    let bytes = dict_bin_with_words(&[("机器学习", 100), ("机器", 50)]);
    let d = JiebaDict::load(&bytes).unwrap();
    let prefixes = d.common_prefix_search("机器学习");
    assert!(prefixes.contains(&"机器".to_string()));
    assert!(prefixes.contains(&"机器学习".to_string()));
}

// ============================================================
// Task 3: DAG 最大概率切分
// ============================================================

#[test]
fn dag_segment_known_words() {
    let bytes = dict_bin_with_words(&[("我", 100), ("爱", 100), ("北京", 200), ("天安门", 300)]);
    let d = std::sync::Arc::new(JiebaDict::load(&bytes).unwrap());
    let tok = JiebaTokenizer::new(d, &[]).unwrap();
    let toks = tok.tokenize("我爱北京天安门");
    let texts: Vec<_> = toks.iter().map(|t| t.text.as_str()).collect();
    assert_eq!(texts, vec!["我", "爱", "北京", "天安门"]);
}

#[test]
fn dag_picks_higher_freq_path() {
    let bytes = dict_bin_with_words(&[("研究", 100), ("研究生", 50), ("生命", 200), ("命", 10)]);
    let d = std::sync::Arc::new(JiebaDict::load(&bytes).unwrap());
    let tok = JiebaTokenizer::new(d, &[]).unwrap();
    let toks = tok.tokenize("研究生命");
    let texts: Vec<_> = toks.iter().map(|t| t.text.as_str()).collect();
    assert!(texts.contains(&"研究"));
    assert!(texts.contains(&"生命"));
}

// ============================================================
// Task 4: HMM 未登录词识别
// ============================================================

#[test]
fn hmm_recognizes_unknown_word() {
    let bytes = dict_bin_with_words(&[("我", 100), ("的", 100)]);
    let d = std::sync::Arc::new(JiebaDict::load(&bytes).unwrap());
    let tok = JiebaTokenizer::new(d, &[]).unwrap();
    let toks = tok.tokenize("蓝瘦香菇");
    assert!(!toks.is_empty(), "HMM 应对未知字产生非空切分");
}

// ============================================================
// Task 5: 中英混排 + position 连续
// ============================================================

#[test]
fn mixed_script_positions_continuous() {
    let bytes = dict_bin_with_words(&[("机器学习", 100)]);
    let d = std::sync::Arc::new(JiebaDict::load(&bytes).unwrap());
    let tok = JiebaTokenizer::new(d, &[]).unwrap();
    let toks = tok.tokenize("机器学习 running");
    assert_eq!(toks[0].text, "机器学习");
    assert_eq!(toks[0].position, 0);
    assert_eq!(toks[1].text, "run");
    assert_eq!(toks[1].position, 1);
}

#[test]
fn latin_run_uses_standard_pipeline() {
    let bytes = dict_bin_with_words(&[]);
    let d = std::sync::Arc::new(JiebaDict::load(&bytes).unwrap());
    let tok = JiebaTokenizer::new(d, &[]).unwrap();
    let toks = tok.tokenize("Running RUNNERS");
    assert_eq!(toks[0].text, "run");
    assert_eq!(toks[1].text, "runner");
}

// ============================================================
// Task 6: 用户词表优先级（§5.3）
// ============================================================

#[test]
fn user_dict_overrides_builtin() {
    let bytes = dict_bin_with_words(&[("机器学习", 100)]);
    let d = std::sync::Arc::new(JiebaDict::load(&bytes).unwrap());
    let user = vec![UserDictEntry::WordWithFreq {
        term: "机器学习".into(),
        freq: 999,
    }];
    let tok = JiebaTokenizer::new(d, &user).unwrap();
    let toks = tok.tokenize("机器学习");
    assert_eq!(toks[0].text, "机器学习");
}

#[test]
fn user_dict_new_word_single_token() {
    // 验收锚点③：生造词注入后单 token 入索引
    let bytes = dict_bin_with_words(&[]);
    let d = std::sync::Arc::new(JiebaDict::load(&bytes).unwrap());
    let user = vec![UserDictEntry::Word("布地奈德".into())];
    let tok = JiebaTokenizer::new(d, &user).unwrap();
    let toks = tok.tokenize("布地奈德治疗效果");
    assert!(
        toks.iter().any(|t| t.text == "布地奈德"),
        "生造词应作为单 token 出现: {:?}",
        toks
    );
}

// ============================================================
// Task 7: build_tokenizer 接入 + TokenizerId（R-3：无二次哈希）
// ============================================================

#[test]
fn jieba_tokenizer_id_independent_of_dict_calendar_version() {
    // R-3：词典日历版本/内容变化不改变 TokenizerId（仅格式变化才改）。
    let dict_v1 =
        std::sync::Arc::new(JiebaDict::load(&dict_bin_with_words(&[("测试", 100)])).unwrap());
    let dict_v2 = std::sync::Arc::new(
        JiebaDict::load(&dict_bin_with_words(&[("测试", 100), ("新词", 50)])).unwrap(),
    );
    let t1 = JiebaTokenizer::new(dict_v1, &[]).unwrap();
    let t2 = JiebaTokenizer::new(dict_v2, &[]).unwrap();
    assert_eq!(
        t1.id(),
        t2.id(),
        "dict content change must not change TokenizerId (R-3, REQUIREMENTS §3.3)"
    );
}

#[test]
fn jieba_tokenizer_id_uses_compute_tokenizer_id() {
    // JiebaTokenizer::id() 直接用 compute_tokenizer_id(Jieba, user_dict)，无二次哈希。
    let d = std::sync::Arc::new(JiebaDict::load(&minimal_dict_bin()).unwrap());
    let user = vec![UserDictEntry::Word("用户词".into())];
    let tok = JiebaTokenizer::new(d, &user).unwrap();
    let expected = crate::tokenizer::compute_tokenizer_id(BuiltinTokenizer::Jieba, &user);
    assert_eq!(tok.id().as_bytes(), expected.as_bytes());
}
