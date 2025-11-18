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
    // M2-10：Executor（SPEC §11）。open 时经 executor::default_executor() 工厂构造，
    // 平台分支集中在 executor/mod.rs（I-5）。search 路径用 Executor::scope 并行搜各段。
    pub(crate) executor: Arc<dyn crate::executor::Executor>,
    // I3：Db 级 fallback，restore 时用（M0 restore 直接用 opts.auto_commit 传入参数；
    // 此字段保留供未来 reopen/动态配置场景，故 allow dead_code）
    #[allow(dead_code)]
    pub(crate) auto_commit: AutoCommitConfig,
    // 07-dict-distribution-node：Db 级 jieba 词典（dict-zh feature 启用时 Db::open 加载）。
    // collection 创建时若 tokenizer=Jieba 且此字段 Some → build_jieba_tokenizer；
    // 否则 build_tokenizer(Jieba) 返回 DictUnavailable，绑定层降级 CjkBigram（Task 3）。
    // pub(crate) 扩展，非 M0 冻结破坏（DbInner 内部结构，不暴露 pub API）。
    // M2-11：改为 RwLock 以支持 FFI vane_load_dict 运行时注入（dict-zh off 时
    // Db::open 设 None，FFI 调 set_jieba_dict 注入 Go embed 词典）。
    #[cfg(feature = "jieba")]
    pub(crate) jieba_dict:
        std::sync::RwLock<Option<std::sync::Arc<crate::tokenizer::jieba::JiebaDict>>>,
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
        // M2-10：Executor 工厂构造（平台分支在 executor/mod.rs，I-5）。
        let executor = crate::executor::default_executor();
        let inner = Arc::new(DbInner {
            vfs: vfs.clone(),
            db_path: path.to_string(),
            manifest_store,
            collections,
            executor,
            auto_commit: opts.auto_commit.clone(),
            #[cfg(feature = "jieba")]
            jieba_dict: std::sync::RwLock::new(jieba_dict),
        });
        let db = Db {
            inner: inner.clone(),
        };
        // 04-wal：崩溃恢复——重放 WAL tombstone + 清理半成品段（SPEC §6.4）。
        // 在 restore 之前调用 recover：半成品段（ULID 不在 manifest）目录被清理，
        // 避免后续操作误触；tombstone 聚合后注入各 CollectionInner。
        let recovered_tombstones = crate::wal::recover(&vfs, path, &manifest)?;
        for (name, meta) in &manifest.collections {
            // I3：restore 时用 OpenOptions.auto_commit 作为 collection 级配置
            let col_inner = CollectionInner::restore_from_manifest(
                &inner,
                name,
                meta.clone(),
                opts.auto_commit.clone(),
            )?;
            // 04-wal：注入恢复的 tombstone（绝对 docid，M-minor-2）。
            // recover 已按 manifest 过滤 ULID，此处双重保险再校验一次。
            if let Some(ulid_map) = recovered_tombstones.get(name) {
                let mut tomb_w = col_inner.tombstones.write().unwrap();
                for (ulid, bm) in ulid_map {
                    if meta.segment_ulids.contains(ulid) && !bm.is_empty() {
                        let existing = tomb_w.entry(ulid.clone()).or_default();
                        for v in bm.iter() {
                            existing.insert(v);
                        }
                    }
                }
            }
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

    // M2-12：实装快照导出（SPEC §4.1 / §15）。签名不变（M0 冻结）。
    // 调 write_snapshot 打包 VANE_SNAP 单文件；只读遍历原库 + 写 dest（I-6）。
    pub fn export(&self, dest: &str) -> Result<()> {
        super::snapshot::write_snapshot(self.inner.vfs.as_ref(), &self.inner.db_path, dest)
    }

    pub fn close(&self) -> Result<()> {
        // M0：无后台线程需 join；flush 由调用方显式调
        Ok(())
    }

    /// jieba 词典是否可用（Db::open 时加载，dict-zh feature 启用）。
    /// 绑定层（vane-node）用此判断 collection 创建时是否需降级 CjkBigram（Task 3）。
    #[cfg(feature = "jieba")]
    pub fn jieba_dict_available(&self) -> bool {
        self.inner.jieba_dict.read().unwrap().is_some()
    }

    /// M2-11：运行时注入 jieba 词典（FFI `vane_load_dict` 调用）。
    ///
    /// dict-zh feature 关闭时 Db::open 设 jieba_dict=None；FFI 绑定层从 Go embed
    /// 读取 dict.bin 字节 → `JiebaDict::load_zstd` → 经此方法注入。注入后后续
    /// `collection(tokenizer=Jieba)` 调用即可用 jieba 分词。
    /// 已创建的 collection 不受影响（tokenizer 在创建时固定）。
    #[cfg(feature = "jieba")]
    pub fn set_jieba_dict(&self, dict: std::sync::Arc<crate::tokenizer::jieba::JiebaDict>) {
        *self.inner.jieba_dict.write().unwrap() = Some(dict);
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

/// 测试专用：注入 jieba 词典（绕过 dict-zh 自动加载）。
///
/// M2 parked minor 2.1.5：jieba 并发测试需在不启用 dict-zh feature 时构造
/// jieba collection。Db::open 后立即调用（此时 inner 无其他 Arc 强引用，
/// Arc::get_mut 成功）。`#[cfg(test)]` 保证不进生产产物。
#[cfg(all(test, feature = "jieba"))]
impl Db {
    pub(crate) fn set_jieba_dict_for_test(
        &mut self,
        dict: std::sync::Arc<crate::tokenizer::jieba::JiebaDict>,
    ) {
        match std::sync::Arc::get_mut(&mut self.inner) {
            Some(inner) => *inner.jieba_dict.write().unwrap() = Some(dict),
            None => panic!("Db::inner has other Arc references, cannot inject jieba dict"),
        }
    }
}
