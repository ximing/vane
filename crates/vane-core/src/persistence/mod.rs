//! 持久化模块（SPEC §6.4 / §7.1）：
//! - Manifest 原子读写（临时文件 → sync → rename，不变量 I-6）
//! - AutoCommitter（计数 + 时间双触发）
//!
//! 本模块不直接读写段内容（段由 04-segment-format 产出），只管 manifest 指针。

use crate::tokenizer::{BuiltinTokenizer, UserDictEntry};
use crate::types::{Schema, TokenizerId};
use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests;

/// SPEC §6.2 manifest.json 结构。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub collections: std::collections::HashMap<String, CollectionMeta>,
}

impl Manifest {
    pub fn empty() -> Self {
        Self {
            version: 1,
            collections: std::collections::HashMap::new(),
        }
    }
}

/// 单个 collection 的持久化元数据（SPEC §6.2）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionMeta {
    pub schema: Schema,
    pub tokenizer_kind: BuiltinTokenizer,
    pub tokenizer_id: TokenizerId,
    pub user_dict: Vec<UserDictEntry>,
    pub segment_ulids: Vec<String>,
}
