#!/usr/bin/env bash
set -euo pipefail
# §13.3: core 生产代码禁 std::fs/std::net/mmap，唯一例外 vfs/std_fs.rs。
# test fixture（cfg(test)）使用 std::fs 搭建临时目录属合法测试基建，不计入。
if grep -rn --include='*.rs' 'std::fs\|std::net\|mmap' crates/vane-core/src/ \
    | grep -v 'crates/vane-core/src/vfs/std_fs.rs' \
    | grep -v 'crates/vane-core/src/vfs/tests.rs'; then
    echo "FAIL: forbidden IO usage outside vfs/std_fs.rs (and test fixtures)" >&2
    exit 1
fi
echo "OK"
