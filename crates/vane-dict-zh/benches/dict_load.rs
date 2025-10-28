//! 词典冷加载 bench（SPEC §13.1 <150ms）。
//!
//! `cargo bench -p vane-dict-zh`：测量 `JiebaDict::load_zstd(DICT_BIN)` 冷加载耗时。

use criterion::{criterion_group, criterion_main, Criterion};
use vane_dict_zh::DICT_BIN;

fn bench_load(c: &mut Criterion) {
    c.bench_function("dict_load", |b| {
        b.iter(|| {
            vane_core::tokenizer::jieba::JiebaDict::load_zstd(DICT_BIN)
                .expect("DICT_BIN must load");
        });
    });
}

criterion_group!(benches, bench_load);
criterion_main!(benches);
