//! VaneDb napi 导出：open / collection / collections / close / export。
//!
//! 异步经 AsyncTask（libuv worker pool），不桥接 tokio（SPEC §9.3）。
//! 每个 async 方法入队前 clone inner（Arc-based 浅克隆）。

use napi::bindgen_prelude::*;
use napi_derive::napi;
use std::sync::Arc;
use vane_core::api::Db;
use vane_core::vfs::std_fs::StdFsVfs;

use crate::collection::VaneCollection;
use crate::convert::{parse_collection_opts, parse_open_opts, parse_schema};
use crate::error::{to_napi_error, NapiResult};

#[napi]
pub struct VaneDb {
    pub(crate) inner: Db,
}

// ---- Open ----

pub struct OpenTask {
    path: String,
    opts: serde_json::Value,
}

#[napi]
impl Task for OpenTask {
    type Output = Db;
    type JsValue = VaneDb;
    fn compute(&mut self) -> NapiResult<Self::Output> {
        let opts = parse_open_opts(&self.opts)?;
        let vfs = Arc::new(StdFsVfs::new());
        Db::open(vfs, &self.path, opts).map_err(to_napi_error)
    }
    fn resolve(&mut self, _env: Env, output: Self::Output) -> NapiResult<Self::JsValue> {
        Ok(VaneDb { inner: output })
    }
}

// ---- Collection ----

pub struct CollectionTask {
    db: Db,
    name: String,
    schema: serde_json::Value,
    opts: serde_json::Value,
}

#[napi]
impl Task for CollectionTask {
    type Output = vane_core::api::Collection;
    type JsValue = VaneCollection;
    fn compute(&mut self) -> NapiResult<Self::Output> {
        let schema = parse_schema(&self.schema)?;
        let opts = parse_collection_opts(&self.opts)?;
        self.db
            .collection(&self.name, schema, opts)
            .map_err(to_napi_error)
    }
    fn resolve(&mut self, _env: Env, output: Self::Output) -> NapiResult<Self::JsValue> {
        Ok(VaneCollection { inner: output })
    }
}

// ---- Close ----

pub struct CloseTask {
    db: Db,
}

#[napi]
impl Task for CloseTask {
    type Output = ();
    type JsValue = ();
    fn compute(&mut self) -> NapiResult<Self::Output> {
        self.db.close().map_err(to_napi_error)
    }
    fn resolve(&mut self, _env: Env, _: ()) -> NapiResult<Self::JsValue> {
        Ok(())
    }
}

// ---- Export（I1：M0 占位 reject E_UNSUPPORTED） ----

pub struct ExportTask {
    db: Db,
    dest: String,
}

#[napi]
impl Task for ExportTask {
    type Output = ();
    type JsValue = ();
    fn compute(&mut self) -> NapiResult<Self::Output> {
        self.db.export(&self.dest).map_err(to_napi_error)
    }
    fn resolve(&mut self, _env: Env, _: ()) -> NapiResult<Self::JsValue> {
        Ok(())
    }
}

#[napi]
impl VaneDb {
    #[napi]
    pub fn open(path: String, opts: serde_json::Value) -> AsyncTask<OpenTask> {
        AsyncTask::new(OpenTask { path, opts })
    }

    #[napi]
    pub fn collection(
        &self,
        name: String,
        schema: serde_json::Value,
        opts: serde_json::Value,
    ) -> AsyncTask<CollectionTask> {
        AsyncTask::new(CollectionTask {
            db: self.inner.clone(),
            name,
            schema,
            opts,
        })
    }

    #[napi]
    pub fn collections(&self) -> napi::Result<Vec<String>> {
        Ok(self.inner.collections())
    }

    #[napi]
    pub fn close(&self) -> AsyncTask<CloseTask> {
        AsyncTask::new(CloseTask {
            db: self.inner.clone(),
        })
    }

    #[napi]
    pub fn export(&self, dest: String) -> AsyncTask<ExportTask> {
        AsyncTask::new(ExportTask {
            db: self.inner.clone(),
            dest,
        })
    }
}
