//! jieba 词典（SPEC §5.2）：双数组 Trie（DAT）序列化 + zstd 压缩。
//!
//! `dict.bin` 物理格式（zstd 压缩，解压后布局，全部小端）：
//! ```text
//! [0..4]   magic              = b"VNDT"
//! [4..8]   format_version     : u32 = 1
//! [8..16]  sha256_prefix      : [u8; 8]   // 词典内容 sha256 前 8 字节
//! [16..18] dict_version_len   : u16
//! [18..]   dict_version       : UTF-8     // 日历版本如 "2026.08"
//!          total_freq         : u64       // Σfreq（概率计算分母）
//!          dat_len            : u32       // base/check/values 数组长度
//!          base               : [i32; dat_len]
//!          check              : [i32; dat_len]   // -1 = 空
//!          values             : [i32; dat_len]   // >= 0 = 词频（终态节点）；-1 = 非终态
//!          hmm_blob_len       : u32
//!          hmm_blob           : [u8]              // HMM 参数（见 hmm.rs）
//! ```
//!
//! DAT 转移：`t = base[node] + char_code`；若 `check[t] == node` 则 t 为合法后继。
//! 终态节点 `values[t] >= 0` 存词频。char_code = Unicode 标量值（u32）。

use crate::types::{Result, VaneError};

use super::hmm::HmmParams;

const MAGIC: &[u8; 4] = b"VNDT";
const FORMAT_VERSION: u32 = 1;

/// jieba 词典（DAT + 词频 + HMM 参数）。
///
/// owned 所有数组（解析时从 bytes 拷贝；冷加载 <150ms，见 §13.1 bench）。
/// 词典**内容**（词条/日历版本）不进 TokenizerId（R-3）；仅**格式**常量进 id（见 id.rs）。
pub struct JiebaDict {
    sha256_prefix: [u8; 8],
    dict_version: String,
    total_freq: u64,
    max_freq: u32,
    base: Vec<i32>,
    check: Vec<i32>,
    values: Vec<i32>,
    hmm: HmmParams,
}

impl JiebaDict {
    /// 解析已解压的 dict.bin 字节（零分配解析头部，数组拷贝）。
    pub fn load(bytes: &[u8]) -> Result<Self> {
        parse(bytes).map_err(|e| {
            crate::types::append_context(
                e,
                " (op=dict load; 建议: 词典数据损坏，重新构建或联系支持)",
            )
        })
    }

    /// 解析 zstd 压缩的 dict.bin 字节（绑定层调用：Node/Go 加载 dict.bin 后调此）。
    pub fn load_zstd(compressed: &[u8]) -> Result<Self> {
        use std::io::Read;
        let mut decoder = ruzstd::streaming_decoder::StreamingDecoder::new(compressed).map_err(|e| {
            VaneError::Corrupt(format!(
                "dict.bin zstd decompress failed: {} (op=dict load; 建议: 词典数据损坏，重新构建或联系支持)",
                e
            ))
        })?;
        let mut buf = Vec::with_capacity(compressed.len() * 4);
        decoder.read_to_end(&mut buf).map_err(|e| {
            VaneError::Corrupt(format!(
                "dict.bin zstd read failed: {} (op=dict load; 建议: 词典数据损坏，重新构建或联系支持)",
                e
            ))
        })?;
        Self::load(&buf)
    }

    /// 词典日历版本（如 "2026.08"），供 §12.3 三渠道一致性 + §3.3 升级警告。不进 TokenizerId。
    pub fn version(&self) -> &str {
        &self.dict_version
    }

    /// 词典内容 sha256 前 8 字节，供一致性校验。不进 TokenizerId。
    pub fn sha256_prefix(&self) -> [u8; 8] {
        self.sha256_prefix
    }

    /// Σfreq（概率计算分母）。
    pub fn total_freq(&self) -> u64 {
        self.total_freq
    }

    /// 内置词典最高词频（用户词缺省 freq，保证 DAG 优先命中，SPEC §5.3）。
    pub fn max_freq(&self) -> u32 {
        self.max_freq
    }

    /// HMM 参数引用。
    pub(super) fn hmm(&self) -> &HmmParams {
        &self.hmm
    }

    /// 查词频。词在 DAT 终态节点 → 返回词频；否则 None。
    pub fn freq(&self, word: &str) -> Option<u32> {
        let node = self.traverse(word.chars())?;
        let v = self.values[node];
        if v >= 0 {
            Some(v as u32)
        } else {
            None
        }
    }

    /// 前缀搜索：返回 `text` 的所有词典前缀词（字符串形式，测试/调试用）。
    pub fn common_prefix_search(&self, text: &str) -> Vec<String> {
        let chars: Vec<char> = text.chars().collect();
        let mut result = Vec::new();
        let mut node: usize = 0;
        for i in 0..chars.len() {
            let cc = chars[i] as u32 as i32;
            let t = match self.base.get(node).map(|b| *b + cc) {
                Some(t) if t >= 0 && (t as usize) < self.check.len() => t as usize,
                _ => break,
            };
            if self.check[t] != node as i32 {
                break;
            }
            node = t;
            if self.values[node] >= 0 {
                result.push(chars[0..=i].iter().collect());
            }
        }
        result
    }

