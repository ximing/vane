// tests/corpus_compat.rs — SPEC §13.3 corpus 格式兼容测试骨架
//
// 此测试冻结 M0 段格式（header.bin / vectors.bin / stored.bin / idmap.bin /
// scalars.col / inverted.bin）。任何格式变更必须保持此测试通过，或 bump
// FORMAT_VERSION + 提供迁移器/双模读取（SPEC §6.2）。
//
// stored.bin 布局（v1.1 起，00-text-persistence 补全 SPEC §6.2 始终要求的格式，
// format_version 保持 1，仓库无发布产物故无迁移）：
//   magic(4)="VANE" | format_version(4 LE)=1 | count(4 LE) |
//   { docid(8 LE) | text_len(4 LE) | text_bytes | meta_json_len(4 LE) | meta_json_bytes }...
// 原文 + JSON meta 分离存储；text_len=0 表示无原文（写期未调 set_text）。
//
// 流程：用 StdFsVfs 在临时目录建 DB → 声明 collection（text+vector+scalar）→
// 灌入若干文档（中文/英文 mixed）→ flush → close → 重新 open 同目录 →
// 验证 manifest 加载、segment 读取、search（hybrid/vector/text 三模式）结果
// 与关闭前基线一致、external_id / stored_json 回填正确。
//
// M0 口径：仓库无历史发布产物（fresh repo），此测试冻结的是清理**后**的格式。
// 真实历史版本 golden fixture 待首个正式发布后补（§13.3）。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use vane_core::api::{
    CollectionOptions, Db, Doc, FusionSpec, OpenOptions, SearchMode, SearchQuery,
};
use vane_core::types::{FieldDef, Metric, ScalarKind, Schema};
use vane_core::vfs::std_fs::StdFsVfs;
use vane_core::vfs::Vfs;

/// 生成唯一临时目录，避免并行测试冲突。
fn unique_dir(label: &str) -> std::path::PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "vane-corpus-compat-{}-{}-{}-{}",
        label,
        std::process::id(),
        n,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
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

/// 构造 corpus：5 篇中英混排文档。
fn corpus_docs() -> Vec<Doc> {
    let mk_meta = |tag: &str| {
        let mut m = std::collections::HashMap::new();
        m.insert(
            "tag".to_string(),
            vane_core::api::ScalarValue::Keyword(tag.into()),
        );
        m
    };
    vec![
        Doc {
            id: "d0".into(),
            text: Some("向量检索 混合搜索 hybrid search engine".into()),
            vector: Some(vec![1.0, 0.0, 0.0, 0.0]),
            meta: Some(mk_meta("a")),
        },
        Doc {
            id: "d1".into(),
            text: Some("BM25 ranking text retrieval".into()),
            vector: Some(vec![0.0, 1.0, 0.0, 0.0]),
            meta: Some(mk_meta("b")),
        },
        Doc {
            id: "d2".into(),
            text: Some("机器学习 与 搜索引擎 ranking".into()),
            vector: Some(vec![0.0, 0.0, 1.0, 0.0]),
            meta: Some(mk_meta("a")),
        },
        Doc {
            id: "d3".into(),
            text: Some("cosine similarity vector space".into()),
            vector: Some(vec![1.0, 1.0, 0.0, 0.0]),
            meta: Some(mk_meta("c")),
        },
        Doc {
            id: "d4".into(),
            text: Some("全文检索 inverted index 倒排".into()),
            vector: Some(vec![0.0, 0.0, 0.0, 1.0]),
            meta: Some(mk_meta("b")),
        },
    ]
}

/// 捕获 search 基线：(id, score, fields_tag) 三元组列表。
fn capture_hits(hits: &[vane_core::api::Hit]) -> Vec<(String, f32, Option<String>)> {
    hits.iter()
        .map(|h| {
            let tag = h.fields.as_ref().and_then(|f| f.get("tag")).cloned();
            (h.id.clone(), h.score, tag)
        })
        .collect()
}

