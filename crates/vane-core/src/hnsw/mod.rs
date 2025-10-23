//! SPEC §3.1/§8.1 段内不可变 HNSW 图。
//!
//! 写期 `HnswWriter` 构建 → `write_hnsw` 落盘；读期 `HnswReader` 加载 → `search`。
//!
//! - M1 全串行搜索（R-4/R-6）：零 `cfg(target)`，无 `thread::scope`/rayon。
//! - 图不原地删（I-3）：`HnswReader` 只读，删除走 tombstone（02 计划）。
//! - 距离：复用 M0 `Metric` 语义（越大越相似）。内部转距离用于图导航
//!   （cosine=1-cos；L2=|a-b|²；dot=-dot），单调等价于 -score。
//!
//! hnsw.bin 格式（README 契约，graph-only——不嵌入向量，向量单一副本存 vectors.bin）：
//! ```text
//! magic(4) | format_version(4 LE) | dim(4 LE) | metric(1) |
//! m(4 LE) | ef_construction(4 LE) | entry_point(4 LE) | max_level(4 LE) |
//! num_nodes(4 LE) |
//! { local_docid(4 LE) | level(1) |
//!     for layer in 0..=level: num_neighbors(4 LE) | neighbors(num_neighbors*4 LE)
//! }
//! ```
//! 向量不进 hnsw.bin（R-hnsw-vec 修复：避免 vectors.bin + hnsw.bin 双存违反
//! SPEC §6.2 + §3.3「50 万不塌红线」）。`HnswReader::search` 由 api 层传入
//! `vectors: &[f32]`（SegmentReader 已加载的 vectors.bin 单一副本）导航。

use crate::types::{Metric, Result, ScoredDoc, VaneError, FORMAT_VERSION, MAGIC};
use crate::vfs::Vfs;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::sync::Arc;

#[cfg(test)]
mod tests;

/// 带距离的候选节点（导航用）。距离越小越近。
#[derive(Debug, Clone, Copy, PartialEq)]
struct DistNode {
    dist: f32,
    node: u32,
}

impl Eq for DistNode {}
impl Ord for DistNode {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // NaN 视为最大距离（最远）
        match (self.dist.is_nan(), other.dist.is_nan()) {
            (true, true) => std::cmp::Ordering::Equal,
            (true, false) => std::cmp::Ordering::Greater,
            (false, true) => std::cmp::Ordering::Less,
            (false, false) => self
                .dist
                .total_cmp(&other.dist)
                .then_with(|| self.node.cmp(&other.node)),
        }
    }
}
impl PartialOrd for DistNode {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// 图节点。`neighbors[lc]` 为第 lc 层的邻居节点索引。
/// 不持有向量（R-hnsw-vec：hnsw.bin graph-only，向量由 search 调用方传入）。
pub struct Node {
    pub local_docid: u32,
    pub level: u32,
    pub neighbors: Vec<Vec<u32>>,
}

/// 段内不可变 HNSW 图（SPEC §3.1/§8.1）。
pub struct HnswGraph {
    pub dim: u32,
    pub metric: Metric,
    pub m: u32,
    pub ef_construction: u32,
    pub entry_point: Option<u32>,
    pub max_level: u32,
    pub nodes: Vec<Node>,
}

impl HnswGraph {
    pub fn doc_count(&self) -> u32 {
        self.nodes.len() as u32
    }

    pub fn entry_point(&self) -> Option<u32> {
        self.entry_point
    }

