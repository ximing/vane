//! SPEC §4.1 Db 句柄：持有 Vfs + ManifestStore + collections 注册表。

use crate::persistence::{AutoCommitConfig, CollectionMeta, Manifest, ManifestStore};
use crate::tokenizer::compute_tokenizer_id;
use crate::types::{Result, Schema, VaneError};
use crate::vfs::Vfs;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use super::collection::{Collection, CollectionInner};
use super::types::{CollectionOptions, OpenOptions};

pub struct Db {
    inner: Arc<DbInner>,
}

pub(crate) struct DbInner {
    pub(crate) vfs: Arc<dyn Vfs>,
    pub(crate) db_path: String,
    pub(crate) manifest_store: ManifestStore,
    pub(crate) collections: RwLock<HashMap<String, Arc<CollectionInner>>>,
    // I3：Db 级 fallback，restore 时用（M0 restore 直接用 opts.auto_commit 传入参数；
    // 此字段保留供未来 reopen/动态配置场景，故 allow dead_code）
    #[allow(dead_code)]
    pub(crate) auto_commit: AutoCommitConfig,
}

impl Db {
    pub fn open(vfs: Arc<dyn Vfs>, path: &str, opts: OpenOptions) -> Result<Self> {
        let manifest_store = ManifestStore::new(vfs.clone(), path);
        let manifest = manifest_store.load()?.unwrap_or_else(Manifest::empty);
        let collections = RwLock::new(HashMap::new());
        let inner = Arc::new(DbInner {
            vfs: vfs.clone(),
            db_path: path.to_string(),
            manifest_store,
            collections,
            auto_commit: opts.auto_commit.clone(),
        });
        let db = Db {
            inner: inner.clone(),
        };
        for (name, meta) in &manifest.collections {
            // I3：restore 时用 OpenOptions.auto_commit 作为 collection 级配置
            let col_inner = CollectionInner::restore_from_manifest(
                &inner,
                name,
                meta.clone(),
                opts.auto_commit.clone(),
            )?;
            db.inner
                .collections
                .write()
                .unwrap()
                .insert(name.clone(), Arc::new(col_inner));
        }
        Ok(db)
    }

    pub fn collection(
        &self,
        name: &str,
        schema: Schema,
        opts: CollectionOptions,
    ) -> Result<Collection> {
        // I2 裁决：幂等校验 schema 与 tokenizer 一致性
        {
            let read = self.inner.collections.read().unwrap();
            if let Some(existing) = read.get(name) {
                if existing.schema.fields != schema.fields {
                    return Err(VaneError::Schema(format!(
                        "collection '{}' exists with different schema",
                        name
                    )));
                }
                let tok_id = compute_tokenizer_id(opts.tokenizer, &opts.user_dict);
                if existing.tokenizer_id != tok_id {
                    return Err(VaneError::Schema(format!(
                        "collection '{}' exists with different tokenizer",
                        name
                    )));
                }
                return Ok(Collection {
                    inner: existing.clone(),
                });
            }
        }
        let tok_id = compute_tokenizer_id(opts.tokenizer, &opts.user_dict);
        let meta = CollectionMeta {
            schema: schema.clone(),
            tokenizer_kind: opts.tokenizer,
            tokenizer_id: tok_id.clone(),
            user_dict: opts.user_dict.clone(),
            segment_ulids: vec![],
        };
        let col_inner =
            CollectionInner::create_new(&self.inner, name, meta, opts.auto_commit.clone())?;
        let arc = Arc::new(col_inner);
        self.inner
            .collections
            .write()
            .unwrap()
            .insert(name.to_string(), arc.clone());
        // 持久化 manifest
        let mut m = self
            .inner
            .manifest_store
            .load()?
            .unwrap_or_else(Manifest::empty);
        m.collections.insert(
            name.to_string(),
            CollectionMeta {
                schema,
                tokenizer_kind: opts.tokenizer,
                tokenizer_id: tok_id,
                user_dict: opts.user_dict,
                segment_ulids: vec![],
            },
        );
        self.inner.manifest_store.save_atomic(&m)?;
        Ok(Collection { inner: arc })
    }

    pub fn collections(&self) -> Vec<String> {
        self.inner
            .collections
            .read()
            .unwrap()
            .keys()
            .cloned()
            .collect()
    }

    // I1 裁决：M0 占位
    pub fn export(&self, _dest: &str) -> Result<()> {
        Err(VaneError::Unsupported)
    }

    pub fn close(&self) -> Result<()> {
        // M0：无后台线程需 join；flush 由调用方显式调
        Ok(())
    }
}

impl Clone for Db {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

// S9 裁决：不写 unsafe impl Send/Sync——DbInner 字段全部自动 Send+Sync
// （Arc<dyn Vfs> 是 Send+Sync，RwLock<HashMap<...>> 是 Send+Sync）。
