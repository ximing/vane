// tests/userdict_reindex.rs — 06-userdict-reindex 集成测试（Task 3/5/6）
//
// 验证 SPEC §7.4 词表状态机端到端：reindex 从原文重新分词重建倒排、
// 段 ULID 全替换、旧段删除、manifest 原子切换、reopen 后新身份生效。

use std::sync::Arc;

use vane_core::api::{
    CollectionOptions, Db, Doc, FusionSpec, OpenOptions, SearchMode, SearchQuery,
};
use vane_core::tokenizer::UserDictEntry;
use vane_core::types::{FieldDef, Metric, Schema};
use vane_core::vfs::memory::MemoryVfs;

fn build_schema() -> Schema {
    Schema::new(vec![
        ("body".into(), FieldDef::Text),
        (
            "v".into(),
            FieldDef::Vector {
                dim: 4,
                metric: Metric::Cosine,
            },
        ),
    ])
    .unwrap()
}

fn doc(id: &str, text: &str) -> Doc {
    Doc {
        id: id.into(),
        text: Some(text.into()),
        vector: Some(vec![1.0, 0.0, 0.0, 0.0]),
        meta: None,
    }
}

fn setup_col_with_docs(db: &Db) -> vane_core::api::Collection {
    let col = db
        .collection("c", build_schema(), CollectionOptions::default())
        .unwrap();
    col.add(&[doc("d0", "机器学习 深度学习"), doc("d1", "hello world")])
        .unwrap();
    col.flush().unwrap();
    col
}

fn text_query(t: &str) -> SearchQuery {
    SearchQuery {
        text: Some(t.into()),
        vector: None,
        top_k: 10,
        mode: SearchMode::Text,
        fusion: FusionSpec::Rrf,
        filter: None,
        candidate_multiplier: 3,
    }
}

// Task 3: reindex 重建倒排（新分词身份）——管线不崩 + 身份切换。
// 注：standard 分词器不消费 user_dict 做切分（仅影响 TokenizerId），
// 故 reindex 后 tokenization 不变，但 tokenizer_id 已变（身份切换验证）。
// jieba 场景的切分改善验证留 10-ci-m1 的 jieba feature job。

#[test]
fn reindex_rebuilds_inverted_with_new_tokenizer() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn vane_core::vfs::Vfs>;
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col = setup_col_with_docs(&db);
    let old_id = col.tokenizer_id();

    // reindex 前：可搜
    let hits_before = col.search(&text_query("hello")).unwrap();
    assert!(!hits_before.is_empty());

    // setUserDict 注入新词
    col.set_user_dict(&[UserDictEntry::Word("机器学习".into())])
        .unwrap();
    let handle = col.reindex().unwrap();
    handle.wait().unwrap();

    // reindex 后：仍可搜（管线不崩）
    let hits_after = col.search(&text_query("hello")).unwrap();
    assert!(
        !hits_after.is_empty(),
        "search must still work after reindex"
    );

    // tokenizer_id 已变（新身份）
    assert_ne!(
        col.tokenizer_id(),
        old_id,
        "tokenizer_id must change after reindex"
    );
}

// Task 5: reindex 完成原子切换 + manifest 持久化

#[test]
fn reindex_atomic_switch_new_identity_active() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn vane_core::vfs::Vfs>;
    let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
    let col = setup_col_with_docs(&db);
    let old_ulids = col.segment_ulids();

    col.set_user_dict(&[UserDictEntry::Word("新词".into())])
        .unwrap();
    let handle = col.reindex().unwrap();
    handle.wait().unwrap();

    // 新段 ULID 全替换
    let new_ulids = col.segment_ulids();
    assert_ne!(old_ulids, new_ulids, "ULIDs must be replaced after reindex");
    // 旧段目录已删
    for ulid in &old_ulids {
        let seg_dir = format!("db/segments/seg_{}", ulid);
        let files = vfs.list(&seg_dir).unwrap_or_default();
        assert!(
            files.is_empty(),
            "old segment dir must be deleted: {} still has {:?}",
            seg_dir,
            files
        );
    }

    // reopen：manifest 持久化了新身份
    let db2 = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col2 = db2
        .collection(
            "c",
            build_schema(),
            CollectionOptions {
                tokenizer: vane_core::tokenizer::BuiltinTokenizer::Standard,
                user_dict: vec![UserDictEntry::Word("新词".into())],
                auto_commit: vane_core::persistence::AutoCommitConfig::default(),
            },
        )
        .unwrap();
    assert_eq!(col2.dict_state(), vane_core::api::DictState::Stable);
    assert_eq!(col2.tokenizer_id(), col.tokenizer_id());
    // reopen 后仍可搜
    let hits = col2.search(&text_query("hello")).unwrap();
    assert!(!hits.is_empty());
}

