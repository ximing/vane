use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::cas::{embed_key, extract_key, Cas};
use crate::chunk::chunk_strategy_id;
use crate::classify::{classify, should_watch_dir};
use crate::config::{ChunkConfig, ResolvedPolicy};
use crate::error::VaneCliError;
use crate::extract::{extract_image, extract_text, CanonicalDoc, IMAGE_MAX_BYTES, TEXT_MAX_BYTES};
use crate::index::{doc_id, index_doc, state_path, ProjectIndex, ProjectState};
use crate::live::{LiveFile, LiveSet};

const EXTRACTOR_VER: &str = "1";

pub struct SyncCtx<'a> {
    pub home: &'a Path,
    pub project_id: &'a str,
    pub cas: &'a Cas,
    pub index: &'a ProjectIndex,
    pub embedder: &'a dyn crate::embed::Embedder,
    pub now: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub added: u64,
    pub deleted: u64,
    pub unchanged: u64,
    pub embedded: u64,
    pub cas_hits: u64,
}

struct DiskFile {
    abs: PathBuf,
    extractor: String,
}

/// Walk `root`, hash files, and apply the working-set table in spec §7.3.
#[allow(clippy::needless_pass_by_ref_mut)] // plan signature; later writer mutates ctx
pub fn reconcile_project(
    ctx: &mut SyncCtx<'_>,
    root: &Path,
    policy: &ResolvedPolicy,
) -> Result<SyncReport, VaneCliError> {
    require_no_rebuild(ctx)?;

    let root_canon = root
        .canonicalize()
        .map_err(|e| VaneCliError::new(format!("canonicalize {}: {e}", root.display())))?;
    let root_str = root_canon.to_string_lossy().into_owned();
    let model_id = ctx.index.model_id();
    let strategy_id = chunk_strategy_id(&policy.chunk, EXTRACTOR_VER);

    let on_disk = collect_files(&root_canon, policy)?;
    let live = LiveSet::load_for_project(ctx.home, ctx.project_id)?;

    let mut report = SyncReport::default();
    let mut new_live = LiveSet::default();
    let mut to_add = Vec::new();
    let mut to_delete = Vec::new();

    for (rel, disk) in &on_disk {
        let hash = match sha256_file(&disk.abs) {
            Ok(h) => h,
            Err(_) => continue,
        };
        let ek = extract_key(&hash, &disk.extractor, EXTRACTOR_VER, &strategy_id);

        if let Some(old) = live.files.get(rel) {
            if old.content_sha256 == hash && old.extract_key == ek {
                report.unchanged += 1;
                touch_entry(ctx.cas, ek.as_str(), model_id, ctx.now);
                new_live.files.insert(rel.clone(), old.clone());
                continue;
            }
            push_ids(&mut to_delete, ctx.project_id, rel, old.chunk_count);
        }

        let docs = match load_or_extract(
            ctx.cas,
            rel,
            &disk.abs,
            &disk.extractor,
            &ek,
            &policy.chunk,
            &mut report,
        ) {
            Ok(Some(docs)) => docs,
            Ok(None) => continue,
            Err(e) => return Err(e),
        };

        let embedded = embed_docs(ctx, model_id, &docs, ctx.index.dim())?;
        report.embedded += embedded.n_embed;
        ctx.cas.touch(&ek, &embedded.keys, ctx.now);

        for (doc, vector) in docs.iter().zip(embedded.vectors) {
            to_add.push(index_doc(ctx.project_id, &root_str, doc, Some(vector)));
        }
        new_live.files.insert(
            rel.clone(),
            LiveFile {
                content_sha256: hash,
                extract_key: ek,
                chunk_count: docs.len() as u32,
            },
        );
        report.added += 1;
    }

    for (rel, old) in &live.files {
        if !new_live.files.contains_key(rel) {
            push_ids(&mut to_delete, ctx.project_id, rel, old.chunk_count);
            report.deleted += 1;
        }
    }

    if !to_delete.is_empty() {
        ctx.index.delete_ids(&to_delete)?;
    }
    if !to_add.is_empty() {
        ctx.index.add_docs(&to_add)?;
    }
    if !to_delete.is_empty() || !to_add.is_empty() {
        ctx.index.flush()?;
        let live_chunks: u64 = new_live
            .files
            .values()
            .map(|f| u64::from(f.chunk_count))
            .sum();
        let _ =
            ctx.index
                .maybe_compact(to_delete.len() as u64, live_chunks, to_delete.len() as u64);
        new_live.save_for_project(ctx.home, ctx.project_id)?;
    } else if live.files != new_live.files {
        new_live.save_for_project(ctx.home, ctx.project_id)?;
    }

    persist_root_state(ctx, &root_str, &strategy_id)?;
    Ok(report)
}