    /// 第 0 层邻居（用于断言图结构正确性）。
    pub fn neighbors(&self, node_idx: u32) -> Vec<u32> {
        self.nodes
            .get(node_idx as usize)
            .and_then(|n| n.neighbors.first())
            .cloned()
            .unwrap_or_default()
    }
}

/// 距离函数（导航用，越小越近）。cosine=1-cos；L2=|a-b|²；dot=-dot。
fn metric_distance(metric: Metric, a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len(), "metric_distance: dim mismatch");
    match metric {
        Metric::Cosine => {
            let (dot, na, nb) = dot_and_norms(a, b);
            let denom = na.sqrt() * nb.sqrt();
            if denom == 0.0 || !denom.is_finite() {
                // 零向量无方向信息，视为最远（cosine 距离上界 2.0）。
                return 2.0;
            }
            1.0 - dot / denom
        }
        Metric::L2 => {
            let mut s = 0.0_f32;
            for i in 0..a.len() {
                let d = a[i] - b[i];
                s += d * d;
            }
            s
        }
        Metric::Dot => {
            let mut s = 0.0_f32;
            for i in 0..a.len() {
                s += a[i] * b[i];
            }
            -s
        }
    }
}

/// score 语义（越大越相似，与 brute_search 一致）。
fn metric_score(metric: Metric, a: &[f32], b: &[f32]) -> f32 {
    match metric {
        Metric::Cosine => {
            let (dot, na, nb) = dot_and_norms(a, b);
            let denom = na.sqrt() * nb.sqrt();
            if denom == 0.0 || !denom.is_finite() {
                0.0
            } else {
                dot / denom
            }
        }
        Metric::L2 => {
            let mut s = 0.0_f32;
            for i in 0..a.len() {
                let d = a[i] - b[i];
                s += d * d;
            }
            -s.sqrt()
        }
        Metric::Dot => {
            let mut s = 0.0_f32;
            for i in 0..a.len() {
                s += a[i] * b[i];
            }
            s
        }
    }
}

#[inline]
fn dot_and_norms(a: &[f32], b: &[f32]) -> (f32, f32, f32) {
    let mut dot = 0.0_f32;
    let mut na = 0.0_f32;
    let mut nb = 0.0_f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    (dot, na, nb)
}

/// 简易确定性伪随机（xorshift64），固定种子保证图结构可复现。
/// 不引入 rand crate（依赖黑名单约束 + 无新依赖）。
struct Rng {
    state: u64,
}

impl Rng {
    fn new(seed: u64) -> Self {
        // 避免全零状态
        Self {
            state: if seed == 0 { 0x9e3779b97f4a7c15 } else { seed },
        }
    }
    /// 返回 (0,1] 的 f64。
    fn next_unit(&mut self) -> f64 {
        // xorshift64
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        // 取高 53 位构造 (0,1]
        let u = (x >> 11) as f64 / ((1u64 << 53) as f64);
        // 避免 0（ln(0) 无定义）
        if u <= 0.0 {
            1.0e-12
        } else {
            u
        }
    }
}

pub struct HnswWriter {
    dim: u32,
    metric: Metric,
    m: u32,
    ef_construction: u32,
    nodes: Vec<Node>,
    /// 写期构建用向量（按 node_idx 索引；不落盘 hnsw.bin）。
    vectors: Vec<Vec<f32>>,
    entry_point: Option<u32>,
    max_level: u32,
    rng: Rng,
}

impl HnswWriter {
    /// M=16/ef_construction=200（SPEC §3.1 默认；可配）。
    pub fn new(dim: u32, metric: Metric, m: u32, ef_construction: u32) -> Self {
        Self {
            dim,
            metric,
            m,
            ef_construction,
            nodes: Vec::new(),
            vectors: Vec::new(),
            entry_point: None,
            max_level: 0,
            rng: Rng::new(0x9e3779b97f4a7c15),
        }
    }

