# P1 报告：浏览器 export 快照下载闭环

> 执行：post-M2 P1。原 SubAgent 完成代码后进程中断；自证由编排者直接跑（SubAgent 连续 2 次失败换策略）。

## 诊断
M2-12 `Db::export(dest)` 写快照到 VFS 路径（VANE_SNAP 单文件）。M2-14 VaneWorker.export 只写 OPFS/overlay 容器内虚拟路径，用户无法把快照下载到本地。缺：读回容器内文件字节 → 浏览器下载闭环。

## 实现
1. **`crates/vane-wasm/src/worker.rs`**：
   - `read_file_sync(&self, path) -> Result<Vec<u8>, VaneError>`：流式 `vfs.read_at`（8KB buf）直到 EOF（n==0）。读 `inner.vfs`（与 Db 共享同一 VFS）。文件不存在 → `VaneError::Io`；close 后 → check_open 拒绝（I-7）。
   - `#[wasm_bindgen(js_name = readFile)] pub fn read_file(&self, path) -> js_sys::Promise`：返 `Uint8Array` Promise（成功）/ reject（失败）。
   - 2 单元测试：`read_file_errors_on_missing_and_closed`（不存在路径 + close 后 Err）、`export_then_read_file_roundtrip`（export→readFile 字节以 VANE_SNAP 魔数起始）。
2. **`demo/worker.js`**：`case "readFile"` → `worker.readFile(msg.path ?? "backup.vane")` 返 Uint8Array。
3. **`demo/main.js`** `exportBackup()`：`export(dest)` → `readFile(dest)` → `new Blob([bytes], {type:"application/octet-stream"})` → `<a download=dest>` click → revokeObjectURL。

## 自证（编排者直接跑）
| 项 | 结果 |
|---|---|
| `cargo check --target wasm32-unknown-unknown -p vane-wasm` | ✅ Finished |
| `cargo test -p vane-wasm` | ✅ 41 passed; 0 failed（lib unittests 含 readFile 2 测试） |
| `cargo clippy -p vane-wasm --all-features -- -D warnings` | ✅ Finished 无警告 |
| wasm 体积（release，未 wasm-opt） | 1.4M raw / **391 KB gzip** ≤800KB 红线 ✅ |
| demo 协议一致性 | ✅ main.js `call("readFile",{path})` → worker.js `case "readFile"` → worker.rs `js_name=readFile` |

## 约束遵守
- 未改 M0/M1/M2 冻结 pub API（`Db::export` 不动）。
- 未触 core std::fs/mmap；readFile 在 vane-wasm binding 层（I-5 OK）。
- 未引入新依赖（js_sys::Uint8Array 已有）。
- 未触及 SPEC（P0-3 闭环是 worker 协议扩展，非 core API）。

## 遗留
- 浏览器端真实下载需手动/E2E 验证（本地 cargo test 验的是 worker.rs 层 round-trip + 魔数；demo JS 下载流程逻辑自审正确，待 P2 Playwright 或手动验收）。
- wasm 体积 391KB gzip 为未 wasm-opt 数；CI check-wasm-size.sh 会过（wasm-opt -Oz 后更小）。
