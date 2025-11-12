// ULID 生成（零 rand 依赖，wasm32 安全）。
//
// N1 裁决：Cargo.toml 中 ulid = { version="1", default-features=false }，
// 禁用了 rand feature，因此 Ulid::new() 不可用（编译失败）。
// 改用 Ulid::from_parts(timestamp_ms, random_bits)：
//   - timestamp_ms 取自 web_time::SystemTime（M2-01：跨平台——native 零开销
//     re-export std::time::SystemTime；wasm32 用 Date.now() 经 js-sys，
//     消解 M0 已知 panic 遗留）。
//   - random_bits 由一个 AtomicU64 计数器递增后转 u128 填充 80 位随机段，
//     保证零 rand 依赖、wasm32 安全、段 ID 单调可排序。
use std::sync::atomic::{AtomicU64, Ordering};
use ulid::Ulid;
use web_time::{SystemTime, UNIX_EPOCH};

static RANDOM_COUNTER: AtomicU64 = AtomicU64::new(0);

/// 生成 26 字符 ULID（单调递增）。
pub fn gen_ulid() -> String {
    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let random_bits = RANDOM_COUNTER.fetch_add(1, Ordering::Relaxed) as u128;
    Ulid::from_parts(timestamp_ms, random_bits).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gen_ulid_returns_26_chars() {
        let s = gen_ulid();
        assert_eq!(s.len(), 26);
    }

    #[test]
    fn gen_ulid_is_monotonic() {
        let a = gen_ulid();
        let b = gen_ulid();
        assert!(b >= a, "ulid should be monotonic: {} vs {}", a, b);
    }
}