    /// 插入一个向量；local_docid 为段内局部 docid（0 起，与 vectors.bin 索引一致）。
    pub fn insert(&mut self, local_docid: u32, vector: &[f32]) {
        debug_assert_eq!(
            vector.len(),
            self.dim as usize,
            "hnsw insert: vector dim mismatch"
        );
        let m = self.m.max(2) as usize;
        let ml = 1.0 / (m as f64).ln();
        let level = (-self.rng.next_unit().ln() * ml).floor() as u32;

        let new_idx = self.nodes.len() as u32;
        let new_node = Node {
            local_docid,
            level,
            neighbors: (0..=level).map(|_| Vec::new()).collect(),
        };

        match self.entry_point {
            None => {
                // 首个节点：直接设为 entry_point
                self.entry_point = Some(new_idx);
                self.max_level = level;
                self.nodes.push(new_node);
                self.vectors.push(vector.to_vec());
                return;
            }
            Some(ep) => {
                // 先入图（neighbors 暂空），保证后续修剪访问 self.nodes[new_idx] 合法。
                self.nodes.push(new_node);
                self.vectors.push(vector.to_vec());
                let mut cur_ep = ep;
                let query = vector;
                // 1) 从最高层贪婪下降到 level+1，每层 ef=1 找最近
                for lc in ((level + 1)..=self.max_level).rev() {
                    let w = self.search_layer(query, &[cur_ep], 1, lc, None, 0);
                    cur_ep = w
                        .into_iter()
                        .min_by(|a, b| a.dist.total_cmp(&b.dist))
                        .map(|dn| dn.node)
                        .unwrap_or(cur_ep);
                }
                // 2) 从 min(level, max_level) 下降到 0，ef_construction 搜索 + 连接
                let top = level.min(self.max_level);
                for lc in (0..=top).rev() {
                    let w = self.search_layer(
                        query,
                        &[cur_ep],
                        self.ef_construction as usize,
                        lc,
                        None,
                        0,
                    );
                    let neighbors = self.select_neighbors(&w, m);
                    // 设置新节点该层邻居
                    self.nodes[new_idx as usize].neighbors[lc as usize] = neighbors.clone();
                    // 双向连接 + 修剪邻居
                    for &nb in &neighbors {
                        let nb_vec = self.vectors[nb as usize].clone();
                        let cur_layer: Vec<u32> = self.nodes[nb as usize]
                            .neighbors
                            .get(lc as usize)
                            .expect("layer exists within nb_node.level")
                            .clone();
                        let mut updated = cur_layer;
                        if !updated.contains(&new_idx) {
                            updated.push(new_idx);
                        }
                        // 修剪 nb 的该层连接到 M_max（lc==0 时 2M，否则 M）
                        let max_conn = if lc == 0 { 2 * m } else { m };
                        if updated.len() > max_conn {
                            let mut ranked: Vec<(f32, u32)> = updated
                                .iter()
                                .map(|&c| {
                                    let cv = self.vectors[c as usize].clone();
                                    (metric_distance(self.metric, &nb_vec, &cv), c)
                                })
                                .collect();
                            ranked.sort_by(|a, b| a.0.total_cmp(&b.0));
                            ranked.truncate(max_conn);
                            updated = ranked.into_iter().map(|(_, c)| c).collect();
                        }
                        self.nodes[nb as usize].neighbors[lc as usize] = updated;
                    }
                    // 下一层 entry point：取 w 中最近
                    cur_ep = w
                        .into_iter()
                        .min_by(|a, b| a.dist.total_cmp(&b.dist))
                        .map(|dn| dn.node)
                        .unwrap_or(cur_ep);
                }
            }
        }

        // 若新节点层级超过 max_level，更新 entry_point
        if level > self.max_level {
            self.max_level = level;
            self.entry_point = Some(new_idx);
        }
    }

    /// 构建完成，消费 self。
    pub fn build(self) -> HnswGraph {
        HnswGraph {
            dim: self.dim,
            metric: self.metric,
            m: self.m,
            ef_construction: self.ef_construction,
            entry_point: self.entry_point,
            max_level: self.max_level,
            nodes: self.nodes,
        }
    }

