//! M2-01 JS 侧行为测试（wasm-bindgen-test，node）。
//!
//! 验证 I-8（binding 薄壳）：open→collection→add→flush→search 端到端，
//! 与 vane-core 等价。使用 MemoryVfs（vane_open 内部构造）。

use wasm_bindgen_test::*;

// 默认在 node 跑（不加 run_in_browser）。

/// I-8 薄壳行为：open → collection → add → flush → search 返回 Hit 数组。
#[wasm_bindgen_test]
fn open_collection_add_flush_search_roundtrip() {
    // open（MemoryVfs）
    let db = vane_wasm::vane_open("test-db", "{}").expect("open");
    assert!(db > 0, "db handle should be positive");

    // collection
    let schema = r#"{"fields":[{"name":"title","type":"text"},{"name":"vec","type":"vector","dim":3,"metric":"cosine"}]}"#;
    let col = vane_wasm::vane_collection(db, "docs", schema, "{}").expect("collection");
    assert!(col > 0, "col handle should be positive");

    // add
    let docs = r#"[
        {"id":"d1","text":"hello world","vector":[1.0,0.0,0.0]},
        {"id":"d2","text":"foo bar","vector":[0.0,1.0,0.0]},
        {"id":"d3","text":"hello foo","vector":[0.0,0.0,1.0]}
    ]"#;
    let accepted = vane_wasm::vane_add(col, docs).expect("add");
    assert_eq!(accepted, 3, "should accept 3 docs");

    // flush
    vane_wasm::vane_flush(col).expect("flush");

    // search (vector mode)
    let query = r#"{"vector":[1.0,0.0,0.0],"topK":3,"mode":"vector"}"#;
    let hits_json = vane_wasm::vane_search(col, query).expect("search vector");
    let hits: serde_json::Value = serde_json::from_str(&hits_json).expect("hits parse");
    let arr = hits.as_array().expect("hits should be array");
    assert!(!arr.is_empty(), "should have hits");
    // d1 (vector [1,0,0]) should be top hit for query [1,0,0] cosine
    let top_id = arr[0].get("id").and_then(|v| v.as_str()).expect("top id");
    assert_eq!(top_id, "d1", "d1 should be top vector hit");

    // search (text mode)
    let query_text = r#"{"text":"hello","topK":3,"mode":"text"}"#;
    let hits_json_t = vane_wasm::vane_search(col, query_text).expect("search text");
    let hits_t: serde_json::Value = serde_json::from_str(&hits_json_t).expect("hits parse");
    let arr_t = hits_t.as_array().expect("hits should be array");
    assert!(!arr_t.is_empty(), "should have text hits");
    // hello appears in d1 and d3
    let ids: Vec<&str> = arr_t
        .iter()
        .map(|h| h.get("id").and_then(|v| v.as_str()).unwrap_or(""))
        .collect();
    assert!(ids.contains(&"d1"), "d1 should be in text hits");
    assert!(ids.contains(&"d3"), "d3 should be in text hits");

    // cleanup
    vane_wasm::vane_close(col).expect("close col");
    vane_wasm::vane_close(db).expect("close db");
}

/// delete + compact 验证。
#[wasm_bindgen_test]
fn delete_and_compact() {
    let db = vane_wasm::vane_open("test-db2", "{}").expect("open");
    let schema = r#"{"fields":[{"name":"title","type":"text"},{"name":"vec","type":"vector","dim":2,"metric":"cosine"}]}"#;
    let col = vane_wasm::vane_collection(db, "docs", schema, "{}").expect("collection");

    let docs = r#"[
        {"id":"a","text":"apple","vector":[1.0,0.0]},
        {"id":"b","text":"banana","vector":[0.0,1.0]}
    ]"#;
    vane_wasm::vane_add(col, docs).expect("add");
    vane_wasm::vane_flush(col).expect("flush");

    // delete
    let deleted = vane_wasm::vane_delete(col, r#"["a"]"#).expect("delete");
    assert_eq!(deleted, 1, "should delete 1 doc");

    // compact
    vane_wasm::vane_compact(col).expect("compact");

    // search after delete — "a" should not appear
    let hits_json =
        vane_wasm::vane_search(col, r#"{"vector":[1.0,0.0],"topK":10,"mode":"vector"}"#)
            .expect("search");
    let hits: serde_json::Value = serde_json::from_str(&hits_json).expect("hits parse");
    let ids: Vec<&str> = hits
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h.get("id").and_then(|v| v.as_str()).unwrap_or(""))
        .collect();
    assert!(!ids.contains(&"a"), "deleted doc 'a' should not appear");

    vane_wasm::vane_close(col).expect("close col");
    vane_wasm::vane_close(db).expect("close db");
}

/// SIMD 探针占位返回 false。
#[wasm_bindgen_test]
fn simd_probe_placeholder_false() {
    assert!(
        !vane_wasm::simd_probe::simd128_supported(),
        "M2-01 占位应返回 false"
    );
}

/// vane_version 返回非空字符串。
#[wasm_bindgen_test]
fn version_nonempty() {
    let v = vane_wasm::vane_version();
    assert!(!v.is_empty(), "version should be non-empty");
}
