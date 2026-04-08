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
