// vane-node: Node.js napi-rs 绑定（薄壳，SPEC §9.3 / §14 I-8）。
// 直连 vane_core::api（不经 C ABI）；异步经 AsyncTask（libuv worker pool），
// 不桥接 tokio（§9.3）。仅做 JSON ↔ Rust 结构转换，无检索逻辑。
#![deny(warnings)]

mod collection;
mod convert;
mod db;
mod error;
