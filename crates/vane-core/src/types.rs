// 基础类型定义；Task 2-5 逐步填充。
use std::fmt;

// SPEC §3.1/§3.2/§3.3/§4.2/§6.1/§6.2/§6.3/§8.2 冻结常量
pub const DIM_MAX: u32 = 4096;
pub const TOPK_MAX: u32 = 1000;
pub const SEGMENT_MAX: usize = 10;
pub const DOC_MAX_BYTES: usize = 16 * 1024 * 1024;
pub const BM25_K1: f32 = 1.2;
pub const BM25_B: f32 = 0.75;
pub const RRF_K: u32 = 60;
pub const PAGE_CACHE_DEFAULT_MB: u32 = 32;
pub const PAGE_SIZE: usize = 64 * 1024;
pub const MAGIC: &[u8; 4] = b"VANE";
/// 全库 schema 版本（SPEC §6.2）：保留作 manifest/整体格式标识。
/// 段文件编解码改用 per-file 常量（见下），此常量仅作 schema 级版本与
/// inverted.bin（未 per-file 化的段文件）的版本字段。
pub const FORMAT_VERSION: u32 = 1;
pub const MAX_SEGMENT_DOCS_SMALL: u32 = 10_000;

// SPEC §6.2 per-file format_version（M2-08）：每段文件独立递增，替代单一
// FORMAT_VERSION 作段文件编解码版本判别。保留 FORMAT_VERSION 作全库 schema 版本。
// 段文件 magic + version(LE) 头由各自常量决定；v1/v2 双模读取保证 corpus 兼容（I-6）。
pub const HEADER_FORMAT_V1: u32 = 1;
pub const VECTORS_FORMAT_V1: u32 = 1;
/// vectors.bin v2：12 字节头 `magic|version=2|dim(4 LE)|payload`（M2-07 dim 来源）。
/// v1 头 8 字节（无 dim 字段，dim 从 payload 长度反推）。
pub const VECTORS_FORMAT_V2: u32 = 2;
pub const STORED_FORMAT_V1: u32 = 1;
/// stored.bin v2：`magic|version=2|raw_payload_len(4 LE)|zstd_block_len(4 LE)|zstd_block`。
/// raw_payload 为 v1 body（count+entries），zstd 块压缩（zstd-encode feature 写，ruzstd 解码）。
pub const STORED_FORMAT_V2: u32 = 2;
pub const IDMAP_FORMAT_V1: u32 = 1;
pub const SCALARS_FORMAT_V1: u32 = 1;
pub const HNSW_FORMAT_V1: u32 = 1;

/// SPEC §10 错误码。code() 返回值与 SPEC §10 表一一对应。
///
/// M4 诊断重构：所有 11 变体统一携带 `ErrorContext`（结构化字段），
/// 替代旧的 `String` payload + `append_context` 拼接模式。消费者可程序化访问
/// `context()` 拿 seg/docid/op/hint 字段，无需 parse Display 字符串。
/// 错误码 -1..-11 + 名称 E_IO 等不变（SPEC §10 硬约束）。
#[derive(Debug, Clone)]
pub enum VaneError {
    Io(ErrorContext),
    Schema(ErrorContext),
    NotFound(ErrorContext),
    Corrupt(ErrorContext),
    Version(ErrorContext),
    TokenizerMismatch(ErrorContext),
    DictTooLarge(ErrorContext),
    DictUnavailable(ErrorContext),
    Busy(ErrorContext),
    Unsupported(ErrorContext),
    InvalidArg(ErrorContext),
}

/// 结构化错误上下文（M4 诊断重构）。
///
/// 替代旧 `String` payload。`message` 是核心描述（必填），
/// `seg`/`docid`/`op`/`hint` 是结构化诊断字段（可选）。
/// Display 输出 `E_CODE: message [seg=... op=... docid=... hint=...]`，
/// `From<String>`/`From<&str>` 让简单构造点低摩擦迁移。
#[derive(Debug, Clone)]
pub struct ErrorContext {
    /// 核心错误描述（必填）。
    pub message: String,
    /// 段 ULID（段级错误附）。
    pub seg: Option<String>,
    /// 文档 ID（文档级错误附）。
    pub docid: Option<u64>,
    /// 操作名（flush/merge/search/open...）。
    pub op: Option<&'static str>,
    /// 建议操作。
    pub hint: Option<String>,
}

