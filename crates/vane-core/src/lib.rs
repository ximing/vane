// 一次性预声明全部模块（B1 裁决：避免 L1/L2 各计划并行改 lib.rs 冲突）
pub mod api;
pub mod bm25;
pub mod executor;
pub mod filter;
pub mod fusion;
pub mod hnsw;
pub mod merge;
pub mod persistence;
pub mod segment;
pub mod tokenizer;
pub mod types;
pub mod vector;
pub mod vfs;
pub mod wal;
