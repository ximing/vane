//! 07-dict-distribution-node：loadDict napi 导出 + 词典信息查询。
//!
//! `loadDict()` 返回预编译 dict.bin（zstd 压缩）的 Buffer 副本，供 JS 侧诊断/缓存。
//! 词典在 Db::open 时已由 vane-core（dict-zh feature）自动加载（SPEC §12.3），
//! collection 创建时若 tokenizer=jieba 自动注入——JS 侧无需手动调 loadDict。

use napi::bindgen_prelude::*;
use napi_derive::napi;

/// 返回预编译 dict.bin（zstd 压缩 DAT + HMM，SPEC §5.2）的 Buffer 副本。
///
/// 词典在 `Db.open` 时已自动加载；此函数供 JS 侧诊断/缓存/分发用。
/// 禁 postinstall（SPEC §12.3）：包结构纯数据 + include_bytes，无网络下载。
#[napi]
#[allow(dead_code)] // napi 导出，Rust 侧不可见调用
pub fn load_dict() -> napi::Result<Buffer> {
    Ok(Buffer::from(vane_dict_zh::DICT_BIN))
}

/// 返回词典日历版本（YYYY.MM，如 "2026.08"）。
#[napi]
#[allow(dead_code)] // napi 导出，Rust 侧不可见调用
pub fn dict_version() -> napi::Result<String> {
    Ok(vane_dict_zh::DICT_VERSION.to_string())
}