impl ErrorContext {
    /// 创建仅含核心消息的 ErrorContext（简单构造点用）。
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            seg: None,
            docid: None,
            op: None,
            hint: None,
        }
    }

    /// 设置段 ULID（builder 链式）。
    pub fn seg(mut self, seg: impl Into<String>) -> Self {
        self.seg = Some(seg.into());
        self
    }

    /// 设置文档 ID（builder 链式）。
    pub fn docid(mut self, docid: u64) -> Self {
        self.docid = Some(docid);
        self
    }

    /// 设置操作名（builder 链式）。
    pub fn op(mut self, op: &'static str) -> Self {
        self.op = Some(op);
        self
    }

    /// 设置建议操作（builder 链式）。
    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

/// `From<String>`/`From<&str>` 让 `VaneError::Io("msg".into())` /
/// `VaneError::Io(format!(...).into())` 直接可用，低摩擦迁移。
impl From<String> for ErrorContext {
    fn from(message: String) -> Self {
        Self::new(message)
    }
}

impl From<&str> for ErrorContext {
    fn from(message: &str) -> Self {
        Self::new(message)
    }
}

impl VaneError {
    pub fn code(&self) -> i32 {
        match self {
            Self::Io(_) => -1,
            Self::Schema(_) => -2,
            Self::NotFound(_) => -3,
            Self::Corrupt(_) => -4,
            Self::Version(_) => -5,
            Self::TokenizerMismatch(_) => -6,
            Self::DictTooLarge(_) => -7,
            Self::DictUnavailable(_) => -8,
            Self::Busy(_) => -9,
            Self::Unsupported(_) => -10,
            Self::InvalidArg(_) => -11,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Io(_) => "E_IO",
            Self::Schema(_) => "E_SCHEMA",
            Self::NotFound(_) => "E_NOT_FOUND",
            Self::Corrupt(_) => "E_CORRUPT",
            Self::Version(_) => "E_VERSION",
            Self::TokenizerMismatch(_) => "E_TOKENIZER_MISMATCH",
            Self::DictTooLarge(_) => "E_DICT_TOO_LARGE",
            Self::DictUnavailable(_) => "E_DICT_UNAVAILABLE",
            Self::Busy(_) => "E_BUSY",
            Self::Unsupported(_) => "E_UNSUPPORTED",
            Self::InvalidArg(_) => "E_INVALID_ARG",
        }
    }

    /// 取结构化上下文的不可变引用（跨绑定层用，替代旧 String payload match）。
    pub fn context(&self) -> &ErrorContext {
        match self {
            Self::Io(c) => c,
            Self::Schema(c) => c,
            Self::NotFound(c) => c,
            Self::Corrupt(c) => c,
            Self::Version(c) => c,
            Self::TokenizerMismatch(c) => c,
            Self::DictTooLarge(c) => c,
            Self::DictUnavailable(c) => c,
            Self::Busy(c) => c,
            Self::Unsupported(c) => c,
            Self::InvalidArg(c) => c,
        }
    }

    /// 取结构化上下文的可变引用（内部 `with_*` 方法用）。
    fn context_mut(&mut self) -> &mut ErrorContext {
        match self {
            Self::Io(c) => c,
            Self::Schema(c) => c,
            Self::NotFound(c) => c,
            Self::Corrupt(c) => c,
            Self::Version(c) => c,
            Self::TokenizerMismatch(c) => c,
            Self::DictTooLarge(c) => c,
            Self::DictUnavailable(c) => c,
            Self::Busy(c) => c,
            Self::Unsupported(c) => c,
            Self::InvalidArg(c) => c,
        }
    }

    /// 追加段 ULID 上下文（替代旧 `append_context`，builder 风格）。
    pub(crate) fn with_seg(mut self, seg: impl Into<String>) -> Self {
        self.context_mut().seg = Some(seg.into());
        self
    }

    /// 追加文档 ID 上下文。
    pub(crate) fn with_docid(mut self, docid: u64) -> Self {
        self.context_mut().docid = Some(docid);
        self
    }

    /// 追加操作名上下文。
    pub(crate) fn with_op(mut self, op: &'static str) -> Self {
        self.context_mut().op = Some(op);
        self
    }

    /// 追加建议操作上下文。
    pub(crate) fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.context_mut().hint = Some(hint.into());
        self
    }
}

