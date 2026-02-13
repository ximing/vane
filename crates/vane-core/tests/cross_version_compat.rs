// tests/cross_version_compat.rs — M4 §3.4 跨版本持久化兼容测试
//
// 验证当前版本能读 v0.1.0 tag 真实生成的 fixture（非当前代码模拟）。
// fixture 生成方式见 tests/fixtures/compat/README.md + scripts/gen_compat_fixture.rs。
//
// fixture 段文件 per-file format_version（v0.1.0 产物，当前版本双模读取）：
//   header.bin   V1   vectors.bin  V2   stored.bin   V1   idmap.bin   V1
//   scalars.col  V1   inverted.bin V1   hnsw.bin     V1
//
// 已知文档集（fixture-gen 确定性输入，baseline 断言用）：
//   v010-d0 vec=[1,0,0,0] tag=a "向量检索 混合搜索 hybrid search engine"
//   v010-d1 vec=[0,1,0,0] tag=b "BM25 ranking text retrieval"
//   v010-d2 vec=[0,0,1,0] tag=a "机器学习 与 搜索引擎 ranking"
//   v010-d3 vec=[1,1,0,0] tag=c "cosine similarity vector space"
//   v010-d4 vec=[0,0,0,1] tag=b "全文检索 inverted index 倒排"
//
// 向量搜索 [1,0,0,0] baseline（cosine_score，score 降序，同分 docid 升序）：
//   d0 score=1.0 > d3 score≈0.707 > d1=d2=d4 score=0.0（按 docid 升序）

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use vane_core::api::{
    CollectionOptions, Db, Doc, FusionSpec, OpenOptions, SearchMode, SearchQuery,
};
use vane_core::types::{FieldDef, Metric, ScalarKind, Schema};
use vane_core::vfs::std_fs::StdFsVfs;
use vane_core::vfs::Vfs;

/// fixture 段 ULID（v0.1.0 确定性产物，当前版本不应修改）。
const FIXTURE_SEG_ULID: &str = "01KZRQ9VAJ0000000000000000";

/// 已知文档 external_id 集合（fixture-gen 确定性输入）。
const KNOWN_IDS: [&str; 5] = ["v010-d0", "v010-d1", "v010-d2", "v010-d3", "v010-d4"];

fn fixture_root() -> String {
    format!(
        "{}/tests/fixtures/compat/v0.1.0",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn build_schema() -> Schema {
    Schema::new(vec![
        ("body".into(), FieldDef::Text),
        (
            "v".into(),
            FieldDef::Vector {
                dim: 4,
                metric: Metric::Cosine,
            },
        ),
        (
            "tag".into(),
            FieldDef::Scalar {
                kind: ScalarKind::Keyword,
            },
        ),
    ])
    .unwrap()
}

fn unique_dir(label: &str) -> std::path::PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "vane-cross-version-{}-{}-{}-{}",
        label,
        std::process::id(),
        n,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

/// 递归复制目录（跨平台，不依赖 cp 命令）。
fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if path.is_dir() {
            copy_dir_recursive(&path, &dst_path)?;
        } else {
            std::fs::copy(&path, &dst_path)?;
        }
    }
    Ok(())
}

/// 复制 fixture 到临时目录（保持 fixture 源不被修改），返回临时目录路径。
fn copy_fixture_to_temp(label: &str) -> std::path::PathBuf {
    let dir = unique_dir(label);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let root = fixture_root();
    let src = std::path::Path::new(&root);
    copy_dir_recursive(src, &dir).expect("复制 fixture 失败");
    dir
}

/// 读段文件 format_version（LE u32，offset=4，magic 之后 4 字节）。
fn read_format_version(vfs: &Arc<dyn Vfs>, path: &str) -> u32 {
    let mut buf = [0u8; 8];
    let n = vfs.read_at(path, &mut buf, 0).expect("read_at 失败");
    assert!(n >= 8, "{} 不足 8 字节", path);
    assert_eq!(&buf[0..4], b"VANE", "{} magic 错误", path);
    u32::from_le_bytes(buf[4..8].try_into().unwrap())
}

/// 收集 hit 的 external_id 集合。
fn hit_ids(hits: &[vane_core::api::Hit]) -> std::collections::HashSet<String> {
    hits.iter().map(|h| h.id.clone()).collect()
}

// =============================================================================
// 测试 1：当前版本读 v0.1.0 fixture
// =============================================================================

