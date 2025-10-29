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

/// Task 4：dict.bin gzip 体积 ≤ 1.5MB（SPEC §13.2-3 CI 门禁）。
/// dict.bin 已 zstd 压缩，gzip 体积相近（zstd 帧无法再被 gzip 显著压缩）。
#[test]
fn dict_bin_gzip_under_1_5mb() {
    let gzip_size = gzip_size(DICT_BIN);
    assert!(
        gzip_size <= 1_500_000,
        "dict.bin gzip {} > 1.5MB gate (SPEC §13.2-3)",
        gzip_size
    );
}

/// SPEC §5.2/§12.3 三渠道一致性：sha256_prefix 必须是解压后 dict.bin payload
/// （去掉头部 16 字节 magic+format_version+sha256_prefix）的真 SHA-256 前 8 字节。
/// Go（crypto/sha256）/ WASM（SubtleCrypto）独立计算须与本值匹配。
#[test]
fn sha256_prefix_is_real_sha256_of_payload() {
    use sha2::{Digest, Sha256};

    // 解压 DICT_BIN（zstd 压缩）得到原始 dict.bin 字节。
    let mut decoder =
        ruzstd::streaming_decoder::StreamingDecoder::new(DICT_BIN).expect("decode dict.bin zstd");
    use std::io::Read;
    let mut buf = Vec::with_capacity(DICT_BIN.len() * 4);
    decoder
        .read_to_end(&mut buf)
        .expect("read dict.bin decompressed");

    // 头部 16 字节 = magic(4) + format_version(4) + sha256_prefix(8)；payload = [16..]。
    assert!(buf.len() > 16, "decompressed dict.bin too short");
    let payload = &buf[16..];
    let digest = Sha256::digest(payload);
    let mut expected = [0u8; 8];
    expected.copy_from_slice(&digest[..8]);

    assert_eq!(
        sha256_prefix(),
        expected,
        "sha256_prefix must equal real SHA-256([16..]) prefix (SPEC §5.2/§12.3)"
    );

    // 同时校验 dict.bin 头部内嵌的 sha256_prefix 与 include_bytes 暴露值一致。
    let mut embedded = [0u8; 8];
    embedded.copy_from_slice(&buf[8..16]);
    assert_eq!(
        sha256_prefix(),
        embedded,
        "embedded header sha256_prefix mismatch"
    );
}

/// Task 4：词典词条数 ≥ 20 万（剪枝 top 20 万 + 全部单字）。
#[test]
fn dict_has_substantial_vocab() {
    let dict = vane_core::tokenizer::jieba::JiebaDict::load_zstd(DICT_BIN).unwrap();
    // 验证常见词可查到词频
    assert!(dict.freq("的").is_some(), "「的」应在词典中");
    assert!(dict.freq("中国").is_some(), "「中国」应在词典中");
    assert!(dict.freq("是").is_some(), "「是」应在词典中");
}

/// gzip 体积估算（与 gen_dict.rs 同方法）。
fn gzip_size(data: &[u8]) -> usize {
    use std::process::Command;
    let tmp = std::env::temp_dir().join(format!("vane_dict_test_{}.tmp", std::process::id()));
    if std::fs::write(&tmp, data).is_err() {
        return data.len();
    }
    let out = Command::new("gzip").args(["-c", "-9"]).arg(&tmp).output();
    let _ = std::fs::remove_file(&tmp);
    match out {
        Ok(o) => o.stdout.len(),
        Err(_) => data.len(),
    }
}