impl fmt::Display for VaneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ctx = self.context();
        write!(f, "{}: {}", self.name(), ctx.message)?;
        // 追加结构化上下文（任一字段存在时输出 [seg=... op=... docid=... hint=...]）。
        // 不分配 Vec——直接流式写入，sep 追踪是否需前导空格。
        let mut sep = " [";
        if let Some(seg) = &ctx.seg {
            write!(f, "{}seg={}", sep, seg)?;
            sep = " ";
        }
        if let Some(op) = ctx.op {
            write!(f, "{}op={}", sep, op)?;
            sep = " ";
        }
        if let Some(docid) = ctx.docid {
            write!(f, "{}docid={}", sep, docid)?;
            sep = " ";
        }
        if let Some(hint) = &ctx.hint {
            write!(f, "{}hint={}", sep, hint)?;
            sep = " ";
        }
        if sep != " [" {
            write!(f, "]")?;
        }
        Ok(())
    }
}

impl std::error::Error for VaneError {}

pub type Result<T> = std::result::Result<T, VaneError>;

/// 检索结果文档（跨 bm25/vector-brute/fusion 模块）。
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScoredDoc {
    pub docid: u64,
    pub score: f32,
}

/// SPEC §3.1 向量距离度量。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Metric {
    Cosine,
    L2,
    Dot,
}

/// SPEC §5.4 分词器身份标识（sha256 产物）。
/// 结构定义在此（workspace），计算逻辑在 02-tokenizer 的 compute_tokenizer_id。
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct TokenizerId(pub [u8; 32]);

impl TokenizerId {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        let mut s = String::with_capacity(64);
        for b in &self.0 {
            s.push_str(&format!("{:02x}", b));
        }
        s
    }

    pub fn from_hex(s: &str) -> Result<Self> {
        if s.len() != 64 {
            return Err(VaneError::InvalidArg(
                format!("TokenizerId hex must be 64 chars, got {}", s.len()).into(),
            ));
        }
        let mut out = [0u8; 32];
        for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
            let hi = hex_val(chunk[0])?;
            let lo = hex_val(chunk[1])?;
            out[i] = hi * 16 + lo;
        }
        Ok(TokenizerId(out))
    }
}

fn hex_val(c: u8) -> Result<u8> {
    match c {
        b'0'..=b'9' => Ok(c - b'0'),
        b'a'..=b'f' => Ok(c - b'a' + 10),
        b'A'..=b'F' => Ok(c - b'A' + 10),
        _ => Err(VaneError::InvalidArg(
            format!("invalid hex char: {:?}", c as char).into(),
        )),
    }
}

/// SPEC §3.1 标量字段类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ScalarKind {
    Int,
    Float,
    Bool,
    Keyword,
}

/// SPEC §3.1 字段定义。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum FieldDef {
    Text,
    Vector { dim: u32, metric: Metric },
    Scalar { kind: ScalarKind },
}

/// SPEC §3.1 Collection schema。创建后仅允许附录式扩展（M0 不实现扩展）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Schema {
    pub fields: Vec<(String, FieldDef)>,
}

impl Schema {
    pub fn new(fields: Vec<(String, FieldDef)>) -> Result<Self> {
        let schema = Self { fields };
        schema.validate()?;
        Ok(schema)
    }

    /// 返回 (name, dim, metric)。§3.1 恰好一个 vector 字段。
    pub fn vector_field(&self) -> Result<(&str, u32, Metric)> {
        let mut found: Option<(&str, u32, Metric)> = None;
        for (name, def) in &self.fields {
            if let FieldDef::Vector { dim, metric } = def {
                if found.is_some() {
                    return Err(VaneError::Schema("multiple vector fields".into()));
                }
                found = Some((name.as_str(), *dim, *metric));
            }
        }
        found.ok_or_else(|| VaneError::Schema("no vector field".into()))
    }

    pub fn text_fields(&self) -> Vec<&str> {
        self.fields
            .iter()
            .filter(|(_, d)| matches!(d, FieldDef::Text))
            .map(|(n, _)| n.as_str())
            .collect()
    }

