//! SPEC §9 inspect API（M4 阶段五 b）：DB 级统计与段级信息。
//!
//! 纯新增 pub API，不改 M0-M3 冻结签名。`Db::stats()` / `Db::segment_info()`
//! 返回强类型结构体（FFI 层序列化为 JSON）。
//!
//! 健康检查（§9.2 inspect API）：
//! - 词典降级：jieba feature on 时，collection 的 tokenizer 是 Jieba 但
//!   `DbInner.jieba_dict` 为 None → Degraded。
//! - 段损坏：`SegmentReader::open` 失败 → Corrupt（header magic/version 校验、
//!   vectors/stored/idmap 解码）。
//! - hnsw 缺失 fallback brute：`CollectionInner.hnsw_readers` 中 None → Degraded。
//!
//! `index_bytes` / `file_sizes`：Vfs trait 无 `size()` 方法（M0 冻结签名），
//! 用 `read_at` 探测 EOF（offset 0 起循环读 8KB buffer 至 n=0，累计推算 size）。
//! inspect 非热路径，性能可接受（§9.2 取舍）。

use crate::api::collection::CollectionInner;
use crate::api::types::DictState;
use crate::segment::SegmentReader;
use crate::tokenizer::BuiltinTokenizer;
use crate::types::TokenizerId;
use crate::vfs::Vfs;
use std::sync::Arc;

// ---- 公共结构体（FFI 层序列化为 JSON；Debug 便于测试）----

/// DB 级统计信息（SPEC §9 inspect API）。
#[derive(Debug, Clone)]
pub struct DbStats {
    pub db_path: String,
    pub collections: Vec<CollectionStats>,
    /// 词典状态（jieba 是否加载）。jieba feature on 时有意义；off 时恒 false。
    pub dict_available: bool,
    pub executor_kind: ExecutorKind,
}

/// 单个 collection 的统计信息。
#[derive(Debug, Clone)]
pub struct CollectionStats {
    pub name: String,
    pub segment_count: usize,
    /// 各段 doc_count 之和（含 tombstoned）。
    pub total_docs: u64,
    /// total_docs - tombstoned_docs。
    pub live_docs: u64,
    pub tombstoned_docs: u64,
    /// 各段文件大小之和（header+vectors+stored+idmap+scalars+inverted+hnsw）。
    pub index_bytes: u64,
    pub dict_state: DictState,
    pub tokenizer_id: TokenizerId,
    pub health: Health,
}

/// 单个段的详细信息。
#[derive(Debug, Clone)]
pub struct SegmentInfo {
    pub ulid: String,
    pub doc_count: u32,
    pub docid_base: u64,
    pub tombstoned_count: u64,
    pub format_versions: FormatVersions,
    pub file_sizes: SegmentFileSizes,
    pub health: Health,
}

/// 段各文件的格式版本（读各文件头 magic + version 字段）。
#[derive(Debug, Clone)]
pub struct FormatVersions {
    pub header: u32,
    pub vectors: u32,
    pub stored: u32,
    pub idmap: u32,
    pub scalars: u32,
    pub inverted: u32,
    pub hnsw: u32,
}

/// 段各文件大小（read_at 探测 EOF 累计；文件缺失 → 0）。
#[derive(Debug, Clone)]
pub struct SegmentFileSizes {
    pub header: u64,
    pub vectors: u64,
    pub stored: u64,
    pub idmap: u64,
    pub scalars: u64,
    pub inverted: u64,
    /// None = 无 hnsw.bin（fallback brute）。
    pub hnsw: Option<u64>,
}

/// 健康状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Healthy,
    /// 词典降级 / hnsw 缺失 fallback brute / 段文件部分缺失但可读。
    Degraded,
    /// 段文件损坏（magic/version/CRC 校验失败）。
    Corrupt,
}

/// 执行器类型（platform cfg 推断）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutorKind {
    Serial,
    Rayon,
}

// ---- pub(crate) 构建函数（db.rs 调用）----

