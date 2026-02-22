use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use vane_core::api::{Filter, FilterCond, FusionSpec, ScalarValue, SearchMode, SearchQuery};

use crate::cas::Cas;
use crate::embed::Embedder;
use crate::error::VaneCliError;
use crate::extract::CanonicalDoc;
use crate::index::{doc_id, ProjectIndex};
use crate::live::LiveSet;
use crate::rrf::rrf_merge;

const DEFAULT_TOP_K: u32 = 8;
const MAX_TOP_K: u32 = 50;
const RRF_K: u32 = 60;
const SNIPPET_CHARS: usize = 240;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub id: String,
    pub path: String,
    pub root: String,
    pub title: String,
    pub snippet: String,
    pub score: f32,
    pub modality: String,
    pub extractor: String,
    pub degraded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadChunk {
    pub id: String,
    pub path: String,
    pub root: String,
    pub text: String,
    pub headings: Vec<String>,
    pub start_byte: u64,
    pub end_byte: u64,
    pub abs_path: String,
    pub modality: String,
    pub extractor: String,
    pub chunk_index: u32,
}

pub struct ProjectSearch<'a> {
    pub index: &'a ProjectIndex,
    pub embedder: &'a dyn Embedder,
    pub cas: &'a Cas,
    pub live: &'a LiveSet,
    pub root: &'a str,
    pub extractor: Option<&'a str>,
}

/// Strip a leading breadcrumb line, then keep the first 240 Unicode scalars.
pub fn snippet(canonical_text: &str) -> String {
    take_snippet(body_after_breadcrumb(canonical_text, None))
}

pub fn search_project(
    project: &ProjectSearch<'_>,
    query: &str,
    top_k: u32,
) -> Result<Vec<SearchHit>, VaneCliError> {
    let top_k = clamp_top_k(top_k);
    let (vector, degraded) = try_embed(project.embedder, query);
    search_with_vec(project, query, top_k, vector, degraded)
}