fn require_no_rebuild(ctx: &SyncCtx<'_>) -> Result<(), VaneCliError> {
    let state = ProjectState::load(&state_path(ctx.home, ctx.project_id))?;
    if state.rebuild.is_some() {
        return Err(VaneCliError::new("model rebuild required"));
    }
    if let Some(id) = state.embed_model_id.as_deref() {
        if id != ctx.index.model_id() {
            return Err(VaneCliError::new("model rebuild required"));
        }
    }
    Ok(())
}

fn persist_root_state(
    ctx: &SyncCtx<'_>,
    root_str: &str,
    strategy_id: &str,
) -> Result<(), VaneCliError> {
    let path = state_path(ctx.home, ctx.project_id);
    let mut state = ProjectState::load(&path)?;
    state.root_path = Some(root_str.to_string());
    state.chunk_strategy_id = Some(strategy_id.to_string());
    state.save_atomic(&path)
}

fn load_or_extract(
    cas: &Cas,
    rel: &str,
    abs: &Path,
    extractor: &str,
    extract_key: &str,
    chunk: &ChunkConfig,
    report: &mut SyncReport,
) -> Result<Option<Vec<CanonicalDoc>>, VaneCliError> {
    if let Some(docs) = cas.get_extract(extract_key) {
        report.cas_hits += 1;
        return Ok(Some(retarget(docs, rel)));
    }
    let meta_len = match fs::metadata(abs) {
        Ok(m) => m.len(),
        Err(_) => return Ok(None),
    };
    if too_large(extractor, meta_len) {
        return Ok(None);
    }
    let bytes = match fs::read(abs) {
        Ok(b) => b,
        Err(_) => return Ok(None),
    };
    match extract_docs(rel, &bytes, extractor, chunk) {
        Ok(docs) => {
            cas.put_extract(extract_key, &docs)?;
            Ok(Some(docs))
        }
        Err(e) if skippable(&e) => Ok(None),
        Err(e) => Err(e),
    }
}

fn extract_docs(
    rel: &str,
    bytes: &[u8],
    extractor: &str,
    chunk: &ChunkConfig,
) -> Result<Vec<CanonicalDoc>, VaneCliError> {
    match extractor {
        "text" => extract_text(rel, bytes, chunk),
        "image" => extract_image(rel, bytes),
        other => Err(VaneCliError::skip(format!("unsupported extractor {other}"))),
    }
}

struct EmbeddedBatch {
    vectors: Vec<Vec<f32>>,
    keys: Vec<String>,
    n_embed: u64,
}

fn embed_docs(
    ctx: &SyncCtx<'_>,
    model_id: &str,
    docs: &[CanonicalDoc],
    dim: u32,
) -> Result<EmbeddedBatch, VaneCliError> {
    let mut vectors: Vec<Option<Vec<f32>>> = Vec::with_capacity(docs.len());
    let mut embed_keys = Vec::with_capacity(docs.len());
    let mut need_idx = Vec::new();
    let mut need_texts = Vec::new();

    for (i, doc) in docs.iter().enumerate() {
        let key = embed_key(&doc.text, model_id);
        if let Some(v) = ctx.cas.get_embed(&key) {
            check_dim(dim, v.len())?;
            vectors.push(Some(v));
        } else {
            vectors.push(None);
            need_idx.push(i);
            need_texts.push(doc.text.clone());
        }
        embed_keys.push(key);
    }

    let mut embedded = 0;
    if !need_texts.is_empty() {
        let got = ctx.embedder.embed(&need_texts)?;
        if got.len() != need_texts.len() {
            return Err(VaneCliError::new(format!(
                "embed count: expected {}, got {}",
                need_texts.len(),
                got.len()
            )));
        }
        embedded = got.len() as u64;
        for (i, v) in need_idx.into_iter().zip(got) {
            check_dim(dim, v.len())?;
            ctx.cas.put_embed(&embed_keys[i], &v)?;
            vectors[i] = Some(v);
        }
    }

    let vectors = vectors
        .into_iter()
        .map(|v| v.ok_or_else(|| VaneCliError::new("missing embedding after embed")))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(EmbeddedBatch {
        vectors,
        keys: embed_keys,
        n_embed: embedded,
    })
}

fn check_dim(expected: u32, got: usize) -> Result<(), VaneCliError> {
    if got != expected as usize {
        return Err(VaneCliError::new(format!(
            "embedding dim changed: expected {expected}, got {got}"
        )));
    }
    Ok(())
}

