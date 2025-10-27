// 06-userdict-reindex 单元测试（Task 1/2/4/6）
//
// 验证 SPEC §7.4 词表状态机：Stable→setUserDict→PendingReindex→reindex→
// Rebuilding→Stable；I-4 单一分词身份；Q-6 Rebuilding 期写路径 E_BUSY。

use crate::api::db::Db;
use crate::api::types::*;
use crate::tokenizer::UserDictEntry;
use crate::types::{FieldDef, Metric, Schema, VaneError};
use crate::vfs::memory::MemoryVfs;
use std::sync::Arc;

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

fn setup_col(db: &Db) -> crate::api::Collection {
    db.collection("c", build_schema(), CollectionOptions::default())
        .unwrap()
}

fn setup_col_with_docs(db: &Db) -> crate::api::Collection {
    let col = setup_col(db);
    col.add(&[
        Doc {
            id: "d0".into(),
            text: Some("机器学习 深度学习".into()),
            vector: Some(vec![1.0, 0.0, 0.0, 0.0]),
            meta: None,
        },
        Doc {
            id: "d1".into(),
            text: Some("hello world".into()),
            vector: Some(vec![0.0, 1.0, 0.0, 0.0]),
            meta: None,
        },
    ])
    .unwrap();
    col.flush().unwrap();
    col
}

// ---- Task 1: DictState + set_user_dict ----

#[test]
fn set_user_dict_enters_pending_reindex() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn crate::vfs::Vfs>;
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col = setup_col(&db);
    assert_eq!(col.dict_state(), DictState::Stable);
    col.set_user_dict(&[UserDictEntry::Word("新词".into())])
        .unwrap();
    assert_eq!(col.dict_state(), DictState::PendingReindex);
}

#[test]
fn pending_reindex_new_writes_use_old_tokenizer() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn crate::vfs::Vfs>;
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col = setup_col_with_docs(&db);
    let old_id = col.tokenizer_id();
    col.set_user_dict(&[UserDictEntry::Word("新词".into())])
        .unwrap();
    // PendingReindex 期 add 仍用旧身份
    col.add(&[Doc {
        id: "d2".into(),
        text: Some("新词".into()),
        vector: None,
        meta: None,
    }])
    .unwrap();
    assert_eq!(
        col.tokenizer_id(),
        old_id,
        "new writes must use old tokenizer (I-4)"
    );
}

#[test]
fn set_user_dict_overwrites_pending() {
    // 多次 setUserDict 覆盖暂存词表（SPEC §7.4「放弃」路径）。
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn crate::vfs::Vfs>;
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col = setup_col(&db);
    col.set_user_dict(&[UserDictEntry::Word("词A".into())])
        .unwrap();
    assert_eq!(col.dict_state(), DictState::PendingReindex);
    // 再次调用覆盖
    col.set_user_dict(&[UserDictEntry::Word("词B".into())])
        .unwrap();
    assert_eq!(col.dict_state(), DictState::PendingReindex);
}

#[test]
fn set_user_dict_rejects_dict_too_large() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn crate::vfs::Vfs>;
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col = setup_col(&db);
    let dict: Vec<UserDictEntry> = (0..=100_000)
        .map(|i| UserDictEntry::Word(format!("w{}", i)))
        .collect();
    let r = col.set_user_dict(&dict);
    assert!(matches!(r, Err(VaneError::DictTooLarge)));
}

// ---- Task 2: reindex 签名变更 + ReindexHandle ----

#[test]
fn reindex_returns_handle_and_progresses() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn crate::vfs::Vfs>;
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col = setup_col_with_docs(&db);
    col.set_user_dict(&[UserDictEntry::Word("新词".into())])
        .unwrap();
    let handle = col.reindex().unwrap();
    let p0 = handle.progress();
    assert!((0.0..=1.0).contains(&p0));
    handle.wait().unwrap();
    assert_eq!(col.dict_state(), DictState::Stable);
    // M1 同步执行：progress 完成后 1.0
    assert!((handle.progress() - 1.0).abs() < 1e-6);
}

#[test]
fn reindex_on_stable_returns_invalid_arg() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn crate::vfs::Vfs>;
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col = setup_col_with_docs(&db);
    // Stable 状态无待重建词表
    let r = col.reindex();
    assert!(matches!(r, Err(VaneError::InvalidArg(_))));
}

// ---- Task 4: Rebuilding 期 E_BUSY ----

#[test]
fn rebuilding_writes_rejected_with_busy() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn crate::vfs::Vfs>;
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col = setup_col_with_docs(&db);
    // 模拟 Rebuilding（M1 同步执行，Rebuilding 窗口短；手动注入测试）
    col.set_state_for_test(DictState::Rebuilding);
    let r = col.add(&[Doc {
        id: "x".into(),
        text: None,
        vector: None,
        meta: None,
    }]);
    assert!(
        matches!(r, Err(VaneError::Busy)),
        "add during Rebuilding must E_BUSY"
    );
    // flush 也被拒
    let r2 = col.flush();
    assert!(matches!(r2, Err(VaneError::Busy)));
    // delete 也被拒
    let r3 = col.delete(&["x".into()]);
    assert!(matches!(r3, Err(VaneError::Busy)));
    // compact 也被拒
    let r4 = col.compact();
    assert!(matches!(r4, Err(VaneError::Busy)));
    // 查询仍可用（旧段只读）
    col.set_state_for_test(DictState::Stable); // 恢复以便查询
    let hits = col.search(&SearchQuery {
        text: Some("hello".into()),
        vector: None,
        top_k: 10,
        mode: SearchMode::Text,
        fusion: FusionSpec::Rrf,
        filter: None,
        candidate_multiplier: 3,
    });
    assert!(hits.is_ok());
}

#[test]
fn set_user_dict_during_rebuilding_returns_busy() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn crate::vfs::Vfs>;
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col = setup_col_with_docs(&db);
    col.set_state_for_test(DictState::Rebuilding);
    let r = col.set_user_dict(&[UserDictEntry::Word("x".into())]);
    assert!(matches!(r, Err(VaneError::Busy)));
}

// ---- Task 6: I-4 单一分词身份 ----

#[test]
fn single_tokenizer_identity_throughout_reindex() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn crate::vfs::Vfs>;
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col = setup_col_with_docs(&db);
    let old_id = col.tokenizer_id();
    col.set_user_dict(&[UserDictEntry::Word("新词".into())])
        .unwrap();
    // PendingReindex：旧身份
    assert_eq!(col.tokenizer_id(), old_id);
    let handle = col.reindex().unwrap();
    handle.wait().unwrap();
    // 完成后新身份
    assert_ne!(col.tokenizer_id(), old_id);
    // 全库只剩新身份段（I-4：段头 tokenizer_id 与 CollectionInner 一致）
    let current_id = col.tokenizer_id();
    for reader in col.snapshot_readers() {
        assert_eq!(
            reader.meta().tokenizer_id,
            current_id,
            "all segments must have the new tokenizer_id (I-4)"
        );
    }
}
