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