    /// 单层贪婪搜索（写期与读期共用）。
    /// 返回动态候选列表 W（按距离）。
    ///
    /// - `filter`：Some 时仅 filter 命中的节点入 W（结果堆），但所有邻居均可作导航点。
    ///   filter 存绝对 docid，内部用 `docid_base` 转换：`filter.contains(local + base)`。
    /// - 写期插入时 filter=None、docid_base=0、vectors 由 self.nodes 提供。
    fn search_layer(
        &self,
        query: &[f32],
        entry_points: &[u32],
        ef: usize,
        lc: u32,
        filter: Option<&roaring::RoaringBitmap>,
        docid_base: u64,
    ) -> Vec<DistNode> {
        if ef == 0 || entry_points.is_empty() {
            return Vec::new();
        }
        let mut visited: std::collections::HashSet<u32> =
            std::collections::HashSet::with_capacity(ef * 2);
        // candidates: min-heap（距离最小在堆顶）= Reverse<DistNode> 在 max-heap 中
        let mut candidates: BinaryHeap<Reverse<DistNode>> = BinaryHeap::with_capacity(ef + 1);
        // W: max-heap（堆顶=距离最大，便于弹出最远）
        let mut w: BinaryHeap<DistNode> = BinaryHeap::with_capacity(ef + 1);

        for &ep in entry_points {
            if visited.insert(ep) {
                let n = &self.nodes[ep as usize];
                let dist = metric_distance(self.metric, query, &self.vectors[ep as usize]);
                candidates.push(Reverse(DistNode { dist, node: ep }));
                let passes = passes_filter(filter, n.local_docid, docid_base);
                if passes {
                    w.push(DistNode { dist, node: ep });
                }
            }
        }

        while let Some(Reverse(c)) = candidates.pop() {
            // 若 W 已满且 c 比 W 中最远还远，停止
            if w.len() >= ef {
                if let Some(furthest) = w.peek() {
                    if c.dist > furthest.dist {
                        break;
                    }
                }
            }
            // 遍历 c 的该层邻居
            let c_node = &self.nodes[c.node as usize];
            let layer_neighbors = c_node.neighbors.get(lc as usize);
            if let Some(layer) = layer_neighbors {
                for &e in layer {
                    if visited.insert(e) {
                        let en = &self.nodes[e as usize];
                        let dist = metric_distance(self.metric, query, &self.vectors[e as usize]);
                        let passes = passes_filter(filter, en.local_docid, docid_base);
                        let should_add_candidate = match w.len().cmp(&ef) {
                            std::cmp::Ordering::Less => true,
                            _ => match w.peek() {
                                Some(furthest) => dist < furthest.dist || passes,
                                None => true,
                            },
                        };
                        if should_add_candidate {
                            candidates.push(Reverse(DistNode { dist, node: e }));
                            if passes {
                                w.push(DistNode { dist, node: e });
                                if w.len() > ef {
                                    w.pop();
                                }
                            }
                        }
                    }
                }
            }
        }

        w.into_vec()
    }

    /// 选 M 个最近邻（简单策略：按距离升序取前 M）。
    fn select_neighbors(&self, w: &[DistNode], m: usize) -> Vec<u32> {
        let mut v: Vec<DistNode> = w.to_vec();
        v.sort();
        v.truncate(m);
        v.into_iter().map(|dn| dn.node).collect()
    }
}

/// 检查 local_docid（+docid_base 得绝对 docid）是否通过 filter。
#[inline]
fn passes_filter(
    filter: Option<&roaring::RoaringBitmap>,
    local_docid: u32,
    docid_base: u64,
) -> bool {
    match filter {
        None => true,
        Some(bm) => {
            let abs = local_docid as u64 + docid_base;
            // roaring 存 u32；绝对 docid 超出 u32 范围视为不命中（防御性）
            if abs > u32::MAX as u64 {
                return false;
            }
            bm.contains(abs as u32)
        }
    }
}