    /// DAG 前缀搜索：从 `chars[start..]` 出发，返回所有 (end_pos_exclusive, freq)。
    /// 供 seg.rs build_dag 使用。
    pub(super) fn prefix_search_freq(&self, chars: &[char], start: usize) -> Vec<(usize, u32)> {
        let mut result = Vec::new();
        let mut node: usize = 0;
        let mut i = start;
        while i < chars.len() {
            let cc = chars[i] as u32 as i32;
            let t = match self.base.get(node).map(|b| *b + cc) {
                Some(t) if t >= 0 && (t as usize) < self.check.len() => t as usize,
                _ => break,
            };
            if self.check[t] != node as i32 {
                break;
            }
            node = t;
            let v = self.values[node];
            if v >= 0 {
                result.push((i + 1, v as u32));
            }
            i += 1;
        }
        result
    }

    /// 遍历 DAT 到词末节点，返回节点索引（非终态也返回——调用方检查 values）。
    fn traverse<I: IntoIterator<Item = char>>(&self, chars: I) -> Option<usize> {
        let mut node: usize = 0;
        for c in chars {
            let cc = c as u32 as i32;
            let t = *self.base.get(node)? + cc;
            if t < 0 || (t as usize) >= self.check.len() {
                return None;
            }
            let t = t as usize;
            if self.check[t] != node as i32 {
                return None;
            }
            node = t;
        }
        Some(node)
    }
}

/// 解析 dict.bin（已解压）字节。
fn parse(bytes: &[u8]) -> Result<JiebaDict> {
    let mut cur = 0usize;
    let magic = take(bytes, &mut cur, 4)
        .ok_or_else(|| VaneError::Corrupt("dict.bin too short for magic".into()))?;
    if magic != MAGIC {
        return Err(VaneError::Corrupt(format!(
            "dict.bin magic mismatch: expected {:?}, got {:?}",
            MAGIC, magic
        )));
    }
    let format_version = take_u32(bytes, &mut cur)
        .ok_or_else(|| VaneError::Corrupt("dict.bin too short for format_version".into()))?;
    if format_version != FORMAT_VERSION {
        return Err(VaneError::Version(format!(
            "dict.bin format_version {} unsupported (expected {})",
            format_version, FORMAT_VERSION
        )));
    }
    let sha256_prefix: [u8; 8] = take(bytes, &mut cur, 8)
        .ok_or_else(|| VaneError::Corrupt("dict.bin too short for sha256_prefix".into()))?
        .try_into()
        .map_err(|_| VaneError::Corrupt("sha256_prefix slice len".into()))?;
    let ver_len = take_u16(bytes, &mut cur)
        .ok_or_else(|| VaneError::Corrupt("dict.bin too short for version_len".into()))?
        as usize;
    let dict_version_bytes = take(bytes, &mut cur, ver_len)
        .ok_or_else(|| VaneError::Corrupt("dict.bin too short for version".into()))?;
    let dict_version = std::str::from_utf8(dict_version_bytes)
        .map_err(|e| VaneError::Corrupt(format!("dict_version not utf8: {}", e)))?
        .to_string();
    let total_freq = take_u64(bytes, &mut cur)
        .ok_or_else(|| VaneError::Corrupt("dict.bin too short for total_freq".into()))?;
    let dat_len = take_u32(bytes, &mut cur)
        .ok_or_else(|| VaneError::Corrupt("dict.bin too short for dat_len".into()))?
        as usize;
    let base = take_i32_slice(bytes, &mut cur, dat_len)
        .ok_or_else(|| VaneError::Corrupt("dict.bin too short for base".into()))?;
    let check = take_i32_slice(bytes, &mut cur, dat_len)
        .ok_or_else(|| VaneError::Corrupt("dict.bin too short for check".into()))?;
    let values = take_i32_slice(bytes, &mut cur, dat_len)
        .ok_or_else(|| VaneError::Corrupt("dict.bin too short for values".into()))?;
    let hmm_blob_len = take_u32(bytes, &mut cur)
        .ok_or_else(|| VaneError::Corrupt("dict.bin too short for hmm_blob_len".into()))?
        as usize;
    let hmm_blob = take(bytes, &mut cur, hmm_blob_len)
        .ok_or_else(|| VaneError::Corrupt("dict.bin too short for hmm_blob".into()))?;
    let hmm = HmmParams::deserialize(hmm_blob)?;

    let max_freq = values
        .iter()
        .copied()
        .filter(|&v| v >= 0)
        .map(|v| v as u32)
        .max()
        .unwrap_or(0);

    Ok(JiebaDict {
        sha256_prefix,
        dict_version,
        total_freq,
        max_freq,
        base,
        check,
        values,
        hmm,
    })
}

// ---- 小端整数读取辅助 ----

fn take<'a>(bytes: &'a [u8], cur: &mut usize, n: usize) -> Option<&'a [u8]> {
    if *cur + n > bytes.len() {
        return None;
    }
    let s = &bytes[*cur..*cur + n];
    *cur += n;
    Some(s)
}

fn take_u16(bytes: &[u8], cur: &mut usize) -> Option<u16> {
    let s = take(bytes, cur, 2)?;
    Some(u16::from_le_bytes([s[0], s[1]]))
}

fn take_u32(bytes: &[u8], cur: &mut usize) -> Option<u32> {
    let s = take(bytes, cur, 4)?;
    Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn take_u64(bytes: &[u8], cur: &mut usize) -> Option<u64> {
    let s = take(bytes, cur, 8)?;
    Some(u64::from_le_bytes([
        s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7],
    ]))
}

fn take_i32_slice(bytes: &[u8], cur: &mut usize, n: usize) -> Option<Vec<i32>> {
    let s = take(bytes, cur, n * 4)?;
    let mut v = Vec::with_capacity(n);
    for chunk in s.chunks_exact(4) {
        v.push(i32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }
    Some(v)
}
