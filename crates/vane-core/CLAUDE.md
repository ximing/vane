# `vane-core` 指令

## 职责

- 这里是检索引擎的唯一业务实现：公开 API、分词、BM25、向量/HNSW、融合、段、持久化、VFS、WAL 与执行器。
- 绑定 crate 只能调用这里的 API；不要把 Node、Go、浏览器胶水或语言特定序列化放入此目录。

## 核心约束

- 保持平台中立：不得引入 `std::fs`、`std::net`、mmap 或绑定层依赖；I/O 必须经 `Vfs` trait，平台并发差异必须经 `Executor`。
- 段文件、manifest、WAL、分词器身份与错误语义属于兼容性协议。改动前核对 `docs/SPEC.md`，格式变更必须包含版本/迁移或兼容读取与回归覆盖。
- 公开 API 保持三端同构；不要让绑定专用需求泄漏为核心业务逻辑。
- 算法模块保持职责单一，优先复用现有 `types` 和错误类型，而不是创建平行表示。

## 验证

- 先运行受影响模块的测试，再按需要运行 `cargo test -p vane-core`。
- 涉及平台可编译性时运行 `cargo check --target wasm32-unknown-unknown -p vane-core`；涉及持久化、召回或分词时补充对应集成回归测试。
