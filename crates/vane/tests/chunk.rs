use vane::chunk::{chunk_strategy_id, chunk_text};
use vane::config::ChunkConfig;

fn cfg(split: &str, max_chars: u32, overlap_chars: u32, min_chars: u32) -> ChunkConfig {
    ChunkConfig {
        split: split.into(),
        max_chars,
        overlap_chars,
        min_chars,
    }
}

#[test]
fn heading_split_and_breadcrumb() {
    let src = "# API\n\nintro text here\n\n## 鉴权\n\nsecret token flow\n";
    let chunks = chunk_text(src, &cfg("markdown", 1200, 200, 1)).unwrap();
    assert_eq!(chunks.len(), 2);

    assert_eq!(chunks[0].headings, vec!["API"]);
    assert!(
        chunks[0].text.starts_with("API\n"),
        "first chunk must start with breadcrumb: {}",
        chunks[0].text
    );
    assert!(chunks[0].text.contains("intro text here"));
    assert_eq!(chunks[0].chunk_index, 0);
    assert_eq!(
        &src[chunks[0].start_byte..chunks[0].end_byte],
        &src[0..chunks[1].start_byte]
    );

    assert_eq!(chunks[1].headings, vec!["API", "鉴权"]);
    assert!(
        chunks[1].text.starts_with("API > 鉴权\n"),
        "nested breadcrumb: {}",
        chunks[1].text
    );
    assert!(chunks[1].text.contains("secret token flow"));
    assert_eq!(chunks[1].chunk_index, 1);
    assert!(src[chunks[1].start_byte..chunks[1].end_byte].contains("secret token flow"));
}

#[test]
fn setext_headings_are_recognized() {
    let src = "API\n===\n\nintro\n\n鉴权\n---\n\nsecret\n";
    let chunks = chunk_text(src, &cfg("markdown", 1200, 200, 1)).unwrap();
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].headings, vec!["API"]);
    assert!(chunks[0].text.starts_with("API\n"));
    assert_eq!(chunks[1].headings, vec!["API", "鉴权"]);
    assert!(chunks[1].text.starts_with("API > 鉴权\n"));
}

#[test]
fn overflow_splits_on_blank_lines() {
    let p1 = "A".repeat(80);
    let p2 = "B".repeat(80);
    let src = format!("{p1}\n\n{p2}");
    let chunks = chunk_text(&src, &cfg("plain", 100, 10, 1)).unwrap();
    assert_eq!(
        chunks.len(),
        2,
        "two paragraphs over max should become two chunks"
    );
    assert!(chunks[0].text.contains(&p1));
    assert!(!chunks[0].text.contains(&p2));
    assert!(chunks[1].text.contains(&p2));
    assert_eq!(chunks[0].chunk_index, 0);
    assert_eq!(chunks[1].chunk_index, 1);

    let body0 = &src[chunks[0].start_byte..chunks[0].end_byte];
    let n = body0.chars().count();
    let overlap: String = body0.chars().skip(n.saturating_sub(10)).collect();
    assert!(
        chunks[1].text.starts_with(&overlap),
        "second chunk body should start with previous tail: {}",
        chunks[1].text
    );
    assert_eq!(src[chunks[0].start_byte..chunks[0].end_byte].trim_end(), p1);
    assert_eq!(&src[chunks[1].start_byte..chunks[1].end_byte], p2.as_str());
}

#[test]
fn overlap_keeps_heading_breadcrumb() {
    let p1 = "甲".repeat(80);
    let p2 = "乙".repeat(80);
    let src = format!("# API\n\n{p1}\n\n{p2}\n");
    let chunks = chunk_text(&src, &cfg("markdown", 100, 10, 1)).unwrap();
    assert!(
        chunks.len() >= 2,
        "long heading section must overflow-split"
    );
    for c in &chunks {
        assert_eq!(c.headings, vec!["API"]);
        assert!(
            c.text.starts_with("API\n"),
            "overlap must not drop breadcrumb: {}",
            c.text
        );
    }
    let body0 = chunks[0].text.strip_prefix("API\n").expect("breadcrumb");
    let body1 = chunks[1].text.strip_prefix("API\n").expect("breadcrumb");
    let n = body0.chars().count();
    let overlap: String = body0.chars().skip(n.saturating_sub(10)).collect();
    assert!(
        body1.starts_with(&overlap),
        "second body should start with previous body tail, not breadcrumb"
    );
}

#[test]
fn short_file_kept() {
    let src = "hi";
    let chunks = chunk_text(src, &cfg("plain", 1200, 200, 50)).unwrap();
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].text, "hi");
    assert!(chunks[0].headings.is_empty());
    assert_eq!(chunks[0].start_byte, 0);
    assert_eq!(chunks[0].end_byte, src.len());
    assert_eq!(chunks[0].chunk_index, 0);
}

#[test]
fn invalid_overlap_rejected() {
    let err = chunk_text("hello", &cfg("plain", 100, 100, 10)).unwrap_err();
    assert!(
        err.message.contains("overlap"),
        "expected overlap error, got {}",
        err.message
    );
}

#[test]
fn chunk_strategy_id_changes_with_max_chars() {
    let a = chunk_strategy_id(&cfg("markdown", 1200, 200, 50), "1");
    let b = chunk_strategy_id(&cfg("markdown", 800, 200, 50), "1");
    assert_ne!(a, b);
    assert_eq!(a.len(), 16);
    assert_eq!(b.len(), 16);
    assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
}
