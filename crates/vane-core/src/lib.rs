// 一次性预声明全部模块（B1 裁决：避免 L1/L2 各计划并行改 lib.rs 冲突）
pub mod types;
pub mod vfs;
pub mod tokenizer;
pub mod fusion;
pub mod vector;
pub mod segment;
pub mod bm25;
pub mod persistence;
pub mod api;