/// 构造单个 collection 的统计信息。
pub(crate) fn build_collection_stats(
    name: &str,
    col_inner: &CollectionInner,
    vfs: &Arc<dyn Vfs>,
    dict_available: bool,
) -> CollectionStats {
    let snap = col_inner.snapshot.read().unwrap();
    let hnsw_readers = col_inner.hnsw_readers.read().unwrap();
    let tombstones = col_inner.tombstones.read().unwrap();
    let dict_state = *col_inner.dict_state.read().unwrap();
    let tokenizer_id = col_inner.tokenizer_id.read().unwrap().clone();

    let segment_count = snap.len();
    let mut total_docs: u64 = 0;
    let mut tombstoned_docs: u64 = 0;
    let mut index_bytes: u64 = 0;
    let mut worst_health = Health::Healthy;

    for (i, reader) in snap.iter().enumerate() {
        let meta = reader.meta();
        total_docs += meta.doc_count as u64;
        let t = tombstones.get(&meta.ulid).map(|b| b.len()).unwrap_or(0);
        tombstoned_docs += t;

        let seg_dir = reader.segment_dir();
        let sizes = probe_segment_file_sizes(vfs.as_ref(), seg_dir);
        index_bytes += sizes.total();

        // 段级健康检查
        let seg_health = segment_health(vfs, seg_dir, hnsw_readers.get(i));
        worst_health = worst(worst_health, seg_health);
    }

    // 词典降级：collection 的 tokenizer 是 Jieba 但 Db 级 dict 不可用 → Degraded
    #[cfg(feature = "jieba")]
    {
        if matches!(col_inner.tokenizer_kind, BuiltinTokenizer::Jieba) && !dict_available {
            worst_health = worst(worst_health, Health::Degraded);
        }
    }
    // jieba feature off 时：Jieba collection 不可能创建成功（build_tokenizer → DictUnavailable），
    // 但保留检查以防未来运行时注入路径。dict_available 此时恒 false，条件等价。
    #[cfg(not(feature = "jieba"))]
    {
        if matches!(col_inner.tokenizer_kind, BuiltinTokenizer::Jieba) && !dict_available {
            worst_health = worst(worst_health, Health::Degraded);
        }
    }

    CollectionStats {
        name: name.to_string(),
        segment_count,
        total_docs,
        live_docs: total_docs - tombstoned_docs,
        tombstoned_docs,
        index_bytes,
        dict_state,
        tokenizer_id,
        health: worst_health,
    }
}

/// 构造单个 collection 的所有段信息。
pub(crate) fn build_segment_info(
    col_inner: &CollectionInner,
    vfs: &Arc<dyn Vfs>,
) -> Vec<SegmentInfo> {
    let snap = col_inner.snapshot.read().unwrap();
    let hnsw_readers = col_inner.hnsw_readers.read().unwrap();
    let tombstones = col_inner.tombstones.read().unwrap();

    let mut result = Vec::with_capacity(snap.len());
    for (i, reader) in snap.iter().enumerate() {
        let meta = reader.meta();
        let ulid = meta.ulid.clone();
        let doc_count = meta.doc_count;
        let docid_base = meta.docid_base;
        let tombstoned_count = tombstones.get(&ulid).map(|b| b.len()).unwrap_or(0);

        let seg_dir = reader.segment_dir();
        let file_sizes = probe_segment_file_sizes(vfs.as_ref(), seg_dir);
        let format_versions = read_format_versions(vfs.as_ref(), seg_dir);
        let health = segment_health(vfs, seg_dir, hnsw_readers.get(i));

        result.push(SegmentInfo {
            ulid,
            doc_count,
            docid_base,
            tombstoned_count,
            format_versions,
            file_sizes,
            health,
        });
    }
    result
}

/// 推断当前执行器类型（platform cfg）。
pub(crate) fn executor_kind() -> ExecutorKind {
    if cfg!(all(
        not(target_arch = "wasm32"),
        feature = "executor-native"
    )) {
        ExecutorKind::Rayon
    } else {
        ExecutorKind::Serial
    }
}

// ---- 内部辅助函数 ----

/// 段级健康检查。
///
/// - `SegmentReader::open` 失败 → Corrupt
/// - open 成功但 hnsw 缺失（None） → Degraded
/// - 否则 Healthy
///
/// 注：§9.2 inspect API 要求"SegmentReader::open 失败 → Corrupt"（健康检查语义）。
/// inspect 非热路径，重新 open 可接受，且能真实检测段损坏（文件被外部篡改后）。
fn segment_health(
    vfs: &Arc<dyn Vfs>,
    seg_dir: &str,
    hnsw: Option<&Option<std::sync::Arc<crate::hnsw::HnswReader>>>,
) -> Health {
    match SegmentReader::open(vfs, seg_dir) {
        Ok(_) => {
            // hnsw 缺失（M0 段无 hnsw.bin）→ Degraded（fallback brute）
            let hnsw_missing = match hnsw {
                Some(Some(_)) => false,
                Some(None) => true,
                None => true, // 索引越界 → 视为缺失
            };
            if hnsw_missing {
                Health::Degraded
            } else {
                Health::Healthy
            }
        }
        Err(_) => Health::Corrupt,
    }
}

