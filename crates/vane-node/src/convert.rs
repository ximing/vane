//! JSON ↔ core 结构转换（binding 薄壳，I-8）。
//!
//! 仅做 serde_json::Value 与 vane_core 公共类型的搬运，不含任何检索逻辑。

use napi::bindgen_prelude::*;
use serde_json::Value;
use vane_core::api::{
    CollectionOptions, Doc, FusionSpec, Hit, OpenOptions, PersistenceMode, ScalarValue, SearchMode,
    SearchQuery,
};
use vane_core::persistence::AutoCommitConfig;
use vane_core::tokenizer::{BuiltinTokenizer, UserDictEntry};
use vane_core::types::{FieldDef, Metric, ScalarKind, Schema, VaneError};

use crate::error::{to_napi_error, NapiResult};

/// napi Task 的 JsValue 需实现 `ToNapiValue + TypeName`；serde_json::Value 在
/// napi 2.16.13 实现了 ToNapiValue/FromNapiValue 但缺 TypeName。用 newtype 包装
/// 补 TypeName，使 Task 可返回 JSON 对象给 JS。
pub struct Json(pub Value);

impl TypeName for Json {
    fn type_name() -> &'static str {
        "JsonValue"
    }
    fn value_type() -> napi::ValueType {
        napi::ValueType::Object
    }
}

impl ToNapiValue for Json {
    unsafe fn to_napi_value(env: napi::sys::napi_env, val: Self) -> Result<napi::sys::napi_value> {
        // 安全：转交给 serde_json::Value 的 ToNapiValue 实现（napi serde-json feature）。
        unsafe { Value::to_napi_value(env, val.0) }
    }
}

fn invalid_arg<T>(msg: &str) -> NapiResult<T> {
    Err(to_napi_error(VaneError::InvalidArg(msg.into())))
}

/// 构造 E_INVALID_ARG 的 napi::Error 值，供 `ok_or_else` + `?` 使用。
fn err_invalid_arg(msg: impl Into<String>) -> Error {
    to_napi_error(VaneError::InvalidArg(msg.into()))
}

// ---- OpenOptions ----

pub fn parse_open_opts(v: &Value) -> NapiResult<OpenOptions> {
    let persistence = match v.get("persistence").and_then(Value::as_str) {
        Some("best-effort") => PersistenceMode::BestEffort,
        _ => PersistenceMode::Persistent,
    };
    let auto_commit = parse_auto_commit(v.get("autoCommit"))?;
    let page_cache_mb = v.get("pageCacheMb").and_then(Value::as_u64).unwrap_or(32) as u32;
    Ok(OpenOptions {
        persistence,
        auto_commit,
        page_cache_mb,
    })
}

fn parse_auto_commit(v: Option<&Value>) -> NapiResult<AutoCommitConfig> {
    match v {
        Some(Value::String(s)) if s == "off" => Ok(AutoCommitConfig::Off),
        Some(Value::String(_)) => invalid_arg("autoCommit string must be 'off'"),
        Some(o) => Ok(AutoCommitConfig::On {
            interval_ms: o.get("intervalMs").and_then(Value::as_u64).unwrap_or(1000) as u32,
            max_docs: o.get("maxDocs").and_then(Value::as_u64).unwrap_or(1000) as u32,
        }),
        None => Ok(AutoCommitConfig::default()),
    }
}

// ---- CollectionOptions ----

pub fn parse_collection_opts(v: &Value) -> NapiResult<CollectionOptions> {
    let tokenizer = match v.get("tokenizer").and_then(Value::as_str) {
        Some("cjk_bigram") => BuiltinTokenizer::CjkBigram,
        Some("jieba") => BuiltinTokenizer::Jieba,
        _ => BuiltinTokenizer::Standard,
    };
    let user_dict = v
        .get("userDict")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(parse_dict_entry).collect::<Result<_, _>>())
        .transpose()?
        .unwrap_or_default();
    let auto_commit = parse_auto_commit(v.get("autoCommit"))?;
    Ok(CollectionOptions {
        tokenizer,
        user_dict,
        auto_commit,
    })
}

pub fn parse_dict_entry(v: &Value) -> NapiResult<UserDictEntry> {
    match v {
        Value::String(s) => Ok(UserDictEntry::Word(s.clone())),
        o => Ok(UserDictEntry::WordWithFreq {
            term: o
                .get("term")
                .and_then(Value::as_str)
                .ok_or_else(|| err_invalid_arg("userDict.term missing"))?
                .to_string(),
            freq: o.get("freq").and_then(Value::as_u64).unwrap_or(0) as u32,
        }),
    }
}

// ---- Schema（B6：数组形式） ----

pub fn parse_schema(v: &Value) -> NapiResult<Schema> {
    let fields_arr = v
        .get("fields")
        .and_then(Value::as_array)
        .ok_or_else(|| err_invalid_arg("schema.fields must be an array"))?;
    let mut fields = Vec::new();
    for entry in fields_arr {
        let name = entry
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| err_invalid_arg("field.name missing"))?
            .to_string();
        let fd = parse_field(entry)?;
        fields.push((name, fd));
    }
    Schema::new(fields).map_err(to_napi_error)
}

