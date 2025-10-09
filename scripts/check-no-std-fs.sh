#!/usr/bin/env bash
set -euo pipefail
# §13.3: core 生产代码禁 std::fs/std::net/mmap，唯一例外 vfs/std_fs.rs（cfg 隔离，合法）。
#
# 稳健化设计：
# - 用 `--exclude='tests.rs'` 排除所有名为 tests.rs 的文件：测试夹具（cfg(test)）
#   用 std::fs::remove_dir_all / create_dir_all 搭建临时目录属合法测试基建，不计入。
# - 模式用 `std::fs::`（带 `::`）匹配实际调用形态，避免注释里 "std::fs。" 这类
#   字面提及造成假阳性；同理 `std::net::`。`mmap` 保持字面匹配。
# - 仍保留对 vfs/std_fs.rs 的排除（生产代码但合法使用，cfg 隔离）。
# - 生产 mod.rs 若出现 `std::fs::` 仍会命中失败。
if grep -rn --include='*.rs' --exclude='tests.rs' 'std::fs::\|std::net::\|mmap' crates/vane-core/src/ \
    | grep -v 'crates/vane-core/src/vfs/std_fs.rs'; then
    echo "FAIL: forbidden IO usage outside vfs/std_fs.rs (tests.rs fixtures excluded)" >&2
    exit 1
fi
echo "OK"
