pub mod types;
// B5 裁决：re-export 公共类型，使 vane_core::api::{Db, OpenOptions, ...} 路径可直接导入
pub use types::*;
pub mod db;
pub use db::*;
pub mod collection;
pub use collection::*;

#[cfg(test)]
mod tests;