/// 写 hnsw.bin 到段目录（SPEC §6.2，graph-only——不写向量）。
pub fn write_hnsw(vfs: &dyn Vfs, segment_dir: &str, graph: &HnswGraph) -> Result<()> {
    let path = format!("{}/hnsw.bin", segment_dir);
    vfs.create(&path)?;
    let mut buf = Vec::with_capacity(64 + graph.nodes.len() * 32);
    buf.extend_from_slice(MAGIC);
    buf.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    buf.extend_from_slice(&graph.dim.to_le_bytes());
    buf.push(metric_to_u8(graph.metric));
    buf.extend_from_slice(&graph.m.to_le_bytes());
    buf.extend_from_slice(&graph.ef_construction.to_le_bytes());
    buf.extend_from_slice(&(graph.entry_point.unwrap_or(u32::MAX)).to_le_bytes());
    buf.extend_from_slice(&graph.max_level.to_le_bytes());
    buf.extend_from_slice(&(graph.nodes.len() as u32).to_le_bytes());
    for n in &graph.nodes {
        buf.extend_from_slice(&n.local_docid.to_le_bytes());
        // level 以 u8 存储（HNSW 层数实际远小于 255）
        if n.level > 255 {
            return Err(VaneError::Corrupt(format!(
                "hnsw node level {} exceeds u8",
                n.level
            )));
        }
        buf.push(n.level as u8);
        for lc in 0..=n.level as usize {
            let layer = &n.neighbors[lc];
            buf.extend_from_slice(&(layer.len() as u32).to_le_bytes());
            for &nb in layer {
                buf.extend_from_slice(&nb.to_le_bytes());
            }
        }
    }
    vfs.write_at(&path, &buf, 0)?;
    vfs.sync(&path)?;
    Ok(())
}

fn metric_to_u8(m: Metric) -> u8 {
    match m {
        Metric::Cosine => 0,
        Metric::L2 => 1,
        Metric::Dot => 2,
    }
}

fn metric_from_u8(b: u8) -> Result<Metric> {
    match b {
        0 => Ok(Metric::Cosine),
        1 => Ok(Metric::L2),
        2 => Ok(Metric::Dot),
        _ => Err(VaneError::Corrupt(format!("unknown metric byte: {}", b))),
    }
}

/// 段读期 HNSW 句柄。从 hnsw.bin 加载，提供 `search`。
pub struct HnswReader {
    dim: u32,
    metric: Metric,
    m: u32,
    ef_construction: u32,
    entry_point: Option<u32>,
    max_level: u32,
    nodes: Vec<Node>,
}

impl HnswReader {
    /// open 缺失 hnsw.bin（M0 corpus 无此文件）时返回 `Err`；
    /// api 层 catch 后 fallback `brute_search`（Q-5）。
    pub fn open(vfs: &Arc<dyn Vfs>, segment_dir: &str) -> Result<Self> {
        let path = format!("{}/hnsw.bin", segment_dir);
        let buf = read_all_vfs(vfs.as_ref(), &path)?;

        if buf.len() < 29 {
            return Err(VaneError::Corrupt("hnsw.bin too short".into()));
        }
        if &buf[0..4] != MAGIC {
            return Err(VaneError::Corrupt("hnsw.bin bad magic".into()));
        }
        let version = u32::from_le_bytes(buf[4..8].try_into().unwrap());
        if version != FORMAT_VERSION {
            return Err(VaneError::Version(format!(
                "hnsw.bin unsupported format_version: {} (expected {})",
                version, FORMAT_VERSION
            )));
        }
        let dim = u32::from_le_bytes(buf[8..12].try_into().unwrap());
        let metric = metric_from_u8(buf[12])?;
        let m = u32::from_le_bytes(buf[13..17].try_into().unwrap());
        let ef_construction = u32::from_le_bytes(buf[17..21].try_into().unwrap());
        let ep_raw = u32::from_le_bytes(buf[21..25].try_into().unwrap());
        let entry_point = if ep_raw == u32::MAX {
            None
        } else {
            Some(ep_raw)
        };
        let max_level = u32::from_le_bytes(buf[25..29].try_into().unwrap());
        let num_nodes = u32::from_le_bytes(buf[29..33].try_into().unwrap()) as usize;

        let mut pos = 33usize;
        let mut nodes = Vec::with_capacity(num_nodes);
        for _ in 0..num_nodes {
            if pos + 5 > buf.len() {
                return Err(VaneError::Corrupt("hnsw.bin node header truncated".into()));
            }
            let local_docid = u32::from_le_bytes(buf[pos..pos + 4].try_into().unwrap());
            pos += 4;
            let level = buf[pos] as u32;
            pos += 1;
            let mut neighbors = Vec::with_capacity(level as usize + 1);
            for _ in 0..=level {
                if pos + 4 > buf.len() {
                    return Err(VaneError::Corrupt(
                        "hnsw.bin neighbor count truncated".into(),
                    ));
                }
                let cnt = u32::from_le_bytes(buf[pos..pos + 4].try_into().unwrap()) as usize;
                pos += 4;
                if pos + cnt * 4 > buf.len() {
                    return Err(VaneError::Corrupt("hnsw.bin neighbors truncated".into()));
                }
                let mut layer = Vec::with_capacity(cnt);
                for _ in 0..cnt {
                    let nb = u32::from_le_bytes(buf[pos..pos + 4].try_into().unwrap());
                    pos += 4;
                    layer.push(nb);
                }
                neighbors.push(layer);
            }
            nodes.push(Node {
                local_docid,
                level,
                neighbors,
            });
        }

        Ok(Self {
            dim,
            metric,
            m,
            ef_construction,
            entry_point,
            max_level,
            nodes,
        })
    }

