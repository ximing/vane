//! 词典加载（SPEC §12.3）：CDN fetch + sha256 校验 + VFS 缓存 + 降级 bigram。
//!
//! ## 降级铁律（SPEC §12.4 / §10）
//! 词典不可用时 **永不抛错**（`E_DICT_UNAVAILABLE` 禁止到达最终用户）：
//! - CDN fetch 失败 / sha256 不匹配 / 网络离线 → 降级 `CjkBigram` + `console.warn`。
//! - 调用方（Worker `collection`）在 `DictUnavailable` 时改用 `CjkBigram` 分词器。
//!
//! ## 三渠道（SPEC §12.3）
//! 1. `dictData` 内联注入（离线场景，opts.dictData = Uint8Array）→ 直接 `JiebaDict::load`。
//! 2. CDN fetch → sha256 校验 → VFS 缓存（二次启动零网络）。
//! 3. 以上均失败 → 降级 `CjkBigram` + warn（不抛错）。
//!
//! ## 词典永不进 wasm（红线）
//! 本模块仅处理运行时词典字节（fetch/内联），不编译词典数据进 wasm。
//! `dict-zh` feature 永不启用；`jieba` feature（仅算法代码 DAT/HMM/seg）可启用须过 800KB 门禁。

use vane_core::types::{Result, VaneError};
use vane_core::vfs::Vfs;

/// 跨平台 console.warn（wasm32 用 web_sys::console，native 用 eprintln）。
/// native 测试环境无浏览器 console，直接调用 web_sys::console::warn_1 会 panic。
fn warn(msg: &str) {
    #[cfg(target_arch = "wasm32")]
    web_sys::console::warn_1(&msg.into());
    #[cfg(not(target_arch = "wasm32"))]
    eprintln!("[vane] {}", msg);
}

/// 词典缓存在 VFS 中的逻辑路径（与 db 容器同 Vfs，独立文件）。
const DICT_CACHE_PATH: &str = "dict.bin.cache";

/// sha256 前 8 字节校验（SPEC §12.3 三渠道一致性）。
pub fn verify_sha256_prefix(bytes: &[u8], expected: &[u8; 8]) -> bool {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let hash = hasher.finalize();
    &hash[..8] == expected.as_slice()
}

/// 读 VFS 缓存（二次启动零网络）。返回缓存的词典字节。
///
/// Vfs trait 无 `size` 方法——按 chunk 增量读到 EOF，避免固定 16MB 分配。
fn read_cache(cache_vfs: &dyn Vfs) -> Result<Vec<u8>> {
    const CHUNK: usize = 256 * 1024;
    let mut buf = Vec::new();
    let mut offset = 0u64;
    loop {
        let mut chunk = [0u8; CHUNK];
        let n = cache_vfs.read_at(DICT_CACHE_PATH, &mut chunk, offset)?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        offset += n as u64;
        if n < CHUNK {
            break;
        }
    }
    if buf.is_empty() {
        return Err(VaneError::NotFound("dict cache empty".into()));
    }
    Ok(buf)
}

/// 写 VFS 缓存（best-effort，失败不阻断）。
///
/// 覆盖写语义：先 delete 旧文件（忽略 not-found），再 create + write_at。
/// 直接 create 对已存在文件返 Err（overlay.rs create 语义），词典更新时缓存
/// 无法刷新 → 退化为恒走 CDN。delete+create 保证缓存可刷新（I-1 fix）。
fn write_cache(cache_vfs: &dyn Vfs, bytes: &[u8]) -> Result<()> {
    // 先删旧缓存（不存在时忽略 Err）。
    let _ = cache_vfs.delete(DICT_CACHE_PATH);
    cache_vfs.create(DICT_CACHE_PATH)?;
    cache_vfs.write_at(DICT_CACHE_PATH, bytes, 0)?;
    Ok(())
}

/// 从 CDN fetch 词典字节（wasm32 浏览器 fetch API）。
///
/// 返回 `None`（而非 Err）表示 fetch 失败——降级不抛错。
/// 使用 js_sys::global() 反射获取全局 `fetch`（兼容 Window + WorkerGlobalScope）。
#[cfg(target_arch = "wasm32")]
async fn fetch_cdn(url: &str) -> Option<Vec<u8>> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    let global = js_sys::global();
    let fetch_fn = js_sys::Reflect::get(&global, &"fetch".into()).ok()?;
    let fetch_fn: js_sys::Function = fetch_fn.dyn_into().ok()?;
    let resp_val = fetch_fn.call1(&global, &url.into()).ok()?;
    let resp: web_sys::Response = resp_val.dyn_into().ok()?;
    if !resp.ok() {
        warn(&format!(
            "vane: dict CDN fetch failed (status {})",
            resp.status()
        ));
        return None;
    }
    let buf_promise = resp.array_buffer().ok()?;
    let buf = JsFuture::from(buf_promise.unchecked_into::<js_sys::Promise>())
        .await
        .ok()?;
    let array = js_sys::Uint8Array::new(&buf);
    Some(array.to_vec())
}

