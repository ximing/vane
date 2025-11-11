# M2-11 Go cgo 绑定——评审报告

**评审对象**：vane-ffi C ABI 实装（`crates/vane-ffi/src/lib.rs` 1070 行）+ 手写 `bindings/go/vane.h` + `bindings/go/vane.go` cgo 包装 + `bindings/go/dict/` embed + wazero 骨架 + host demo + CI matrix + vane-core `Db::set_jieba_dict` 最小改动。

**评审范围**：只读，不跑 cargo。diff BASE dc8e296..HEAD ff266a7。

**判定**：**PASS_WITH_FINDINGS**（1 Blocker / 4 Issue / 3 Minor）

---

## B-1（Blocker）extern "C" 函数全部缺 `catch_unwind`，panic 跨 FFI 边界

**证据**：
- 全仓库零 `catch_unwind`（`grep -rn "catch_unwind" crates/` 无命中）。
- 无 `panic = "abort"`（`grep` Cargo.toml 无命中），edition 2021，无 `rust-toolchain.toml`（未钉工具链）。
- 9 处 `.unwrap()` 直接落在 extern "C" 函数体或其同步调用链上，poison 即 panic：
  - `crates/vane-ffi/src/lib.rs:84` `alloc_handle` → `REGISTRY.write().unwrap()`（被 `vane_open`/`vane_collection`/`vane_reindex` 同步调用）
  - `lib.rs:91` `lookup_db`、`lib.rs:99` `lookup_col`、`lib.rs:110` `with_reindex_handle`、`lib.rs:117` `remove_handle`（`vane_close`）
  - `lib.rs:170` `arena_alloc_tracked` → `ARENA_LAYOUTS.write().unwrap()`（`vane_search`/`vane_dict_version`）
  - `lib.rs:735` `vane_load_dict` → `*DICT_VERSION_INFO.write().unwrap()`
  - `lib.rs:746` `vane_dict_version` → `DICT_VERSION_INFO.read().unwrap()`
  - `lib.rs:836` `vane_string_free` → `ARENA_LAYOUTS.write().unwrap()`
- 间接 panic 向量：每个 extern "C" 入口同步调用 vane-core（`Db::open`/`col.search`/`col.add`/`col.reindex`/`rh.wait` 等），core 内部任意 `.unwrap()`/索引越界/算术溢出都会沿栈 unwind 直冲 extern "C" 边界。

**判定**：
- Rust < 1.81（未钉工具链，贡献者可能使用）：panic 跨 `extern "C"` = **UB**（运行时打印 "Rust panic in extern C function" 后 abort，但语言层面为 UB）。
- Rust >= 1.81（2026 默认 stable）：`extern "C"` 定义为 abort-on-unwind，非 UB，但**直接 crash 宿主 Go 进程**（嵌入式库不可接受，DoS 级缺陷）。
- 任务约定「panic 跨 FFI = UB 是阻塞级」；且工程标准实践是每个 `extern "C"` 入口包 `std::panic::catch_unwind`（返回 E_UNSUPPORTED/-11 或新设 E_PANIC）。

**修复建议**：每个 `#[no_mangle] extern "C"` 函数体包 `catch_unwind`；对 RwLock `.unwrap()` 改 `map_err(|_| VaneError::InvalidArg("lock poisoned".into()))` 显式降级为错误码，避免 poison 连锁。

---

## I-1（Issue）Go 侧 thread-local 错误丢失：goroutine 迁移导致空错误消息

**证据**：
- `crates/vane-ffi/src/lib.rs:127-140`：错误存 `thread_local! LAST_ERROR`。
- `lib.rs:806-823` `vane_last_error_message` 读 thread-local。
- `bindings/go/vane.go:146-161` `checkError` → `lastErrorMessage` → `C.vane_last_error_message`。Go 未 `runtime.LockOSThread`。

**问题**：cgo 调用之间 Go 调度器可在 safepoint 把 goroutine 迁到另一 OS 线程。`vane_flush`（线程 X 设错误）返回后 goroutine 可能被迁到线程 Y，`vane_last_error_message` 在线程 Y 读到空 → Go 侧 `Message` 为空串。错误码（负 i32）仍正确，仅描述丢失；非 UB，但严重损害可调试性，且为间歇性（单线程测试不触发，生产高并发才暴露）。

**修复建议**：任选其一——(a) Go 侧 `checkError` 前 `runtime.LockOSThread()`/`UnlockOSThread()`；(b) FFI 侧改 per-handle 错误（`RegistryEntry` 内嵌 `Mutex<String>`，`vane_last_error_message(handle)` 用 handle 取，`_handle` 参数本就存在但目前未用）。

## I-2（Issue）`vane_collection` 签名偏离 M1 README §09 契约