    pub fn doc_count(&self) -> u32 {
        self.nodes.len() as u32
    }

    pub fn dim(&self) -> u32 {
        self.dim
    }

    /// 建图 ef_construction；api 层据此推 `ef_search = max(ef_construction, topk*4)`。
    pub fn ef_construction(&self) -> u32 {
        self.ef_construction
    }

    pub fn m(&self) -> u32 {
        self.m
    }

    /// 段级搜索：返回 topk 候选（绝对 docid + score）。
    /// filter 存绝对 docid，内部减 docid_base 转 local。
    /// ef_search 控制精度，默认 max(ef_construction, topk*4)。
    /// `vectors` 为段内全部向量连续排布（vectors.bin，由 api 层 SegmentReader.vectors() 传入），
    /// 按 `local_docid * dim` 索引取节点向量算导航/结果距离（R-hnsw-vec：hnsw.bin graph-only）。
    #[allow(clippy::too_many_arguments)]
    pub fn search(
        &self,
        query: &[f32],
        topk: usize,
        ef_search: usize,
        filter: Option<&roaring::RoaringBitmap>,
        docid_base: u64,
        vectors: &[f32],
    ) -> Vec<ScoredDoc> {
        if topk == 0 {
            return Vec::new();
        }
        let ep = match self.entry_point {
            Some(e) => e,
            None => return Vec::new(),
        };
        if self.nodes.is_empty() {
            return Vec::new();
        }
        let ef = ef_search.max(topk);

        // 1) 从最高层贪婪下降到第 1 层，每层 ef=1 找最近（导航，不过滤）
        let mut cur_ep = ep;
        for lc in (1..=self.max_level).rev() {
            let w = self.search_layer(query, &[cur_ep], 1, lc, None, 0, vectors);
            cur_ep = w
                .into_iter()
                .min_by(|a, b| a.dist.total_cmp(&b.dist))
                .map(|dn| dn.node)
                .unwrap_or(cur_ep);
        }
        // 2) 第 0 层 ef 搜索（应用 filter 到结果堆）
        let w = self.search_layer(query, &[cur_ep], ef, 0, filter, docid_base, vectors);

        // 转 ScoredDoc（score 用 metric score 语义），按 score 降序取 topk
        let mut out: Vec<ScoredDoc> = w
            .into_iter()
            .map(|dn| {
                let n = &self.nodes[dn.node as usize];
                let nv = node_vector(vectors, n.local_docid, self.dim);
                let score = metric_score(self.metric, query, nv);
                ScoredDoc {
                    docid: n.local_docid as u64 + docid_base,
                    score,
                }
            })
            .collect();
        // 降序；同分按 docid 升序（与 brute_search 一致，保证确定性）
        out.sort_by(|a, b| {
            // score 降序，同分 docid 升序
            match b.score.total_cmp(&a.score) {
                std::cmp::Ordering::Equal => a.docid.cmp(&b.docid),
                other => other,
            }
        });
        out.truncate(topk);
        out
    }

