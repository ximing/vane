use vane::classify::{classify, should_watch_dir, SkipReason};
use vane::config::{
    default_exclude, default_types, ChunkConfig, EmbedConfig, ResolvedPolicy, TypeRule,
};

fn policy(exclude: &[&str], types: &[(&str, &str, bool)]) -> ResolvedPolicy {
    ResolvedPolicy {
        embed: EmbedConfig {
            provider: "ollama".into(),
            model: "nomic-embed-text".into(),
            base_url: "http://127.0.0.1:11434".into(),
            api_key: None,
        },
        chunk: ChunkConfig {
            split: "markdown".into(),
            max_chars: 1200,
            overlap_chars: 200,
            min_chars: 50,
        },
        exclude: exclude.iter().map(|s| (*s).to_string()).collect(),
        types: types
            .iter()
            .map(|(glob, extractor, enabled)| TypeRule {
                glob: (*glob).into(),
                extractor: (*extractor).into(),
                enabled: *enabled,
            })
            .collect(),
    }
}

#[test]
fn classify_exclude_wins() {
    let pol = policy(
        &["**/*.log"],
        &[
            ("**/*.{md,txt}", "text", true),
            ("**/*.png", "image", false),
        ],
    );
    assert_eq!(classify("a.log", &pol), Err(SkipReason::Excluded));
    let rule = classify("a.md", &pol).expect("markdown is text");
    assert_eq!(rule.extractor, "text");
    assert_eq!(classify("a.png", &pol), Err(SkipReason::Disabled));
    assert_eq!(classify("a.rs", &pol), Err(SkipReason::NoType));
}

#[test]
fn should_watch_dir_skips_excluded_trees() {
    let pol = policy(
        &["**/node_modules/**", "**/target/**", "secret/**"],
        &[("**/*.md", "text", true)],
    );
    assert!(!should_watch_dir("node_modules", &pol));
    assert!(!should_watch_dir("app/node_modules", &pol));
    assert!(!should_watch_dir("src/target", &pol));
    assert!(!should_watch_dir("secret", &pol));
    assert!(should_watch_dir("docs", &pol));
    assert!(should_watch_dir("app", &pol));
}

#[test]
fn default_exclude_and_types_keep_markdown() {
    let pol = ResolvedPolicy {
        embed: EmbedConfig {
            provider: "ollama".into(),
            model: "nomic-embed-text".into(),
            base_url: "http://127.0.0.1:11434".into(),
            api_key: None,
        },
        chunk: ChunkConfig {
            split: "markdown".into(),
            max_chars: 1200,
            overlap_chars: 200,
            min_chars: 50,
        },
        exclude: default_exclude(),
        types: default_types(),
    };
    assert!(should_watch_dir("docs", &pol), "docs/ must be watched");
    let rule = classify("docs/auth.md", &pol).expect("docs/auth.md is text");
    assert_eq!(rule.extractor, "text");
}

#[test]
fn reserved_extractor_is_skipped() {
    let pol = policy(
        &[],
        &[("**/*.pdf", "pdf", true), ("**/*.docx", "docx", true)],
    );
    assert_eq!(classify("spec.pdf", &pol), Err(SkipReason::Disabled));
    assert_eq!(classify("notes.docx", &pol), Err(SkipReason::Disabled));
}
