//! SPEC §4.2 公共 API 类型定义（M0 冻结）。

use crate::persistence::AutoCommitConfig;
use crate::tokenizer::{BuiltinTokenizer, UserDictEntry};

/// SPEC §6.4 持久化模式（M0 仅 Persistent 落盘，BestEffort 占位）。
pub enum PersistenceMode {
    Persistent,
    BestEffort,
}

pub struct OpenOptions {
    pub persistence: PersistenceMode,
    pub auto_commit: AutoCommitConfig,
    pub page_cache_mb: u32,
}
impl Default for OpenOptions {
    fn default() -> Self {
        Self {
            persistence: PersistenceMode::Persistent,
            auto_commit: AutoCommitConfig::default(),
            page_cache_mb: 32,
        }
    }
}

pub struct CollectionOptions {
    pub tokenizer: BuiltinTokenizer,
    pub user_dict: Vec<UserDictEntry>,
    // I3 裁决：collection 级 auto-commit 配置
    pub auto_commit: AutoCommitConfig,
}
impl Default for CollectionOptions {
    fn default() -> Self {
        Self {
            tokenizer: BuiltinTokenizer::Standard,
            user_dict: vec![],
            auto_commit: AutoCommitConfig::default(),
        }
    }
}

// SearchMode::Auto 为内部推断标记，JS/Go 绑定层不暴露 "auto" 字符串（S8）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SearchMode {
    Hybrid,
    Vector,
    Text,
    Auto,
}

#[derive(Debug, Clone)]
pub enum FusionSpec {
    Rrf,
    Linear { alpha: f32 },
}

#[derive(Debug, Clone)]
pub enum ScalarValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    Keyword(String),
}

#[derive(Debug, Clone)]
pub enum FilterCond {
    Eq(ScalarValue),
    In(Vec<ScalarValue>),
    Gte(ScalarValue),
    Lte(ScalarValue),
}

#[derive(Debug, Clone)]
pub struct Filter {
    pub fields: Vec<(String, FilterCond)>,
}

pub struct SearchQuery {
    pub text: Option<String>,
    pub vector: Option<Vec<f32>>,
    pub top_k: u32,
    pub mode: SearchMode,
    pub fusion: FusionSpec,
    pub filter: Option<Filter>,
    pub candidate_multiplier: u32,
}
impl Default for SearchQuery {
    fn default() -> Self {
        Self {
            text: None,
            vector: None,
            top_k: 10,
            mode: SearchMode::Auto,
            fusion: FusionSpec::Rrf,
            filter: None,
            candidate_multiplier: 3,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Hit {
    pub id: String,
    pub score: f32,
    pub fields: Option<std::collections::HashMap<String, String>>,
}

pub struct AddReport {
    pub accepted: u64,
    pub visible_after_flush: bool,
}

pub struct Doc {
    pub id: String,
    pub text: Option<String>,
    pub vector: Option<Vec<f32>>,
    pub meta: Option<std::collections::HashMap<String, ScalarValue>>,
}