fn run_searches(col: &vane_core::api::Collection) -> Vec<Vec<(String, f32, Option<String>)>> {
    let qvec = vec![1.0, 0.0, 0.0, 0.0];
    let modes = [
        (
            SearchMode::Hybrid,
            Some("search ranking".to_string()),
            Some(qvec.clone()),
        ),
        (SearchMode::Vector, None, Some(qvec.clone())),
        (SearchMode::Text, Some("检索 ranking".to_string()), None),
    ];
    modes
        .iter()
        .map(|(mode, text, vector)| {
            let hits = col
                .search(&SearchQuery {
                    text: text.clone(),
                    vector: vector.clone(),
                    top_k: 5,
                    mode: *mode,
                    fusion: FusionSpec::Rrf,
                    filter: None,
                    candidate_multiplier: 3,
                })
                .unwrap();
            capture_hits(&hits)
        })
        .collect()
}

#[test]
fn corpus_format_compat_roundtrip() {
    let dir = unique_dir("roundtrip");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let vfs = Arc::new(StdFsVfs::with_root(dir.to_str().unwrap())) as Arc<dyn Vfs>;

    // ---- 第一次 open：建库 + 灌数据 + flush + close ----
    let baseline = {
        let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
        let col = db
            .collection("docs", build_schema(), CollectionOptions::default())
            .unwrap();
        let report = col.add(&corpus_docs()).unwrap();
        assert_eq!(report.accepted, 5);
        col.flush().unwrap();
        let baseline = run_searches(&col);
        // 验证 external_id / stored_json 回填（hybrid 结果应非空）
        assert!(!baseline[0].is_empty(), "hybrid 应有命中");
        // tag 回填：stored.bin 序列化为 JSON，回填时 v.to_string() 保留引号，
        // 故 Keyword("a") 回填为 "\"a\""。校验 tag 非空且为 a/b/c 之一的 JSON 串。
        for (_, _, tag) in &baseline[0] {
            assert!(
                matches!(
                    tag.as_deref(),
                    Some("\"a\"") | Some("\"b\"") | Some("\"c\"")
                ),
                "tag 回填异常: {:?}",
                tag
            );
        }
        db.close().unwrap();
        baseline
    };

    // ---- 第二次 open：同目录，验证 manifest + segment 读回 ----
    {
        let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
        // manifest restore：collections 含 "docs"
        assert!(db.collections().iter().any(|c| c == "docs"));
        let col = db
            .collection("docs", build_schema(), CollectionOptions::default())
            .unwrap();
        let reopened = run_searches(&col);

        assert_eq!(reopened.len(), baseline.len(), "三模式搜索数量应一致");
        for (i, (got, want)) in reopened.iter().zip(baseline.iter()).enumerate() {
            assert_eq!(got.len(), want.len(), "模式 {} 命中数不一致", i);
            for (j, ((gid, gscore, gtag), (wid, wscore, wtag))) in
                got.iter().zip(want.iter()).enumerate()
            {
                assert_eq!(gid, wid, "模式 {} 第 {} 条 external_id 不一致", i, j);
                assert!(
                    (gscore - wscore).abs() < 1e-6,
                    "模式 {} 第 {} 条 score 不一致: {} vs {}",
                    i,
                    j,
                    gscore,
                    wscore
                );
                assert_eq!(gtag, wtag, "模式 {} 第 {} 条 stored tag 不一致", i, j);
            }
        }
        db.close().unwrap();
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn corpus_segment_files_have_magic_version_headers() {
    // SPEC §6.2：所有段文件以 4 字节 magic + 4 字节 format_version 开头。
    // 此测试校验 header.bin / vectors.bin / stored.bin / idmap.bin / scalars.col
    // 均含 VANE magic + format_version(LE=1) 头。
    let dir = unique_dir("headers");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let vfs = Arc::new(StdFsVfs::with_root(dir.to_str().unwrap())) as Arc<dyn Vfs>;

    let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
    let col = db
        .collection("docs", build_schema(), CollectionOptions::default())
        .unwrap();
    col.add(&corpus_docs()).unwrap();
    col.flush().unwrap();
    db.close().unwrap();

    // 列出 segments 目录下的段
    let segs = vfs.list("db/segments").unwrap();
    let seg_dir = segs
        .iter()
        .find(|s| s.starts_with("seg_"))
        .expect("应存在 seg_<ulid> 目录")
        .clone();
    let seg_path = format!("db/segments/{}", seg_dir);

    for fname in [
        "header.bin",
        "vectors.bin",
        "stored.bin",
        "idmap.bin",
        "scalars.col",
        "inverted.bin",
    ] {
        let path = format!("{}/{}", seg_path, fname);
        let mut buf = Vec::new();
        let mut tmp = [0u8; 8192];
        let mut off = 0u64;
        loop {
            let n = vfs.read_at(&path, &mut tmp, off).unwrap();
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
            off += n as u64;
        }
        assert!(
            buf.len() >= 8,
            "{} 应至少 8 字节（magic+version），实际 {}",
            fname,
            buf.len()
        );
        assert_eq!(&buf[0..4], b"VANE", "{} magic 错误", fname);
        // M2-08：per-file format_version。vectors.bin v2（含 dim 头）、stored.bin v1/v2
        // （zstd-encode 时 v2）。其余段文件仍 v1。校验 version ∈ 文件预期集合。
        let ver = u32::from_le_bytes(buf[4..8].try_into().unwrap());
        let ok = match fname {
            "vectors.bin" => ver == 1 || ver == 2,
            "stored.bin" => ver == 1 || ver == 2,
            _ => ver == 1,
        };
        assert!(ok, "{} format_version {} 不在预期集合", fname, ver);
    }

    // SPEC §6.2（00-text-persistence + M2-08）：stored.bin 含原文 + meta 分离存储。
    // 校验首条记录 text_len > 0 且 text_bytes 等于 corpus_docs()[0].text 的 UTF-8 字节。
    // v1 布局：magic(4)|version(4)|count(4)|{docid(8)|text_len(4)|text_bytes|meta_json_len(4)|...}
    // v2 布局：magic(4)|version(4)|raw_payload_len(4)|zstd_block_len(4)|zstd_block（zstd-encode 时）
    //   —— v2 时 body 校验跳过（zstd 压缩，由 decode_stored roundtrip 覆盖）。
    {
        let mut buf = Vec::new();
        let mut tmp = [0u8; 8192];
        let mut off = 0u64;
        loop {
            let n = vfs
                .read_at(&format!("{}/stored.bin", seg_path), &mut tmp, off)
                .unwrap();
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
            off += n as u64;
        }
        let sver = u32::from_le_bytes(buf[4..8].try_into().unwrap());
        if sver == 1 {
            // v1：跳过 12 字节头（magic+version+count），读首条 docid(8) + text_len(4)
            assert!(buf.len() >= 12 + 8 + 4, "stored.bin 首条记录头不全");
            let text_len = u32::from_le_bytes(buf[20..24].try_into().unwrap()) as usize;
            let corpus = corpus_docs();
            let expected_text = corpus[0].text.as_ref().unwrap().as_bytes();
            assert!(
                text_len > 0,
                "首条记录 text_len 应 > 0（corpus 首文档有原文）"
            );
            assert_eq!(
                text_len,
                expected_text.len(),
                "首条记录 text_len 应等于 corpus_docs()[0].text 字节数"
            );
            let text_bytes = &buf[24..24 + text_len];
            assert_eq!(
                text_bytes, expected_text,
                "首条记录 text_bytes 应等于 corpus_docs()[0].text UTF-8 字节"
            );
            // meta_json 紧随其后
            let meta_off = 24 + text_len;
            let meta_len =
                u32::from_le_bytes(buf[meta_off..meta_off + 4].try_into().unwrap()) as usize;
            assert!(
                meta_len > 0,
                "首条记录 meta_json_len 应 > 0（flush 落 {{}}）"
            );
        }
        // v2 body 校验由 decode_stored roundtrip（corpus_format_compat_roundtrip）覆盖。
    }

    let _ = std::fs::remove_dir_all(&dir);
}

// =============================================================================
// M2-08 corpus 兼容 v2 roundtrip（SPEC §13.3 + §6.2 stored v2 zstd）
// =============================================================================

/// 测试 6：corpus 兼容 roundtrip v2——zstd-encode 启用时写 v2 stored.bin（zstd 块），
/// close → open → search 基线一致（SPEC §13.3 冻结兼容）。
/// vectors.bin v2（含 dim 头）始终写（无 feature 门）；stored v2 仅 zstd-encode 时。
#[cfg(feature = "zstd-encode")]
#[test]
fn corpus_format_compat_v2_roundtrip() {
    let dir = unique_dir("v2-roundtrip");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let vfs = Arc::new(StdFsVfs::with_root(dir.to_str().unwrap())) as Arc<dyn Vfs>;

    let baseline = {
        let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
        let col = db
            .collection("docs", build_schema(), CollectionOptions::default())
            .unwrap();
        col.add(&corpus_docs()).unwrap();
        col.flush().unwrap();
        let baseline = run_searches(&col);
        assert!(!baseline[0].is_empty(), "hybrid 应有命中");
        db.close().unwrap();
        baseline
    };

    // 校验段文件确实为 v2（vectors.bin v2 + stored.bin v2）
    {
        let segs = vfs.list("db/segments").unwrap();
        let seg_dir = segs.iter().find(|s| s.starts_with("seg_")).unwrap().clone();
        let seg_path = format!("db/segments/{}", seg_dir);
        // vectors.bin v2
        let mut hdr = [0u8; 12];
        let _ = vfs
            .read_at(&format!("{}/vectors.bin", seg_path), &mut hdr, 0)
            .unwrap();
        assert_eq!(
            u32::from_le_bytes(hdr[4..8].try_into().unwrap()),
            2,
            "vectors v2"
        );
        assert_eq!(
            u32::from_le_bytes(hdr[8..12].try_into().unwrap()),
            4,
            "vectors v2 dim=4"
        );
        // stored.bin v2
        let mut shdr = [0u8; 8];
        let _ = vfs
            .read_at(&format!("{}/stored.bin", seg_path), &mut shdr, 0)
            .unwrap();
        assert_eq!(
            u32::from_le_bytes(shdr[4..8].try_into().unwrap()),
            2,
            "stored v2"
        );
    }

    // 重新 open：v2 stored → ruzstd 解码 → search 基线一致
    {
        let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
        assert!(db.collections().iter().any(|c| c == "docs"));
        let col = db
            .collection("docs", build_schema(), CollectionOptions::default())
            .unwrap();
        let reopened = run_searches(&col);
        assert_eq!(reopened.len(), baseline.len());
        for (i, (got, want)) in reopened.iter().zip(baseline.iter()).enumerate() {
            assert_eq!(got.len(), want.len(), "v2 模式 {} 命中数不一致", i);
            for (j, ((gid, gscore, gtag), (wid, wscore, wtag))) in
                got.iter().zip(want.iter()).enumerate()
            {
                assert_eq!(gid, wid, "v2 模式 {} 第 {} 条 id 不一致", i, j);
                assert!(
                    (gscore - wscore).abs() < 1e-6,
                    "v2 模式 {} 第 {} 条 score 不一致",
                    i,
                    j
                );
                assert_eq!(gtag, wtag, "v2 模式 {} 第 {} 条 tag 不一致", i, j);
            }
        }
        db.close().unwrap();
    }

    let _ = std::fs::remove_dir_all(&dir);
}