// Task 5: reindex 保留 tombstone（delete 后 reindex，已删文档不复活）

#[test]
fn reindex_preserves_tombstone() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn vane_core::vfs::Vfs>;
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col = setup_col_with_docs(&db);

    // 删除 d0
    col.delete(&["d0".into()]).unwrap();
    // reindex 前 d0 不可搜
    let hits_before = col.search(&text_query("机器学习")).unwrap();
    assert!(
        hits_before.iter().all(|h| h.id != "d0"),
        "d0 must be deleted before reindex"
    );

    col.set_user_dict(&[UserDictEntry::Word("机器学习".into())])
        .unwrap();
    let handle = col.reindex().unwrap();
    handle.wait().unwrap();

    // reindex 后 d0 仍不可搜（tombstone 保留）
    let hits_after = col.search(&text_query("机器学习")).unwrap();
    assert!(
        hits_after.iter().all(|h| h.id != "d0"),
        "d0 must remain deleted after reindex (tombstone preserved)"
    );
}

// Task 6: 多段 reindex（验证逐段重建）

#[test]
fn reindex_multi_segment() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn vane_core::vfs::Vfs>;
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col = db
        .collection("c", build_schema(), CollectionOptions::default())
        .unwrap();
    // 两个 flush 产生两个段
    col.add(&[doc("a", "alpha beta")]).unwrap();
    col.flush().unwrap();
    col.add(&[doc("b", "gamma delta")]).unwrap();
    col.flush().unwrap();
    assert_eq!(col.segment_count(), 2);

    let old_ulids = col.segment_ulids();
    col.set_user_dict(&[UserDictEntry::Word("新词".into())])
        .unwrap();
    let handle = col.reindex().unwrap();
    handle.wait().unwrap();

    let new_ulids = col.segment_ulids();
    assert_ne!(old_ulids, new_ulids);
    assert_eq!(col.segment_count(), 2, "segment count preserved");
    // 全库新身份
    let current_id = col.tokenizer_id();
    for reader in col.snapshot_readers() {
        assert_eq!(
            reader.meta().tokenizer_id,
            current_id,
            "I-4: all segments new identity"
        );
    }
    // 仍可搜
    let hits = col.search(&text_query("alpha")).unwrap();
    assert!(!hits.is_empty());
    assert_eq!(hits[0].id, "a");
}

// Task 6: reindex 后再 add 用新身份（I-4）

#[test]
fn add_after_reindex_uses_new_tokenizer() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn vane_core::vfs::Vfs>;
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col = setup_col_with_docs(&db);
    col.set_user_dict(&[UserDictEntry::Word("新词".into())])
        .unwrap();
    let handle = col.reindex().unwrap();
    handle.wait().unwrap();

    let new_id = col.tokenizer_id();
    // reindex 后 add + flush，新段应使用新身份
    col.add(&[doc("d2", "新词测试")]).unwrap();
    col.flush().unwrap();
    assert_eq!(
        col.tokenizer_id(),
        new_id,
        "identity unchanged after post-reindex add"
    );
    // 新段也用新身份
    for reader in col.snapshot_readers() {
        assert_eq!(
            reader.meta().tokenizer_id,
            new_id,
            "post-reindex segment must use new tokenizer_id (I-4)"
        );
    }
}
