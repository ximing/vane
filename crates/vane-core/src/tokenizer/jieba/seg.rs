//! jieba 切分内核（SPEC §5.1）：前缀 DAG 最大概率切分 + HMM 未登录词识别。
//!
//! 算法与 jieba-rs 完全一致（红线：不发明切分规则）：
//! 1. `build_dag`：对 CJK run 每个起始位置，前缀搜索词典 + 用户词表，构建 DAG。
//! 2. `calc`：动态规划求最大概率路径（权重 = ln(freq/total)）。
//! 3. `cut`：走最大概率路径；连续单字缓冲后交 HMM Viterbi 识别新词。

use std::collections::HashMap;

use super::dict::JiebaDict;

/// 用户词表 trie（运行期注入，与内置 DAT 并行查询）。
///
/// 用户词 freq 覆盖内置同词（SPEC §5.3：用户词 > 内置词）。
/// `UserDictEntry::Word(term)` 缺省 freq = 内置词典最高频值（保证 DAG 优先命中）。
pub(super) struct UserTrie {
    children: Vec<HashMap<u32, usize>>,
    freqs: Vec<i32>, // -1 = 非终态，否则词频
}

impl UserTrie {
    pub(super) fn new() -> Self {
        UserTrie {
            children: vec![HashMap::new()],
            freqs: vec![-1],
        }
    }

    /// 插入用户词。freq 缺省值由调用方传入（= dict.max_freq()）。
    pub(super) fn insert(&mut self, word: &str, freq: u32) {
        let mut node = 0usize;
        for c in word.chars() {
            let cc = c as u32;
            let next = self.children[node].get(&cc).copied();
            match next {
                Some(n) => node = n,
                None => {
                    let new_id = self.children.len();
                    self.children.push(HashMap::new());
                    self.freqs.push(-1);
                    self.children[node].insert(cc, new_id);
                    node = new_id;
                }
            }
        }
        self.freqs[node] = freq as i32;
    }

    /// 前缀搜索：从 chars[start..] 出发，返回 (end_exclusive, freq)。
    fn prefix_search(&self, chars: &[char], start: usize) -> Vec<(usize, u32)> {
        let mut result = Vec::new();
        let mut node = 0usize;
        let mut i = start;
        while i < chars.len() {
            let cc = chars[i] as u32;
            match self.children[node].get(&cc).copied() {
                Some(n) => {
                    node = n;
                    if self.freqs[node] >= 0 {
                        result.push((i + 1, self.freqs[node] as u32));
                    }
                    i += 1;
                }
                None => break,
            }
        }
        result
    }
}

/// 对 CJK run 执行 DAG + HMM 切分，返回词列表。
pub(super) fn cut(chars: &[char], dict: &JiebaDict, user: &UserTrie) -> Vec<String> {
    let n = chars.len();
    if n == 0 {
        return Vec::new();
    }
    let dag = build_dag(chars, dict, user);
    let route = calc(chars, &dag, dict.total_freq());

    let mut words = Vec::new();
    let mut buf: Vec<char> = Vec::new();
    let mut x = 0;
    while x < n {
        let end = route[x].1;
        if end - x == 1 {
            buf.push(chars[x]);
        } else {
            if !buf.is_empty() {
                words.extend(dict.hmm().cut(&buf));
                buf.clear();
            }
            words.push(chars[x..end].iter().collect());
        }
        x = end;
    }
    if !buf.is_empty() {
        words.extend(dict.hmm().cut(&buf));
    }
    words
}

/// 构建 DAG：dag[i] = [(end_exclusive, freq), ...]。
fn build_dag(chars: &[char], dict: &JiebaDict, user: &UserTrie) -> Vec<Vec<(usize, u32)>> {
    let n = chars.len();
    let mut dag = vec![Vec::new(); n];
    for (i, dag_i) in dag.iter_mut().enumerate() {
        let mut matches = dict.prefix_search_freq(chars, i);
        for (end, freq) in user.prefix_search(chars, i) {
            if let Some(pos) = matches.iter().position(|(e, _)| *e == end) {
                matches[pos].1 = freq; // 用户词覆盖内置同词
            } else {
                matches.push((end, freq));
            }
        }
        if matches.is_empty() {
            *dag_i = vec![(i + 1, 0)]; // 单字兜底（freq=0 → calc 按 1 计）
        } else {
            *dag_i = matches;
        }
    }
    dag
}

/// 最大概率路径（动态规划，从末尾向前）。
/// route[i] = (累积 ln 概率, end_exclusive)。
fn calc(chars: &[char], dag: &[Vec<(usize, u32)>], total_freq: u64) -> Vec<(f64, usize)> {
    let n = chars.len();
    let mut route = vec![(0.0f64, n); n + 1];
    let log_total = (total_freq.max(1) as f64).ln();
    for i in (0..n).rev() {
        let mut best = f64::NEG_INFINITY;
        let mut best_end = i + 1;
        for &(end, freq) in &dag[i] {
            let f = if freq == 0 { 1.0 } else { freq as f64 };
            let log_p = f.ln() - log_total + route[end].0;
            if log_p > best {
                best = log_p;
                best_end = end;
            }
        }
        route[i] = (best, best_end);
    }
    route
}