#[cfg(not(target_arch = "wasm32"))]
async fn fetch_cdn(_url: &str) -> Option<Vec<u8>> {
    // 非 wasm32（node 单测）：无 fetch API，返回 None（降级 bigram）。
    None
}

/// 加载词典（三渠道）。
///
/// 1. `dict_data` 内联 → 直接返回（跳过 CDN）。
/// 2. CDN fetch → sha256 校验 → VFS 缓存。
/// 3. 失败 → 返回 `None`（调用方降级 `CjkBigram` + warn）。
///
/// `cache_vfs` 为 `None` 时不读写缓存（无持久化 VFS 场景）。
pub async fn load_dict(
    dict_data: Option<&[u8]>,
    cdn_url: Option<&str>,
    expected_sha256: Option<&[u8; 8]>,
    cache_vfs: Option<&dyn Vfs>,
) -> Option<Vec<u8>> {
    // 渠道 1：内联 dictData（离线场景）。
    if let Some(data) = dict_data {
        if let Some(expected) = expected_sha256 {
            if !verify_sha256_prefix(data, expected) {
                warn("vane: inline dictData sha256 mismatch, falling back to bigram");
                return None;
            }
        }
        return Some(data.to_vec());
    }

    // 渠道 2：CDN fetch。
    let url = cdn_url?;

    // 2a. 读缓存（二次启动零网络）。
    if let Some(vfs) = cache_vfs {
        if let Ok(cached) = read_cache(vfs) {
            if expected_sha256
                .map(|exp| verify_sha256_prefix(&cached, exp))
                .unwrap_or(true)
            {
                return Some(cached);
            }
        }
    }

    // 2b. CDN fetch。
    let bytes = fetch_cdn(url).await?;

    // 2c. sha256 校验。
    if let Some(expected) = expected_sha256 {
        if !verify_sha256_prefix(&bytes, expected) {
            warn("vane: dict CDN sha256 mismatch, falling back to bigram");
            return None;
        }
    }

    // 2d. 写缓存（best-effort）。
    if let Some(vfs) = cache_vfs {
        let _ = write_cache(vfs, &bytes);
    }

    Some(bytes)
}

/// 词典降级通知（SPEC §12.4）：打印 warn，不抛错。
/// 调用方在 `collection(tokenizer=Jieba)` 收到 `DictUnavailable` 时调用本函数，
/// 改用 `CjkBigram` 分词器。`E_DICT_UNAVAILABLE` 禁止到达最终用户。
pub fn dict_unavailable_fallback() {
    warn("vane: jieba dict unavailable, falling back to cjk_bigram tokenizer");
}

#[cfg(test)]
mod tests {
    use super::*;
    use vane_core::vfs::memory::MemoryVfs;

