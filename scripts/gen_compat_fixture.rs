// scripts/gen_compat_fixture.rs — v0.1.0 fixture generator（M4 §3.4 跨版本兼容）
//
// 此文件**不是 workspace 编译目标**——放在 scripts/ 目录，不被 cargo build 识别。
// 它是 v0.1.0 tag `crates/vane-core/tests/gen_compat_fixture.rs` 的镜像副本，
// 仅供文档/复现参考。在 v0.1.0 tag 离线 worktree 运行，产物拷贝至主工作树
// `crates/vane-core/tests/fixtures/compat/v0.1.0/` 后入仓。
//
// 流程：StdFsVfs rooted at /tmp/v010-fixture/ → Db::open → 声明 collection →
// 灌入确定性已知文档集（5 docs，vector+text+stored meta）→ flush → close。
//
// 产物段文件格式（v0.1.0 per-file format_version）：
//   header.bin   HEADER_FORMAT_V1
//   vectors.bin  VECTORS_FORMAT_V2（v0.1.0 始终写 v2，含 dim 头）
//   stored.bin   STORED_FORMAT_V1（无 zstd-encode feature，裸 JSON）
//   idmap.bin     IDMAP_FORMAT_V1
//   scalars.col   SCALARS_FORMAT_V1
//   inverted.bin  FORMAT_VERSION=1
//
// 已知文档集 baseline（cross_version_compat.rs 断言用）：
//   external_id ∈ {v010-d0, v010-d1, v010-d2, v010-d3, v010-d4}
//   d0/d2 tag=a, d1/d4 tag=b, d3 tag=c
//   d0 vec=[1,0,0,0], d1=[0,1,0,0], d2=[0,0,1,0], d3=[1,1,0,0], d4=[0,0,0,1]

use std::sync::Arc;

use vane_core::api::{CollectionOptions, Db, Doc, OpenOptions};
use vane_core::types::{FieldDef, Metric, ScalarKind, Schema};
use vane_core::vfs::std_fs::StdFsVfs;
use vane_core::vfs::Vfs;

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

fn known_docs() -> Vec<Doc> {
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
            id: "v010-d0".into(),
            text: Some("向量检索 混合搜索 hybrid search engine".into()),
            vector: Some(vec![1.0, 0.0, 0.0, 0.0]),
            meta: Some(mk_meta("a")),
        },
        Doc {
            id: "v010-d1".into(),
            text: Some("BM25 ranking text retrieval".into()),
            vector: Some(vec![0.0, 1.0, 0.0, 0.0]),
            meta: Some(mk_meta("b")),
        },
        Doc {
            id: "v010-d2".into(),
            text: Some("机器学习 与 搜索引擎 ranking".into()),
            vector: Some(vec![0.0, 0.0, 1.0, 0.0]),
            meta: Some(mk_meta("a")),
        },
        Doc {
            id: "v010-d3".into(),
            text: Some("cosine similarity vector space".into()),
            vector: Some(vec![1.0, 1.0, 0.0, 0.0]),
            meta: Some(mk_meta("c")),
        },
        Doc {
            id: "v010-d4".into(),
            text: Some("全文检索 inverted index 倒排".into()),
            vector: Some(vec![0.0, 0.0, 0.0, 1.0]),
            meta: Some(mk_meta("b")),
        },
    ]
}

#[test]
fn gen_v0_1_0_fixture() {
    let root = "/tmp/v010-fixture";
    let _ = std::fs::remove_dir_all(root);
    std::fs::create_dir_all(root).unwrap();
    let vfs = Arc::new(StdFsVfs::with_root(root)) as Arc<dyn Vfs>;

    let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
    let col = db
        .collection("docs", build_schema(), CollectionOptions::default())
        .unwrap();
    let report = col.add(&known_docs()).unwrap();
    assert_eq!(report.accepted, 5);
    col.flush().unwrap();
    db.close().unwrap();

    // 验证段文件存在
    let segs = vfs.list("db/segments").unwrap();
    assert!(!segs.is_empty(), "应至少有一个段目录");
    let seg_dir = segs
        .iter()
        .find(|s| s.starts_with("seg_"))
        .expect("应存在 seg_<ulid> 目录")
        .clone();
    let seg_path = format!("db/segments/{}", seg_dir);
    for fname in ["header.bin", "vectors.bin", "stored.bin", "idmap.bin", "scalars.col", "inverted.bin"] {
        let path = format!("{}/{}", seg_path, fname);
        let mut tmp = [0u8; 1];
        let _ = vfs.read_at(&path, &mut tmp, 0).expect(&format!("{} 可读", fname));
    }
}
