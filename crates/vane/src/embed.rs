use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Deserialize;

use crate::config::EmbedConfig;
use crate::error::VaneCliError;

const OPENAI_BATCH: usize = 64;
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);
const PROBE_TEXT: &str = "probe";

pub trait Embedder: Send {
    fn probe_dim(&self) -> Result<u32, VaneCliError>;
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, VaneCliError>;
}

pub struct MockEmbedder {
    pub dim: u32,
    pub fail: bool,
    pub calls: Arc<Mutex<Vec<String>>>,
}

impl Embedder for MockEmbedder {
    fn probe_dim(&self) -> Result<u32, VaneCliError> {
        if self.fail {
            return Err(VaneCliError::new("embed failed"));
        }
        Ok(self.dim)
    }

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, VaneCliError> {
        if let Ok(mut calls) = self.calls.lock() {
            calls.extend(texts.iter().cloned());
        }
        if self.fail {
            return Err(VaneCliError::new("embed failed"));
        }
        let dim = self.dim as usize;
        Ok(texts.iter().map(|_| vec![0.0; dim]).collect())
    }
}

pub fn embed_model_id(provider: &str, model: &str, dim: u32) -> String {
    format!("{provider}:{model}:{dim}")
}

pub fn embedder_from_config(cfg: &EmbedConfig) -> Box<dyn Embedder> {
    match cfg.provider.as_str() {
        "openai_compat" => Box::new(openai_embedder(cfg)),
        _ => Box::new(ollama_embedder(cfg)),
    }
}

pub struct OllamaEmbedder {
    agent: ureq::Agent,
    url: String,
    model: String,
    dim: Mutex<Option<u32>>,
}

pub fn ollama_embedder(cfg: &EmbedConfig) -> OllamaEmbedder {
    OllamaEmbedder {
        agent: http_agent(),
        url: join_url(&cfg.base_url, "/api/embeddings"),
        model: cfg.model.clone(),
        dim: Mutex::new(None),
    }
}

impl Embedder for OllamaEmbedder {
    fn probe_dim(&self) -> Result<u32, VaneCliError> {
        if let Some(d) = cached_dim(&self.dim)? {
            return Ok(d);
        }
        let vecs = self.embed(&[PROBE_TEXT.to_string()])?;
        first_dim(&vecs)
    }

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, VaneCliError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let mut out = Vec::with_capacity(texts.len());
        for text in texts {
            let resp = self
                .agent
                .post(&self.url)
                .set("Content-Type", "application/json")
                .send_json(serde_json::json!({
                    "model": self.model,
                    "prompt": text,
                }))
                .map_err(|e| http_err("ollama", e))?;
            let parsed: OllamaResp = resp
                .into_json()
                .map_err(|e| VaneCliError::new(format!("ollama embed decode: {e}")))?;
            remember_dim(&self.dim, parsed.embedding.len() as u32)?;
            out.push(parsed.embedding);
        }
        Ok(out)
    }
}

#[derive(Deserialize)]
struct OllamaResp {
    embedding: Vec<f32>,
}

pub struct OpenAiCompatEmbedder {
    agent: ureq::Agent,
    url: String,
    model: String,
    api_key: Option<String>,
    dim: Mutex<Option<u32>>,
}

pub fn openai_embedder(cfg: &EmbedConfig) -> OpenAiCompatEmbedder {
    OpenAiCompatEmbedder {
        agent: http_agent(),
        url: openai_embeddings_url(&cfg.base_url),
        model: cfg.model.clone(),
        api_key: cfg.api_key.clone(),
        dim: Mutex::new(None),
    }
}