    /// 最小 block_on（测试专用，无外部 async runtime 依赖）。
    /// load_dict 在非 wasm32 下不真正 park（fetch_cdn 立即返 None），spin poll 即可。
    fn block_on<F: std::future::Future>(fut: F) -> F::Output {
        use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
        fn dummy_raw_waker() -> RawWaker {
            fn no_op(_: *const ()) {}
            fn clone_fn(_: *const ()) -> RawWaker {
                dummy_raw_waker()
            }
            static VTABLE: RawWakerVTable = RawWakerVTable::new(clone_fn, no_op, no_op, no_op);
            RawWaker::new(std::ptr::null(), &VTABLE)
        }
        let waker = unsafe { Waker::from_raw(dummy_raw_waker()) };
        let mut cx = Context::from_waker(&waker);
        let mut fut = std::pin::pin!(fut);
        loop {
            if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
                return v;
            }
        }
    }

    #[test]
    fn sha256_prefix_verify_ok() {
        use sha2::Digest;
        let bytes = b"hello";
        let mut hasher = sha2::Sha256::new();
        hasher.update(bytes);
        let hash = hasher.finalize();
        let mut prefix = [0u8; 8];
        prefix.copy_from_slice(&hash[..8]);
        assert!(verify_sha256_prefix(bytes, &prefix));
    }

    #[test]
    fn sha256_prefix_verify_mismatch() {
        let prefix = [0u8; 8];
        assert!(!verify_sha256_prefix(b"hello", &prefix));
    }

    #[test]
    fn inline_dict_data_skips_cdn() {
        let data = b"fake-dict-bytes";
        let result = block_on(load_dict(
            Some(data),
            Some("https://cdn.example/dict.bin"),
            None,
            None,
        ));
        assert_eq!(result, Some(data.to_vec()));
    }

    #[test]
    fn inline_dict_data_sha256_mismatch_returns_none() {
        let data = b"fake-dict-bytes";
        let bad_prefix = [0u8; 8];
        let result = block_on(load_dict(Some(data), None, Some(&bad_prefix), None));
        assert!(result.is_none(), "sha256 mismatch → None (降级 bigram)");
    }

    #[test]
    fn no_dict_data_no_cdn_returns_none() {
        let result = block_on(load_dict(None, None, None, None));
        assert!(result.is_none());
    }

    #[test]
    fn cdn_fetch_failure_returns_none() {
        // 非 wasm32：fetch_cdn 恒返 None → 降级 bigram（不抛错）。
        let result = block_on(load_dict(
            None,
            Some("https://cdn.example/dict.bin"),
            None,
            None,
        ));
        assert!(result.is_none(), "fetch 失败 → None（降级 bigram，不抛错）");
    }

    #[test]
    fn cache_round_trip() {
        let vfs = MemoryVfs::new();
        let bytes = b"cached-dict";
        write_cache(&vfs, bytes).unwrap();
        let read = read_cache(&vfs).unwrap();
        assert_eq!(read, bytes);
    }

    #[test]
    fn cache_hit_skips_fetch() {
        let vfs = MemoryVfs::new();
        let bytes = b"cached-dict";
        write_cache(&vfs, bytes).unwrap();
        let result = block_on(load_dict(
            None,
            Some("https://cdn.example/dict.bin"),
            None,
            Some(&vfs),
        ));
        assert_eq!(result, Some(bytes.to_vec()));
    }

    #[test]
    fn cache_sha256_mismatch_falls_back() {
        let vfs = MemoryVfs::new();
        let bytes = b"cached-dict";
        write_cache(&vfs, bytes).unwrap();
        let bad_prefix = [0u8; 8];
        let result = block_on(load_dict(
            None,
            Some("https://cdn.example/dict.bin"),
            Some(&bad_prefix),
            Some(&vfs),
        ));
        assert!(result.is_none(), "缓存 sha256 不匹配 + fetch 失败 → None");
    }

    /// I-1：词典更新后缓存能刷新（delete+create 覆盖旧缓存）。
    /// 首次缓存 v1 → 词典变更 v2 → write_cache 成功 → 二次启动命中 v2 缓存。
    #[test]
    fn cache_refresh_after_dict_update() {
        let vfs = MemoryVfs::new();
        let v1 = b"dict-version-1";
        let v2 = b"dict-version-2-longer";

        // 首次缓存 v1。
        write_cache(&vfs, v1).unwrap();
        let read1 = read_cache(&vfs).unwrap();
        assert_eq!(read1, v1);

        // 词典更新 → 覆盖写 v2（delete+create+write_at）。
        write_cache(&vfs, v2).unwrap();
        let read2 = read_cache(&vfs).unwrap();
        assert_eq!(read2, v2, "缓存刷新后应读回 v2");

        // 二次启动命中缓存（sha256 匹配，零网络）。
        let v2_prefix = {
            use sha2::Digest;
            let mut h = sha2::Sha256::new();
            h.update(v2);
            let hash = h.finalize();
            let mut p = [0u8; 8];
            p.copy_from_slice(&hash[..8]);
            p
        };
        let result = block_on(load_dict(
            None,
            Some("https://cdn.example/dict.bin"),
            Some(&v2_prefix),
            Some(&vfs),
        ));
        assert_eq!(
            result,
            Some(v2.to_vec()),
            "二次启动应命中刷新后的缓存（sha256 匹配，零网络）"
        );
    }

    /// M-6：read_cache 不固定分配 16MB——增量读到 EOF。
    #[test]
    fn read_cache_incremental_read() {
        let vfs = MemoryVfs::new();
        // 写入超过单 chunk（256KB）的数据，验证增量读。
        let bytes = vec![0xABu8; 300 * 1024];
        write_cache(&vfs, &bytes).unwrap();
        let read = read_cache(&vfs).unwrap();
        assert_eq!(read.len(), 300 * 1024);
        assert!(read.iter().all(|&b| b == 0xAB));
    }
}
