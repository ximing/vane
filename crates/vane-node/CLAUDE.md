# `vane-node` 指令

## 职责

- 此 crate 通过 napi-rs 直接适配 `vane-core`，并配套 JavaScript/TypeScript 入口和平台包分发配置。
- 保持绑定层薄：只做 N-API 生命周期、异步调度与 JSON/JS 值转换；不得复制检索、持久化或分词业务逻辑。

## 约束

- 对外参数名、错误码和可观察行为必须与 `docs/SPEC.md` 的语言无关 IDL 以及其他绑定一致。
- 异步使用 napi-rs 既有机制，不引入 tokio 或第二套线程模型。
- 改动 Rust 导出时同步检查 `index.js`、`main.js`、声明文件和 `__tests__/` 中的契约；不手改二进制 `.node` 产物。

## 验证

- Rust 改动至少运行目标 crate 的 Cargo 检查/测试。
- JS 行为改动运行 `npm test`；分发或体积改动还运行 `npm run check:thin`。
