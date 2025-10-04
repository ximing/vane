use super::memory::MemoryVfs;
use super::page_cache::PageCache;
use super::Vfs;

pub fn run_conformance_tests<V: Vfs>(vfs: &V) {
    // create + write_at + read_at
    vfs.create("a.bin").unwrap();
    vfs.write_at("a.bin", b"hello", 0).unwrap();
    let mut buf = [0u8; 5];
    let n = vfs.read_at("a.bin", &mut buf, 0).unwrap();
    assert_eq!(n, 5);
    assert_eq!(&buf, b"hello");

    // append 返回起始 offset
    let off = vfs.append("a.bin", b" world").unwrap();
    assert_eq!(off, 5);
    let mut buf2 = [0u8; 11];
    vfs.read_at("a.bin", &mut buf2, 0).unwrap();
    assert_eq!(&buf2, b"hello world");

    // write_at 覆盖
    vfs.write_at("a.bin", b"HELLO", 0).unwrap();
    let mut buf3 = [0u8; 5];
    vfs.read_at("a.bin", &mut buf3, 0).unwrap();
    assert_eq!(&buf3, b"HELLO");

    // list
    vfs.create("b.bin").unwrap();
    let files = vfs.list(".").unwrap();
    assert!(files.contains(&"a.bin".to_string()));
    assert!(files.contains(&"b.bin".to_string()));

    // rename 原子覆盖
    vfs.create("c.bin").unwrap();
    vfs.write_at("c.bin", b"replaced", 0).unwrap();
    vfs.rename("a.bin", "c.bin").unwrap();
    let mut buf4 = [0u8; 8];
    vfs.read_at("c.bin", &mut buf4, 0).unwrap();
    assert_eq!(&buf4, b"HELLO wo"); // a.bin 的前 8 字节覆盖 c.bin

    // delete
    vfs.delete("c.bin").unwrap();
    assert!(vfs.read_at("c.bin", &mut [0u8; 1], 0).is_err());

    // read 不存在文件报错
    assert!(vfs.read_at("nonexistent", &mut [0u8; 1], 0).is_err());

    // list 按 dir 过滤
    vfs.create("sub/x.bin").unwrap();
    vfs.create("sub/y.bin").unwrap();
    let sub_files = vfs.list("sub").unwrap();
    assert!(sub_files.contains(&"x.bin".to_string()));
    assert!(sub_files.contains(&"y.bin".to_string()));
    // 根目录 list 不含 sub/ 下的文件
    let root_files = vfs.list(".").unwrap();
    assert!(!root_files
        .iter()
        .any(|f| f.contains("x.bin") && f.contains("sub")));
}

#[test]
fn memory_vfs_conformance() {
    let vfs = MemoryVfs::new();
    run_conformance_tests(&vfs);
}

#[cfg(not(target_arch = "wasm32"))]
mod std_fs_tests {
    use super::super::std_fs::StdFsVfs;
    use super::run_conformance_tests;
    use std::path::PathBuf;

    fn tmpdir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vane-vfs-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn std_fs_vfs_conformance() {
        let dir = tmpdir();
        let vfs = StdFsVfs::with_root(dir.to_str().unwrap());
        run_conformance_tests(&vfs);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[test]
fn page_cache_read_through_and_lru_eviction() {
    let vfs = MemoryVfs::new();
    vfs.create("data.bin").unwrap();
    // 写 4 页数据，每页 64 字节（测试用小页）
    let page_size = 64usize;
    let mut data = vec![0u8; page_size * 4];
    for (i, b) in data.iter_mut().enumerate() {
        *b = (i / page_size) as u8; // page 0 = 0, page 1 = 1, ...
    }
    vfs.write_at("data.bin", &data, 0).unwrap();

    // capacity = 2 页
    let mut cache = PageCache::new(page_size * 2, page_size);
    // 读 page 0
    let r0 = cache.read(&vfs, "data.bin", 0, page_size).unwrap();
    assert_eq!(r0[0], 0);
    // 读 page 1
    let r1 = cache
        .read(&vfs, "data.bin", page_size as u64, page_size)
        .unwrap();
    assert_eq!(r1[0], 1);
    // 读 page 2 → 淘汰 page 0
    let r2 = cache
        .read(&vfs, "data.bin", (page_size * 2) as u64, page_size)
        .unwrap();
    assert_eq!(r2[0], 2);
    // 再读 page 0 → 应重新从 vfs 加载（缓存未命中）
    let r0b = cache.read(&vfs, "data.bin", 0, page_size).unwrap();
    assert_eq!(r0b[0], 0);
}

#[test]
fn page_cache_invalidate() {
    let vfs = MemoryVfs::new();
    vfs.create("f.bin").unwrap();
    vfs.write_at("f.bin", &[1, 2, 3], 0).unwrap();
    let mut cache = PageCache::new(1024, 64);
    cache.read(&vfs, "f.bin", 0, 3).unwrap();
    // 修改底层文件后 invalidate
    vfs.write_at("f.bin", &[9, 9, 9], 0).unwrap();
    cache.invalidate("f.bin");
    let r = cache.read(&vfs, "f.bin", 0, 3).unwrap();
    assert_eq!(&r[..3], &[9, 9, 9]);
}

#[test]
fn memory_vfs_sync_is_noop() {
    let vfs = MemoryVfs::new();
    vfs.create("s.bin").unwrap();
    vfs.write_at("s.bin", b"data", 0).unwrap();
    // sync 不报错
    vfs.sync("s.bin").unwrap();
}

#[test]
fn memory_vfs_large_append_across_pages() {
    let vfs = MemoryVfs::new();
    vfs.create("big.bin").unwrap();
    let chunk = vec![42u8; 100_000];
    let off1 = vfs.append("big.bin", &chunk).unwrap();
    assert_eq!(off1, 0);
    let off2 = vfs.append("big.bin", &chunk).unwrap();
    assert_eq!(off2, 100_000);
    // 读回校验
    let mut buf = vec![0u8; 50];
    vfs.read_at("big.bin", &mut buf, 99_990).unwrap();
    assert!(buf.iter().all(|&b| b == 42));
}
