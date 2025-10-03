// 基础类型定义；Task 2-5 逐步填充。
use std::fmt;

/// SPEC §10 错误码。code() 返回值与 SPEC §10 表一一对应。
#[derive(Debug, Clone)]
pub enum VaneError {
    Io(String),
    Schema(String),
    NotFound(String),
    Corrupt(String),
    Version(String),
    TokenizerMismatch(String),
    DictTooLarge,
    DictUnavailable,
    Busy,
    Unsupported,
    InvalidArg(String),
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
            Self::DictTooLarge => -7,
            Self::DictUnavailable => -8,
            Self::Busy => -9,
            Self::Unsupported => -10,
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
            Self::DictTooLarge => "E_DICT_TOO_LARGE",
            Self::DictUnavailable => "E_DICT_UNAVAILABLE",
            Self::Busy => "E_BUSY",
            Self::Unsupported => "E_UNSUPPORTED",
            Self::InvalidArg(_) => "E_INVALID_ARG",
        }
    }
}

impl fmt::Display for VaneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(m) => write!(f, "E_IO: {}", m),
            Self::Schema(m) => write!(f, "E_SCHEMA: {}", m),
            Self::NotFound(m) => write!(f, "E_NOT_FOUND: {}", m),
            Self::Corrupt(m) => write!(f, "E_CORRUPT: {}", m),
            Self::Version(m) => write!(f, "E_VERSION: {}", m),
            Self::TokenizerMismatch(m) => write!(f, "E_TOKENIZER_MISMATCH: {}", m),
            Self::DictTooLarge => write!(f, "E_DICT_TOO_LARGE"),
            Self::DictUnavailable => write!(f, "E_DICT_UNAVAILABLE"),
            Self::Busy => write!(f, "E_BUSY"),
            Self::Unsupported => write!(f, "E_UNSUPPORTED"),
            Self::InvalidArg(m) => write!(f, "E_INVALID_ARG: {}", m),
        }
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
            return Err(VaneError::InvalidArg(format!(
                "TokenizerId hex must be 64 chars, got {}",
                s.len()
            )));
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
        _ => Err(VaneError::InvalidArg(format!("invalid hex char: {:?}", c as char))),
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
        assert_eq!(VaneError::DictTooLarge.code(), -7);
        assert_eq!(VaneError::DictUnavailable.code(), -8);
        assert_eq!(VaneError::Busy.code(), -9);
        assert_eq!(VaneError::Unsupported.code(), -10);
        assert_eq!(VaneError::InvalidArg("x".into()).code(), -11);
    }

    #[test]
    fn error_name_matches_spec() {
        assert_eq!(VaneError::Io("x".into()).name(), "E_IO");
        assert_eq!(VaneError::Schema("x".into()).name(), "E_SCHEMA");
        assert_eq!(VaneError::NotFound("x".into()).name(), "E_NOT_FOUND");
        assert_eq!(VaneError::Corrupt("x".into()).name(), "E_CORRUPT");
        assert_eq!(VaneError::Version("x".into()).name(), "E_VERSION");
        assert_eq!(VaneError::TokenizerMismatch("x".into()).name(), "E_TOKENIZER_MISMATCH");
        assert_eq!(VaneError::DictTooLarge.name(), "E_DICT_TOO_LARGE");
        assert_eq!(VaneError::DictUnavailable.name(), "E_DICT_UNAVAILABLE");
        assert_eq!(VaneError::Busy.name(), "E_BUSY");
        assert_eq!(VaneError::Unsupported.name(), "E_UNSUPPORTED");
        assert_eq!(VaneError::InvalidArg("x".into()).name(), "E_INVALID_ARG");
    }

    #[test]
    fn error_is_display_and_std_error() {
        let e = VaneError::InvalidArg("topK exceeds 1000".into());
        assert!(format!("{}", e).contains("topK exceeds 1000"));
        // std::error::Error trait 可调用 source()
        assert!(std::error::Error::source(&e).is_none());
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
    fn metric_variants() {
        let m = Metric::Cosine;
        assert_eq!(format!("{:?}", m), "Cosine");
    }
}
