//! VaneCollection napi 导出：add / flush / search / delete / reindex。
//!
//! 异步经 AsyncTask（libuv worker pool），不桥接 tokio（SPEC §9.3）。
//! delete / reindex 为 M0 占位，core 直接返回 E_UNSUPPORTED，binding 透传。

use napi::bindgen_prelude::*;
use napi_derive::napi;
use vane_core::api::{AddReport, Collection};
use vane_core::tokenizer::UserDictEntry;

use crate::convert::{
    add_report_to_json, hits_to_json, parse_dict_entry, parse_docs, parse_search_query, Json,
};
use crate::error::{to_napi_error, NapiResult};

#[napi]
pub struct VaneCollection {
    pub(crate) inner: Collection,
}

// ---- Add ----

pub struct AddTask {
    col: Collection,
    docs: serde_json::Value,
}

#[napi]
impl Task for AddTask {
    type Output = AddReport;
    type JsValue = Json;
    fn compute(&mut self) -> NapiResult<Self::Output> {
        let docs = parse_docs(&self.docs)?;
        self.col.add(&docs).map_err(to_napi_error)
    }
    fn resolve(&mut self, _env: Env, output: Self::Output) -> NapiResult<Self::JsValue> {
        Ok(Json(add_report_to_json(
            output.accepted,
            output.visible_after_flush,
        )))
    }
}

// ---- Flush ----

pub struct FlushTask {
    col: Collection,
}

#[napi]
impl Task for FlushTask {
    type Output = ();
    type JsValue = ();
    fn compute(&mut self) -> NapiResult<Self::Output> {
        self.col.flush().map_err(to_napi_error)
    }
    fn resolve(&mut self, _env: Env, _: ()) -> NapiResult<Self::JsValue> {
        Ok(())
    }
}

// ---- Search ----

pub struct SearchTask {
    col: Collection,
    query: serde_json::Value,
}

#[napi]
impl Task for SearchTask {
    type Output = Vec<vane_core::api::Hit>;
    type JsValue = Json;
    fn compute(&mut self) -> NapiResult<Self::Output> {
        let q = parse_search_query(&self.query)?;
        self.col.search(&q).map_err(to_napi_error)
    }
    fn resolve(&mut self, _env: Env, output: Self::Output) -> NapiResult<Self::JsValue> {
        Ok(Json(hits_to_json(output)))
    }
}

// ---- Delete（M0 占位 reject E_UNSUPPORTED） ----

pub struct DeleteTask {
    col: Collection,
    ids: Vec<String>,
}

#[napi]
impl Task for DeleteTask {
    type Output = u64;
    // u64 在 napi 2.16.13 缺 TypeName，用 BigInt（napi6）承载 u64（JS 侧为 BigInt）。
    type JsValue = BigInt;
    fn compute(&mut self) -> NapiResult<Self::Output> {
        self.col.delete(&self.ids).map_err(to_napi_error)
    }
    fn resolve(&mut self, _env: Env, n: u64) -> NapiResult<Self::JsValue> {
        Ok(BigInt::from(n))
    }
}

// ---- Reindex（06-userdict-reindex：返回 ReindexHandle） ----

pub struct ReindexTask {
    col: Collection,
}

#[napi]
impl Task for ReindexTask {
    type Output = vane_core::api::ReindexHandle;
    type JsValue = VaneReindexHandle;
    fn compute(&mut self) -> NapiResult<Self::Output> {
        self.col.reindex().map_err(to_napi_error)
    }
    fn resolve(&mut self, _env: Env, output: Self::Output) -> NapiResult<Self::JsValue> {
        Ok(VaneReindexHandle { inner: output })
    }
}

/// SPEC §4.1 ReindexHandle napi 包装（progress/wait）。
#[napi]
pub struct VaneReindexHandle {
    pub(crate) inner: vane_core::api::ReindexHandle,
}

#[napi]
impl VaneReindexHandle {
    #[napi]
    pub fn progress(&self) -> f32 {
        self.inner.progress()
    }

    #[napi]
    pub fn wait(&self) -> Result<()> {
        self.inner.wait().map_err(to_napi_error)
    }
}

// ---- setUserDict / dictState（06-userdict-reindex） ----

pub struct SetUserDictTask {
    col: Collection,
    dict: serde_json::Value,
}

#[napi]
impl Task for SetUserDictTask {
    type Output = ();
    type JsValue = ();
    fn compute(&mut self) -> NapiResult<Self::Output> {
        let entries: Vec<UserDictEntry> = self
            .dict
            .as_array()
            .map(|a| a.iter().map(parse_dict_entry).collect::<Result<_, _>>())
            .transpose()?
            .unwrap_or_default();
        self.col.set_user_dict(&entries).map_err(to_napi_error)
    }
    fn resolve(&mut self, _env: Env, _: ()) -> NapiResult<Self::JsValue> {
        Ok(())
    }
}

pub struct DictStateTask {
    col: Collection,
}

#[napi]
impl Task for DictStateTask {
    type Output = String;
    type JsValue = String;
    fn compute(&mut self) -> NapiResult<Self::Output> {
        Ok(match self.col.dict_state() {
            vane_core::api::DictState::Stable => "stable".to_string(),
            vane_core::api::DictState::PendingReindex => "pendingReindex".to_string(),
            vane_core::api::DictState::Rebuilding => "rebuilding".to_string(),
        })
    }
    fn resolve(&mut self, _env: Env, output: Self::Output) -> NapiResult<Self::JsValue> {
        Ok(output)
    }
}

#[napi]
impl VaneCollection {
    #[napi]
    pub fn add(&self, docs: serde_json::Value) -> AsyncTask<AddTask> {
        AsyncTask::new(AddTask {
            col: self.inner.clone(),
            docs,
        })
    }

    #[napi]
    pub fn flush(&self) -> AsyncTask<FlushTask> {
        AsyncTask::new(FlushTask {
            col: self.inner.clone(),
        })
    }

    #[napi]
    pub fn search(&self, query: serde_json::Value) -> AsyncTask<SearchTask> {
        AsyncTask::new(SearchTask {
            col: self.inner.clone(),
            query,
        })
    }

    #[napi]
    pub fn delete(&self, ids: Vec<String>) -> AsyncTask<DeleteTask> {
        AsyncTask::new(DeleteTask {
            col: self.inner.clone(),
            ids,
        })
    }

    #[napi]
    pub fn reindex(&self) -> AsyncTask<ReindexTask> {
        AsyncTask::new(ReindexTask {
            col: self.inner.clone(),
        })
    }

    #[napi]
    pub fn set_user_dict(&self, dict: serde_json::Value) -> AsyncTask<SetUserDictTask> {
        AsyncTask::new(SetUserDictTask {
            col: self.inner.clone(),
            dict,
        })
    }

    #[napi]
    pub fn dict_state(&self) -> AsyncTask<DictStateTask> {
        AsyncTask::new(DictStateTask {
            col: self.inner.clone(),
        })
    }
}
