//! HMM 未登录词识别（SPEC §5.1）：B/M/E/S 四状态 Viterbi 解码。
//!
//! 算法与 jieba-rs 完全一致（红线：不发明切分规则）。转移矩阵 + 发射矩阵
//! 从 dict.bin 的 hmm_blob 反序列化；07 的生成脚本写入 jieba 原版参数。
//!
//! hmm_blob 格式（小端）：
//! ```text
//! start_p : [f64; 4]                    // B, M, E, S 起始概率（log）
//! trans   : [f64; 16]                   // trans[from*4 + to]，from/to ∈ {B,M,E,S}
//! emit_counts : [u32; 4]                // 各状态发射条目数
//! for each state s:
//!     repeat emit_counts[s]:
//!         char_code : u32               // Unicode 标量值
//!         prob      : f64               // log 概率
//! ```

use std::collections::HashMap;

use crate::types::{Result, VaneError};

/// jieba HMM 极小概率（log 空间，近似 -inf）。
const MIN_FLOAT: f64 = -3.14e100;

/// 状态索引：B=0, M=1, E=2, S=3（与 jieba 一致）。
const B: usize = 0;
const M: usize = 1;
const E: usize = 2;
const S: usize = 3;
const STATES: [usize; 4] = [B, M, E, S];

/// HMM 参数（转移矩阵 + 发射矩阵，从 dict.bin 反序列化）。
pub struct HmmParams {
    start_p: [f64; 4],
    trans: [[f64; 4]; 4], // trans[from][to]
    emit: [HashMap<u32, f64>; 4],
}

impl HmmParams {
    /// 从 hmm_blob 反序列化。
    pub fn deserialize(blob: &[u8]) -> Result<Self> {
        let mut cur = 0usize;
        let start_p_vec = take_f64_array(blob, &mut cur, 4)
            .ok_or_else(|| VaneError::Corrupt("hmm_blob too short for start_p".into()))?;
        let mut start_p = [0.0f64; 4];
        start_p.copy_from_slice(&start_p_vec);
        let trans_flat = take_f64_array(blob, &mut cur, 16)
            .ok_or_else(|| VaneError::Corrupt("hmm_blob too short for trans".into()))?;
        let mut trans = [[0.0f64; 4]; 4];
        for i in 0..4 {
            for j in 0..4 {
                trans[i][j] = trans_flat[i * 4 + j];
            }
        }
        let emit_counts = take_u32_array(blob, &mut cur, 4)
            .ok_or_else(|| VaneError::Corrupt("hmm_blob too short for emit_counts".into()))?;
        let mut emit: [HashMap<u32, f64>; 4] = Default::default();
        for s in 0..4 {
            let n = emit_counts[s] as usize;
            for _ in 0..n {
                let cc = take_u32(blob, &mut cur)
                    .ok_or_else(|| VaneError::Corrupt("hmm_blob too short for emit char".into()))?;
                let prob = take_f64(blob, &mut cur)
                    .ok_or_else(|| VaneError::Corrupt("hmm_blob too short for emit prob".into()))?;
                emit[s].insert(cc, prob);
            }
        }
        Ok(HmmParams {
            start_p,
            trans,
            emit,
        })
    }

    /// 发射概率（未知字返回 MIN_FLOAT，与 jieba 一致）。
    fn emit_p(&self, state: usize, c: char) -> f64 {
        self.emit[state]
            .get(&(c as u32))
            .copied()
            .unwrap_or(MIN_FLOAT)
    }

