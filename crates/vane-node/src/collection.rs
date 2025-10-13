//! VaneCollection napi 导出：add / flush / search / delete / reindex。
//!
//! 异步经 AsyncTask（libuv worker pool），不桥接 tokio（SPEC §9.3）。
//! delete / reindex 为 M0 占位，core 直接返回 E_UNSUPPORTED，binding 透传。

use napi::bindgen_prelude::*;
use napi_derive::napi;
use vane_core::api::{AddReport, Collection};

use crate::convert::{add_report_to_json, hits_to_json, parse_docs, parse_search_query, Json};
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

// ---- Reindex（I1：M0 占位 reject E_UNSUPPORTED） ----

pub struct ReindexTask {
    col: Collection,
}

#[napi]
impl Task for ReindexTask {
    type Output = ();
    type JsValue = ();
    fn compute(&mut self) -> NapiResult<Self::Output> {
        self.col.reindex().map_err(to_napi_error)
    }
    fn resolve(&mut self, _env: Env, _: ()) -> NapiResult<Self::JsValue> {
        Ok(())
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
}