/// 探测段目录下各文件大小（read_at 循环读至 EOF 累计）。
fn probe_segment_file_sizes(vfs: &dyn Vfs, seg_dir: &str) -> SegmentFileSizes {
    let header = probe_file_size(vfs, &format!("{seg_dir}/header.bin"));
    let vectors = probe_file_size(vfs, &format!("{seg_dir}/vectors.bin"));
    let stored = probe_file_size(vfs, &format!("{seg_dir}/stored.bin"));
    let idmap = probe_file_size(vfs, &format!("{seg_dir}/idmap.bin"));
    let scalars = probe_file_size(vfs, &format!("{seg_dir}/scalars.col"));
    let inverted = probe_file_size(vfs, &format!("{seg_dir}/inverted.bin"));
    let hnsw = {
        let sz = probe_file_size(vfs, &format!("{seg_dir}/hnsw.bin"));
        if sz == 0 {
            // 文件不存在（read_at 返 Err → 0）或空文件；均视为无 hnsw.bin
            None
        } else {
            Some(sz)
        }
    };
    SegmentFileSizes {
        header,
        vectors,
        stored,
        idmap,
        scalars,
        inverted,
        hnsw,
    }
}

impl SegmentFileSizes {
    fn total(&self) -> u64 {
        self.header
            + self.vectors
            + self.stored
            + self.idmap
            + self.scalars
            + self.inverted
            + self.hnsw.unwrap_or(0)
    }
}

/// read_at 探测文件大小：从 offset=0 循环读 8KB buffer，n=0 即 EOF，累计推算 size。
/// 文件不存在（read_at 返 Err）→ 返回 0。
fn probe_file_size(vfs: &dyn Vfs, path: &str) -> u64 {
    let mut total = 0u64;
    let mut offset = 0u64;
    let mut tmp = [0u8; 8192];
    loop {
        let n = match vfs.read_at(path, &mut tmp, offset) {
            Ok(n) => n,
            Err(_) => return total, // 文件不存在 → 已读 total（0）
        };
        if n == 0 {
            break;
        }
        total += n as u64;
        offset += n as u64;
    }
    total
}

/// 读段各文件头的 format_version（magic(4) + version(4 LE)）。
/// 文件不存在或 magic 不匹配 → 返回 0。
fn read_format_versions(vfs: &dyn Vfs, seg_dir: &str) -> FormatVersions {
    FormatVersions {
        header: read_version_field(vfs, &format!("{seg_dir}/header.bin")),
        vectors: read_version_field(vfs, &format!("{seg_dir}/vectors.bin")),
        stored: read_version_field(vfs, &format!("{seg_dir}/stored.bin")),
        idmap: read_version_field(vfs, &format!("{seg_dir}/idmap.bin")),
        scalars: read_version_field(vfs, &format!("{seg_dir}/scalars.col")),
        inverted: read_version_field(vfs, &format!("{seg_dir}/inverted.bin")),
        hnsw: read_version_field(vfs, &format!("{seg_dir}/hnsw.bin")),
    }
}

/// 读文件头 8 字节（magic(4) + version(4 LE)），校验 magic 后返回 version。
/// 文件不存在 / 过短 / magic 不匹配 → 返回 0。
fn read_version_field(vfs: &dyn Vfs, path: &str) -> u32 {
    let mut buf = [0u8; 8];
    let n = match vfs.read_at(path, &mut buf, 0) {
        Ok(n) => n,
        Err(_) => return 0,
    };
    if n < 8 || &buf[0..4] != crate::types::MAGIC {
        return 0;
    }
    u32::from_le_bytes(buf[4..8].try_into().unwrap())
}

/// 取两个 Health 中更严重的（Corrupt > Degraded > Healthy）。
fn worst(a: Health, b: Health) -> Health {
    match (a, b) {
        (Health::Corrupt, _) | (_, Health::Corrupt) => Health::Corrupt,
        (Health::Degraded, _) | (_, Health::Degraded) => Health::Degraded,
        _ => Health::Healthy,
    }
}

