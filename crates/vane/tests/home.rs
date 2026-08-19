use std::path::PathBuf;
use vane::home::resolve_home;

#[test]
fn cli_flag_beats_env_beats_default() {
    let fallback = PathBuf::from("/Users/me/.vane");
    let env = std::ffi::OsString::from("/tmp/env-vane");
    let cli = PathBuf::from("/tmp/cli-vane");
    assert_eq!(
        resolve_home(Some(&cli), Some(&env), &fallback),
        PathBuf::from("/tmp/cli-vane")
    );
    assert_eq!(
        resolve_home(None, Some(&env), &fallback),
        PathBuf::from("/tmp/env-vane")
    );
    assert_eq!(resolve_home(None, None, &fallback), fallback);
}
