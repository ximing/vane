// 同一 db 路径同时只能有一个 Db（含它发出的 Collection）。
// 第二个 open 必须是 E_BUSY，且不能改已有文件；句柄全部 drop 后才能再打开。

use std::path::PathBuf;
use std::sync::Arc;

use vane_core::api::{
    CollectionOptions, Db, Doc, FusionSpec, OpenOptions, SearchMode, SearchQuery,
};
use vane_core::types::{FieldDef, Metric, Schema, VaneError};
use vane_core::vfs::memory::MemoryVfs;
use vane_core::vfs::std_fs::StdFsVfs;
use vane_core::vfs::Vfs;

fn schema() -> Schema {
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

fn text_query() -> SearchQuery {
    SearchQuery {
        text: Some("hello".into()),
        vector: None,
        top_k: 10,
        mode: SearchMode::Text,
        fusion: FusionSpec::Rrf,
        filter: None,
        candidate_multiplier: 3,
    }
}

fn assert_open_busy(opened: Result<Db, VaneError>) {
    match opened {
        Err(VaneError::Busy(ctx)) => {
            assert_eq!(ctx.op, Some("open"));
            let hint = ctx.hint.unwrap_or_default();
            assert!(
                hint.contains("进程"),
                "hint should say to close the other process, got {hint}"
            );
        }
        Err(other) => panic!("expected E_BUSY, got {other}"),
        Ok(_) => panic!("second open must be E_BUSY, got Ok"),
    }
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "vane-db-lock-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn stdfs_second_open_busy_until_db_dropped() {
    let dir = TempDir::new("stdfs");
    let vfs = Arc::new(StdFsVfs::with_root(dir.0.to_str().unwrap())) as Arc<dyn Vfs>;
    let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
    let col = db
        .collection("c", schema(), CollectionOptions::default())
        .unwrap();
    col.add(&[Doc {
        id: "d0".into(),
        text: Some("hello lock".into()),
        vector: Some(vec![1.0, 0.0, 0.0, 0.0]),
        meta: None,
    }])
    .unwrap();
    col.flush().unwrap();

    assert_open_busy(Db::open(vfs.clone(), "db", OpenOptions::default()));

    let hits = col.search(&text_query()).unwrap();
    assert!(
        hits.iter().any(|h| h.id == "d0"),
        "holder must still search the flushed doc"
    );

    drop(col);
    drop(db);

    let lock_file = dir.0.join("db").join(".vane.lock");
    assert!(
        lock_file.is_file(),
        ".vane.lock must exist and must not make the db look corrupt"
    );
    let db3 = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col3 = db3
        .collection("c", schema(), CollectionOptions::default())
        .unwrap();
    let hits = col3.search(&text_query()).unwrap();
    assert!(
        hits.iter().any(|h| h.id == "d0"),
        "reopen after unlock must keep the doc, not treat .vane.lock as corruption"
    );
}

#[test]
fn stdfs_collection_holds_lock_after_db_drop() {
    let dir = TempDir::new("col");
    let vfs = Arc::new(StdFsVfs::with_root(dir.0.to_str().unwrap())) as Arc<dyn Vfs>;
    let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
    let col = db
        .collection("c", schema(), CollectionOptions::default())
        .unwrap();
    drop(db);

    assert_open_busy(Db::open(vfs.clone(), "db", OpenOptions::default()));

    drop(col);
    Db::open(vfs, "db", OpenOptions::default()).unwrap();
}

#[test]
fn memory_second_open_busy_until_db_dropped() {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();

    // 同一实例的其它路径不受影响。
    let other_path = Db::open(vfs.clone(), "other", OpenOptions::default()).unwrap();
    drop(other_path);

    // 另一个 MemoryVfs 实例不共享这把锁。
    let other_vfs = Arc::new(MemoryVfs::new());
    let other_db = Db::open(other_vfs, "db", OpenOptions::default()).unwrap();
    drop(other_db);

    assert_open_busy(Db::open(vfs.clone(), "db", OpenOptions::default()));

    drop(db);
    Db::open(vfs, "db", OpenOptions::default()).unwrap();
}
