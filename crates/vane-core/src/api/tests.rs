use super::types::*;
use crate::tokenizer::BuiltinTokenizer;

#[test]
fn open_options_default() {
    let o = OpenOptions::default();
    assert_eq!(o.page_cache_mb, 32);
    assert!(matches!(
        o.auto_commit,
        crate::persistence::AutoCommitConfig::On { .. }
    ));
}

#[test]
fn search_query_default() {
    let q = SearchQuery::default();
    assert_eq!(q.top_k, 10);
    assert!(matches!(q.mode, SearchMode::Auto));
    assert!(matches!(q.fusion, FusionSpec::Rrf));
    assert_eq!(q.candidate_multiplier, 3);
    assert!(q.filter.is_none());
}

#[test]
fn collection_options_default_tokenizer_standard() {
    let o = CollectionOptions::default();
    assert!(matches!(o.tokenizer, BuiltinTokenizer::Standard));
}
