//! SPEC §10 错误码透传：core VaneError → napi::Error。
//!
//! reason 编码为 `"{code}:{name}:{msg}"`，JS 侧 main.js 的 wrapErr 解析回
//! VaneError(.code/.name)。code 原值透传，不吞并/重编（§10）。
//!
//! 注意（S14）：napi::Error 的 `reason` 字段在 napi 2.16.13 为 pub。本 crate
//! 锁定 `napi = "=2.16.13"`，故可直接访问；若版本升级需改用 Display 重建。
//!
//! 偏离说明：计划原文写 `impl From<CoreErr> for Error`，但 VaneError 与
//! napi::Error 均为外部类型，违反 orphan rule（E0117）。改用自由函数
//! `to_napi_error`，行为等价，调用处用 `.map_err(to_napi_error)`。

use napi::{bindgen_prelude::*, Status};
use vane_core::types::VaneError as CoreErr;

/// 取 VaneError 的纯消息（不含 name 前缀），保证 reason 编码为
/// `{code}:{name}:{msg}` 而非 `{code}:{name}:{name}: {msg}`。
/// M4 诊断重构：所有变体统一携带 ErrorContext，直接取 context().message。
fn message(e: &CoreErr) -> String {
    e.context().message.clone()
}

/// napi::Status 映射（S15/S20 裁决）：M0 略粗糙，JS 侧用 `.code` 判定。
/// napi 2.16.13 的 Status 无 NotFound/WouldBlock 变体；E_NOT_FOUND/E_BUSY 暂归
/// GenericFailure，code 仍由 reason 前缀原值透传（§10）。
fn status_of(code: i32) -> Status {
    match code {
        -11 | -2 => Status::InvalidArg, // E_INVALID_ARG / E_SCHEMA
        // 其余（E_IO/E_NOT_FOUND/E_CORRUPT/E_UNSUPPORTED/E_BUSY/...）→ GenericFailure
        _ => Status::GenericFailure,
    }
}

/// 把 core VaneError 转成 napi::Error，reason 编码 "{code}:{name}:{msg}"。
pub fn to_napi_error(e: CoreErr) -> Error {
    let code = e.code();
    let name = e.name();
    let msg = message(&e);
    Error::new(status_of(code), format!("{code}:{name}:{msg}"))
}

/// binding 内统一 Result 别名（napi::Error）。
pub type NapiResult<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reason_round_trip_schema() {
        let e = to_napi_error(CoreErr::Schema("dim mismatch".into()));
        // S14：napi 2.16.13 reason 为 pub 字段。
        assert_eq!(e.reason, "-2:E_SCHEMA:dim mismatch");
    }

    #[test]
    fn reason_round_trip_unsupported() {
        let e = to_napi_error(CoreErr::Unsupported("platform capability missing".into()));
        assert_eq!(e.reason, "-10:E_UNSUPPORTED:platform capability missing");
    }

    #[test]
    fn reason_round_trip_invalid_arg() {
        let e = to_napi_error(CoreErr::InvalidArg("bad".into()));
        assert_eq!(e.reason, "-11:E_INVALID_ARG:bad");
    }

    #[test]
    fn reason_round_trip_not_found() {
        let e = to_napi_error(CoreErr::NotFound("missing".into()));
        assert_eq!(e.reason, "-3:E_NOT_FOUND:missing");
    }

    #[test]
    fn code_passthrough_not_remapped() {
        // §10：code 原值透传，不得吞并为 GenericFailure 等模糊码。
        // 即便 E_UNSUPPORTED 的 Status 是 GenericFailure，reason 仍带 -10。
        let e = to_napi_error(CoreErr::Unsupported("platform capability missing".into()));
        assert!(e.reason.starts_with("-10:E_UNSUPPORTED"));
    }
}
