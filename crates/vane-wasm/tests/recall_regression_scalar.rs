//! M2-06 scalar 变体召回回归（wasm-bindgen-test，node）。
//!
//! 由 CI 以默认 RUSTFLAGS（无 simd128）构建，跑五档×三模式 recall@10≥0.95
//! + Jaccard 探针。SPEC §8.4（双变体召回回归）/ §13.2-1（recall@10≥0.95 五档）。
//!
//! 测试逻辑在 `common` 模块（复用 M1 方法论）；本二进制仅作 scalar 变体运行载体（I-8）。

mod common;
