pub mod types;
// B5 裁决：re-export 公共类型，使 vane_core::api::{Db, OpenOptions, ...} 路径可直接导入
pub use types::*;
pub mod db;
pub use db::*;
pub mod collection;
pub use collection::*;
pub mod reindex;
pub use reindex::*;
pub mod snapshot;
pub use snapshot::{read_snapshot, write_snapshot};
// M4 §3.6 inspect API：DB 级统计与段级信息（纯新增，不改 M0-M3 冻结签名）。
pub mod inspect;
pub use inspect::*;

#[cfg(test)]
mod reindex_tests;

#[cfg(test)]
mod tests;