    /// Viterbi 解码 → 状态序列（B/M/E/S）→ 词列表。
    ///
    /// 与 jieba `viterbi` 一致：末位仅考虑 E/S（词必须以 E 或 S 结尾）。
    pub fn cut(&self, chars: &[char]) -> Vec<String> {
        if chars.is_empty() {
            return Vec::new();
        }
        let n = chars.len();

        // V[t][state] = 最佳累积概率；path[t][state] = 前驱状态
        let mut full_v: Vec<[f64; 4]> = Vec::with_capacity(n);
        let mut full_path: Vec<[usize; 4]> = Vec::with_capacity(n);

        // t=0
        let mut row = [MIN_FLOAT; 4];
        for &s in &STATES {
            row[s] = self.start_p[s] + self.emit_p(s, chars[0]);
        }
        full_v.push(row);

        // t=1..n
        for t in 1..n {
            let mut cur_row = [MIN_FLOAT; 4];
            let mut cur_path = [0usize; 4];
            for &s in &STATES {
                let mut best = MIN_FLOAT;
                let mut best_prev = 0usize;
                for &p in &STATES {
                    let cand = full_v[t - 1][p] + self.trans[p][s];
                    if cand > best {
                        best = cand;
                        best_prev = p;
                    }
                }
                cur_row[s] = best + self.emit_p(s, chars[t]);
                cur_path[s] = best_prev;
            }
            full_v.push(cur_row);
            full_path.push(cur_path);
        }

        // 末位仅 E/S（词必须以 E 或 S 结尾，与 jieba viterbi 一致）
        let best_state = if full_v[n - 1][E] > full_v[n - 1][S] {
            E
        } else {
            S
        };

        // 回溯
        let mut states: Vec<usize> = Vec::with_capacity(n);
        states.push(best_state);
        for t in (1..n).rev() {
            let prev = full_path[t - 1][states[states.len() - 1]];
            states.push(prev);
        }
        states.reverse();

        // 状态序列 → 词（B..E 为一词，S 为单字词）
        decode_states(chars, &states)
    }
}

/// 将 B/M/E/S 状态序列解码为词列表。
fn decode_states(chars: &[char], states: &[usize]) -> Vec<String> {
    let mut words = Vec::new();
    let n = chars.len();
    let mut i = 0;
    while i < n {
        match states[i] {
            B => {
                // 找到对应的 E
                let mut j = i + 1;
                while j < n && states[j] != E {
                    j += 1;
                }
                if j < n {
                    words.push(chars[i..=j].iter().collect());
                    i = j + 1;
                } else {
                    // 未找到 E（异常），单字兜底
                    words.push(chars[i].to_string());
                    i += 1;
                }
            }
            S => {
                words.push(chars[i].to_string());
                i += 1;
            }
            _ => {
                // M/E 出现在非 B 起始（异常），单字兜底
                words.push(chars[i].to_string());
                i += 1;
            }
        }
    }
    words
}

// ---- 小端读取辅助 ----

fn take_u32(bytes: &[u8], cur: &mut usize) -> Option<u32> {
    let s = take(bytes, cur, 4)?;
    Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn take_f64(bytes: &[u8], cur: &mut usize) -> Option<f64> {
    let s = take(bytes, cur, 8)?;
    Some(f64::from_le_bytes([
        s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7],
    ]))
}

fn take<'a>(bytes: &'a [u8], cur: &mut usize, n: usize) -> Option<&'a [u8]> {
    if *cur + n > bytes.len() {
        return None;
    }
    let s = &bytes[*cur..*cur + n];
    *cur += n;
    Some(s)
}

fn take_f64_array(bytes: &[u8], cur: &mut usize, n: usize) -> Option<Vec<f64>> {
    let s = take(bytes, cur, n * 8)?;
    let mut v = Vec::with_capacity(n);
    for chunk in s.as_chunks::<8>().0 {
        v.push(f64::from_le_bytes(*chunk));
    }
    Some(v)
}

fn take_u32_array(bytes: &[u8], cur: &mut usize, n: usize) -> Option<Vec<u32>> {
    let s = take(bytes, cur, n * 4)?;
    let mut v = Vec::with_capacity(n);
    for chunk in s.as_chunks::<4>().0 {
        v.push(u32::from_le_bytes(*chunk));
    }
    Some(v)
}