fn parse_field(v: &Value) -> NapiResult<FieldDef> {
    let t = v
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| err_invalid_arg("field.type missing"))?;
    Ok(match t {
        "text" => FieldDef::Text,
        "vector" => FieldDef::Vector {
            dim: v
                .get("dim")
                .and_then(Value::as_u64)
                .ok_or_else(|| err_invalid_arg("vector.dim missing"))? as u32,
            metric: match v.get("metric").and_then(Value::as_str) {
                Some("l2") => Metric::L2,
                Some("dot") => Metric::Dot,
                _ => Metric::Cosine,
            },
        },
        "scalar" => FieldDef::Scalar {
            kind: match v.get("kind").and_then(Value::as_str) {
                Some("int") => ScalarKind::Int,
                Some("float") => ScalarKind::Float,
                Some("bool") => ScalarKind::Bool,
                _ => ScalarKind::Keyword,
            },
        },
        other => return invalid_arg(&format!("unknown field type {other}")),
    })
}

// ---- Doc / AddReport ----

pub fn parse_docs(v: &Value) -> NapiResult<Vec<Doc>> {
    let arr = v
        .as_array()
        .ok_or_else(|| err_invalid_arg("docs must be array"))?;
    arr.iter().map(parse_doc).collect()
}

fn parse_doc(v: &Value) -> NapiResult<Doc> {
    let id = v
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| err_invalid_arg("doc.id missing"))?
        .to_string();
    let text = v.get("text").and_then(Value::as_str).map(String::from);
    let vector = v.get("vector").and_then(Value::as_array).map(|a| {
        a.iter()
            .filter_map(|x| x.as_f64().map(|f| f as f32))
            .collect::<Vec<_>>()
    });
    let meta = v.get("meta").and_then(Value::as_object).map(|o| {
        o.iter()
            .filter_map(|(k, vv)| parse_scalar(vv).map(|sv| (k.clone(), sv)))
            .collect()
    });
    Ok(Doc {
        id,
        text,
        vector,
        meta,
    })
}

fn parse_scalar(v: &Value) -> Option<ScalarValue> {
    match v {
        Value::Number(n) if n.is_i64() => Some(ScalarValue::Int(n.as_i64().unwrap())),
        Value::Number(n) if n.is_f64() => Some(ScalarValue::Float(n.as_f64().unwrap())),
        Value::Bool(b) => Some(ScalarValue::Bool(*b)),
        Value::String(s) => Some(ScalarValue::Keyword(s.clone())),
        _ => None,
    }
}

pub fn add_report_to_json(accepted: u64, visible_after_flush: bool) -> Value {
    serde_json::json!({ "accepted": accepted, "visibleAfterFlush": visible_after_flush })
}

// ---- SearchQuery ----

pub fn parse_search_query(v: &Value) -> NapiResult<SearchQuery> {
    let text = v.get("text").and_then(Value::as_str).map(String::from);
    let vector = v.get("vector").and_then(Value::as_array).map(|a| {
        a.iter()
            .filter_map(|x| x.as_f64().map(|f| f as f32))
            .collect::<Vec<_>>()
    });
    if text.is_none() && vector.is_none() {
        return invalid_arg("text or vector required");
    }
    let top_k = v.get("topK").and_then(Value::as_u64).unwrap_or(10) as u32;
    let mode = match v.get("mode").and_then(Value::as_str) {
        Some("hybrid") => SearchMode::Hybrid,
        Some("vector") => SearchMode::Vector,
        Some("text") => SearchMode::Text,
        _ => SearchMode::Auto,
    };
    let fusion = match v.get("fusion") {
        Some(Value::String(s)) if s == "rrf" => FusionSpec::Rrf,
        Some(o) => FusionSpec::Linear {
            alpha: o
                .get("linear")
                .and_then(|l| l.get("alpha"))
                .and_then(Value::as_f64)
                .unwrap_or(0.5) as f32,
        },
        None => FusionSpec::Rrf,
    };
    // M0 不支持 filter：非 null 即 reject（不静默吞并）。
    if v.get("filter").is_some_and(|f| !f.is_null()) {
        return invalid_arg("filter not supported in M0");
    }
    let filter = None;
    let candidate_multiplier = v
        .get("candidateMultiplier")
        .and_then(Value::as_u64)
        .unwrap_or(3) as u32;
    Ok(SearchQuery {
        text,
        vector,
        top_k,
        mode,
        fusion,
        filter,
        candidate_multiplier,
    })
}

// ---- Hit ----