    /// 单层贪婪搜索（读期）。复用 HnswWriter 的算法但只读 nodes。
    /// `vectors` 按 local_docid 索引（R-hnsw-vec：向量不进 hnsw.bin，由调用方传入）。
    #[allow(clippy::too_many_arguments)]
    fn search_layer(
        &self,
        query: &[f32],
        entry_points: &[u32],
        ef: usize,
        lc: u32,
        filter: Option<&roaring::RoaringBitmap>,
        docid_base: u64,
        vectors: &[f32],
    ) -> Vec<DistNode> {
        if ef == 0 || entry_points.is_empty() {
            return Vec::new();
        }
        let mut visited: std::collections::HashSet<u32> =
            std::collections::HashSet::with_capacity(ef * 2);
        let mut candidates: BinaryHeap<Reverse<DistNode>> = BinaryHeap::with_capacity(ef + 1);
        let mut w: BinaryHeap<DistNode> = BinaryHeap::with_capacity(ef + 1);

        for &ep in entry_points {
            if visited.insert(ep) {
                let n = &self.nodes[ep as usize];
                let dist = metric_distance(
                    self.metric,
                    query,
                    node_vector(vectors, n.local_docid, self.dim),
                );
                candidates.push(Reverse(DistNode { dist, node: ep }));
                if passes_filter(filter, n.local_docid, docid_base) {
                    w.push(DistNode { dist, node: ep });
                }
            }
        }

        while let Some(Reverse(c)) = candidates.pop() {
            if w.len() >= ef {
                if let Some(furthest) = w.peek() {
                    if c.dist > furthest.dist {
                        break;
                    }
                }
            }
            let c_node = &self.nodes[c.node as usize];
            if let Some(layer) = c_node.neighbors.get(lc as usize) {
                for &e in layer {
                    if visited.insert(e) {
                        let en = &self.nodes[e as usize];
                        let dist = metric_distance(
                            self.metric,
                            query,
                            node_vector(vectors, en.local_docid, self.dim),
                        );
                        let passes = passes_filter(filter, en.local_docid, docid_base);
                        let should_add = match w.len().cmp(&ef) {
                            std::cmp::Ordering::Less => true,
                            _ => match w.peek() {
                                Some(furthest) => dist < furthest.dist || passes,
                                None => true,
                            },
                        };
                        if should_add {
                            candidates.push(Reverse(DistNode { dist, node: e }));
                            if passes {
                                w.push(DistNode { dist, node: e });
                                if w.len() > ef {
                                    w.pop();
                                }
                            }
                        }
                    }
                }
            }
        }

        w.into_vec()
    }
}

/// 循环 read_at 直到 EOF，拼出完整文件字节。
fn read_all_vfs(vfs: &dyn Vfs, path: &str) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    let mut off = 0u64;
    loop {
        let n = vfs.read_at(path, &mut tmp, off)?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        off += n as u64;
    }
    Ok(buf)
}

/// 从连续排布的 vectors 切片中按 local_docid 取该节点向量（R-hnsw-vec：向量不进 hnsw.bin）。
#[inline]
fn node_vector(vectors: &[f32], local_docid: u32, dim: u32) -> &[f32] {
    let d = dim as usize;
    let s = local_docid as usize * d;
    &vectors[s..s + d]
}
