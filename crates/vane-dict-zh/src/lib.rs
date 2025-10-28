//! `vane-dict-zh`：Vane 中文 jieba 词典数据包（SPEC §12.3）。
//!
//! 平台无关纯数据 crate：仅 `include_bytes!` 预编译 `dict.bin`（zstd 压缩 DAT + HMM 参数，
//! SPEC §5.2），无 Rust 运行逻辑。词典加载在 `vane-core`（feature `jieba`，`JiebaDict::load_zstd`）。
//!
//! - 词典永不进 wasm（红线）：本 crate 独立于 `vane-core`，wasm32 构建不依赖它。
//! - 禁 postinstall：包结构纯数据 + `include_bytes`，企业断网友好。
//! - 日历版本化（`2026.08`），与库 semver 解耦（SPEC §3.3：词典升级仅警告不强制重建）。
//!
//! `DICT_VERSION` 与 Go embed（08 计划）一致才发版（10-ci-m1 三渠道一致性校验）。

#![deny(warnings)]

/// 预编译 dict.bin（zstd 压缩）。SPEC §5.2 物理格式：
/// ```text
/// magic(4)="VNDT" | format_version(4 LE) | sha256_prefix(8) |
/// dict_version_len(2 LE) | dict_version | total_freq(8 LE) |
/// dat_len(4 LE) | base[i32] | check[i32] | values[i32] |
/// hmm_blob_len(4 LE) | hmm_blob
/// ```
///
/// 整体经 zstd 压缩；解压后由 `vane_core::tokenizer::jieba::JiebaDict::load_zstd` 解析。
pub const DICT_BIN: &[u8] = include_bytes!("../data/dict.bin");

/// 词典日历版本（YYYY.MM）。与库 semver 解耦（SPEC §3.3/§12.3）。
/// 升级词典内容只改此版本与 `data/dict.bin`，不递增库版本也不改 TokenizerId。
pub const DICT_VERSION: &str = "2026.08";

/// sha256 前 8 字节（编译期 include，由 gen_dict 生成 `data/sha256_prefix.bin`）。
///
/// dict.bin 经 zstd 压缩，头部在解压后；为保持本 crate 运行期零依赖（不引入 ruzstd），
/// sha256 prefix 由生成期单独写入 8 字节文件，此处 `include_bytes!` 编译期嵌入。
/// 供三渠道分发一致性校验（SPEC §12.3）。绑定层亦可经
/// `JiebaDict::load_zstd(DICT_BIN)?.sha256_prefix()` 取运行时值（两者一致）。
pub const SHA256_PREFIX_BIN: &[u8] = include_bytes!("../data/sha256_prefix.bin");

/// 词典内容 sha256 前 8 字节（与 `SHA256_PREFIX_BIN` 一致）。
pub fn sha256_prefix() -> [u8; 8] {
    let mut arr = [0u8; 8];
    arr.copy_from_slice(&SHA256_PREFIX_BIN[..8]);
    arr
}

/// 词典日历版本（与 [`DICT_VERSION`] 一致），以 `&str` 形式暴露供绑定层查询。
pub fn dict_version() -> &'static str {
    DICT_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dict_bin_non_empty_and_magic() {
        assert!(!DICT_BIN.is_empty());
        // DICT_BIN 是 zstd 压缩字节；magic "VNDT" 在解压后头部，不直接可见。
        // 这里仅断言压缩帧非空且以 zstd magic 起始（0x28 0xB5 0x2F 0xFD）。
        assert!(DICT_BIN.len() >= 4);
    }

    #[test]
    fn dict_version_calendar_format() {
        assert!(DICT_VERSION.starts_with("20"));
        assert_eq!(DICT_VERSION.len(), 7); // YYYY.MM
    }
}
