//! criterion bench：batch add 吞吐（docs/s）。
//!
//! DoD 要求 benchmark CI 产生基线数据（批量 add 吞吐）。
//! 每次迭代 add 一批 N 条文档（含 flush），criterion 报告每次迭代耗时，
//! 吞吐 = batch_size / iter_time。SPEC §13.2 回退 >10% 报警。

use std::sync::Arc;

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use vane_core::api::{CollectionOptions, Db, Doc, OpenOptions};
use vane_core::types::{FieldDef, Metric, Schema};
use vane_core::vfs::memory::MemoryVfs;

const DIM: usize = 384;

fn make_docs(batch: usize, seed: usize) -> Vec<Doc> {
    let mut docs = Vec::with_capacity(batch);
    for i in 0..batch {
        let mut v = vec![0.0f32; DIM];
        for (d, slot) in v.iter_mut().enumerate() {
            let h = (((seed + i) as u32)
                .wrapping_mul(2654435761)
                .wrapping_add(d as u32)) as f32;
            *slot = ((h % 1000.0) / 500.0) - 1.0;
        }
        docs.push(Doc {
            id: format!("doc_{}_{}", seed, i),
            text: Some(format!("batch{} term{} common", seed, i % 16)),
            vector: Some(v),
            meta: None,
        });
    }
    docs
}

fn bench_batch_add(c: &mut Criterion) {
    let batch_sizes: &[usize] = &[100, 500];

    let mut group = c.benchmark_group("batch_add");
    for &batch in batch_sizes {
        group.throughput(Throughput::Elements(batch as u64));
        group.bench_with_input(BenchmarkId::from_parameter(batch), &batch, |b, &batch| {
            b.iter_batched_ref(
                || {
                    // 每次 iteration 重建独立 db，避免 docid 爆炸 / 段数膨胀
                    let vfs = Arc::new(MemoryVfs::new());
                    let db = Db::open(vfs, "bench_add", OpenOptions::default()).unwrap();
                    let schema = Schema::new(vec![
                        ("body".into(), FieldDef::Text),
                        (
                            "v".into(),
                            FieldDef::Vector {
                                dim: DIM as u32,
                                metric: Metric::Cosine,
                            },
                        ),
                    ])
                    .unwrap();
                    let col = db
                        .collection("docs", schema, CollectionOptions::default())
                        .unwrap();
                    let docs = make_docs(batch, 0);
                    (col, docs)
                },
                |(col, docs)| {
                    col.add(black_box(docs)).unwrap();
                    col.flush().unwrap();
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

criterion_group!(benches, bench_batch_add);
criterion_main!(benches);
