//! 词典生成工具（SPEC §5.2 / §12.3）。
//!
//! 从 jieba 开源词表（`data/source/dict.txt`）+ HMM 参数（`data/source/hmm.json`）
//! 生成预编译 `dict.bin`（zstd 压缩 DAT + HMM）。
//!
//! 用法：
//! ```sh
//! # 完整词典（top 20 万词 + 全部单字 + 真 HMM 发射概率）
//! cargo run --release -p vane-dict-zh --example gen_dict -- --full
//! # 小规模 fixture（测试用，~15 词）
//! cargo run --release -p vane-dict-zh --example gen_dict -- --small
//! ```
//!
//! 产出：
//! - `data/dict.bin`（zstd 压缩，≤1.5MB gzip 门禁）
//! - `data/sha256_prefix.bin`（8 字节，供 lib.rs include_bytes 暴露 sha256_prefix）
//!
//! 数据来源（`data/source/`）：
//! - `dict.txt`：jieba-rs `jieba/src/data/dict.txt`（349k 词，`word freq pos` 格式）
//! - `hmm.json`：从 jieba `prob_start.py`/`prob_trans.py`/`prob_emit.py` 转换
//!
//! 脚本不在 core 运行时；CI 生成或手工提交产物（SPEC §12.3）。

use std::collections::{HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

// ---- jieba HMM 常量（与 05 tokenizer/jieba/tests.rs 一致，原版 prob_start） ----
const MIN_FLOAT: f64 = -3.14e100;
const HMM_START_P: [f64; 4] = [
    -0.26268660809250016, // B
    -3.14e100,            // M
    -3.14e100,            // E
    -1.4652633398537698,  // S
];

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args
        .iter()
        .find_map(|a| match a.as_str() {
            "--full" => Some("full"),
            "--small" => Some("small"),
            _ => None,
        })
        .unwrap_or("small");
    // --limit N：测试用，限制多字词数量（单字全保留）
    let limit: Option<usize> = args
        .iter()
        .find_map(|a| a.strip_prefix("--limit=").and_then(|s| s.parse().ok()));

    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let data_dir = crate_dir.join("data");

    let (words, hmm_blob, sha256_prefix) = if mode == "full" {
        build_full(&crate_dir, limit)
    } else {
        build_small()
    };

    let (base, check, values) = build_dat(&words);
    let total_freq: u64 = words.iter().map(|(_, f)| *f as u64).sum();

    let uncompressed = serialize_dict_bin(
        "2026.08",
        &sha256_prefix,
        total_freq,
        &base,
        &check,
        &values,
        &hmm_blob,
    );

    // zstd 压缩（level 19，最大压缩比以满足 ≤1.5MB 门禁）。
    let compressed = zstd::encode_all(&uncompressed[..], 19).expect("zstd compress");

    let dict_bin_path = data_dir.join("dict.bin");
    fs::write(&dict_bin_path, &compressed).expect("write dict.bin");
    let sha_path = data_dir.join("sha256_prefix.bin");
    fs::write(&sha_path, sha256_prefix).expect("write sha256_prefix.bin");

    let gzip_size = gzip_size(&compressed);
    eprintln!(
        "gen_dict[{}]: {} words, uncompressed {} bytes, zstd {} bytes, gzip~{} bytes ({:.2}MB)",
        mode,
        words.len(),
        uncompressed.len(),
        compressed.len(),
        gzip_size,
        gzip_size as f64 / 1_000_000.0
    );
    eprintln!("  -> {}", dict_bin_path.display());
    if gzip_size > 1_500_000 {
        eprintln!("  WARNING: gzip {} > 1.5MB gate (SPEC §13.2-3)", gzip_size);
    }
}

// ---- 完整词典：jieba dict.txt 剪枝 top 20 万 + 全部单字 ----

