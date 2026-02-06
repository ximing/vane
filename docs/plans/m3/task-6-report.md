# Task 6 报告：扩展词典哈希校验至四渠道

## 状态
✅ 完成

## Commits
- `d750558` feat(dict): Task 6 扩展词典哈希校验至四渠道（加 Web npm dictData 通道）

## 测试摘要
`bash scripts/check-dict-hash.sh` 四渠道全过 exit 0；`cargo test -p vane-dict-zh` 8 passed（含新增 `npm_package_json_references_source_dict_bin`）；`cargo clippy --all-targets --all-features -- -D warnings` 0 warning；`cargo fmt --all -- --check` 通过。

## 四渠道校验逻辑说明

SPEC §12.3 词典分发四渠道，本任务扩展校验覆盖第四渠道：

| 渠道 | 来源 | 本脚本校验方式 |
|------|------|---------------|
| 1. Node | vane-dict-zh include_bytes（crates/vane-dict-zh/data/dict.bin） | sha256_prefix.bin 存在 + 8 字节；完整校验在 Rust 测试 |
| 2. Go | bindings/go/dict/dict.bin.gz（gzip 再压缩，go:embed） | gunzip ↔ Node sha256 + DictVersion + zstd 头部 prefix |
| 3. WASM CDN | fetch jsdelivr（fallback） | 运行时 sha256_prefix 校验，本脚本不覆盖 |
| 4. WASM npm dictData（新） | @vane-rs/dict-zh npm 包 data/dict.bin | package.json files + exports 元数据 + npm pack 字节比对 |

**第四渠道同源语义**：@vane-rs/dict-zh npm 包的 data/dict.bin 就是 crates/vane-dict-zh/data/dict.bin（package.json `files` 字段直接引用源文件路径，非拷贝）。故第四渠道与第一渠道（Node include_bytes）同源。

**第四渠道两层校验**：
1. **元数据校验**（必跑，grep -F 字面匹配）：
   - `files` 数组含 `"data/dict.bin"` + `"data/sha256_prefix.bin"`
   - `exports."./dict.bin"` = `"./data/dict.bin"`（确保 `import dictBinUrl from '@vane-rs/dict-zh/dict.bin'` 解析到源文件）
   - `exports."./sha256_prefix.bin"` = `"./data/sha256_prefix.bin"`
2. **字节级比对**（npm 可用时跑，最严谨）：
   - `npm pack --pack-destination TMPDIR` 实际生成 tarball
   - `tar -xzf TARBALL -O package/data/dict.bin` 提取产物内 dict.bin
   - sha256 ↔ Node 源 data/dict.bin sha256 比对

实测：npm pack 产物 dict.bin sha256 = Node dict.bin sha256 = `efa4eee3467f3333d9c533be1b9f8ce168c206933d86298b03f2fc81bfa6b525`，字节级一致。

**Rust 测试补充**：`npm_package_json_references_source_dict_bin` 用 serde_json 解析 package.json（dev-dependency 已有），校验 files + exports 配置正确。用 `env!("CARGO_MANIFEST_DIR")` 编译期定位 package.json，不依赖运行时 CWD。字节级 npm pack 比对在 shell 脚本（CI 门禁）中，Rust 测试只做元数据校验（无法跑 npm pack）。

## 新增/修改文件清单
| 文件 | 操作 | 说明 |
|------|------|------|
| `scripts/check-dict-hash.sh` | 修改 | 三渠道→四渠道：头部注释更新；compute_sha256 函数提到顶部；新增第四渠道校验段（files + exports + npm pack 字节比对） |
| `crates/vane-dict-zh/tests/dict_test.rs` | 修改 | 新增 `npm_package_json_references_source_dict_bin` 测试（serde_json 校验 package.json files + exports） |

未触碰冻结文件：vane-wasm/.rs、vane-dict-zh/src/、vane-dict-zh/data/、vane-dict-zh/Cargo.toml、vane-dict-zh/package.json（只读校验）。

## Concerns
- **无功能 concern**。第四渠道与第一渠道同源（npm files 字段直接引用源文件路径），校验逻辑严密：元数据校验防止 files/exports 配置错误，npm pack 字节比对防止 npm 处理导致字节变化（虽实测无变化）。
- **npm 可用性降级**：CI 环境 npm 必可用，本地若 npm 缺失则降级为仅元数据校验（files + exports 字段校验已间接保证同源，因 npm pack 的 files 字段就是直接引用源文件）。脚本以 `command -v npm` 检测并 INFO 提示。
- **SPEC §12.3 修订**：v1.5 修订合并至 Task 13，本任务只管校验实现，不改 SPEC 文档。
- **release.yml 集成**：Task 11 范围，本任务不改 CI workflow。
