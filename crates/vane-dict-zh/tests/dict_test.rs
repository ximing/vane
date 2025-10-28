//! vane-dict-zh 集成测试（Task 1）：校验 DICT_BIN 可被 vane-core 加载。
//!
//! DICT_BIN 是 zstd 压缩字节（SPEC §5.2），经 `JiebaDict::load_zstd` 解压解析。

#![deny(warnings)]

use vane_dict_zh::{sha256_prefix, DICT_BIN, DICT_VERSION};

#[test]
fn dict_bin_non_empty_and_zstd_frame() {
    assert!(!DICT_BIN.is_empty());
    // zstd magic: 0x28 0xB5 0x2F 0xFD
    assert!(
        DICT_BIN.len() >= 4 && DICT_BIN[0] == 0x28 && DICT_BIN[1] == 0xB5,
        "DICT_BIN should be a zstd frame"
    );
}

#[test]
fn dict_version_is_calendar_format() {
    assert!(DICT_VERSION.starts_with("20"));
    assert_eq!(DICT_VERSION.len(), 7); // YYYY.MM
}

#[test]
fn dict_loadable_by_core() {
    let dict = vane_core::tokenizer::jieba::JiebaDict::load_zstd(DICT_BIN)
        .expect("core must load bundled dict.bin");
    assert_eq!(dict.version(), DICT_VERSION);
}

#[test]
fn sha256_prefix_matches_loaded() {
    // include_bytes 暴露的 sha256_prefix 应与 load_zstd 后取到的一致。
    let dict = vane_core::tokenizer::jieba::JiebaDict::load_zstd(DICT_BIN).unwrap();
    assert_eq!(sha256_prefix(), dict.sha256_prefix());
}
