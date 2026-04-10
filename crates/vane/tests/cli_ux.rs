static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct TempHome {
    path: std::path::PathBuf,
}

fn temp_home(tag: &str) -> TempHome {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path =
        std::env::temp_dir().join(format!("vane-ux-{tag}-{}-{nanos}-{n}", std::process::id()));
    std::fs::create_dir_all(&path).unwrap();
    TempHome { path }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let tmp = std::env::temp_dir();
        if self.path.starts_with(&tmp) && self.path != tmp {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

/// Call only while holding ENV_LOCK: sets VANE_ALLOW_EMBED_FAIL so the embed
/// probe fails closed and init continues without a live embedder.
fn init_home() -> TempHome {
    let dir = temp_home("init");
    let answers = vane::wizard::InitAnswers {
        install_service: false,
        ..Default::default()
    };
    std::env::set_var("VANE_ALLOW_EMBED_FAIL", "1");
    vane::wizard::run_init(&dir.path, std::io::empty(), std::io::sink(), Some(answers)).unwrap();
    dir
}

mod fsutil_tests {
    use std::path::{Path, PathBuf};

    #[test]
    fn atomic_write_creates_parent_and_renames() {
        let dir = std::env::temp_dir().join(format!("vane-fsutil-{}", std::process::id()));
        let path = dir.join("a").join("b.json");
        vane::fsutil::atomic_write(&path, b"{}", "b.json").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"{}");
        assert!(!dir.join("a").join("b.json.tmp").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn expand_tilde_handles_all_forms() {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap();
        assert_eq!(vane::fsutil::expand_tilde(Path::new("~")), home);
        assert_eq!(
            vane::fsutil::expand_tilde(Path::new("~/notes")),
            home.join("notes")
        );
        assert_eq!(
            vane::fsutil::expand_tilde(Path::new("/abs/x")),
            PathBuf::from("/abs/x")
        );
        assert_eq!(
            vane::fsutil::expand_tilde(Path::new("rel/x")),
            PathBuf::from("rel/x")
        );
    }
}

mod i18n_tests {
    use vane::i18n::{pick, tr, Lang};

    #[test]
    fn tables_have_identical_keys() {
        let mut en: Vec<&str> = vane::i18n::EN_TABLE.iter().map(|(k, _)| *k).collect();
        let mut zh: Vec<&str> = vane::i18n::ZH_TABLE.iter().map(|(k, _)| *k).collect();
        en.sort_unstable();
        zh.sort_unstable();
        assert_eq!(en, zh, "EN/ZH key sets diverged");
    }

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k: &str| {
            pairs
                .iter()
                .find(|(key, _)| *key == k)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn detect_priority_and_zh_prefix() {
        assert_eq!(Lang::detect_with(&env(&[])), Lang::En);
        assert_eq!(
            Lang::detect_with(&env(&[("LANG", "zh_CN.UTF-8")])),
            Lang::Zh
        );
        assert_eq!(
            Lang::detect_with(&env(&[("LANG", "zh_CN.UTF-8"), ("LC_ALL", "en_US.UTF-8")])),
            Lang::En
        );
        assert_eq!(
            Lang::detect_with(&env(&[("LC_ALL", "en_US"), ("VANE_LANG", "zh")])),
            Lang::Zh
        );
        assert_eq!(Lang::detect_with(&env(&[("LANG", "ZH_TW")])), Lang::Zh);
    }

    #[test]
    fn pick_forces_english_off_tty() {
        assert_eq!(pick(Lang::Zh, false, "time.just_now"), "just now");
        assert_eq!(pick(Lang::Zh, true, "time.just_now"), "刚刚");
        assert_eq!(tr(Lang::En, "nonexistent.key"), "missing-i18n-key");
    }
}

mod humanize_tests {
    use vane::humanize::{abs_date, rel_time};
    use vane::i18n::Lang;

    const NOW: u64 = 1_755_700_000; // fixed reference

    #[test]
    fn all_buckets_en_and_zh() {
        assert_eq!(rel_time(0, NOW, Lang::En), "never");
        assert_eq!(rel_time(NOW + 100, NOW, Lang::En), "just now"); // clock skew
        assert_eq!(rel_time(NOW - 5, NOW, Lang::En), "just now");
        assert_eq!(rel_time(NOW - 30, NOW, Lang::En), "30s ago");
        assert_eq!(rel_time(NOW - 180, NOW, Lang::En), "3 min ago");
        assert_eq!(rel_time(NOW - 5 * 3600, NOW, Lang::En), "5 hours ago");
        assert_eq!(rel_time(NOW - 10 * 86_400, NOW, Lang::En), "10 days ago");
        assert_eq!(
            rel_time(NOW - 40 * 86_400, NOW, Lang::En),
            abs_date(NOW - 40 * 86_400)
        );
        assert_eq!(rel_time(NOW - 180, NOW, Lang::Zh), "3 分钟前");
        assert_eq!(rel_time(0, NOW, Lang::Zh), "从未");
    }

    #[test]
    fn abs_date_known_epoch() {
        assert_eq!(abs_date(0), "1970-01-01");
        assert_eq!(abs_date(1_755_700_000), "2025-08-20");
    }
}

mod query_display_tests {
    use vane::i18n::Lang;

    #[test]
    fn collapse_home_folds_home_prefix() {
        let home = std::env::var("HOME").unwrap();
        assert_eq!(vane::ui::collapse_home(&format!("{home}/notes")), "~/notes");
        assert_eq!(vane::ui::collapse_home("/var/tmp"), "/var/tmp");
    }

    #[test]
    fn scope_header_single_root_and_all() {
        let one = vane::ui::format_scope_header(Some("~/notes"), 1, 12, false, Lang::En, false);
        assert_eq!(one, "searching ~/notes · 12 live files · hybrid");
        let all = vane::ui::format_scope_header(None, 3, 42, false, Lang::En, false);
        assert_eq!(all, "searching 3 roots · 42 live files · hybrid");
        let deg = vane::ui::format_scope_header(Some("~/notes"), 1, 12, true, Lang::Zh, false);
        assert!(deg.contains("BM25（降级：embedder 不可达）"));
    }

    #[test]
    fn hit_lines_omit_root_single_scope_and_id_unless_verbose() {
        let hit = serde_json::json!({
            "id": "p1:notes/a.md#0", "path": "notes/a.md", "root": "/abs/notes",
            "snippet": "hello", "score": 0.42, "degraded": false
        });
        let single = vane::ui::hit_lines(
            &hit,
            0,
            &vane::ui::HitLineOpts {
                all: false,
                verbose: false,
                header_degraded: true,
            },
            false,
        );
        assert!(
            !single.iter().any(|l| l.contains("/abs/notes")),
            "single scope must not repeat root"
        );
        assert!(
            !single.iter().any(|l| l.contains("p1:notes/a.md#0")),
            "id hidden by default"
        );
        let all = vane::ui::hit_lines(
            &hit,
            0,
            &vane::ui::HitLineOpts {
                all: true,
                verbose: true,
                header_degraded: true,
            },
            false,
        );
        assert!(all.iter().any(|l| l.contains("/abs/notes")));
        assert!(all.iter().any(|l| l.contains("p1:notes/a.md#0")));
        let deg = serde_json::json!({"id":"p:x#0","path":"x","root":"/r","snippet":"","score":0.1,"degraded":true});
        let lines = vane::ui::hit_lines(
            &deg,
            0,
            &vane::ui::HitLineOpts {
                all: false,
                verbose: false,
                header_degraded: true,
            },
            false,
        );
        assert!(
            !lines.iter().any(|l| l.contains("degraded")),
            "header already aggregated it"
        );
    }
}

mod dispatch_tests {
    use vane::dispatch::{decide_query_arg, QueryArg};

    #[test]
    fn query_arg_branches() {
        assert_eq!(
            decide_query_arg(Some("foo".into()), true),
            QueryArg::Run("foo".into())
        );
        assert_eq!(decide_query_arg(None, true), QueryArg::Prompt);
        assert_eq!(decide_query_arg(None, false), QueryArg::MissingError);
    }
}

mod subprocess_tests {
    use std::process::{Command, Stdio};

    #[test]
    fn query_without_arg_non_tty_is_single_line_exit_2() {
        let _g = crate::ENV_LOCK.lock().unwrap();
        let home = crate::init_home();
        let out = Command::new(env!("CARGO_BIN_EXE_vane"))
            .args(["--home", &home.path.display().to_string(), "query"])
            .env("VANE_LANG", "en")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2));
        let stderr = String::from_utf8(out.stderr).unwrap();
        assert!(stderr.contains("missing query"), "got: {stderr}");
        assert!(
            !stderr.contains("Usage:"),
            "must not dump clap help: {stderr}"
        );
        assert_eq!(stderr.lines().count(), 1);
    }
}

mod last_query_tests {
    use vane::last_query::*;

    fn sample() -> LastQuery {
        LastQuery {
            query: "foo".into(),
            at: 1_755_700_000,
            scope_root: Some("/abs/notes".into()),
            hits: vec![CachedHit {
                id: "p1:notes/a.md#0".into(),
                path: "notes/a.md".into(),
                root: "/abs/notes".into(),
                score: 0.42,
            }],
        }
    }

    #[test]
    fn roundtrip_and_corrupt_cache() {
        let dir = crate::temp_home("lq-roundtrip");
        save_last_query(&dir.path, &sample()).unwrap();
        assert_eq!(load_last_query(&dir.path).unwrap(), sample());
        std::fs::write(last_query_path(&dir.path), b"not json").unwrap();
        assert!(
            load_last_query(&dir.path).is_none(),
            "corrupt cache must be None"
        );
    }

    #[test]
    fn read_outcome_errors() {
        let dir = crate::temp_home("lq-errors");
        let q = sample();
        // no live set / CAS → stale (chunk not found)
        assert!(matches!(
            read_outcome(&dir.path, &q, 1),
            Err(ReadError::Stale { n: 1 })
        ));
        assert!(matches!(
            read_outcome(&dir.path, &q, 5),
            Err(ReadError::OutOfRange { n: 5, k: 1 })
        ));
        let empty = LastQuery {
            hits: vec![],
            ..sample()
        };
        assert!(matches!(
            read_outcome(&dir.path, &empty, 1),
            Err(ReadError::Empty)
        ));
    }

    #[test]
    fn read_outcome_reads_chunk_from_cas() {
        // Minimal live set + CAS: one file, one chunk.
        let dir = crate::temp_home("lq-cas");
        let home = dir.path.as_path();
        let pid = "p1";
        let mut live = vane::live::LiveSet::default();
        live.files.insert(
            "notes/a.md".into(),
            vane::live::LiveFile {
                content_sha256: "x".into(),
                extract_key: "k1".into(),
                chunk_count: 1,
            },
        );
        live.save_for_project(home, pid).unwrap();
        let cas = vane::cas::Cas::new(home.join("rag").join("cas"));
        cas.put_extract(
            "k1",
            &[vane::extract::CanonicalDoc {
                text: "hello world chunk".into(),
                headings: vec![],
                path: "notes/a.md".into(),
                chunk_index: 0,
                start_byte: 0,
                end_byte: 17,
                modality: "text".into(),
                extractor: "text".into(),
            }],
        )
        .unwrap();
        let out = read_outcome(home, &sample(), 1).unwrap();
        assert_eq!(out.text, "hello world chunk");
        assert_eq!(out.chunk_index, 0);
    }
}

mod status_tests {
    use vane::i18n::Lang;

    fn sample_status() -> serde_json::Value {
        serde_json::json!({
            "home": "/h/.vane",
            "running": true,
            "dirty_queue_size": 0,
            "disk": { "home_bytes": 2048, "cas_bytes": 1024 },
            "roots": [{
                "path": "/abs/notes", "live_files": 12,
                "last_reconcile": 1_755_699_700u64, // 300s before NOW
                "model": "nomic-embed-text", "dim": 768,
                "dirty_queue_size": 0, "skip_count": 12
            }]
        })
    }

    const NOW: u64 = 1_755_700_000;

    #[test]
    fn watching_and_humanized_root_lines() {
        let view = vane::ui::status_view(&sample_status(), None);
        let lines = vane::ui::format_status_lines(&view, Lang::En, NOW);
        let joined = lines.join("\n");
        assert!(joined.contains("watching"), "{joined}");
        assert!(joined.contains("indexed 5 min ago"), "{joined}");
        assert!(joined.contains("12 skipped — run vane issues"), "{joined}");
        assert!(
            !joined.contains("1755699700"),
            "no raw unix seconds: {joined}"
        );
        assert!(
            !joined.contains("pending changes"),
            "dirty=0 must not print"
        );
    }

    #[test]
    fn indexing_state_and_never_indexed() {
        let mut v = sample_status();
        v["roots"][0]["last_reconcile"] = serde_json::Value::Null;
        let view = vane::ui::status_view(&v, Some((34, 120)));
        let lines = vane::ui::format_status_lines(&view, Lang::Zh, NOW).join("\n");
        assert!(lines.contains("索引中 34/120"), "{lines}");
        assert!(lines.contains("从未索引"), "{lines}");
        assert!(lines.contains("12 个文件被跳过"), "{lines}");
    }

    #[test]
    fn daemon_stopped_keeps_existing_warning() {
        let mut v = sample_status();
        v["running"] = serde_json::json!(false);
        let view = vane::ui::status_view(&v, None);
        let lines = vane::ui::format_status_lines(&view, Lang::En, NOW).join("\n");
        assert!(lines.contains("daemon not running — vane start"), "{lines}");
        assert!(!lines.contains("watching"));
    }
}

mod add_summary_tests {
    use vane::i18n::Lang;

    #[test]
    fn summary_with_and_without_skips() {
        assert_eq!(
            vane::ui::format_add_summary(5, 75, 0, Lang::En),
            "indexed 80 files"
        );
        assert_eq!(
            vane::ui::format_add_summary(5, 75, 3, Lang::En),
            "indexed 80 files, 3 skipped — run vane issues"
        );
        assert_eq!(
            vane::ui::format_add_summary(5, 75, 0, Lang::Zh),
            "已索引 80 个文件"
        );
        assert_eq!(
            vane::ui::format_add_summary(5, 75, 3, Lang::Zh),
            "已索引 80 个文件，跳过 3 个 — 运行 vane issues 查看"
        );
    }

    #[test]
    fn progress_style_choice() {
        assert_eq!(
            vane::progress::choose_progress_style(0),
            vane::progress::ProgressStyle::Spinner
        );
        assert_eq!(
            vane::progress::choose_progress_style(120),
            vane::progress::ProgressStyle::Bar(120)
        );
        assert_eq!(vane::progress::clamp_pos(34, 120), 34);
        assert_eq!(vane::progress::clamp_pos(150, 120), 120); // spec 13b: pos 封顶
    }
}