/// 验证当前版本能读 v0.1.0 tag 真实生成的 fixture：
/// - manifest restore：collection "docs" 可见
/// - 段文件 format_version 与 fixture 一致（非 vacuous）
/// - external_id 全回填 == 已知集
/// - vector search baseline 一致（d0 排第一 score≈1.0，d3 第二 score≈0.707）
/// - text/hybrid search 命中（非 vacuous）
#[test]
fn reads_v0_1_0_fixture() {
    let dir = copy_fixture_to_temp("reads");
    let vfs = Arc::new(StdFsVfs::with_root(dir.to_str().unwrap())) as Arc<dyn Vfs>;

    let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
    // manifest restore：collections 含 "docs"
    assert!(
        db.collections().iter().any(|c| c == "docs"),
        "manifest 应 restore collection 'docs'"
    );
    let col = db
        .collection("docs", build_schema(), CollectionOptions::default())
        .unwrap();

    // 段文件 format_version 与 fixture 一致（非 vacuous：检 per-file version 常量）
    let seg_path = format!("db/segments/seg_{}", FIXTURE_SEG_ULID);
    assert_eq!(
        read_format_version(&vfs, &format!("{}/header.bin", seg_path)),
        vane_core::types::HEADER_FORMAT_V1,
        "header.bin V1"
    );
    assert_eq!(
        read_format_version(&vfs, &format!("{}/vectors.bin", seg_path)),
        vane_core::types::VECTORS_FORMAT_V2,
        "vectors.bin V2（v0.1.0 始终写 v2）"
    );
    assert_eq!(
        read_format_version(&vfs, &format!("{}/stored.bin", seg_path)),
        vane_core::types::STORED_FORMAT_V1,
        "stored.bin V1（v0.1.0 无 zstd-encode）"
    );
    assert_eq!(
        read_format_version(&vfs, &format!("{}/idmap.bin", seg_path)),
        vane_core::types::IDMAP_FORMAT_V1,
        "idmap.bin V1"
    );
    assert_eq!(
        read_format_version(&vfs, &format!("{}/scalars.col", seg_path)),
        vane_core::types::SCALARS_FORMAT_V1,
        "scalars.col V1"
    );
    assert_eq!(
        read_format_version(&vfs, &format!("{}/hnsw.bin", seg_path)),
        vane_core::types::HNSW_FORMAT_V1,
        "hnsw.bin V1"
    );

    // ---- vector search baseline：query=[1,0,0,0] top_k=10 ----
    let hits = col
        .search(&SearchQuery {
            text: None,
            vector: Some(vec![1.0, 0.0, 0.0, 0.0]),
            top_k: 10,
            mode: SearchMode::Vector,
            fusion: FusionSpec::Rrf,
            filter: None,
            candidate_multiplier: 3,
        })
        .unwrap();

    // 非空断言：应返回全部 5 文档
    assert!(!hits.is_empty(), "vector search 应返回结果");
    assert_eq!(hits.len(), 5, "vector search 应返回全部 5 文档");

    // external_id 全回填 == 已知集
    let ids = hit_ids(&hits);
    for id in &KNOWN_IDS {
        assert!(ids.contains(*id), "缺少已知文档 {}", id);
    }
    assert_eq!(ids.len(), 5, "external_id 集合应恰好 5 条");

    // baseline 顺序：d0(score≈1.0) > d3(score≈0.707) > d1=d2=d4(score=0.0, docid 升序)
    assert_eq!(hits[0].id, "v010-d0", "d0 应排第一（cosine=1.0）");
    assert!(
        (hits[0].score - 1.0).abs() < 1e-6,
        "d0 score 应为 1.0，实际 {}",
        hits[0].score
    );
    assert_eq!(hits[1].id, "v010-d3", "d3 应排第二（cosine≈0.707）");
    let expected_d3 = 1.0_f32 / 2.0_f32.sqrt();
    assert!(
        (hits[1].score - expected_d3).abs() < 1e-6,
        "d3 score 应为 {}，实际 {}",
        expected_d3,
        hits[1].score
    );
    // 同分 docid 升序：d1 < d2 < d4
    assert_eq!(hits[2].id, "v010-d1");
    assert_eq!(hits[3].id, "v010-d2");
    assert_eq!(hits[4].id, "v010-d4");

    // stored fields 回填（tag 字段）
    for h in &hits {
        assert!(h.fields.is_some(), "stored fields 应回填 for {}", h.id);
        let fields = h.fields.as_ref().unwrap();
        assert!(fields.contains_key("tag"), "tag 字段应回填 for {}", h.id);
    }

    // ---- text search：query="检索"（d0/d2/d4 含"检索"）----
    let text_hits = col
        .search(&SearchQuery {
            text: Some("检索".into()),
            vector: None,
            top_k: 10,
            mode: SearchMode::Text,
            fusion: FusionSpec::Rrf,
            filter: None,
            candidate_multiplier: 3,
        })
        .unwrap();
    assert!(!text_hits.is_empty(), "text search 应有命中（非 vacuous）");
    let text_ids = hit_ids(&text_hits);
    // "检索" 出现在 d0, d2, d4
    assert!(text_ids.contains("v010-d0"), "text search 应命中 d0");
    assert!(text_ids.contains("v010-d2"), "text search 应命中 d2");
    assert!(text_ids.contains("v010-d4"), "text search 应命中 d4");

    // ---- hybrid search：text="检索" + vector=[1,0,0,0] ----
    let hybrid_hits = col
        .search(&SearchQuery {
            text: Some("检索".into()),
            vector: Some(vec![1.0, 0.0, 0.0, 0.0]),
            top_k: 10,
            mode: SearchMode::Hybrid,
            fusion: FusionSpec::Rrf,
            filter: None,
            candidate_multiplier: 3,
        })
        .unwrap();
    assert!(
        !hybrid_hits.is_empty(),
        "hybrid search 应有命中（非 vacuous）"
    );
    let hybrid_ids = hit_ids(&hybrid_hits);
    for id in &KNOWN_IDS {
        assert!(hybrid_ids.contains(*id), "hybrid search 应包含 {}", id);
    }

    db.close().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

// =============================================================================
// 测试 2：v1/v2 段共存（fixture v1 stored.bin + 当前 flush 新段）
// =============================================================================

/// 同一 DB 内混合 v1（旧 fixture 段 stored.bin V1）+ v2（当前 flush 新段 stored.bin V2
/// 仅 zstd-encode feature 启用时；否则 V1）。search 结果应包含两段文档，一致。
#[test]
fn v1_and_v2_segments_coexist() {
    let dir = copy_fixture_to_temp("v1v2");
    let vfs = Arc::new(StdFsVfs::with_root(dir.to_str().unwrap())) as Arc<dyn Vfs>;

    let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
    let col = db
        .collection("docs", build_schema(), CollectionOptions::default())
        .unwrap();

    // ---- 旧段（fixture）格式版本确认 ----
    let segs_before = vfs.list("db/segments").unwrap();
    assert_eq!(
        segs_before.len(),
        1,
        "初始应有 1 段（fixture 段 {}）",
        FIXTURE_SEG_ULID
    );
    let old_seg = segs_before[0].clone();
    assert_eq!(old_seg, format!("seg_{}", FIXTURE_SEG_ULID));
    let old_stored_ver = read_format_version(&vfs, &format!("db/segments/{}/stored.bin", old_seg));
    assert_eq!(
        old_stored_ver,
        vane_core::types::STORED_FORMAT_V1,
        "fixture 段 stored.bin 应为 V1"
    );

    // ---- 添加新文档 + flush → 新段 ----
    let new_docs = vec![
        Doc {
            id: "v010-d5".into(),
            text: Some("新文档 cross version compatibility".into()),
            vector: Some(vec![1.0, 1.0, 1.0, 0.0]),
            meta: None,
        },
        Doc {
            id: "v010-d6".into(),
            text: Some("compatibility test segment coexist".into()),
            vector: Some(vec![0.0, 1.0, 1.0, 1.0]),
            meta: None,
        },
    ];
    let report = col.add(&new_docs).unwrap();
    assert_eq!(report.accepted, 2);
    col.flush().unwrap();

    // ---- 确认 2 段 ----
    let segs_after = vfs.list("db/segments").unwrap();
    assert_eq!(segs_after.len(), 2, "flush 后应有 2 段");
    let new_seg = segs_after
        .iter()
        .find(|s| **s != old_seg)
        .expect("应有新段")
        .clone();

    // 新段 stored.bin 版本：zstd-encode 启用 → V2（真 v1/v2 共存）；否则 V1
    let new_stored_ver = read_format_version(&vfs, &format!("db/segments/{}/stored.bin", new_seg));
    #[cfg(feature = "zstd-encode")]
    {
        assert_eq!(
            new_stored_ver,
            vane_core::types::STORED_FORMAT_V2,
            "新段 stored.bin 应为 V2（zstd-encode 启用）→ 真 v1/v2 共存"
        );
    }
    #[cfg(not(feature = "zstd-encode"))]
    {
        assert_eq!(
            new_stored_ver,
            vane_core::types::STORED_FORMAT_V1,
            "新段 stored.bin V1（无 zstd-encode）→ 双段同格式共存"
        );
    }

    // ---- search 一致：应返回全部 7 文档（5 旧 + 2 新）----
    let hits = col
        .search(&SearchQuery {
            text: None,
            vector: Some(vec![1.0, 0.0, 0.0, 0.0]),
            top_k: 10,
            mode: SearchMode::Vector,
            fusion: FusionSpec::Rrf,
            filter: None,
            candidate_multiplier: 3,
        })
        .unwrap();
    assert_eq!(hits.len(), 7, "应返回全部 7 文档（5 旧 + 2 新）");

    // 旧文档全部可见
    let ids = hit_ids(&hits);
    for id in &KNOWN_IDS {
        assert!(ids.contains(*id), "旧文档 {} 应仍可见", id);
    }
    // 新文档可见
    assert!(ids.contains("v010-d5"), "新文档 v010-d5 应可见");
    assert!(ids.contains("v010-d6"), "新文档 v010-d6 应可见");

    // d0 仍排第一（score=1.0，来自旧段）
    assert_eq!(hits[0].id, "v010-d0", "d0 仍应排第一");
    assert!((hits[0].score - 1.0).abs() < 1e-6, "d0 score 仍应为 1.0");

    // ---- hybrid search 也一致 ----
    let hybrid_hits = col
        .search(&SearchQuery {
            text: Some("检索 compatibility".into()),
            vector: Some(vec![1.0, 0.0, 0.0, 0.0]),
            top_k: 10,
            mode: SearchMode::Hybrid,
            fusion: FusionSpec::Rrf,
            filter: None,
            candidate_multiplier: 3,
        })
        .unwrap();
    assert!(!hybrid_hits.is_empty(), "hybrid search 应有命中");
    let hybrid_ids = hit_ids(&hybrid_hits);
    // d0 来自旧段（含"检索"），d5/d6 来自新段（含"compatibility"）
    assert!(hybrid_ids.contains("v010-d0"), "hybrid 应命中旧段 d0");
    assert!(
        hybrid_ids.contains("v010-d5") || hybrid_ids.contains("v010-d6"),
        "hybrid 应命中新段文档"
    );

    db.close().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

// =============================================================================
// 测试 3：迁移占位（未来格式升级时实现迁移器）
// =============================================================================

/// 当前 v1 不需迁移（双模读取 v1/v2 已覆盖兼容）。
/// 未来格式升级（如 v3 stored/vectors）时实现迁移器：
/// 1. 遍历所有旧格式段
/// 2. 读 v1/v2 数据
/// 3. 用新格式重写（flush 新段 + manifest 切换 + WAL）
/// 4. 删旧段
///
/// 当前标 `#[ignore]`——骨架在此，实现留待未来格式 bump。
#[test]
#[ignore = "当前 v1/v2 双模读取覆盖兼容；未来格式升级（v3+）时实现迁移器"]
fn migrates_v0_1_0_via_reindex() {
    let dir = copy_fixture_to_temp("migrate");
    let vfs = Arc::new(StdFsVfs::with_root(dir.to_str().unwrap())) as Arc<dyn Vfs>;

    let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
    let col = db
        .collection("docs", build_schema(), CollectionOptions::default())
        .unwrap();

    // 未来迁移器调用骨架：
    // col.migrate_segments(FormatMigrationTarget::V3).unwrap();

    // 当前：验证 v1 段可读（无需迁移）
    let hits = col
        .search(&SearchQuery {
            text: None,
            vector: Some(vec![1.0, 0.0, 0.0, 0.0]),
            top_k: 10,
            mode: SearchMode::Vector,
            fusion: FusionSpec::Rrf,
            filter: None,
            candidate_multiplier: 3,
        })
        .unwrap();
    assert_eq!(hits.len(), 5, "v1 段可读，5 文档全可见");

    db.close().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}