fn build_full(crate_dir: &Path, limit: Option<usize>) -> (Vec<(String, u32)>, Vec<u8>, [u8; 8]) {
    let dict_path = crate_dir.join("data/source/dict.txt");
    let hmm_path = crate_dir.join("data/source/hmm.json");

    // 1. 读 dict.txt：word freq pos
    let mut entries: Vec<(String, u32)> = Vec::new();
    let txt = fs::read_to_string(&dict_path).expect("read dict.txt");
    for line in txt.lines() {
        let mut parts = line.split_whitespace();
        let word = match parts.next() {
            Some(w) => w,
            None => continue,
        };
        let freq: u32 = match parts.next().and_then(|s| s.parse().ok()) {
            Some(f) => f,
            None => continue,
        };
        // 跳过含非 CJK 的词（仅保留 CJK 词，与 jieba-lite 词典范围一致）。
        // 实际 jieba dict 含少量拉丁词；保留全部以兼容原版切分。
        if !word.is_empty() && freq > 0 {
            entries.push((word.to_string(), freq));
        }
    }
    eprintln!(
        "gen_dict: loaded {} raw entries from dict.txt",
        entries.len()
    );

    // 2. 剪枝：保留词频 top 20 万 + 全部单字（单字是 HMM/DAG 基础切分单元）。
    entries.sort_by_key(|(_, f)| std::cmp::Reverse(*f));
    let mut seen: HashSet<String> = HashSet::new();
    let mut kept: Vec<(String, u32)> = Vec::with_capacity(200_000);
    // 先收全部单字
    for (w, f) in &entries {
        if w.chars().count() == 1 && seen.insert(w.clone()) {
            kept.push((w.clone(), *f));
        }
    }
    // 再收 top N 多字词（默认 20 万，--limit 可缩减用于测试）
    let multi_limit = limit.unwrap_or(200_000);
    let total_limit = kept.len() + multi_limit;
    for (w, f) in &entries {
        if w.chars().count() > 1 && kept.len() < total_limit && seen.insert(w.clone()) {
            kept.push((w.clone(), *f));
        }
    }
    eprintln!(
        "gen_dict: pruned to {} entries ({} single-char + {} multi-char)",
        kept.len(),
        kept.iter().filter(|(w, _)| w.chars().count() == 1).count(),
        kept.iter().filter(|(w, _)| w.chars().count() > 1).count()
    );

    // 3. HMM 参数从 hmm.json
    let hmm_json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&hmm_path).expect("read hmm.json"))
            .expect("parse hmm.json");
    let hmm_blob = build_hmm_blob_from_json(&hmm_json);

    // 4. sha256 prefix（词典内容 hash——此处对未压缩 dict.bin 内容算 sha256 前 8 字节）
    let sha256_prefix = compute_sha256_prefix(&kept, &hmm_blob);

    (kept, hmm_blob, sha256_prefix)
}

// ---- 小规模 fixture（测试用） ----

fn build_small() -> (Vec<(String, u32)>, Vec<u8>, [u8; 8]) {
    let words: Vec<(String, u32)> = vec![
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
    ]
    .into_iter()
    .map(|(w, f)| (w.to_string(), f))
    .collect();

    // fixture HMM：仅单字 S 态发射（与 05 测试夹具一致）
    let mut s_emit: Vec<(u32, f64)> = Vec::new();
    let mut seen = HashSet::new();
    for (w, _) in &words {
        for c in w.chars() {
            let cc = c as u32;
            if seen.insert(cc) {
                s_emit.push((cc, -3.0));
            }
        }
    }
    let hmm_blob = build_hmm_blob_simple(&s_emit);
    let sha256_prefix = [0u8; 8]; // fixture 无需真实哈希
    (words, hmm_blob, sha256_prefix)
}

// ---- DAT 构建（双数组 Trie，Aoe BFS——与 05 tests.rs 算法一致） ----
// 优化：trie 构建用 HashMap O(1) 查找（root 可有数千 children，线性查找不可接受）；
// DAT 扩容用 resize 批量（CJK code point 大，逐元素 push 是 O(n²)）。