    /// §3.1 约束：恰好 1 个 vector 字段；dim ≤ 4096。
    pub fn validate(&self) -> Result<()> {
        let mut vec_count = 0;
        for (_, def) in &self.fields {
            if let FieldDef::Vector { dim, .. } = def {
                vec_count += 1;
                if *dim > DIM_MAX {
                    return Err(VaneError::Schema(
                        format!("dim {} exceeds max {}", dim, DIM_MAX).into(),
                    ));
                }
            }
        }
        // SPEC §3.1：恰好一个 vector 字段（M0–M2 限制）
        if vec_count != 1 {
            return Err(VaneError::Schema(
                format!("expected exactly 1 vector field, got {}", vec_count).into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_code_matches_spec_section_10() {
        assert_eq!(VaneError::Io("x".into()).code(), -1);
        assert_eq!(VaneError::Schema("x".into()).code(), -2);
        assert_eq!(VaneError::NotFound("x".into()).code(), -3);
        assert_eq!(VaneError::Corrupt("x".into()).code(), -4);
        assert_eq!(VaneError::Version("x".into()).code(), -5);
        assert_eq!(VaneError::TokenizerMismatch("x".into()).code(), -6);
        assert_eq!(VaneError::DictTooLarge("x".into()).code(), -7);
        assert_eq!(VaneError::DictUnavailable("x".into()).code(), -8);
        assert_eq!(VaneError::Busy("x".into()).code(), -9);
        assert_eq!(VaneError::Unsupported("x".into()).code(), -10);
        assert_eq!(VaneError::InvalidArg("x".into()).code(), -11);
    }

    #[test]
    fn error_name_matches_spec() {
        assert_eq!(VaneError::Io("x".into()).name(), "E_IO");
        assert_eq!(VaneError::Schema("x".into()).name(), "E_SCHEMA");
        assert_eq!(VaneError::NotFound("x".into()).name(), "E_NOT_FOUND");
        assert_eq!(VaneError::Corrupt("x".into()).name(), "E_CORRUPT");
        assert_eq!(VaneError::Version("x".into()).name(), "E_VERSION");
        assert_eq!(
            VaneError::TokenizerMismatch("x".into()).name(),
            "E_TOKENIZER_MISMATCH"
        );
        assert_eq!(
            VaneError::DictTooLarge("x".into()).name(),
            "E_DICT_TOO_LARGE"
        );
        assert_eq!(
            VaneError::DictUnavailable("x".into()).name(),
            "E_DICT_UNAVAILABLE"
        );
        assert_eq!(VaneError::Busy("x".into()).name(), "E_BUSY");
        assert_eq!(VaneError::Unsupported("x".into()).name(), "E_UNSUPPORTED");
        assert_eq!(VaneError::InvalidArg("x".into()).name(), "E_INVALID_ARG");
    }

    #[test]
    fn error_is_display_and_std_error() {
        let e = VaneError::InvalidArg("topK exceeds 1000".into());
        assert!(format!("{}", e).contains("topK exceeds 1000"));
        // std::error::Error trait 可调用 source()
        assert!(std::error::Error::source(&e).is_none());
    }

    /// M4 诊断重构：ErrorContext 结构化字段 + builder + with_* 链式 + Display 格式。
    #[test]
    fn error_context_structured_fields_and_display() {
        // builder 链式构造
        let e = VaneError::Corrupt(
            ErrorContext::new("vectors.bin bad magic")
                .seg("01HXYZ")
                .op("open vectors.bin")
                .hint("检查段文件完整性"),
        );
        assert_eq!(e.code(), -4);
        assert_eq!(e.name(), "E_CORRUPT");
        let ctx = e.context();
        assert_eq!(ctx.message, "vectors.bin bad magic");
        assert_eq!(ctx.seg.as_deref(), Some("01HXYZ"));
        assert_eq!(ctx.op, Some("open vectors.bin"));
        assert_eq!(ctx.hint.as_deref(), Some("检查段文件完整性"));
        assert!(ctx.docid.is_none());

        // Display 含 message + 结构化字段
        let msg = format!("{}", e);
        assert!(msg.contains("E_CORRUPT: vectors.bin bad magic"));
        assert!(msg.contains("seg=01HXYZ"));
        assert!(msg.contains("op=open vectors.bin"));
        assert!(msg.contains("hint=检查段文件完整性"));

        // with_* 链式（替代旧 append_context）
        let e2 = VaneError::Io("disk full".into())
            .with_seg("01HABC")
            .with_op("flush")
            .with_hint("检查磁盘空间");
        assert_eq!(e2.code(), -1);
        let ctx2 = e2.context();
        assert_eq!(ctx2.seg.as_deref(), Some("01HABC"));
        assert_eq!(ctx2.op, Some("flush"));

        // 无结构化字段的 Display 仅输出 name: message（无方括号）
        let e3 = VaneError::InvalidArg("bad arg".into());
        let msg3 = format!("{}", e3);
        assert_eq!(msg3, "E_INVALID_ARG: bad arg");
        assert!(!msg3.contains("["));

        // From<String> 让简单构造低摩擦
        let e4: ErrorContext = "simple".into();
        assert_eq!(e4.message, "simple");
        let e5: ErrorContext = String::from("owned").into();
        assert_eq!(e5.message, "owned");
    }

    #[test]
    fn tokenizer_id_hex_roundtrip() {
        let raw = [0u8; 32];
        let id = TokenizerId(raw);
        let hex = id.to_hex();
        assert_eq!(hex.len(), 64);
        let back = TokenizerId::from_hex(&hex).unwrap();
        assert_eq!(back.as_bytes(), &raw);
    }

    #[test]
    fn tokenizer_id_from_hex_rejects_bad_input() {
        assert!(TokenizerId::from_hex("short").is_err());
        assert!(TokenizerId::from_hex("zz").is_err());
    }

    #[test]
    fn tokenizer_id_from_hex_rejects_bad_chars_at_full_length() {
        // 64 chars long but contains illegal hex char 'z'; exercises hex_val branch
        assert!(TokenizerId::from_hex(&"z".repeat(64)).is_err());
    }

    #[test]
    fn metric_variants() {
        let m = Metric::Cosine;
        assert_eq!(format!("{:?}", m), "Cosine");
    }

    #[test]
    fn schema_with_single_vector_field_is_valid() {
        let s = Schema::new(vec![
            ("title".into(), FieldDef::Text),
            (
                "vec".into(),
                FieldDef::Vector {
                    dim: 384,
                    metric: Metric::Cosine,
                },
            ),
        ])
        .unwrap();
        assert_eq!(s.vector_field().unwrap().0, "vec");
        assert_eq!(s.vector_field().unwrap().1, 384);
        assert_eq!(s.text_fields(), vec!["title"]);
    }

    #[test]
    fn schema_with_zero_vector_fields_is_invalid() {
        // SPEC §3.1：恰好一个 vector 字段（M0–M2 限制）
        let r = Schema::new(vec![("body".into(), FieldDef::Text)]);
        assert!(matches!(r, Err(VaneError::Schema(_))));
    }

    #[test]
    fn schema_with_two_vector_fields_is_invalid() {
        let r = Schema::new(vec![
            (
                "v1".into(),
                FieldDef::Vector {
                    dim: 128,
                    metric: Metric::Dot,
                },
            ),
            (
                "v2".into(),
                FieldDef::Vector {
                    dim: 256,
                    metric: Metric::Cosine,
                },
            ),
        ]);
        assert!(matches!(r, Err(VaneError::Schema(_))));
    }

    #[test]
    fn schema_dim_over_4096_rejected() {
        let r = Schema::new(vec![(
            "v".into(),
            FieldDef::Vector {
                dim: 4097,
                metric: Metric::Cosine,
            },
        )]);
        assert!(matches!(r, Err(VaneError::Schema(_))));
    }

    #[test]
    fn frozen_constants_match_spec() {
        assert_eq!(DIM_MAX, 4096);
        assert_eq!(TOPK_MAX, 1000);
        assert_eq!(SEGMENT_MAX, 10);
        assert_eq!(DOC_MAX_BYTES, 16 * 1024 * 1024);
        assert_eq!(BM25_K1, 1.2);
        assert_eq!(BM25_B, 0.75);
        assert_eq!(RRF_K, 60);
        assert_eq!(PAGE_CACHE_DEFAULT_MB, 32);
        assert_eq!(PAGE_SIZE, 64 * 1024);
        assert_eq!(MAGIC, b"VANE");
        assert_eq!(FORMAT_VERSION, 1);
        assert_eq!(MAX_SEGMENT_DOCS_SMALL, 10_000);
    }

    /// M2-08 测试 1：per-file format_version 常量各自独立值（SPEC §6.2）。
    #[test]
    fn per_file_format_versions_independent() {
        assert_eq!(HEADER_FORMAT_V1, 1);
        assert_eq!(VECTORS_FORMAT_V1, 1);
        assert_eq!(VECTORS_FORMAT_V2, 2);
        assert_eq!(STORED_FORMAT_V1, 1);
        assert_eq!(STORED_FORMAT_V2, 2);
        assert_eq!(IDMAP_FORMAT_V1, 1);
        assert_eq!(SCALARS_FORMAT_V1, 1);
        assert_eq!(HNSW_FORMAT_V1, 1);
        // v2 常量与 v1 区分（判别位）
        assert_ne!(VECTORS_FORMAT_V1, VECTORS_FORMAT_V2);
        assert_ne!(STORED_FORMAT_V1, STORED_FORMAT_V2);
    }
}
