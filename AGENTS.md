# Vane 项目指令

## 项目概览

- Vane 是以 Rust 为核心的嵌入式混合检索库：向量检索、BM25 与 RRF 融合。
- Cargo workspace 包含 `vane-core`、C ABI (`vane-ffi`)、Node N-API (`vane-node`)、浏览器 WASM (`vane-wasm`) 和中文词典包。
- `bindings/go/` 通过 cgo 链接 `vane-ffi`；`demo/` 与 `examples/` 是消费端示例，不是核心实现位置。
- `docs/REQUIREMENTS.md` 定义需求合同，`docs/SPEC.md` 定义精确接口、格式和门禁；实现不得静默偏离它们。

## 全局规则

- 保持依赖方向：绑定层可以依赖 `vane-core`，但 `vane-core` 不得依赖绑定层或平台专属实现。
- 优先在现有模块和测试中做最小、可验证的改动；不要提交构建产物或临时文件。
- 公共 API、持久化格式、错误码和跨语言行为是兼容性边界。修改前核对 `docs/SPEC.md`；若需偏离，先同步更新规范及对应测试。
- Rust 遵循 `rustfmt.toml`，并保持 Clippy 在 `-D warnings` 下通过。新增依赖需符合 CI 的依赖黑名单与 WASM 体积约束。

## 常用验证入口

- 格式：`cargo fmt --all -- --check`
- 静态检查：`cargo clippy --all-targets --all-features -- -D warnings`
- 工作区测试：`cargo test --workspace --all-features`
- WASM 基线：`cargo check --target wasm32-unknown-unknown -p vane-core` 与 `cargo check --target wasm32-unknown-unknown -p vane-wasm`
- Node 绑定测试：`cd crates/vane-node && npm test`
- Go 绑定验证：先构建 `vane-ffi`，再执行 `cd bindings/go && go test ./...`

## 局部规则导航

- `crates/vane-core/CLAUDE.md`：核心引擎、存储格式与跨平台边界。
- `crates/vane-wasm/CLAUDE.md`：浏览器/WASM 胶水、Worker 与 VFS 后端。
- `crates/vane-node/CLAUDE.md`：N-API 薄绑定与 JS 契约。
- `bindings/go/CLAUDE.md`：cgo 静态链接、Go API 与构建标签。
- `docs/CLAUDE.md`：需求合同、技术规范与里程碑记录。
- `.claude/rules/rust-tests.md`：Rust 测试文件的横切规则。
