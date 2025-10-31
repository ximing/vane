//! criterion bench：冷启动打开 10 万文档库（SPEC §13.1）。
//!
//! 目标：open 10 万文档 StdFsVfs 库 <1s。若 >2s 则降级为分级指标
//! （元数据 <1s、首次查询 <3s）。
//!
//! ## fixture 生成
//! 10 万×384 维向量 ≈154MB，体积过大不提交。bench 自包含 setup 在
//! tempdir 内确定性生成 10 万文档库（100 批 × 1000 文档，每批 flush
//! 触发 auto-merge 到 ≤10 段）。OnceLock 保证单次 bench 运行内只生成一次。
//!
//! ## 两阶段测量
//! - 阶段 1 `open_100k_metadata`：Db::open + collection restore（M0 SegmentReader
//!   全加载 vectors/inverted/hnsw/scalars/text）。
//! - 阶段 2 `open_100k_full_and_first_query`：阶段 1 + 首次 vector search。
//!
//! M0 SegmentReader::open 一次性全加载（无懒加载，签名冻结）。若阶段 1 >1s，
//! 不调低断言——走 SPEC §13.1 降级路径（首次查询 <3s），懒加载留 M2。

use std::sync::{Arc, OnceLock};

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use vane_core::api::{CollectionOptions, Db, Doc, OpenOptions, SearchMode, SearchQuery};
use vane_core::types::{FieldDef, Metric, Schema};
use vane_core::vfs::std_fs::StdFsVfs;
use vane_core::vfs::Vfs;

const DOC_COUNT: usize = 100_000;
const BATCH_SIZE: usize = 1_000; // 100 批 → 100 次 flush → auto-merge 到 ≤10 段
const DIM: usize = 384;

/// 测试用 schema（与 fixture 生成一致；collection() 幂等返回已 restore 句柄）。
fn test_schema() -> Schema {
    Schema::new(vec![
        ("body".into(), FieldDef::Text),
        (
            "v".into(),
            FieldDef::Vector {
                dim: DIM as u32,
                metric: Metric::Cosine,
            },
        ),
    ])
    .unwrap()
}

/// 生成确定性 10 万文档向量（不依赖 rand crate，避免新依赖）。
fn make_vector(id: usize) -> Vec<f32> {
    let mut v = vec![0.0f32; DIM];
    for (d, slot) in v.iter_mut().enumerate() {
        let h = ((id as u32).wrapping_mul(2654435761).wrapping_add(d as u32)) as f32;
        *slot = ((h % 1000.0) / 500.0) - 1.0;
    }
    v
}

/// fixture 句柄：(root 绝对路径, db_path)。OnceLock 持有，bench 进程生命周期内常驻。
/// 目录位于 OS tempdir 下带 PID，避免并发 bench 冲突；进程退出后残留由 OS/CI 清理。
struct Fixture {
    root: String,
    db_path: String,
}

static FIXTURE: OnceLock<Fixture> = OnceLock::new();

/// 首次访问时生成 10 万文档库；后续直接返回缓存。
fn ensure_fixture() -> &'static Fixture {
    FIXTURE.get_or_init(|| {
        let root = std::env::temp_dir()
            .join(format!("vane-cold-start-100k-{}", std::process::id()))
            .to_string_lossy()
            .into_owned();
        // 清理上次残留（崩溃/中断留下的半成品目录）
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create tempdir for cold-start fixture");

        let db_path = "cold_100k".to_string();
        let vfs = Arc::new(StdFsVfs::with_root(&root));
        // open 前清理可能的旧 db 目录（root 已重建，但 with_root 会 recreate，保险起见）
        let _ = vfs.delete(&db_path);
        let db = Db::open(vfs.clone(), &db_path, OpenOptions::default())
            .expect("open db for fixture generation");
        let col = db
            .collection("docs", test_schema(), CollectionOptions::default())
            .expect("create docs collection");

        let batches = DOC_COUNT / BATCH_SIZE;
        for batch in 0..batches {
            let docs: Vec<Doc> = (0..BATCH_SIZE)
                .map(|i| {
                    let id = batch * BATCH_SIZE + i;
                    Doc {
                        id: format!("d{}", id),
                        text: Some(format!("document {} batch {}", id, batch)),
                        vector: Some(make_vector(id)),
                        meta: None,
                    }
                })
                .collect();
            col.add(&docs).expect("add batch");
            // 每次 flush 触发段落盘；段数超 SEGMENT_MAX(10) 时 auto-merge 两小段。
            col.flush().expect("flush batch");
        }
        db.close().expect("close db after fixture generation");

        eprintln!(
            "[vane-cold-start] fixture ready: {} docs, {} batches, root={}",
            DOC_COUNT, batches, root
        );
        Fixture { root, db_path }
    })
}

/// 阶段 1：Db::open + collection restore（含 vectors/inverted/hnsw/scalars/text 全加载）。
fn bench_open_metadata(c: &mut Criterion) {
    let fixture = ensure_fixture();
    c.bench_function("open_100k_metadata", |b| {
        b.iter(|| {
            let vfs = Arc::new(StdFsVfs::with_root(&fixture.root));
            let db = Db::open(vfs, &fixture.db_path, OpenOptions::default()).unwrap();
            // collection() 幂等返回 open 时已 restore 的句柄（schema 一致校验）。
            let _col = db
                .collection("docs", test_schema(), CollectionOptions::default())
                .unwrap();
            db.close().unwrap();
        });
    });
}

/// 阶段 2：open + 首次 vector search（topK=10）。
fn bench_open_full_and_first_query(c: &mut Criterion) {
    let fixture = ensure_fixture();
    let query_vec = make_vector(0); // 命中 d0 附近，稳定 topK
    c.bench_function("open_100k_full_and_first_query", |b| {
        b.iter(|| {
            let vfs = Arc::new(StdFsVfs::with_root(&fixture.root));
            let db = Db::open(vfs, &fixture.db_path, OpenOptions::default()).unwrap();
            let col = db
                .collection("docs", test_schema(), CollectionOptions::default())
                .unwrap();
            let _hits = col
                .search(black_box(&SearchQuery {
                    text: None,
                    vector: Some(query_vec.clone()),
                    top_k: 10,
                    mode: SearchMode::Vector,
                    ..Default::default()
                }))
                .unwrap();
            db.close().unwrap();
        });
    });
}

criterion_group!(
    benches,
    bench_open_metadata,
    bench_open_full_and_first_query
);
criterion_main!(benches);
