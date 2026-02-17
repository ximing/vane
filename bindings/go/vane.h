// vane.h — Vane C ABI 头文件（手写，与 vane-ffi src/lib.rs extern "C" 签名严格一致）
//
// M2-11：cbindgen CLI 不可用且作为 build-dep 会引入 regex（deny.toml 黑名单），
// 故手写此头文件。签名逐字对齐 M1 README §09 契约 + vane-ffi 实装。
//
// 错误码（SPEC §10）：
//   0=OK, -1=E_IO, -2=E_SCHEMA, -3=E_NOT_FOUND, -4=E_CORRUPT,
//   -5=E_VERSION, -6=E_TOKENIZER_MISMATCH, -7=E_DICT_TOO_LARGE,
//   -8=E_DICT_UNAVAILABLE, -9=E_BUSY, -10=E_UNSUPPORTED, -11=E_INVALID_ARG
//
// 内存铁律 I-7：
//   - vane_search/vane_dict_version 的 out_arena 须由调用方用 vane_string_free 释放
//   - vane_last_error_message 返回线程局部指针，不需 free
//   - 句柄注销后使用返回 E_NOT_FOUND（非 UB）

#ifndef VANE_H
#define VANE_H

#include <stddef.h>  // size_t
#include <stdint.h>  // uint64_t

#ifdef __cplusplus
extern "C" {
#endif

// ---- Db ----

// 打开数据库。path=UTF-8 路径，opts_json=OpenOptions JSON（可空/null）。
// 成功返回 0，out_handle 写入 Db 句柄。
int32_t vane_open(const uint8_t *path_ptr, size_t path_len,
                  const uint8_t *opts_json, size_t opts_len,
                  uint64_t *out_handle);

// 创建或获取 collection。schema_json=Schema JSON，opts_json=CollectionOptions JSON。
int32_t vane_collection(uint64_t db_h,
                        const uint8_t *name_ptr, size_t name_len,
                        const uint8_t *schema_json, size_t schema_len,
                        const uint8_t *opts_json, size_t opts_len,
                        uint64_t *out_handle);

// 导出快照（M2-12 接入前返 E_UNSUPPORTED）。
int32_t vane_export(uint64_t db_h, const uint8_t *dest_ptr, size_t dest_len);

// ---- Collection ----

// 追加文档。docs_json=Doc[] JSON。
int32_t vane_add(uint64_t col_h, const uint8_t *docs_json, size_t docs_len);

// 刷新缓冲区，持久化段。
int32_t vane_flush(uint64_t col_h);

// 搜索。query_json=SearchQuery JSON。out_arena 返回 Hit[] JSON（须 vane_string_free）。
int32_t vane_search(uint64_t col_h,
                    const uint8_t *query_json, size_t query_len,
                    uint8_t **out_arena, size_t *out_len);

// 删除文档。ids_json=string[] JSON。out_count 返回已删除数。
int32_t vane_delete(uint64_t col_h,
                    const uint8_t *ids_json, size_t ids_len,
                    uint64_t *out_count);

// 段合并。
int32_t vane_compact(uint64_t col_h);

// ---- Reindex ----

// 触发 reindex。out_handle 返回 ReindexHandle 句柄。
int32_t vane_reindex(uint64_t col_h, uint64_t *out_handle);

// 查询 reindex 进度（0.0..1.0）。
int32_t vane_reindex_progress(uint64_t h, float *out_progress);

// 阻塞等待 reindex 完成。
int32_t vane_reindex_wait(uint64_t h);

// ---- 词典 ----

// 加载 jieba 词典（zstd 压缩 dict.bin 字节），注入到 db 句柄对应的 Db。
int32_t vane_load_dict(uint64_t h, const uint8_t *dict_ptr, size_t dict_len);

// 查询词典版本（JSON）。out_ptr 须 vane_string_free。未加载返回 E_DICT_UNAVAILABLE。
int32_t vane_dict_version(uint8_t **out_ptr, size_t *out_len);

// ---- M4 §9 inspect API ----

// DB 级统计（collections / 文档数 / 健康状态）。out_arena 返回 DbStats JSON（须 vane_string_free）。
int32_t vane_db_stats(uint64_t db_h, uint8_t **out_arena, size_t *out_len);

// 各段详细信息（ULID / doc_count / format_versions / file_sizes / health）。
// out_arena 返回 SegmentInfo[] JSON（须 vane_string_free）。
int32_t vane_db_segment_info(uint64_t db_h, uint8_t **out_arena, size_t *out_len);

// ---- 生命周期 ----

// 关闭句柄（Db/Collection/Reindex 均可）。注销后使用返 E_NOT_FOUND。
int32_t vane_close(uint64_t handle);

// 查询最近一次错误描述（线程局部，不需 free，无错误返 null）。
const uint8_t *vane_last_error_message(uint64_t handle);

// 释放 vane_search/vane_dict_version 返回的 arena。null 安全。
void vane_string_free(uint8_t *ptr);

#ifdef __cplusplus
}  // extern "C"
#endif

#endif  // VANE_H
