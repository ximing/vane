# M2-01 vane-wasm cdylib + 体积门禁

## 1. 目标
在 M2-00 vane-wasm 骨架（仅 `vane_version()` 占位）基础上，加入真实检索/管理 API 的 wasm-bindgen 胶水 + SIMD 探针占位，CI wasm32-size job 切到 vane-wasm 真实 deliverable 口径，强制 800KB gzip 门禁（SPEC §12.1/§12.2/§13.2-3）。

SPEC 节号：§12.1（workspace vane-wasm）、§12.2（wasm32 双变体目标矩阵）、§13.2-3（核心 wasm ≤800KB）。

## 2. 涉及文件
- **Modify** `crates/vane-wasm/Cargo.toml`：增 feature 开关（`opfs`/`idb`/`worker` 占位，M2-02/03/04 启用）；增 `web-sys`/`js-sys` optional dep（feature-gated，本模块仅引入最小 subset，不启用浏览器 API）。
- **Modify** `crates/vane-wasm/src/lib.rs`（M2-00 占位 `vane_version()`）：新增 wasm-bindgen 导出胶水——`vane_open`/`vane_collection`/`vane_add`/`vane_flush`/`vane_search`/`vane_delete`/`vane_compact`/`vane_reindex`/`vane_export`/`vane_close`，内部调 `vane_core::api`（薄壳，I-8）。
- **Modify** `crates/vane-wasm/src/simd_probe.rs`（Create）：SIMD 探针占位（`simd128_supported()` 返回 `false`，M2-05 落实 `WebAssembly.validate`）。
- **Modify** `.github/workflows/ci.yml`（`wasm32-size` job，line 115-128 区间）：增加 `cargo build --target wasm32-unknown-unknown -p vane-wasm --release` + 体积测量；保留 vane-core `--export-all` 上界对照。
- **Modify** `scripts/check-wasm-size.sh`：新增 vane-wasm default 构建测量分支（真实 deliverable 口径）。

## 3. 接口契约
### Consumes from
- M0/M1 `vane_core::api`：`Db::open(vfs: Arc<dyn Vfs>, path: &str, opts: OpenOptions) -> Result<Db>`（`api/db.rs:35`，首参 `vfs: Arc<dyn Vfs>`，reviewer A-I1）、`Db::collection`、`Db::export(dest)`（`api/db.rs:164`）、`Db::close()`（`api/db.rs:168`）、`Collection::{add,flush,search,delete,compact,reindex}`（`api/collection.rs`，**Collection 无 export/close**，reviewer A-I1）。
- M2-00 vane-wasm 骨架：`vane_version()` 占位（`crates/vane-wasm/src/lib.rs`）。

### Produces for
- `#[wasm_bindgen]` 导出胶水（见 README M2-01 节，签名同 vane-core api 但返 `Promise`/`JsValue`）。下游 M2-02/03/04 在此 crate 加 feature-gated 模块。
- `simd_probe::simd128_supported() -> bool`（M2-04/M2-05 消费）。

## 4. TDD 测试清单
1. `cargo check --target wasm32-unknown-unknown -p vane-wasm` 编译通过（wasm32 可达，无 std::fs）。
2. `grep -rn 'std::fs::\|std::net::\|mmap' crates/vane-wasm/src/` 无输出（`scripts/check-no-std-fs.sh` 扩展覆盖 vane-wasm）。
3. **体积门禁**：`cargo build --release --target wasm32-unknown-unknown -p vane-wasm` → `wasm-opt -Oz` → `gzip | wc -c` ≤ 800KB（SPEC §13.2-3）。记录 default 与 `--export-all` 两口径数值。
4. **体积回归基线**：相比 M2-00 骨架（default 9.46KB），加入真实 API 后 default 体积上升——断言上升后仍 ≤800KB，且 `--export-all` 上界 ≤800KB。
5. **胶水薄壳行为**（JS 侧行为测试，`crates/vane-wasm/tests/` 或 node wasm 尾）：`vane_open(MemoryVfs 路径)` → `vane_collection` → `vane_add` → `vane_flush` → `vane_search` 返回 Hit 数组，与 vane-core 等价（I-8 binding 薄壳）。
6. `simd_probe::simd128_supported()` 占位返回 `false`（M2-05 落实真实探针）。
7. CI `wasm32-size` job 跑 vane-wasm 测量脚本，失败即阻断 PR。

## 5. 验收标准
- vane-wasm default gzip ≤800KB，`--export-all` gzip ≤800KB。
- `cargo check --target wasm32-unknown-unknown -p vane-wasm` + `-p vane-core` 双通过。
- `check-no-std-fs.sh` 覆盖 vane-wasm，无 std::fs/std::net/mmap。
- clippy `--target wasm32-unknown-unknown -p vane-wasm` clean。
- cargo deny check ok（wasm-bindgen/web-sys/js-sys 不在黑名单，license 兼容）。
- JS 侧行为测试：open→collection→add→flush→search 端到端通过（MemoryVfs）。

## 6. 前置依赖
- M2-00 vane-wasm 骨架（已完成）。

## 7. 不变量覆盖
- **I-5**：vane-wasm 非 core 算法；vane-wasm 内 `cfg(target)` 允许（VFS/binding 层），但本模块不引入平台分支（仅 feature-gated 模块占位）。测试 1+2 守护。
- **I-8 binding 薄壳**：wasm-bindgen 胶水无检索逻辑，行为测试在 core。测试 5 守护。
- **词典永不进 wasm**：vane-wasm default features 不启 `dict-zh`（红线，捆绑词典数据）；`jieba` feature（仅算法代码）可在非 default 启用须过 800KB 门禁实测（Cargo.toml 验证，README 全局约束表已放宽）。体积门禁测试 3 守护。
