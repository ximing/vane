use serde::{Deserialize, Serialize};

use crate::chunk::chunk_text;
use crate::config::ChunkConfig;
use crate::error::VaneCliError;

pub const TEXT_MAX_BYTES: usize = 8 * 1024 * 1024;
pub const IMAGE_MAX_BYTES: usize = 20 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CanonicalDoc {
    pub text: String,
    pub headings: Vec<String>,
    pub path: String,
    pub chunk_index: u32,
    pub start_byte: u64,
    pub end_byte: u64,
    pub modality: String,
    pub extractor: String,
}

pub fn extract_text(
    rel_path: &str,
    bytes: &[u8],
    cfg: &ChunkConfig,
) -> Result<Vec<CanonicalDoc>, VaneCliError> {
    if bytes.len() > TEXT_MAX_BYTES {
        return Err(VaneCliError::skip("too large"));
    }
    let src = std::str::from_utf8(bytes)
        .map_err(|_| VaneCliError::new(format!("invalid UTF-8 in {rel_path}")))?;
    let cfg = html_forces_plain(rel_path, cfg);
    let chunks = chunk_text(src, cfg.as_ref())?;
    Ok(chunks
        .into_iter()
        .map(|c| CanonicalDoc {
            text: c.text,
            headings: c.headings,
            path: posix_path(rel_path),
            chunk_index: c.chunk_index,
            start_byte: c.start_byte as u64,
            end_byte: c.end_byte as u64,
            modality: "text".into(),
            extractor: "text".into(),
        })
        .collect())
}

pub fn extract_image(rel_path: &str, bytes: &[u8]) -> Result<Vec<CanonicalDoc>, VaneCliError> {
    if bytes.len() > IMAGE_MAX_BYTES {
        return Err(VaneCliError::skip("too large"));
    }
    Ok(vec![CanonicalDoc {
        text: image_text(rel_path),
        headings: Vec::new(),
        path: posix_path(rel_path),
        chunk_index: 0,
        start_byte: 0,
        end_byte: bytes.len() as u64,
        modality: "image".into(),
        extractor: "image".into(),
    }])
}

fn html_forces_plain<'a>(
    rel_path: &str,
    cfg: &'a ChunkConfig,
) -> std::borrow::Cow<'a, ChunkConfig> {
    let lower = rel_path.rsplit('/').next().unwrap_or(rel_path);
    let is_html = lower.ends_with(".html") || lower.ends_with(".htm");
    if is_html && cfg.split != "plain" {
        let mut plain = cfg.clone();
        plain.split = "plain".into();
        std::borrow::Cow::Owned(plain)
    } else {
        std::borrow::Cow::Borrowed(cfg)
    }
}

fn image_text(rel_path: &str) -> String {
    let name = rel_path.rsplit('/').next().unwrap_or(rel_path);
    match name.rfind('.') {
        Some(i) if i > 0 => name[..i].to_string(),
        _ => rel_path.to_string(),
    }
}

fn posix_path(rel_path: &str) -> String {
    rel_path.replace('\\', "/")
}