impl Embedder for OpenAiCompatEmbedder {
    fn probe_dim(&self) -> Result<u32, VaneCliError> {
        if let Some(d) = cached_dim(&self.dim)? {
            return Ok(d);
        }
        let vecs = self.embed(&[PROBE_TEXT.to_string()])?;
        first_dim(&vecs)
    }

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, VaneCliError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let mut out = Vec::with_capacity(texts.len());
        for chunk in texts.chunks(OPENAI_BATCH) {
            let mut req = self
                .agent
                .post(&self.url)
                .set("Content-Type", "application/json");
            if let Some(key) = resolve_api_key(self.api_key.as_deref()) {
                req = req.set("Authorization", &format!("Bearer {key}"));
            }
            let resp = req
                .send_json(serde_json::json!({
                    "model": self.model,
                    "input": chunk,
                }))
                .map_err(|e| http_err("openai_compat", e))?;
            let parsed: OpenAiResp = resp
                .into_json()
                .map_err(|e| VaneCliError::new(format!("openai_compat embed decode: {e}")))?;
            if parsed.data.len() != chunk.len() {
                return Err(VaneCliError::new(format!(
                    "openai_compat embed count: expected {}, got {}",
                    chunk.len(),
                    parsed.data.len()
                )));
            }
            let mut items = parsed.data;
            items.sort_by_key(|item| item.index.unwrap_or(0));
            for item in items {
                remember_dim(&self.dim, item.embedding.len() as u32)?;
                out.push(item.embedding);
            }
        }
        Ok(out)
    }
}

#[derive(Deserialize)]
struct OpenAiResp {
    data: Vec<OpenAiItem>,
}

#[derive(Deserialize)]
struct OpenAiItem {
    embedding: Vec<f32>,
    #[serde(default)]
    index: Option<u32>,
}

fn http_agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout(HTTP_TIMEOUT).build()
}

fn join_url(base: &str, path: &str) -> String {
    let base = base.trim_end_matches('/');
    let path = path.trim_start_matches('/');
    format!("{base}/{path}")
}

fn openai_embeddings_url(base: &str) -> String {
    let base = base.trim_end_matches('/');
    if base.ends_with("/embeddings") {
        base.to_string()
    } else if base.ends_with("/v1") {
        format!("{base}/embeddings")
    } else {
        format!("{base}/v1/embeddings")
    }
}

fn resolve_api_key(cfg_key: Option<&str>) -> Option<String> {
    if let Some(k) = cfg_key {
        if !k.is_empty() {
            return Some(k.to_string());
        }
    }
    match std::env::var("OPENAI_API_KEY") {
        Ok(k) if !k.is_empty() => return Some(k),
        _ => {}
    }
    match std::env::var("VANE_EMBED_API_KEY") {
        Ok(k) if !k.is_empty() => Some(k),
        _ => None,
    }
}

fn cached_dim(slot: &Mutex<Option<u32>>) -> Result<Option<u32>, VaneCliError> {
    let guard = slot
        .lock()
        .map_err(|_| VaneCliError::new("embedder lock poisoned"))?;
    Ok(*guard)
}

fn remember_dim(slot: &Mutex<Option<u32>>, got: u32) -> Result<(), VaneCliError> {
    let mut guard = slot
        .lock()
        .map_err(|_| VaneCliError::new("embedder lock poisoned"))?;
    match *guard {
        None => {
            *guard = Some(got);
            Ok(())
        }
        Some(d) if d == got => Ok(()),
        Some(d) => Err(VaneCliError::new(format!(
            "embedding dim changed: expected {d}, got {got}"
        ))),
    }
}

fn first_dim(vecs: &[Vec<f32>]) -> Result<u32, VaneCliError> {
    vecs.first()
        .map(|v| v.len() as u32)
        .ok_or_else(|| VaneCliError::new("embed probe returned no vectors"))
}

fn http_err(provider: &str, err: ureq::Error) -> VaneCliError {
    match err {
        ureq::Error::Status(code, resp) => {
            let body = resp.into_string().unwrap_or_default();
            let snippet: String = body.chars().take(200).collect();
            VaneCliError::new(format!("{provider} embed HTTP {code}: {snippet}"))
        }
        ureq::Error::Transport(t) => VaneCliError::new(format!("{provider} embed transport: {t}")),
    }
}