**证据**：
- `docs/plans/m1/README.md:471`：`vane_collection(db_h, name, name_len, schema_json, schema_len, out_handle) -> i32`（6 参数，无 opts）。
- `crates/vane-ffi/src/lib.rs:454-463` + `bindings/go/vane.h:35-39`：8 参数（增 `opts_json`/`opts_len`）。
- M2-11 计划 `docs/plans/m2/modules/M2-11-go-cgo-binding.md:37` 显式扩展，但 M1 README 标注为「单一事实源」「唯一沟通渠道」。

**问题**：契约漂移——M1 README §09 未同步更新。下游任何按 M1 §09 写的 C 消费者（非 Go）会签名错配。

**修复建议**：回写 M1 README §09（或 SPEC §9）补充 `opts_json`/`opts_len` 参数，或在报告显式标注「M2-11 契约扩展」并通知编排者登记 SPEC 修订。

## I-3（Issue）collection 创建时 jieba_dict TOCTOU，潜在 I-4 违反

**证据**：`crates/vane-core/src/api/collection.rs`（M2-11 diff）：
```
+let dict_guard = db.jieba_dict.read().unwrap();   // line 155：读 dict A 建 tokenizer
 build_collection_tokenizer(dict_guard.as_ref(), ...)
 ...
 jieba_dict: db.jieba_dict.read().unwrap().clone(), // line 191：再读，可能已是 dict B
```
- M2-11 将 `DbInner.jieba_dict` 从 `Option<Arc<JiebaDict>>` 改为 `RwLock<Option<Arc<JiebaDict>>>`（`db.rs:33`），使运行时 `set_jieba_dict` 可并发改写。
- 两次 `read()` 之间若另一线程调 `Db::set_jieba_dict`，则 tokenizer 用 dict A 构建，而 `CollectionInner.jieba_dict` 存 dict B。`CollectionInner.jieba_dict` 供 reindex 重建分词器（M1 06 计划），若 reindex 用 dict B 重建，则与建库时 dict A 的分词身份不一致 → 违反 I-4（单一分词身份，reindex 应同身份或经 set_user_dict 显式切换）。

**修复建议**：`create_new` 内一次 `read()` 锁定 dict，同时用于 tokenizer 构建与 clone 存储；或 `set_jieba_dict` 拒绝在有活跃 collection 后调用。

## I-4（Issue）`vane_reindex_wait` 持注册表读锁阻塞，阻塞所有 handle 写操作

**证据**：`crates/vane-ffi/src/lib.rs:109-114`
```rust
fn with_reindex_handle<R>(h: u64, f: impl FnOnce(&ReindexHandle) -> R) -> Option<R> {
    let reg = REGISTRY.read().unwrap();          // 读锁持整段
    reg.as_ref().and_then(|m| m.get(&h))
        .and_then(|e| e.reindex.as_ref().map(f))  // f = rh.wait() 阻塞
}
```
- `vane_reindex_wait`（`lib.rs:706`）调用上述闭包，`rh.wait()` 可能阻塞秒级。
- 期间任何 `vane_open`/`vane_close`/`vane_collection`/`vane_reindex`（需写锁）在其他线程全部阻塞。非死锁（读锁不互斥读），但 Go host 多 goroutine 场景下整体停顿。

**修复建议**：`with_reindex_handle` 在锁内 clone `ReindexHandle`（若 ReindexHandle 可 Clone）或克隆其内部 Arc 后释放锁，再在锁外调 `wait()`。若 ReindexHandle 不暴露 Clone，考虑注册表存 `Arc<ReindexHandle>`（需 ReindexHandle 内部 Arc 化）。

---

## M-1（Minor）TDD 缺口：reindex / load_dict / dict_version 成功路径无 Rust FFI 测试

**证据**：`crates/vane-ffi/src/lib.rs:848-1069` 7 个测试，无 `vane_reindex`/`vane_reindex_progress`/`vane_reindex_wait` 测试；`vane_load_dict` 无测试（dict.bin 需 dict-zh feature）；`vane_dict_version` 仅测 unavailable 路径（`lib.rs:1015`）。计划 §4 item 8/9/10 要求。

**建议**：补 reindex 三函数 roundtrip 测试（小 schema + add + reindex + progress + wait）；若 Rust 测试不便依赖 dict.bin，至少在 Go 侧（已 embed）补 load_dict + dict_version 成功路径测试。

## M-2（Minor）无 panic 跨边界测试

**证据**：无测试验证「注入畸形 JSON / null out_arena / 句柄误用」等场景不触发 abort。B-1 修复后应补 panic 注入测试（构造 core panic，断言 extern "C" 返错误码非 abort）。

## M-3（Minor）`vane_collection` 注释误导

