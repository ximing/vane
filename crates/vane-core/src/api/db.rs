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
    // 07-dict-distribution-node：Db 级 jieba 词典（dict-zh feature 启用时 Db::open 加载）。
    // collection 创建时若 tokenizer=Jieba 且此字段 Some → build_jieba_tokenizer；
    // 否则 build_tokenizer(Jieba) 返回 DictUnavailable，绑定层降级 CjkBigram（Task 3）。
    // pub(crate) 扩展，非 M0 冻结破坏（DbInner 内部结构，不暴露 pub API）。
    #[cfg(feature = "jieba")]
    pub(crate) jieba_dict: Option<std::sync::Arc<crate::tokenizer::jieba::JiebaDict>>,
}

impl Db {
    pub fn open(vfs: Arc<dyn Vfs>, path: &str, opts: OpenOptions) -> Result<Self> {
        let manifest_store = ManifestStore::new(vfs.clone(), path);
        let manifest = manifest_store.load()?.unwrap_or_else(Manifest::empty);
        let collections = RwLock::new(HashMap::new());
        // 07：dict-zh feature 启用时 Db::open 自动加载预编译 dict.bin（冷加载 <150ms，§13.1）。
        // 加载失败不抛错（SPEC §13.2-2 ④）：jieba_dict=None → collection 创建时降级 CjkBigram。
        #[cfg(feature = "jieba")]
        let jieba_dict = load_default_jieba_dict();
        let inner = Arc::new(DbInner {
            vfs: vfs.clone(),
            db_path: path.to_string(),
            manifest_store,
            collections,
            auto_commit: opts.auto_commit.clone(),
            #[cfg(feature = "jieba")]
            jieba_dict,
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
                if *existing.tokenizer_id.read().unwrap() != tok_id {
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

    /// jieba 词典是否可用（Db::open 时加载，dict-zh feature 启用）。
    /// 绑定层（vane-node）用此判断 collection 创建时是否需降级 CjkBigram（Task 3）。
    #[cfg(feature = "jieba")]
    pub fn jieba_dict_available(&self) -> bool {
        self.inner.jieba_dict.is_some()
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

// ---- 07：dict-zh feature 启用时 Db::open 加载预编译 jieba 词典 ----

#[cfg(feature = "jieba")]
fn load_default_jieba_dict() -> Option<Arc<crate::tokenizer::jieba::JiebaDict>> {
    #[cfg(feature = "dict-zh")]
    {
        use crate::tokenizer::jieba::JiebaDict;
        match JiebaDict::load_zstd(vane_dict_zh::DICT_BIN) {
            Ok(d) => Some(Arc::new(d)),
            Err(e) => {
                // SPEC §13.2-2 ④：加载失败不抛错，降级 CjkBigram + warn。
                eprintln!(
                    "[vane] failed to load bundled jieba dict (vane-dict-zh): {} \
                     — jieba tokenizer will fall back to cjk_bigram",
                    e
                );
                None
            }
        }
    }
    // jieba feature on 但 dict-zh off：无捆绑词典，返回 None（绑定层降级）。
    #[cfg(not(feature = "dict-zh"))]
    None
}
