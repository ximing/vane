use super::memory::MemoryVfs;
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
    assert!(!root_files.iter().any(|f| f.contains("x.bin") && f.contains("sub")));
}

#[test]
fn memory_vfs_conformance() {
    let vfs = MemoryVfs::new();
    run_conformance_tests(&vfs);
}
