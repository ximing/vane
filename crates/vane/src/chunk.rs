use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::config::ChunkConfig;
use crate::error::VaneCliError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub text: String,
    pub headings: Vec<String>,
    pub start_byte: usize,
    pub end_byte: usize,
    pub chunk_index: u32,
}

#[derive(Serialize)]
struct StrategyCanon<'a> {
    split: &'a str,
    max_chars: u32,
    overlap_chars: u32,
    min_chars: u32,
    extractor_ver: &'a str,
}

/// SHA-256 of the canonical strategy JSON, hex prefix of 16 chars.
pub fn chunk_strategy_id(cfg: &ChunkConfig, extractor_ver: &str) -> String {
    let canon = StrategyCanon {
        split: &cfg.split,
        max_chars: cfg.max_chars,
        overlap_chars: cfg.overlap_chars,
        min_chars: cfg.min_chars,
        extractor_ver,
    };
    let payload = serde_json::to_vec(&canon).unwrap_or_else(|_| Vec::new());
    let digest = Sha256::digest(&payload);
    hex16(&digest)
}

pub fn chunk_text(src: &str, cfg: &ChunkConfig) -> Result<Vec<Chunk>, VaneCliError> {
    validate_chunk_cfg(cfg)?;
    let sections = match cfg.split.as_str() {
        "plain" => vec![Section {
            start: 0,
            end: src.len(),
            headings: Vec::new(),
        }],
        "markdown" => markdown_sections(src),
        other => {
            return Err(VaneCliError::new(format!(
                "invalid chunk split {other:?}, expected markdown or plain"
            )));
        }
    };

    let mut pieces = Vec::new();
    for sec in sections {
        if sec.start >= sec.end {
            continue;
        }
        for (start, end) in split_span(src, sec.start, sec.end, cfg.max_chars) {
            if start < end {
                pieces.push(Piece {
                    start,
                    end,
                    headings: sec.headings.clone(),
                });
            }
        }
    }

    Ok(finalize(src, &pieces, cfg))
}

fn validate_chunk_cfg(cfg: &ChunkConfig) -> Result<(), VaneCliError> {
    if cfg.max_chars == 0 {
        return Err(VaneCliError::new(
            "invalid chunk: max_chars must be >= 1".to_string(),
        ));
    }
    if cfg.overlap_chars >= cfg.max_chars {
        return Err(VaneCliError::new(format!(
            "invalid chunk: overlap_chars ({}) must be < max_chars ({})",
            cfg.overlap_chars, cfg.max_chars
        )));
    }
    if cfg.min_chars > cfg.max_chars {
        return Err(VaneCliError::new(format!(
            "invalid chunk: min_chars ({}) must be <= max_chars ({})",
            cfg.min_chars, cfg.max_chars
        )));
    }
    match cfg.split.as_str() {
        "markdown" | "plain" => Ok(()),
        other => Err(VaneCliError::new(format!(
            "invalid chunk split {other:?}, expected markdown or plain"
        ))),
    }
}

struct Section {
    start: usize,
    end: usize,
    headings: Vec<String>,
}

struct Piece {
    start: usize,
    end: usize,
    headings: Vec<String>,
}

struct HeadingMark {
    start: usize,
    level: u8,
    title: String,
}

fn markdown_sections(src: &str) -> Vec<Section> {
    let marks = scan_headings(src);
    if marks.is_empty() {
        return vec![Section {
            start: 0,
            end: src.len(),
            headings: Vec::new(),
        }];
    }

    let mut sections = Vec::new();
    if marks[0].start > 0 {
        sections.push(Section {
            start: 0,
            end: marks[0].start,
            headings: Vec::new(),
        });
    }

    let mut stack: Vec<(u8, String)> = Vec::new();
    for (i, mark) in marks.iter().enumerate() {
        while stack.last().is_some_and(|(level, _)| *level >= mark.level) {
            stack.pop();
        }
        stack.push((mark.level, mark.title.clone()));
        let end = marks.get(i + 1).map(|next| next.start).unwrap_or(src.len());
        sections.push(Section {
            start: mark.start,
            end,
            headings: stack.iter().map(|(_, title)| title.clone()).collect(),
        });
    }
    sections
}

fn scan_headings(src: &str) -> Vec<HeadingMark> {
    let mut marks = Vec::new();
    let mut offset = 0;
    let mut prev: Option<(usize, &str)> = None;

    while offset < src.len() {
        let (line_start, next, content) = next_line(src, offset);
        if let Some(level) = setext_level(content) {
            if let Some((pstart, pcontent)) = prev {
                if !pcontent.trim().is_empty() && parse_atx(pcontent).is_none() {
                    marks.push(HeadingMark {
                        start: pstart,
                        level,
                        title: pcontent.trim().to_string(),
                    });
                    prev = None;
                    offset = next;
                    continue;
                }
            }
        }
        if let Some((level, title)) = parse_atx(content) {
            marks.push(HeadingMark {
                start: line_start,
                level,
                title,
            });
            prev = None;
            offset = next;
            continue;
        }
        prev = Some((line_start, content));
        offset = next;
    }
    marks
}

fn next_line(src: &str, from: usize) -> (usize, usize, &str) {
    let rest = &src[from..];
    match rest.find('\n') {
        Some(i) => {
            let content_end = if i > 0 && rest.as_bytes()[i - 1] == b'\r' {
                from + i - 1
            } else {
                from + i
            };
            (from, from + i + 1, &src[from..content_end])
        }
        None => (from, src.len(), rest),
    }
}

