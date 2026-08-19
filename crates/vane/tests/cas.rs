use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};
use vane::cas::{embed_key, extract_key, Cas};
use vane::chunk::chunk_strategy_id;
use vane::config::ChunkConfig;
use vane::extract::{extract_image, extract_text};

struct TempHome {
    path: PathBuf,
}

fn tempfile_dir() -> TempHome {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "vane-cas-test-{}-{}-{}",
        std::process::id(),
        nanos,
        n
    ));
    fs::create_dir_all(&path).unwrap();
    // Isolation: never touch the user's real ~/.vane.
    std::env::set_var("VANE_HOME", &path);
    TempHome { path }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let tmp = std::env::temp_dir();
        if self.path.starts_with(&tmp) && self.path != tmp {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

impl std::ops::Deref for TempHome {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.path
    }
}

fn md_cfg(max_chars: u32) -> ChunkConfig {
    ChunkConfig {
        split: "markdown".into(),
        max_chars,
        overlap_chars: 20,
        min_chars: 1,
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for &b in digest.iter() {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

#[test]
fn extract_text_roundtrip_docs() {
    let _tmp = tempfile_dir();
    let src = "# API\n\nhello world\n";
    let docs = extract_text("docs/auth.md", src.as_bytes(), &md_cfg(1200)).unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].path, "docs/auth.md");
    assert_eq!(docs[0].modality, "text");
    assert_eq!(docs[0].extractor, "text");
    assert_eq!(docs[0].chunk_index, 0);
    assert_eq!(docs[0].headings, vec!["API"]);
    assert!(docs[0].text.starts_with("API\n"));
    assert!(docs[0].text.contains("hello world"));
    assert_eq!(docs[0].start_byte, 0);
    assert_eq!(docs[0].end_byte, src.len() as u64);
}

#[test]
fn extract_cas_hit_same_bytes_and_strategy() {
    let tmp = tempfile_dir();
    let cas = Cas::new(&*tmp);
    let src = b"# API\n\nhello world\n";
    let cfg = md_cfg(1200);
    let docs = extract_text("docs/auth.md", src, &cfg).unwrap();
    let sid = chunk_strategy_id(&cfg, "1");
    let key = extract_key(&sha256_hex(src), "text", "1", &sid);
    cas.put_extract(&key, &docs).unwrap();

    let again = extract_key(&sha256_hex(src), "text", "1", &sid);
    assert_eq!(key, again);
    let hit = cas.get_extract(&key).expect("same bytes+strategy must hit");
    assert_eq!(hit, docs);
}

#[test]
fn extract_cas_miss_when_strategy_changes() {
    let tmp = tempfile_dir();
    let cas = Cas::new(&*tmp);
    let src = b"# API\n\nhello world\n";
    let cfg = md_cfg(1200);
    let docs = extract_text("docs/auth.md", src, &cfg).unwrap();
    let key = extract_key(&sha256_hex(src), "text", "1", &chunk_strategy_id(&cfg, "1"));
    cas.put_extract(&key, &docs).unwrap();

    let other = extract_key(
        &sha256_hex(src),
        "text",
        "1",
        &chunk_strategy_id(&md_cfg(80), "1"),
    );
    assert_ne!(key, other);
    assert!(
        cas.get_extract(&other).is_none(),
        "changing chunk strategy must miss extract CAS"
    );
}

#[test]
fn embed_key_changes_with_model_id() {
    let tmp = tempfile_dir();
    let cas = Cas::new(&*tmp);
    let text = "API\nhello world";
    let a = embed_key(text, "ollama:nomic-embed-text:768");
    let b = embed_key(text, "openai_compat:text-embedding-3-small:1536");
    assert_ne!(a, b);
    assert_eq!(a, embed_key(text, "ollama:nomic-embed-text:768"));

    cas.put_embed(&a, &[0.1, 0.2, 0.3]).unwrap();
    assert_eq!(cas.get_embed(&a).unwrap(), vec![0.1, 0.2, 0.3]);
    assert!(cas.get_embed(&b).is_none());
}

#[test]
fn image_extractor_uses_filename_and_image_modality() {
    let _tmp = tempfile_dir();
    let docs = extract_image("docs/photo.png", b"\x89PNG").unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].text, "photo");
    assert_eq!(docs[0].modality, "image");
    assert_eq!(docs[0].extractor, "image");
    assert_eq!(docs[0].path, "docs/photo.png");
    assert_eq!(docs[0].chunk_index, 0);
    assert!(docs[0].headings.is_empty());

    let no_ext = extract_image("docs/photo", b"raw").unwrap();
    assert_eq!(no_ext[0].text, "docs/photo");
    assert_eq!(no_ext[0].modality, "image");
}

#[test]
fn html_extract_forces_plain_split() {
    let _tmp = tempfile_dir();
    let src = "# Looks like heading\n\nbody text here";
    let docs = extract_text("page.html", src.as_bytes(), &md_cfg(1200)).unwrap();
    assert_eq!(docs.len(), 1);
    assert!(docs[0].headings.is_empty());
    assert_eq!(docs[0].text, src);
}

#[test]
fn invalid_utf8_is_error() {
    let _tmp = tempfile_dir();
    let err = extract_text("a.md", &[0xff, 0xfe, 0x00], &md_cfg(1200)).unwrap_err();
    assert!(
        err.message.to_ascii_lowercase().contains("utf-8")
            || err.message.to_ascii_lowercase().contains("utf8"),
        "expected utf-8 error, got {}",
        err.message
    );
}

#[test]
fn oversized_text_is_skip() {
    let _tmp = tempfile_dir();
    let bytes = vec![b'a'; 8 * 1024 * 1024 + 1];
    let err = extract_text("big.md", &bytes, &md_cfg(1200)).unwrap_err();
    assert!(err.is_skip(), "too-large text must be a skip error");
    assert!(
        err.message.to_ascii_lowercase().contains("too large")
            || err.message.to_ascii_lowercase().contains("8"),
        "expected too-large message, got {}",
        err.message
    );
}

#[test]
fn touch_refreshes_last_seen() {
    let tmp = tempfile_dir();
    let cas = Cas::new(&*tmp);
    let src = b"hello";
    let cfg = md_cfg(1200);
    let docs = extract_text("a.md", src, &cfg).unwrap();
    let ek = extract_key(&sha256_hex(src), "text", "1", &chunk_strategy_id(&cfg, "1"));
    cas.put_extract(&ek, &docs).unwrap();
    let vk = embed_key(&docs[0].text, "ollama:nomic-embed-text:4");
    cas.put_embed(&vk, &[1.0, 0.0, 0.0, 0.0]).unwrap();

    assert!(cas.last_seen(&ek).is_none());
    cas.touch(&ek, std::slice::from_ref(&vk), 1_700_000_123);
    assert_eq!(cas.last_seen(&ek), Some(1_700_000_123));
    assert_eq!(cas.last_seen(&vk), Some(1_700_000_123));
}