// ---- 单测 ----

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::db::Db;
    use crate::api::types::*;
    use crate::persistence::AutoCommitConfig;
    use crate::types::{FieldDef, Metric, Schema};
    use crate::vfs::memory::MemoryVfs;
    use std::sync::Arc;

    /// 构造测试用 Db + collection + add + flush：
    /// 1 collection "docs"，schema = [Text body, Vector v dim=2]，2 docs，1 flush → 1 segment。
    fn setup_db_with_one_segment() -> Db {
        let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn crate::vfs::Vfs>;
        let db = Db::open(vfs, "testdb", OpenOptions::default()).unwrap();
        let schema = Schema::new(vec![
            ("body".into(), FieldDef::Text),
            (
                "v".into(),
                FieldDef::Vector {
                    dim: 2,
                    metric: Metric::Cosine,
                },
            ),
        ])
        .unwrap();
        let col = db
            .collection("docs", schema, CollectionOptions::default())
            .unwrap();
        col.add(&[
            Doc {
                id: "a".into(),
                text: Some("hello world".into()),
                vector: Some(vec![1.0, 0.0]),
                meta: None,
            },
            Doc {
                id: "b".into(),
                text: Some("foo bar".into()),
                vector: Some(vec![0.0, 1.0]),
                meta: None,
            },
        ])
        .unwrap();
        col.flush().unwrap();
        db
    }

    #[test]
    fn stats_returns_db_path_and_executor() {
        let db = setup_db_with_one_segment();
        let stats = db.stats();
        assert_eq!(stats.db_path, "testdb");
        // executor_kind 取决于 cfg，检查变体匹配
        assert!(matches!(
            stats.executor_kind,
            ExecutorKind::Serial | ExecutorKind::Rayon
        ));
    }

    #[test]
    fn stats_returns_one_collection_with_correct_counts() {
        let db = setup_db_with_one_segment();
        let stats = db.stats();
        assert_eq!(stats.collections.len(), 1);
        let col = &stats.collections[0];
        assert_eq!(col.name, "docs");
        assert_eq!(col.segment_count, 1, "1 flush → 1 segment");
        assert_eq!(col.total_docs, 2, "2 docs added");
        assert_eq!(col.live_docs, 2, "no tombstones");
        assert_eq!(col.tombstoned_docs, 0);
        assert!(matches!(col.dict_state, DictState::Stable));
    }

    #[test]
    fn stats_index_bytes_nonzero() {
        let db = setup_db_with_one_segment();
        let stats = db.stats();
        let col = &stats.collections[0];
        // 段文件已落盘 → index_bytes > 0（header+vectors+stored+idmap+scalars+inverted）
        assert!(
            col.index_bytes > 0,
            "index_bytes should be > 0 after flush, got {}",
            col.index_bytes
        );
    }

    #[test]
    fn stats_health_healthy_for_fresh_segment() {
        let db = setup_db_with_one_segment();
        let stats = db.stats();
        let col = &stats.collections[0];
        // 新段、无 hnsw → Degraded（fallback brute）；若 hnsw 写了 → Healthy
        // M0 段在 flush 时写 hnsw.bin（01-hnsw write_hnsw），故 hnsw_readers[0] = Some
        // → Health = Healthy
        assert_eq!(col.health, Health::Healthy);
    }

    #[test]
    fn segment_info_returns_one_segment_with_correct_fields() {
        let db = setup_db_with_one_segment();
        let infos = db.segment_info();
        assert_eq!(infos.len(), 1, "1 segment after 1 flush");
        let info = &infos[0];
        assert!(!info.ulid.is_empty(), "ULID non-empty");
        assert_eq!(info.doc_count, 2, "2 docs");
        assert_eq!(info.docid_base, 0, "first segment base = 0");
        assert_eq!(info.tombstoned_count, 0, "no tombstones");
    }

    #[test]
    fn segment_info_format_versions_correct() {
        let db = setup_db_with_one_segment();
        let infos = db.segment_info();
        let info = &infos[0];
        // 各文件 format_version 应匹配写入时常量
        assert_eq!(info.format_versions.header, crate::types::HEADER_FORMAT_V1);
        assert_eq!(
            info.format_versions.vectors,
            crate::types::VECTORS_FORMAT_V2
        );
        assert!(
            info.format_versions.stored == crate::types::STORED_FORMAT_V1
                || info.format_versions.stored == crate::types::STORED_FORMAT_V2
        );
        assert_eq!(info.format_versions.idmap, crate::types::IDMAP_FORMAT_V1);
        assert_eq!(
            info.format_versions.scalars,
            crate::types::SCALARS_FORMAT_V1
        );
        assert_eq!(info.format_versions.inverted, crate::types::FORMAT_VERSION);
        assert_eq!(info.format_versions.hnsw, crate::types::HNSW_FORMAT_V1);
    }

    #[test]
    fn segment_info_file_sizes_nonzero() {
        let db = setup_db_with_one_segment();
        let infos = db.segment_info();
        let info = &infos[0];
        assert!(info.file_sizes.header > 0, "header.bin non-empty");
        assert!(info.file_sizes.vectors > 0, "vectors.bin non-empty");
        assert!(info.file_sizes.stored > 0, "stored.bin non-empty");
        assert!(info.file_sizes.idmap > 0, "idmap.bin non-empty");
        assert!(info.file_sizes.inverted > 0, "inverted.bin non-empty");
        // hnsw 可能存在（flush 写 hnsw.bin）
        assert!(
            info.file_sizes.hnsw.is_some(),
            "hnsw.bin present after flush"
        );
    }

    #[test]
    fn segment_info_health_healthy() {
        let db = setup_db_with_one_segment();
        let infos = db.segment_info();
        let info = &infos[0];
        assert_eq!(info.health, Health::Healthy);
    }

    #[test]
    fn collection_segment_info_returns_some_for_existing() {
        let db = setup_db_with_one_segment();
        let infos = db.collection_segment_info("docs");
        assert!(infos.is_some(), "collection 'docs' exists");
        assert_eq!(infos.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn collection_segment_info_returns_none_for_missing() {
        let db = setup_db_with_one_segment();
        let infos = db.collection_segment_info("nonexistent");
        assert!(infos.is_none(), "collection 'nonexistent' does not exist");
    }

    #[test]
    fn stats_multiple_collections() {
        let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn crate::vfs::Vfs>;
        let db = Db::open(vfs, "multidb", OpenOptions::default()).unwrap();
        let schema1 = Schema::new(vec![
            ("body".into(), FieldDef::Text),
            (
                "v".into(),
                FieldDef::Vector {
                    dim: 2,
                    metric: Metric::Cosine,
                },
            ),
        ])
        .unwrap();
        let schema2 = Schema::new(vec![(
            "v".into(),
            FieldDef::Vector {
                dim: 4,
                metric: Metric::Cosine,
            },
        )])
        .unwrap();
        let _c1 = db
            .collection("col1", schema1, CollectionOptions::default())
            .unwrap();
        let _c2 = db
            .collection("col2", schema2, CollectionOptions::default())
            .unwrap();
        let stats = db.stats();
        assert_eq!(stats.collections.len(), 2);
        // 排序后 col1 < col2
        assert_eq!(stats.collections[0].name, "col1");
        assert_eq!(stats.collections[1].name, "col2");
        // 无 flush → 0 segments
        assert_eq!(stats.collections[0].segment_count, 0);
        assert_eq!(stats.collections[1].segment_count, 0);
    }

    #[test]
    fn stats_empty_db() {
        let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn crate::vfs::Vfs>;
        let db = Db::open(vfs, "emptydb", OpenOptions::default()).unwrap();
        let stats = db.stats();
        assert_eq!(stats.db_path, "emptydb");
        assert!(stats.collections.is_empty());
        // dict_available 取决于 jieba feature；检查与 jieba_dict_available() 一致
        #[cfg(feature = "jieba")]
        {
            assert_eq!(stats.dict_available, db.jieba_dict_available());
        }
        #[cfg(not(feature = "jieba"))]
        {
            assert!(!stats.dict_available);
        }
    }

    #[test]
    fn segment_info_empty_collection() {
        let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn crate::vfs::Vfs>;
        let db = Db::open(vfs, "emptyseg", OpenOptions::default()).unwrap();
        let schema = Schema::new(vec![(
            "v".into(),
            FieldDef::Vector {
                dim: 2,
                metric: Metric::Cosine,
            },
        )])
        .unwrap();
        let _col = db
            .collection("docs", schema, CollectionOptions::default())
            .unwrap();
        // 未 flush → 无段
        let infos = db.segment_info();
        assert!(infos.is_empty());
    }

    #[test]
    fn probe_file_size_empty_file_returns_zero() {
        let vfs = MemoryVfs::new();
        // 不存在的文件 → 0
        assert_eq!(probe_file_size(&vfs, "nonexistent.bin"), 0);
    }

    #[test]
    fn probe_file_size_correct_for_known_content() {
        let vfs = MemoryVfs::new();
        // 写 100 字节文件
        vfs.create("test.bin").unwrap();
        let data = vec![42u8; 100];
        vfs.write_at("test.bin", &data, 0).unwrap();
        assert_eq!(probe_file_size(&vfs, "test.bin"), 100);
    }

    #[test]
    fn read_version_field_missing_file_returns_zero() {
        let vfs = MemoryVfs::new();
        assert_eq!(read_version_field(&vfs, "nonexistent.bin"), 0);
    }

    #[test]
    fn read_version_field_correct_for_vane_magic() {
        let vfs = MemoryVfs::new();
        vfs.create("v.bin").unwrap();
        let mut buf = Vec::new();
        buf.extend_from_slice(crate::types::MAGIC);
        buf.extend_from_slice(&crate::types::VECTORS_FORMAT_V2.to_le_bytes());
        vfs.write_at("v.bin", &buf, 0).unwrap();
        assert_eq!(
            read_version_field(&vfs, "v.bin"),
            crate::types::VECTORS_FORMAT_V2
        );
    }

    #[test]
    fn worst_function_ordering() {
        assert_eq!(worst(Health::Healthy, Health::Healthy), Health::Healthy);
        assert_eq!(worst(Health::Healthy, Health::Degraded), Health::Degraded);
        assert_eq!(worst(Health::Degraded, Health::Healthy), Health::Degraded);
        assert_eq!(worst(Health::Healthy, Health::Corrupt), Health::Corrupt);
        assert_eq!(worst(Health::Corrupt, Health::Degraded), Health::Corrupt);
        assert_eq!(worst(Health::Degraded, Health::Degraded), Health::Degraded);
    }

    #[test]
    fn executor_kind_consistent_with_cfg() {
        let kind = executor_kind();
        if cfg!(all(
            not(target_arch = "wasm32"),
            feature = "executor-native"
        )) {
            assert_eq!(kind, ExecutorKind::Rayon);
        } else {
            assert_eq!(kind, ExecutorKind::Serial);
        }
    }

    // ---- AutoCommitConfig Off 测试：用 disable auto-commit 避免 flush 干扰 ----

    #[test]
    fn stats_after_tombstone_delete() {
        let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn crate::vfs::Vfs>;
        let opts = OpenOptions {
            persistence: PersistenceMode::Persistent,
            auto_commit: AutoCommitConfig::Off,
            page_cache_mb: 32,
        };
        let db = Db::open(vfs, "tombdb", opts).unwrap();
        let schema = Schema::new(vec![
            ("body".into(), FieldDef::Text),
            (
                "v".into(),
                FieldDef::Vector {
                    dim: 2,
                    metric: Metric::Cosine,
                },
            ),
        ])
        .unwrap();
        let col = db
            .collection(
                "docs",
                schema,
                CollectionOptions {
                    tokenizer: BuiltinTokenizer::Standard,
                    user_dict: vec![],
                    auto_commit: AutoCommitConfig::Off,
                },
            )
            .unwrap();
        col.add(&[
            Doc {
                id: "a".into(),
                text: Some("hello world".into()),
                vector: Some(vec![1.0, 0.0]),
                meta: None,
            },
            Doc {
                id: "b".into(),
                text: Some("foo bar".into()),
                vector: Some(vec![0.0, 1.0]),
                meta: None,
            },
        ])
        .unwrap();
        col.flush().unwrap();
        // 删除文档 a
        col.delete(&["a".to_string()]).unwrap();
        col.flush().unwrap();

        let stats = db.stats();
        let col_stats = &stats.collections[0];
        assert_eq!(col_stats.total_docs, 2, "total_docs includes tombstoned");
        assert_eq!(col_stats.tombstoned_docs, 1, "1 deleted");
        assert_eq!(col_stats.live_docs, 1, "2 - 1 = 1 live");

        let infos = db.segment_info();
        // tombstoned_count 可能分布在段上
        let total_tombstoned: u64 = infos.iter().map(|i| i.tombstoned_count).sum();
        assert_eq!(total_tombstoned, 1, "1 tombstoned across segments");
    }
}
