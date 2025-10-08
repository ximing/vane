use super::*;
use crate::tokenizer::{BuiltinTokenizer, UserDictEntry};
use crate::types::{FieldDef, Metric, Schema, TokenizerId};

#[test]
fn manifest_empty_serialize_roundtrip() {
    let m = Manifest::empty();
    let json = serde_json::to_string(&m).unwrap();
    let back: Manifest = serde_json::from_str(&json).unwrap();
    assert_eq!(back.version, 1);
    assert!(back.collections.is_empty());
}

#[test]
fn manifest_with_collection_roundtrip() {
    let schema = Schema::new(vec![
        ("body".into(), FieldDef::Text),
        (
            "v".into(),
            FieldDef::Vector {
                dim: 384,
                metric: Metric::Cosine,
            },
        ),
    ])
    .unwrap();
    let mut m = Manifest::empty();
    m.collections.insert(
        "docs".into(),
        CollectionMeta {
            schema,
            tokenizer_kind: BuiltinTokenizer::Standard,
            tokenizer_id: TokenizerId([0xab; 32]),
            user_dict: vec![UserDictEntry::Word("test".into())],
            segment_ulids: vec!["01HZX...".into()],
        },
    );
    let json = serde_json::to_string(&m).unwrap();
    let back: Manifest = serde_json::from_str(&json).unwrap();
    assert_eq!(back.collections.len(), 1);
    let col = &back.collections["docs"];
    assert_eq!(col.tokenizer_kind, BuiltinTokenizer::Standard);
    assert_eq!(col.segment_ulids, vec!["01HZX...".to_string()]);
}
