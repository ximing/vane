# post-M2 收尾与 v0.1.0 发版总结

> 2026-08-10。post-M2 目标：建仓推送 → CI 跑通修复 → 产品化收尾 → 发版 v0.1.0（三端 prebuilt）。全部达成。

## 目标达成
M2 功能闭环后，补上"远程 CI 验证 → 可信交付 → 正式发版"一脚。仓库从无 remote、CI 从未远程跑，到 v0.1.0 三端 prebuilt 正式发布。

## 交付清单

### 阶段零：CI 预审 + 建仓
- **Z0** CI 静态预审：1 必炸（go-cross zig 版本）+ 14 隐患
- **Z1** 预修：go-cross pin zig 0.15.2 + cargo-zigbuild 0.23.0；permissions/rust-cache/timeout/concurrency/xxd 等 8 项
- **Z2** release.yml 确认：Node 就绪，用户决策补全三端
- **Z3** 建仓：https://github.com/ximing/vane（private 起步 → 发版前转 public）

### 阶段一：CI 修复循环
- CI 首跑 16 jobs：14 绿 + go-cross 4 失败 + cold-start cancelled
- **1 轮修复**：go-cross `--target` 改 Rust triple（matrix.target）+ cold-start 加 `--release`
- run 31366858024 全绿，"本地绿→可信交付"达成

### 阶段二：产品化收尾
- **P1** 浏览器 export 下载闭环：worker.rs `readFile` op（流式 vfs.read_at + wasm_bindgen Uint8Array + 2 测试）+ demo Blob 下载。CI 绿。
- **P2** 5万 Playwright CI：用户决策跳过（不做浏览器验收）
- **P3** 词典 CDN jsdelivr gh：demo DICT_URL + check-dict-hash.sh Go 渠道启用（三层校验：源字节 sha256 + DICT_VERSION + zstd 头部 prefix，全绿）
- **Z2-补** Go .a + WASM 双产物发布：release.yml build-go（zig 交叉 4 平台）+ build-wasm（双变体）+ release job（softprops GitHub Release assets）

### 阶段三：发版 v0.1.0
- **R1** version 0.1.0 三端同步 + go.mod module 路径 vane/vane→ximing/vane
- **R1b** npm scope @vane/node→@vane-rs/node（用户 scope 是 @vane-rs）
- **R2** napi-rs 发布流程（6 轮调试，见下）
- **R3** 发版说明（引 M2-SUMMARY）+ 三渠道词典哈希（P3 已验证）

## 发版产物 v0.1.0
- **npm** `@vane-rs/node@0.1.0` + 4 平台包（linux-x64-gnu / darwin-arm64 / darwin-x64 / win32-x64-msvc）
- **GitHub Release v0.1.0**：Go `libvane_ffi-<platform>.a`×4 + WASM `vane_wasm_simd.wasm`/`vane_wasm_scalar.wasm` + .node×4
- **词典 CDN**：jsdelivr gh（dict.bin），三渠道哈希一致（Node include_bytes / Go embed / WASM CDN fetch）

## CI
16 jobs 远程持续全绿（fmt/clippy/test/recall/wasm32-check/deny/corpus/cold-start/wasm32-size/dict-size/dict-hash/go-host/go-cross/wasm-recall/jieba-compat/ndcg-wiki）。

## napi-rs 发布流程调试历程（6 轮）
M0 napi-rs 用法从未 CI 验证，release.yml 暴露连环兼容问题：
1. `napi artifacts --target` 不支持（2.x/3.x 均无）
2. build/release 架构重构（per-platform upload .node / release job 聚合 artifacts）
3. `triples`→`targets`（3.8.5 配置字段）
4. pin `@napi-rs/cli@^2.18.0`（与 devDependencies 对齐，2.x/3.8.5 版本不匹配是根因）
5. **官方流程**（napi.rs/docs/deep-dive/release）：build upload .node → release job `napi create-npm-dir` + `napi artifacts --dir .` + `npm publish`（prepublishOnly 发平台包）
6. `publishConfig.access: public`（scoped 包公开发布，修 E402）

最终正解：2.x 命令 + 官方流程 + publishConfig.access=public。package.json napi 双字段（triples 2.x 必读 + targets 3.x 兼容）。

## 遗留
- **P2 5万 Playwright 浏览器 CI**：用户跳过，post-发版可选
- **napi-rs 2.x 锁定**：3.x 未跟进，未来升级需重测发布流程
- **f32 距离 SIMD 未向量化**（M2-05 遗留）：post-发版 trait Distance SPEC 修订
- **百万规模 #[ignore] CI heavy job**：可选增强
- **双变体体积门禁**：可选增强

## 关键裁决
- OPFS headless Playwright 风险 → 用户跳过 P2
- napi-rs 3.8.5 深坑 → 回退 2.x（项目原版本）+ 官方流程
- 转 public 时机：CI 持续绿后发版前
- npm scope @vane-rs（用户决策，非 @vane）

## 指标
- CI 16 jobs 远程绿（持续 5+ run）
- 三端 prebuilt（Node npm 4 平台 + Go .a 4 平台 + WASM 双变体）
- 词典三渠道哈希一致
- v0.1.0 npm + GitHub Release published

详见 `docs/plans/m2/M2-SUMMARY.md` + `docs/plans/post-m2/EXECUTION-NOTES.md`。