pub fn hits_to_json(hits: Vec<Hit>) -> Value {
    Value::Array(
        hits.into_iter()
            .map(|h| {
                let fields = h
                    .fields
                    .map(|m| {
                        let mut o = serde_json::Map::new();
                        for (k, val) in m {
                            o.insert(k, Value::String(val));
                        }
                        Value::Object(o)
                    })
                    .unwrap_or(Value::Null);
                serde_json::json!({ "id": h.id, "score": h.score, "fields": fields })
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn open_opts_defaults() {
        let o = parse_open_opts(&v("{}")).unwrap();
        assert!(matches!(o.persistence, PersistenceMode::Persistent));
        assert!(matches!(o.auto_commit, AutoCommitConfig::On { .. }));
        assert_eq!(o.page_cache_mb, 32);
    }

    #[test]
    fn open_opts_auto_commit_off() {
        let o = parse_open_opts(&v(r#"{"autoCommit":"off"}"#)).unwrap();
        assert!(matches!(o.auto_commit, AutoCommitConfig::Off));
    }

    #[test]
    fn collection_opts_jieba_passthrough() {
        let o = parse_collection_opts(&v(r#"{"tokenizer":"jieba"}"#)).unwrap();
        assert!(matches!(o.tokenizer, BuiltinTokenizer::Jieba));
    }

    #[test]
    fn schema_single_vector_ok() {
        let s = parse_schema(&v(
            r#"{"fields":[{"name":"t","type":"text"},{"name":"v","type":"vector","dim":3}]}"#,
        ))
        .unwrap();
        assert_eq!(s.vector_field().unwrap().1, 3);
    }

    #[test]
    fn schema_zero_vector_rejects_e_schema() {
        // 零 vector 字段 → core Schema::new 返回 E_SCHEMA（code -2）
        let r = parse_schema(&v(r#"{"fields":[{"name":"t","type":"text"}]}"#));
        assert!(r.is_err());
        let e = r.err().unwrap();
        assert_eq!(
            e.reason,
            "-2:E_SCHEMA:expected exactly 1 vector field, got 0"
        );
    }

    #[test]
    fn schema_two_vector_rejects_e_schema() {
        let r = parse_schema(&v(
            r#"{"fields":[{"name":"a","type":"vector","dim":3},{"name":"b","type":"vector","dim":4}]}"#,
        ));
        assert!(r.is_err());
        assert!(r.err().unwrap().reason.starts_with("-2:E_SCHEMA:"));
    }

    #[test]
    fn schema_missing_fields_array_invalid_arg() {
        let r = parse_schema(&v(r#"{}"#));
        assert!(r.is_err());
        assert!(r.err().unwrap().reason.starts_with("-11:E_INVALID_ARG:"));
    }

    #[test]
    fn search_query_defaults() {
        let q = parse_search_query(&v(r#"{"text":"hi"}"#)).unwrap();
        assert_eq!(q.top_k, 10);
        assert!(matches!(q.mode, SearchMode::Auto));
        assert!(matches!(q.fusion, FusionSpec::Rrf));
        assert_eq!(q.candidate_multiplier, 3);
    }

    #[test]
    fn search_query_mode_fusion_mapping() {
        let q = parse_search_query(&v(
            r#"{"vector":[1.0],"mode":"hybrid","fusion":{"linear":{"alpha":0.7}}}"#,
        ))
        .unwrap();
        assert!(matches!(q.mode, SearchMode::Hybrid));
        match q.fusion {
            FusionSpec::Linear { alpha } => assert!((alpha - 0.7).abs() < 1e-6),
            _ => panic!("expected linear"),
        }
    }

    #[test]
    fn search_query_filter_rejects_invalid_arg() {
        let r = parse_search_query(&v(r#"{"text":"hi","filter":{"x":1}}"#));
        assert!(r.is_err());
        assert!(r
            .err()
            .unwrap()
            .reason
            .starts_with("-11:E_INVALID_ARG:filter not supported"));
    }

    #[test]
    fn search_query_requires_text_or_vector() {
        let r = parse_search_query(&v(r#"{}"#));
        assert!(r.is_err());
    }

    #[test]
    fn docs_parse_ok() {
        let d = parse_docs(&v(
            r#"[{"id":"a","text":"hi","vector":[1,2,3]},{"id":"b"}]"#,
        ))
        .unwrap();
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].id, "a");
        assert_eq!(d[0].vector.as_ref().unwrap().len(), 3);
    }

    #[test]
    fn docs_missing_id_rejects() {
        let r = parse_docs(&v(r#"[{"text":"hi"}]"#));
        assert!(r.is_err());
        assert!(r
            .err()
            .unwrap()
            .reason
            .starts_with("-11:E_INVALID_ARG:doc.id"));
    }

    #[test]
    fn hits_to_json_null_fields_when_absent() {
        let hits = vec![Hit {
            id: "a".into(),
            score: 1.5,
            fields: None,
        }];
        let j = hits_to_json(hits);
        assert_eq!(j[0]["id"], "a");
        assert_eq!(j[0]["score"], 1.5);
        assert!(j[0]["fields"].is_null());
    }
}
