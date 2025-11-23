# Go 绑定指令

## 职责

- 此目录为 `vane-ffi` 的 cgo 消费端：提供惯用 Go API、C ABI 参数转换、平台静态库链接和可选 wazero 实现。
- C ABI 和核心检索行为不在此重写；行为差异应回到 `vane-core` 或 `vane-ffi` 修复。

## 约束

- 默认 cgo 路径与 `wazero` build tag 是独立实现边界；修改公共 API 时同时核对构建标签、`vane.h` 和两条路径的可用性。
- FFI 调用后读取 thread-local 错误信息必须维持在同一 OS 线程；不得移除现有的线程固定保护。
- 新增 Go 文件遵循 `gofmt`，不要把本地构建生成的静态库当作源代码更新。

## 验证

- 需要 cgo 的改动先执行 `cargo build --release -p vane-ffi`，再运行 `go build ./...` 和 `go test ./...`。
- 变更 wazero 路径时额外以 `-tags wazero` 验证对应包。
