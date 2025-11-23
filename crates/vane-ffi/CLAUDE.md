# `vane-ffi` 指令

## 职责

- 此 crate 将 `vane-core` 的公开能力暴露为稳定 C ABI，供 Go cgo 等消费者链接。
- 仅负责句柄管理、字节/JSON 转换、错误边界与 ABI 内存所有权；检索、持久化和分词业务逻辑必须留在 `vane-core`。

## ABI 约束

- 保持不透明 `uint64_t` 句柄、状态码与 `vane_last_error_message()` 的既有契约。跨边界内存遵循“分配方负责释放”，避免把 Rust 所有权转交给调用方。
- 改动导出函数、错误码或参数布局时，同时核对 `bindings/go/vane.h`、Go 包与 `docs/SPEC.md` 的 IDL；这是兼容性变更，必须有测试覆盖。
- 不为单一消费者增加业务分支。消费者专属适配应放在对应绑定目录。

## 验证

- 运行 `cargo test -p vane-ffi` 及受影响的核心测试。
- 改动静态库或头文件契约时，构建 `cargo build --release -p vane-ffi` 并执行 Go 绑定的 build/test 验证。
