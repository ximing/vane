# M0 执行笔记（编排者维护）

> 记录执行阶段发现的计划偏离、裁决、跨计划副作用，防上下文压缩丢失。
> 每条标注来源计划、裁决依据、影响范围。

---

## L0 · 00-workspace（进行中）

### N1. ulid 禁用默认 feature（wasm32 门禁）
- **现象**：`ulid = "1"` 默认拉 rand→getrandom，getrandom 拒绝在 wasm32-unknown-unknown 编译，§13.3 门禁失败。
- **裁决**：根 Cargo.toml `ulid = { version = "1", default-features = false }`（默认 feature 仅 "rand"，禁用后编码 API 仍可用）。
- **依据**：SPEC §13.3 wasm32 check 为 M0 第一天硬门禁；ULID 仅用于段目录命名，不需密码学随机。
- **副作用 → 04-segment-format**：`gen_ulid()` 不能用 `Ulid::new()`（需 rand feature），改用 `Ulid::from_parts(timestamp_ms, counter_as_u128)`，计数器替代随机位。**L2 派发 04 时必须告知 implementer。**

### N2. text_fields() 返回 Vec<&str>
- **现象**：00 计划正文与 README 全局契约不一致（计划 Vec<String>，契约 Vec<&str>）。
- **裁决**：统一为 `Vec<&str>`，对齐 README 单一事实源 + vector_field() 的借用风格。
- **影响**：下游 05/07 按 Vec<&str> 消费。

### N3. DIM_MAX 定义顺序
- 00 计划 Task 4 引用 DIM_MAX 但 Task 5 才定义，implementer 提前到 Task 4。最终常量集一致，非问题。

---

## L1 · 待派发（01-vfs / 02-tokenizer / 03-fusion / 06-vector-brute 四路并行）

### 派发前确认
- 00 已提交且 wasm32 check 通过后，L1 四路用 isolation: worktree 并行。
- 各计划不改 lib.rs/Cargo.toml（B1 裁决），仅填各自模块文件。
