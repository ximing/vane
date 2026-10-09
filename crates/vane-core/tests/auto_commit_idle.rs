//! close 刷盘与空闲 auto-commit（SPEC §7.1）。
//!
//! `add()` 返回后数据仍不可搜。两条落盘路径：
//! - `Db::close()` 刷掉每个 collection 的 buffer，与 auto-commit 开关无关；
//! - `interval_ms` 到点后，即使没有下一次 `add()`、也没有显式 `flush()`，native 后台线程提交。
//!
//! `Drop` 不刷盘。未 close、且 interval 未到就丢掉句柄，等于进程被杀掉，buffer 丢失。

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use vane_core::api::{
    Collection, CollectionOptions, Db, Doc, FusionSpec, OpenOptions, SearchMode, SearchQuery,
};
use vane_core::persistence::AutoCommitConfig;
use vane_core::types::{FieldDef, Metric, Schema};
use vane_core::vfs::memory::MemoryVfs;
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

fn doc(id: &str) -> Doc {
    Doc {
        id: id.into(),
        text: Some("hello world".into()),
        vector: Some(vec![1.0, 0.0, 0.0, 0.0]),
        meta: None,
    }
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

fn col_opts(auto_commit: AutoCommitConfig) -> CollectionOptions {
    CollectionOptions {
        auto_commit,
        ..Default::default()
    }
}

fn open(vfs: Arc<dyn Vfs>, auto_commit: AutoCommitConfig) -> Db {
    Db::open(
        vfs,
        "db",
        OpenOptions {
            auto_commit,
            ..Default::default()
        },
    )
    .unwrap()
}

fn sees(col: &Collection, id: &str) -> bool {
    col.search(&text_query())
        .unwrap()
        .iter()
        .any(|hit| hit.id == id)
}

fn wait_until_visible(col: &Collection, id: &str) -> bool {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if sees(col, id) {
            return true;
        }
        thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
fn idle_interval_flushes_without_another_add() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    let auto_commit = AutoCommitConfig::On {
        interval_ms: 50,
        max_docs: 1_000_000,
    };
    {
        let db = open(vfs.clone(), auto_commit.clone());
        let col = db.collection("c", schema(), col_opts(auto_commit)).unwrap();
        col.add(&[doc("d0")]).unwrap();
        assert!(
            wait_until_visible(&col, "d0"),
            "interval 到点后应自动 flush，不需要第二次 add 或显式 flush"
        );
    }
    let db = open(vfs, AutoCommitConfig::Off);
    let col = db
        .collection("c", schema(), col_opts(AutoCommitConfig::Off))
        .unwrap();
    assert!(sees(&col, "d0"), "空闲提交必须落盘，重开后仍能搜到");
}

#[test]
fn close_flushes_every_collection_even_when_auto_commit_is_off() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    {
        let db = open(vfs.clone(), AutoCommitConfig::Off);
        let a = db
            .collection("a", schema(), col_opts(AutoCommitConfig::Off))
            .unwrap();
        let b = db
            .collection("b", schema(), col_opts(AutoCommitConfig::Off))
            .unwrap();
        a.add(&[doc("da")]).unwrap();
        b.add(&[doc("db")]).unwrap();
        assert!(!sees(&a, "da") && !sees(&b, "db"));
        db.close().unwrap();
        assert!(sees(&a, "da"), "close 后 a 可搜");
        assert!(sees(&b, "db"), "close 后 b 可搜");
    }
    let db = open(vfs, AutoCommitConfig::Off);
    let a = db
        .collection("a", schema(), col_opts(AutoCommitConfig::Off))
        .unwrap();
    let b = db
        .collection("b", schema(), col_opts(AutoCommitConfig::Off))
        .unwrap();
    assert!(sees(&a, "da"), "close 必须把 a 落盘");
    assert!(sees(&b, "db"), "close 必须把 b 落盘");
}

#[test]
fn close_on_empty_buffer_is_ok() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    let db = open(vfs, AutoCommitConfig::Off);
    db.collection("c", schema(), col_opts(AutoCommitConfig::Off))
        .unwrap();
    db.close().unwrap();
    db.close().unwrap();
}

#[test]
fn drop_without_close_before_interval_drops_the_buffer() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    let auto_commit = AutoCommitConfig::On {
        interval_ms: 60_000,
        max_docs: 1_000_000,
    };
    {
        let db = open(vfs.clone(), auto_commit.clone());
        let col = db.collection("c", schema(), col_opts(auto_commit)).unwrap();
        col.add(&[doc("d0")]).unwrap();
        // 不 close。Drop 不是干净关机。
    }
    let db = open(vfs, AutoCommitConfig::Off);
    let col = db
        .collection("c", schema(), col_opts(AutoCommitConfig::Off))
        .unwrap();
    assert!(
        !sees(&col, "d0"),
        "未 close 且 interval 未到就 drop，buffer 必须丢失"
    );
}

#[test]
fn auto_commit_off_does_not_flush_on_idle() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
    {
        // Db 级默认是 On。collection 级 Off 必须盖过它。
        let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
        let col = db
            .collection("c", schema(), col_opts(AutoCommitConfig::Off))
            .unwrap();
        col.add(&[doc("d0")]).unwrap();
        thread::sleep(Duration::from_millis(150));
        assert!(!sees(&col, "d0"), "auto-commit Off 不能被空闲线程刷盘");
    }
    let db = open(vfs, AutoCommitConfig::Off);
    let col = db
        .collection("c", schema(), col_opts(AutoCommitConfig::Off))
        .unwrap();
    assert!(!sees(&col, "d0"), "Off 的 buffer 在 drop 后仍然丢失");
}

#[cfg(feature = "fault-injection")]
#[test]
fn close_returns_flush_error_and_keeps_docs_for_retry() {
    use vane_core::vfs::fault::{Fault, FaultVfs, VfsOp};

    let vfs = Arc::new(FaultVfs::wrap_memory());
    let db = Db::open(
        vfs.clone(),
        "db",
        OpenOptions {
            auto_commit: AutoCommitConfig::Off,
            ..Default::default()
        },
    )
    .unwrap();
    let col = db
        .collection("c", schema(), col_opts(AutoCommitConfig::Off))
        .unwrap();
    col.add(&[doc("d0")]).unwrap();
    vfs.inject(Fault::Enospc {
        op: VfsOp::WriteAt,
        path_pattern: "*vectors.bin".to_string(),
        one_shot: true,
        trigger_on_nth: 0,
    });

    let err = db.close();
    assert!(err.is_err(), "close 遇到刷盘失败必须返回错误");
    assert!(!sees(&col, "d0"), "失败的 close 不能把文档暴露成可搜");

    db.close().unwrap();
    assert!(sees(&col, "d0"), "故障消失后再次 close 应把同一批文档写上");
}