fn parse_atx(line: &str) -> Option<(u8, String)> {
    let indent = line.bytes().take_while(|&b| b == b' ').count();
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let hashes = rest.bytes().take_while(|&b| b == b'#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let after = &rest[hashes..];
    if !after.is_empty() && !after.starts_with(' ') && !after.starts_with('\t') {
        return None;
    }
    let trimmed = after.trim();
    let title = if trimmed.chars().all(|c| c == '#') {
        String::new()
    } else {
        trimmed.trim_end_matches('#').trim_end().to_string()
    };
    Some((hashes as u8, title))
}

fn setext_level(line: &str) -> Option<u8> {
    let indent = line.bytes().take_while(|&b| b == b' ').count();
    if indent > 3 {
        return None;
    }
    let marks = line[indent..].trim_end();
    if marks.is_empty() {
        return None;
    }
    if marks.bytes().all(|b| b == b'=') {
        Some(1)
    } else if marks.bytes().all(|b| b == b'-') {
        Some(2)
    } else {
        None
    }
}

fn split_span(src: &str, start: usize, end: usize, max_chars: u32) -> Vec<(usize, usize)> {
    let slice = &src[start..end];
    if char_count(slice) <= max_chars {
        return if start < end {
            vec![(start, end)]
        } else {
            Vec::new()
        };
    }
    let units = overflow_units(slice, max_chars);
    pack_units(&units, slice, max_chars)
        .into_iter()
        .map(|(a, b)| (start + a, start + b))
        .collect()
}

fn overflow_units(s: &str, max_chars: u32) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for (a, b) in split_blank_lines(s) {
        let piece = &s[a..b];
        if char_count(piece) <= max_chars {
            out.push((a, b));
            continue;
        }
        for (x, y) in split_newlines(piece) {
            let line = &piece[x..y];
            if char_count(line) <= max_chars {
                out.push((a + x, a + y));
                continue;
            }
            for (p, q) in hard_cut(line, max_chars) {
                out.push((a + x + p, a + x + q));
            }
        }
    }
    out
}

fn split_blank_lines(s: &str) -> Vec<(usize, usize)> {
    if s.is_empty() {
        return Vec::new();
    }
    let bytes = s.as_bytes();
    let mut units = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            let mut j = i + 1;
            while j < bytes.len() && matches!(bytes[j], b' ' | b'\t' | b'\r') {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'\n' {
                let end = j + 1;
                units.push((start, end));
                start = end;
                i = end;
                continue;
            }
        }
        i += 1;
    }
    if start < s.len() {
        units.push((start, s.len()));
    }
    units
}

fn split_newlines(s: &str) -> Vec<(usize, usize)> {
    if s.is_empty() {
        return Vec::new();
    }
    let mut units = Vec::new();
    let mut start = 0;
    for (i, c) in s.char_indices() {
        if c == '\n' {
            units.push((start, i + 1));
            start = i + 1;
        }
    }
    if start < s.len() {
        units.push((start, s.len()));
    }
    units
}

fn hard_cut(s: &str, max_chars: u32) -> Vec<(usize, usize)> {
    let max = max_chars as usize;
    if max == 0 || s.is_empty() {
        return Vec::new();
    }
    let mut units = Vec::new();
    let mut start = 0;
    let mut n = 0usize;
    for (i, _) in s.char_indices() {
        if n == max {
            units.push((start, i));
            start = i;
            n = 0;
        }
        n += 1;
    }
    if start < s.len() {
        units.push((start, s.len()));
    }
    units
}

fn pack_units(units: &[(usize, usize)], s: &str, max_chars: u32) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut cur: Option<(usize, usize, u32)> = None;
    for &(a, b) in units {
        let n = char_count(&s[a..b]);
        match cur {
            Some((st, _, ch)) if ch.saturating_add(n) <= max_chars => {
                cur = Some((st, b, ch + n));
            }
            Some((st, en, _)) => {
                out.push((st, en));
                cur = Some((a, b, n));
            }
            None => cur = Some((a, b, n)),
        }
    }
    if let Some((st, en, _)) = cur {
        out.push((st, en));
    }
    out
}

fn finalize(src: &str, pieces: &[Piece], cfg: &ChunkConfig) -> Vec<Chunk> {
    let keep_short = pieces.len() <= 1;
    let selected: Vec<&Piece> = pieces
        .iter()
        .filter(|p| keep_short || char_count(&src[p.start..p.end]) >= cfg.min_chars)
        .collect();

    let mut out = Vec::with_capacity(selected.len());
    let mut prev_body = String::new();
    for (idx, piece) in selected.iter().enumerate() {
        let unique = &src[piece.start..piece.end];
        let body = if idx == 0 || cfg.overlap_chars == 0 {
            unique.to_string()
        } else {
            let mut s = tail_chars(&prev_body, cfg.overlap_chars);
            s.push_str(unique);
            s
        };
        out.push(Chunk {
            text: prefix_breadcrumb(&piece.headings, &body),
            headings: piece.headings.clone(),
            start_byte: piece.start,
            end_byte: piece.end,
            chunk_index: idx as u32,
        });
        prev_body = body;
    }
    out
}

fn prefix_breadcrumb(headings: &[String], body: &str) -> String {
    if headings.is_empty() {
        return body.to_string();
    }
    let crumb = headings.join(" > ");
    let mut text = String::with_capacity(crumb.len() + 1 + body.len());
    text.push_str(&crumb);
    text.push('\n');
    text.push_str(body);
    text
}

fn tail_chars(s: &str, n: u32) -> String {
    let n = n as usize;
    let count = s.chars().count();
    if count <= n {
        s.to_string()
    } else {
        s.chars().skip(count - n).collect()
    }
}

fn char_count(s: &str) -> u32 {
    s.chars().count() as u32
}

fn hex16(digest: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(16);
    for &b in digest.iter().take(8) {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}