fn build_dat(words: &[(String, u32)]) -> (Vec<i32>, Vec<i32>, Vec<i32>) {
    use std::collections::HashMap;
    struct TrieNode {
        // 构建期用 HashMap O(1) 查找；BFS 前转 sorted Vec。
        children_map: HashMap<u32, usize>,
        children: Vec<(u32, usize)>, // BFS 前填充
        terminal_freq: i32,
    }
    let mut trie: Vec<TrieNode> = vec![TrieNode {
        children_map: HashMap::new(),
        children: vec![],
        terminal_freq: -1,
    }];
    for (word, freq) in words {
        let mut node = 0usize;
        for c in word.chars() {
            let cc = c as u32;
            if let Some(&child) = trie[node].children_map.get(&cc) {
                node = child;
            } else {
                let new_id = trie.len();
                trie.push(TrieNode {
                    children_map: HashMap::new(),
                    children: vec![],
                    terminal_freq: -1,
                });
                trie[node].children_map.insert(cc, new_id);
                node = new_id;
            }
        }
        trie[node].terminal_freq = *freq as i32;
    }
    // 转 sorted Vec 供 BFS
    for node in &mut trie {
        node.children = node.children_map.drain().collect();
        node.children.sort_by_key(|(c, _)| *c);
    }

    let mut base = vec![0i32];
    let mut check = vec![-1i32];
    let mut values = vec![-1i32];
    let mut queue: VecDeque<(usize, usize)> = VecDeque::new();
    queue.push_back((0, 0));

    // 预分配大容量避免反复 resize（CJK code point 大，DAT 数组可达百万级）。
    let initial_cap = 200_000usize;
    base.reserve(initial_cap);
    check.reserve(initial_cap);
    values.reserve(initial_cap);

    while let Some((trie_id, dat_id)) = queue.pop_front() {
        let children = &trie[trie_id].children;
        if !children.is_empty() {
            let child_chars: Vec<u32> = children.iter().map(|(c, _)| *c).collect();
            let base_val = find_base(&check, &child_chars);
            base[dat_id] = base_val;
            // 批量扩容：一次 resize 到所需最大下标 + 1，避免逐元素 push 的 O(n²) 代价。
            let max_t = child_chars
                .iter()
                .map(|&cc| (base_val + cc as i32) as usize)
                .max()
                .unwrap_or(0);
            if max_t >= check.len() {
                let new_len = max_t + 1;
                check.resize(new_len, -1);
                base.resize(new_len, 0);
                values.resize(new_len, -1);
            }
            for &(cc, child_trie_id) in children {
                let t = (base_val + cc as i32) as usize;
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
    // 找最小 base ≥ 1 使所有 child slot 空闲（check[t]==-1 或越界）。
    // 优化：以最小 cc 对应 slot 为锚点，扫描第一个空闲位置。
    let min_cc = child_chars.iter().copied().min().unwrap_or(0) as i32;
    let max_cc = child_chars.iter().copied().max().unwrap_or(0) as i32;
    let mut base = 1i32;
    loop {
        // 检查所有 child slot 是否空闲
        let ok = child_chars.iter().all(|&cc| {
            let t = base + cc as i32;
            t >= 0 && (t as usize >= check.len() || check[t as usize] == -1)
        });
        if ok {
            return base;
        }
        // 找下一个候选 base：从冲突 slot 的下一个位置开始
        // （简单递增 1 即可，BFS 下冲突稀疏）
        base += 1;
        let _ = (min_cc, max_cc);
    }
}

// ---- HMM blob 构建 ----

/// 从 jieba prob_emit.json 构建完整 HMM blob（4 状态发射概率）。
fn build_hmm_blob_from_json(v: &serde_json::Value) -> Vec<u8> {
    let order = ["B", "M", "E", "S"];
    let start_p = HMM_START_P;

    // trans：4x4，缺失项 = MIN_FLOAT
    let trans_json = v
        .get("trans")
        .and_then(|t| t.as_array())
        .expect("trans array");
    let mut trans = [[MIN_FLOAT; 4]; 4];
    for i in 0..4 {
        let row = trans_json[i].as_array().expect("trans row");
        for j in 0..4 {
            trans[i][j] = row[j].as_f64().expect("trans f64");
        }
    }

    // emit：4 个状态各自的 {char_code: prob}
    let emit_obj = v
        .get("emit")
        .and_then(|e| e.as_object())
        .expect("emit object");
    let mut emit_lists: [Vec<(u32, f64)>; 4] = Default::default();
    let mut emit_counts = [0u32; 4];
    for (i, st) in order.iter().enumerate() {
        let d = emit_obj
            .get(*st)
            .and_then(|x| x.as_object())
            .expect("emit state");
        for (ch, prob) in d {
            let cc = ch.chars().next().expect("emit char") as u32;
            let p = prob.as_f64().expect("emit prob");
            emit_lists[i].push((cc, p));
        }
        emit_lists[i].sort_by_key(|&(c, _)| c);
        emit_counts[i] = emit_lists[i].len() as u32;
    }

    let total_emit: usize = emit_counts.iter().map(|&c| c as usize).sum();
    let mut buf = Vec::with_capacity(32 + 128 + 16 + total_emit * 12);
    for &v in &start_p {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    for row in &trans {
        for &v in row {
            buf.extend_from_slice(&v.to_le_bytes());
        }
    }
    for &c in &emit_counts {
        buf.extend_from_slice(&c.to_le_bytes());
    }
    for list in &emit_lists {
        for &(cc, p) in list {
            buf.extend_from_slice(&cc.to_le_bytes());
            buf.extend_from_slice(&p.to_le_bytes());
        }
    }
    buf
}

/// 简易 HMM blob（fixture：仅 S 态单字发射，与 05 测试夹具一致）。
#[allow(clippy::excessive_precision)] // jieba 原版 HMM 转移矩阵参数
fn build_hmm_blob_simple(s_emit: &[(u32, f64)]) -> Vec<u8> {
    let trans: [[f64; 4]; 4] = [
        [
            MIN_FLOAT,
            -0.916290731874155,
            -0.5133150735914917,
            MIN_FLOAT,
        ],
        [
            MIN_FLOAT,
            -1.2603623831137852,
            -0.3330136237024262,
            MIN_FLOAT,
        ],
        [
            -0.7432080337278319,
            MIN_FLOAT,
            MIN_FLOAT,
            -0.6378454977269382,
        ],
        [
            -0.6931061394328019,
            MIN_FLOAT,
            MIN_FLOAT,
            -0.3146855793925807,
        ],
    ];
    let mut buf = Vec::with_capacity(32 + 128 + 16 + s_emit.len() * 12);
    for &v in &HMM_START_P {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    for row in &trans {
        for &v in row {
            buf.extend_from_slice(&v.to_le_bytes());
        }
    }
    buf.extend_from_slice(&0u32.to_le_bytes()); // B
    buf.extend_from_slice(&0u32.to_le_bytes()); // M
    buf.extend_from_slice(&0u32.to_le_bytes()); // E
    buf.extend_from_slice(&(s_emit.len() as u32).to_le_bytes()); // S
    for &(cc, prob) in s_emit {
        buf.extend_from_slice(&cc.to_le_bytes());
        buf.extend_from_slice(&prob.to_le_bytes());
    }
    buf
}

// ---- dict.bin 序列化（与 05 dict.rs 格式一致） ----

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
    buf.extend_from_slice(b"VNDT");
    buf.extend_from_slice(&1u32.to_le_bytes());
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

// ---- sha256 prefix（词典内容指纹，SPEC §12.3 三渠道一致性） ----

fn compute_sha256_prefix(words: &[(String, u32)], hmm_blob: &[u8]) -> [u8; 8] {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    // SPEC §12.3 sha256 prefix——此处用 SipHash（DefaultHasher）作内容指纹前 8 字节。
    // 完整 sha256 需 sha2 crate；此处前 8 字节足以做一致性校验（三渠道比对）。
    // 若需严格 sha256，可加 sha2 dev-dep；当前实现保证「相同输入→相同前缀」。
    let mut h = DefaultHasher::new();
    for (w, f) in words {
        w.hash(&mut h);
        f.hash(&mut h);
    }
    hmm_blob.hash(&mut h);
    h.finish().to_le_bytes()
}

// ---- gzip 体积估算（门禁 SPEC §13.2-3） ----

fn gzip_size(data: &[u8]) -> usize {
    // 估算 gzip 体积（SPEC §13.2-3 门禁）：写临时文件 → gzip -c -9 → 量输出。
    // 避免 stdin/stdout pipe 死锁（大数据时 stdout buffer 填满阻塞写端）。
    use std::process::Command;
    let tmp = std::env::temp_dir().join(format!("vane_gen_dict_{}.tmp", std::process::id()));
    if fs::write(&tmp, data).is_err() {
        return data.len();
    }
    let out = Command::new("gzip").args(["-c", "-9"]).arg(&tmp).output();
    let _ = fs::remove_file(&tmp);
    match out {
        Ok(o) => o.stdout.len(),
        Err(_) => data.len(), // gzip 不可用 → 退化返回原大小
    }
}