pub fn search_all(
    projects: &[ProjectSearch<'_>],
    query: &str,
    top_k: u32,
) -> Result<Vec<SearchHit>, VaneCliError> {
    if projects.is_empty() {
        return Ok(Vec::new());
    }
    let top_k = clamp_top_k(top_k);
    let mut cache: HashMap<String, (Option<Vec<f32>>, bool)> = HashMap::new();
    let mut lists = Vec::with_capacity(projects.len());
    for project in projects {
        let model_id = project.index.model_id().to_string();
        let (vector, degraded) = if let Some(cached) = cache.get(&model_id) {
            cached.clone()
        } else {
            let got = try_embed(project.embedder, query);
            cache.insert(model_id, got.clone());
            got
        };
        lists.push(search_with_vec(project, query, top_k, vector, degraded)?);
    }
    Ok(rrf_merge(lists, RRF_K, top_k as usize))
}

pub fn read_by_id(
    cas: &Cas,
    live: &LiveSet,
    project_id: &str,
    root: &Path,
    id: &str,
) -> Result<ReadChunk, VaneCliError> {
    let (pid, path, chunk_index) = parse_doc_id(id)?;
    let pid = if project_id.is_empty() {
        pid.as_str()
    } else {
        project_id
    };
    read_by_path(cas, live, pid, root, &path)?
        .into_iter()
        .find(|c| c.chunk_index == chunk_index)
        .ok_or_else(|| VaneCliError::new(format!("chunk not found: {id}")))
}

pub fn read_by_path(
    cas: &Cas,
    live: &LiveSet,
    project_id: &str,
    root: &Path,
    rel_path: &str,
) -> Result<Vec<ReadChunk>, VaneCliError> {
    let rel_path = rel_path.replace('\\', "/");
    let file = live
        .files
        .get(&rel_path)
        .ok_or_else(|| VaneCliError::new(format!("path not in working set: {rel_path}")))?;
    let mut docs = cas
        .get_extract(&file.extract_key)
        .ok_or_else(|| VaneCliError::new(format!("extract CAS miss for {rel_path}")))?;
    docs.sort_by_key(|d| d.chunk_index);
    Ok(docs
        .into_iter()
        .map(|d| read_chunk_from_doc(project_id, root, d))
        .collect())
}

pub fn parse_doc_id(id: &str) -> Result<(String, String, u32), VaneCliError> {
    let (left, chunk_s) = id
        .rsplit_once('#')
        .ok_or_else(|| VaneCliError::new(format!("invalid doc id: {id}")))?;
    let chunk_index: u32 = chunk_s
        .parse()
        .map_err(|_| VaneCliError::new(format!("invalid doc id chunk: {id}")))?;
    let (project_id, rel_path) = left
        .split_once(':')
        .ok_or_else(|| VaneCliError::new(format!("invalid doc id: {id}")))?;
    if project_id.is_empty() || rel_path.is_empty() {
        return Err(VaneCliError::new(format!("invalid doc id: {id}")));
    }
    Ok((project_id.to_string(), rel_path.to_string(), chunk_index))
}

pub fn clamp_top_k(n: u32) -> u32 {
    if n == 0 {
        DEFAULT_TOP_K
    } else {
        n.min(MAX_TOP_K)
    }
}

fn try_embed(embedder: &dyn Embedder, query: &str) -> (Option<Vec<f32>>, bool) {
    match embedder.embed(&[query.to_string()]) {
        Ok(mut vs) => match vs.pop() {
            Some(v) if !v.is_empty() => (Some(v), false),
            _ => (None, true),
        },
        Err(_) => (None, true),
    }
}

fn search_with_vec(
    project: &ProjectSearch<'_>,
    query: &str,
    top_k: u32,
    vector: Option<Vec<f32>>,
    degraded: bool,
) -> Result<Vec<SearchHit>, VaneCliError> {
    let filter = Some(build_filter(project.root, project.extractor));
    let run = |vector: Option<Vec<f32>>, mode: SearchMode, degraded: bool| {
        let q = SearchQuery {
            text: Some(query.to_string()),
            vector,
            top_k,
            mode,
            fusion: FusionSpec::Rrf,
            filter: filter.clone(),
            candidate_multiplier: 3,
        };
        project
            .index
            .search(&q)
            .map(|hits| hits_from_core(hits, project, degraded))
    };

    if let Some(vec) = vector {
        match run(Some(vec), SearchMode::Hybrid, degraded) {
            Ok(hits) => return Ok(hits),
            Err(_) => return run(None, SearchMode::Text, true),
        }
    }
    run(None, SearchMode::Text, true)
}

fn build_filter(root: &str, extractor: Option<&str>) -> Filter {
    let mut fields = vec![(
        "root".into(),
        FilterCond::Eq(ScalarValue::Keyword(root.to_string())),
    )];
    if let Some(ext) = extractor {
        if !ext.is_empty() {
            fields.push((
                "extractor".into(),
                FilterCond::Eq(ScalarValue::Keyword(ext.to_string())),
            ));
        }
    }
    Filter { fields }
}

fn hits_from_core(
    hits: Vec<vane_core::api::Hit>,
    project: &ProjectSearch<'_>,
    degraded: bool,
) -> Vec<SearchHit> {
    hits.into_iter()
        .map(|hit| {
            let mut path = stored_string(hit.fields.as_ref(), "path");
            if path.is_empty() {
                if let Ok((_, p, _)) = parse_doc_id(&hit.id) {
                    path = p;
                }
            }
            let stored_root = stored_string(hit.fields.as_ref(), "root");
            let root = if stored_root.is_empty() {
                project.root.to_string()
            } else {
                stored_root
            };
            let modality = stored_string(hit.fields.as_ref(), "modality");
            let extractor = stored_string(hit.fields.as_ref(), "extractor");
            let chunk_index = stored_u32(hit.fields.as_ref(), "chunk_index");
            let (title, snippet) = lookup_display(project.cas, project.live, &path, chunk_index)
                .unwrap_or_else(|| (file_title(&path), String::new()));
            SearchHit {
                id: hit.id,
                path,
                root,
                title,
                snippet,
                score: hit.score,
                modality: if modality.is_empty() {
                    "text".into()
                } else {
                    modality
                },
                extractor: if extractor.is_empty() {
                    "text".into()
                } else {
                    extractor
                },
                degraded,
            }
        })
        .collect()
}

fn lookup_display(
    cas: &Cas,
    live: &LiveSet,
    path: &str,
    chunk_index: Option<u32>,
) -> Option<(String, String)> {
    let file = live.files.get(path)?;
    let docs = cas.get_extract(&file.extract_key)?;
    let doc = match chunk_index {
        Some(idx) => docs
            .iter()
            .find(|d| d.chunk_index == idx)
            .or_else(|| docs.first())?,
        None => docs.first()?,
    };
    let title = if doc.headings.is_empty() {
        file_title(&doc.path)
    } else {
        doc.headings.join(" > ")
    };
    Some((title, snippet_from_doc(doc)))
}

fn snippet_from_doc(doc: &CanonicalDoc) -> String {
    let crumb = if doc.headings.is_empty() {
        None
    } else {
        Some(doc.headings.join(" > "))
    };
    take_snippet(body_after_breadcrumb(&doc.text, crumb.as_deref()))
}

fn body_after_breadcrumb<'a>(text: &'a str, headings_crumb: Option<&str>) -> &'a str {
    let Some((first, rest)) = text.split_once('\n') else {
        return text;
    };
    if first.contains(" > ") || headings_crumb.is_some_and(|c| first == c) {
        rest
    } else {
        text
    }
}

fn take_snippet(body: &str) -> String {
    body.chars().take(SNIPPET_CHARS).collect()
}

fn file_title(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

fn stored_string(fields: Option<&HashMap<String, String>>, key: &str) -> String {
    let Some(raw) = fields.and_then(|f| f.get(key)) else {
        return String::new();
    };
    if let Ok(s) = serde_json::from_str::<String>(raw) {
        return s;
    }
    raw.trim_matches('"').to_string()
}

fn stored_u32(fields: Option<&HashMap<String, String>>, key: &str) -> Option<u32> {
    let raw = stored_string(fields, key);
    if raw.is_empty() {
        return None;
    }
    raw.parse().ok()
}

fn read_chunk_from_doc(project_id: &str, root: &Path, doc: CanonicalDoc) -> ReadChunk {
    ReadChunk {
        id: doc_id(project_id, &doc.path, doc.chunk_index),
        path: doc.path.clone(),
        root: root.to_string_lossy().into_owned(),
        abs_path: root.join(&doc.path).to_string_lossy().into_owned(),
        text: doc.text,
        headings: doc.headings,
        start_byte: doc.start_byte,
        end_byte: doc.end_byte,
        modality: doc.modality,
        extractor: doc.extractor,
        chunk_index: doc.chunk_index,
    }
}