**证据**：`crates/vane-ffi/src/lib.rs:500-504` 注释讨论「Collection impl Clone → 存 Collection」「Arc 不行」，但实际代码 `Arc::new(col)` 正确存 `Arc<Collection>`。代码对，注释矛盾，易误导后续维护。建议清理。

---

## 逐项核查结论

| 评审点 | 结论 | 证据 |
|---|---|---|
| I-7 句柄注销后使用 | ✅ 非 UB | `lib.rs:116` remove_handle；lookup 返 None → E_NOT_FOUND。测试 `lib.rs:882` 守护 |
| I-7 Arc clone 在 read lock 内 | ✅ 安全 | `lib.rs:89-96` clone 后 guard 释 |
| I-7 arena layout 匹配 | ✅ | `lib.rs:158-176` 存 Layout，`lib.rs:832-842` 查 Layout 后 dealloc |
| I-7 double-free 防护 | ✅ | `map.remove` 后第二次查无 → no-op |
| I-7 null 返回处理 | ✅ | `arena_alloc_tracked` 返 null → `vane_search`/`vane_dict_version` 返错误码 |
| I-7 跨边界只借不还 | ✅ | `slice_from_raw` 仅调用期间借用，Rust 不持有 |
| I-7 search arena 一次 free | ✅ | `lib.rs:590` alloc，Go `vane.go:263` free |
| null 指针防御 | ✅ | 所有 out_* 入口检 null（`lib.rs:420,464,565,612,668,692,743`）；输入 null 经 `slice_from_raw`→空切片→serde 报错 |
| **panic 安全** | ❌ 见 B-1 | 无 catch_unwind |
| M1 §09 契约对齐 | ⚠️ 见 I-2 | vane_collection 偏离 |
| 返回 i32 错误码 §10 | ✅ | 所有函数返 i32；vane.h int32_t |
| set_jieba_dict additive | ✅ | `db.rs:189` pub 新增，不改现有签名 |
| set_jieba_dict 线程安全 | ✅ RwLock | `db.rs:190` |
| set_jieba_dict I-4 影响 | ⚠️ 见 I-3 | 已建 collection 不受影响（注释正确），但 create_new 内 TOCTOU |
| set_jieba_dict 与 reindex 状态机 | ✅ 不冲突 | set_jieba_dict 不触 reindex；reindex 用 collection 固定 tokenizer |
| vane.h 与 extern "C" 逐字一致 | ✅ | 16 函数签名/返回/长度参数全对齐（I-2 的 opts 扩展除外，impl 与 vane.h 一致） |
| Go cgo 薄壳 I-8 | ✅ | `vane.go` 仅 JSON 序列化 + 参数搬运，无检索逻辑 |
| Go 错误码映射 | ✅ | `vane.go:33-46` 错误码常量 + `checkError` |
| Go 内存释放 defer free | ✅ | `vane.go:263,343` Search/DictVersion 立即 free |
| dict embed <2MB | ✅ | `dict.bin.gz` 1,477,876 bytes = 1.41MB |
| go:embed + vane_nodict + DictVersion + LoadDict | ✅ | `dict.go`/`dict_nodict.go` 齐全 |
| 三渠道版本一致 | ✅（文档级） | 同源 `crates/vane-dict-zh/data/dict.bin`；⚠️ 未逐字节比对，按报告采信 |
| wazero 骨架 | ✅（骨架） | `bindings/go/wazero/` API 对齐 + ErrWazeroNotImplemented，实装留 M2 后续 |
| CI go-host + go-cross | ✅（配置） | `.github/workflows/ci.yml:172,199`；⚠️ zig 本地不可用，4 平台交叉仅远程 CI 可验证 |
| 黑名单合规 | ✅ | 用 `std::sync::RwLock`，无 dashmap/parking_lot；cbindgen 未引入（手写 vane.h） |
| 其他不变量 | 无回归 | I-1~I-6/I-8 不受 vane-ffi 影响 |

---

## 总结

M2-11 实装完整、结构清晰，I-7 内存铁律（句柄注册表/arena tracked Layout/null 防御/double-free 防护）落实到位，vane.h 与 extern "C" 签名逐字对齐，Go cgo 薄壳合规，dict embed 体积达标。vane-core `set_jieba_dict` 改动 additive 且不改现有 pub API。

**阻塞项**为 B-1：全部 extern "C" 入口缺 `catch_unwind`，panic 跨 FFI 在未钉工具链（<1.81）下为 UB、在 >=1.81 下为 abort（crash 宿主）。9 处锁 `.unwrap()` + vane-core 间接 panic 均为真实向量。须补 catch_unwind 后方可合并。

I-1（Go thread-local 错误丢失）、I-3（jieba_dict TOCTOU 潜在 I-4 违反）为合并前建议修复的 Issue；I-2/I-4 可接受但建议登记。TDD 缺 reindex/load_dict 成功路径测试（M-1）。