fn touch_entry(cas: &Cas, extract_key: &str, model_id: &str, now: u64) {
    let embed_keys = cas
        .get_extract(extract_key)
        .map(|docs| {
            docs.iter()
                .map(|d| embed_key(&d.text, model_id))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    cas.touch(extract_key, &embed_keys, now);
}

fn push_ids(out: &mut Vec<String>, project_id: &str, rel: &str, chunk_count: u32) {
    for i in 0..chunk_count {
        out.push(doc_id(project_id, rel, i));
    }
}

fn retarget(mut docs: Vec<CanonicalDoc>, rel: &str) -> Vec<CanonicalDoc> {
    for d in &mut docs {
        d.path = rel.to_string();
    }
    docs
}

fn skippable(err: &VaneCliError) -> bool {
    if err.is_skip() {
        return true;
    }
    let m = err.message.to_ascii_lowercase();
    m.contains("utf-8") || m.contains("utf8")
}

fn too_large(extractor: &str, len: u64) -> bool {
    match extractor {
        "text" => len > TEXT_MAX_BYTES as u64,
        "image" => len > IMAGE_MAX_BYTES as u64,
        _ => false,
    }
}

fn collect_files(
    root: &Path,
    policy: &ResolvedPolicy,
) -> Result<BTreeMap<String, DiskFile>, VaneCliError> {
    let mut out = BTreeMap::new();
    let mut stack = vec![WalkFrame {
        dir: root.to_path_buf(),
        rel: String::new(),
        chain: vec![root.to_path_buf()],
    }];

    while let Some(frame) = stack.pop() {
        let entries = match fs::read_dir(&frame.dir) {
            Ok(e) => e,
            Err(e) if frame.rel.is_empty() => {
                return Err(VaneCliError::new(format!(
                    "read root {}: {e}",
                    frame.dir.display()
                )));
            }
            Err(_) => continue,
        };
        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let name = match entry.file_name().into_string() {
                Ok(n) if n != "." && n != ".." => n,
                _ => continue,
            };
            let child_rel = join_rel(&frame.rel, &name);
            let child_path = entry.path();
            let ft = match entry.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };

            if ft.is_symlink() {
                let target = match fs::canonicalize(&child_path) {
                    Ok(t) => t,
                    Err(_) => continue,
                };
                if !is_path_prefix(root, &target) {
                    continue;
                }
                if target.is_dir() {
                    push_dir(
                        &mut stack,
                        child_path,
                        child_rel,
                        target,
                        &frame.chain,
                        policy,
                    );
                } else if target.is_file() {
                    maybe_add_file(&mut out, child_rel, child_path, policy);
                }
                continue;
            }

            if ft.is_dir() {
                let canon = match fs::canonicalize(&child_path) {
                    Ok(t) => t,
                    Err(_) => continue,
                };
                if !is_path_prefix(root, &canon) {
                    continue;
                }
                push_dir(
                    &mut stack,
                    child_path,
                    child_rel,
                    canon,
                    &frame.chain,
                    policy,
                );
            } else if ft.is_file() {
                maybe_add_file(&mut out, child_rel, child_path, policy);
            }
        }
    }
    Ok(out)
}

struct WalkFrame {
    dir: PathBuf,
    rel: String,
    chain: Vec<PathBuf>,
}

fn push_dir(
    stack: &mut Vec<WalkFrame>,
    dir: PathBuf,
    rel: String,
    canon: PathBuf,
    chain: &[PathBuf],
    policy: &ResolvedPolicy,
) {
    if !should_watch_dir(&rel, policy) {
        return;
    }
    if chain.iter().any(|p| p == &canon) {
        return;
    }
    let mut next = chain.to_vec();
    next.push(canon);
    stack.push(WalkFrame {
        dir,
        rel,
        chain: next,
    });
}

fn maybe_add_file(
    out: &mut BTreeMap<String, DiskFile>,
    rel: String,
    abs: PathBuf,
    policy: &ResolvedPolicy,
) {
    if let Ok(rule) = classify(&rel, policy) {
        out.insert(
            rel,
            DiskFile {
                abs,
                extractor: rule.extractor.clone(),
            },
        );
    }
}

fn join_rel(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.replace('\\', "/")
    } else {
        format!("{parent}/{name}")
    }
}

fn is_path_prefix(prefix: &Path, path: &Path) -> bool {
    let mut path_c = path.components();
    for c in prefix.components() {
        match path_c.next() {
            Some(pc) if pc == c => {}
            _ => return false,
        }
    }
    true
}

fn sha256_file(path: &Path) -> Result<String, VaneCliError> {
    let mut file =
        File::open(path).map_err(|e| VaneCliError::new(format!("open {}: {e}", path.display())))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| VaneCliError::new(format!("read {}: {e}", path.display())))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

fn hex(digest: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(digest.len() * 2);
    for &b in digest {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}
