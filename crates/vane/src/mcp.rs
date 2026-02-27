use std::fs::{self, File};
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{json, Value};

use crate::error::VaneCliError;
use crate::ipc::{rpc_call, INTERNAL_ERROR, INVALID_REQUEST, METHOD_NOT_FOUND};

pub const PROTOCOL_VERSION: &str = "2024-11-05";
pub const IMAGE_INLINE_MAX_BYTES: usize = 4 * 1024 * 1024;

/// JSON-RPC 2.0 MCP stdio loop. Proxies tools to the daemon socket; does not open Vane.
pub fn serve_stdio(home: PathBuf) -> Result<(), VaneCliError> {
    let config = home.join("config").join("config.toml");
    if !config.is_file() {
        return Err(VaneCliError::new("not initialized"));
    }
    let sock = crate::daemon::socket_path(&home);
    if std::os::unix::net::UnixStream::connect(&sock).is_err() {
        return Err(VaneCliError::new(
            "daemon is not running; run `vane start` or check the user service",
        ));
    }

    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    let stdout = std::io::stdout();
    let mut writer = stdout.lock();
    loop {
        match read_message(&mut reader) {
            Ok(msg) => {
                if let Some(resp) = handle_mcp_message(&msg, &home) {
                    write_message(&mut writer, &resp)?;
                }
            }
            Err(e) if is_stdio_eof(&e) => return Ok(()),
            Err(e) => {
                let err = json!({
                    "jsonrpc": "2.0",
                    "id": Value::Null,
                    "error": {"code": -32700, "message": e.message},
                });
                write_message(&mut writer, &err)?;
            }
        }
    }
}

pub fn read_message<R: BufRead>(reader: &mut R) -> Result<Value, VaneCliError> {
    let mut headers = String::new();
    loop {
        let mut line = String::new();
        let n = reader
            .read_line(&mut line)
            .map_err(|e| VaneCliError::new(format!("read mcp header: {e}")))?;
        if n == 0 {
            if headers.is_empty() {
                return Err(VaneCliError::new("eof"));
            }
            return Err(VaneCliError::new("truncated mcp headers"));
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        headers.push_str(&line);
    }
    let mut content_length = None;
    for raw in headers.split('\n') {
        let line = raw.trim_end_matches('\r').trim();
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("content-length") {
            let trimmed = value.trim();
            let v = trimmed
                .parse::<usize>()
                .map_err(|_| VaneCliError::new(format!("invalid Content-Length: {trimmed}")))?;
            content_length = Some(v);
        }
    }
    let len = content_length.ok_or_else(|| VaneCliError::new("missing Content-Length"))?;
    let mut buf = vec![0u8; len];
    Read::read_exact(reader, &mut buf)
        .map_err(|e| VaneCliError::new(format!("read mcp body: {e}")))?;
    serde_json::from_slice(&buf).map_err(|e| VaneCliError::new(format!("parse mcp json: {e}")))
}

pub fn write_message<W: Write>(writer: &mut W, msg: &Value) -> Result<(), VaneCliError> {
    let body =
        serde_json::to_vec(msg).map_err(|e| VaneCliError::new(format!("encode mcp: {e}")))?;
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())
        .map_err(|e| VaneCliError::new(format!("write mcp header: {e}")))?;
    writer
        .write_all(&body)
        .map_err(|e| VaneCliError::new(format!("write mcp body: {e}")))?;
    writer
        .flush()
        .map_err(|e| VaneCliError::new(format!("flush mcp: {e}")))
}

/// Handle one MCP JSON-RPC message. Returns `None` for notifications (no response).
pub fn handle_mcp_message(msg: &Value, home: &Path) -> Option<Value> {
    let method = match msg.get("method").and_then(Value::as_str) {
        Some(m) => m,
        None => {
            return Some(rpc_error(
                msg.get("id").cloned(),
                INVALID_REQUEST,
                "missing method",
            ));
        }
    };
    let id = msg.get("id").cloned();
    let has_id = id.as_ref().is_some_and(|v| !v.is_null());

    match method {
        "notifications/initialized" | "initialized" | "notifications/cancelled" => {
            return None;
        }
        _ if !has_id => return None,
        _ => {}
    }

    let result = match method {
        "initialize" => Ok(initialize_result(msg)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(tools_list()),
        "tools/call" => call_tool(msg, home),
        other => {
            return Some(rpc_error(
                id,
                METHOD_NOT_FOUND,
                format!("method not found: {other}"),
            ));
        }
    };
    Some(match result {
        Ok(v) => rpc_ok(id, v),
        Err(e) if method == "tools/call" => rpc_ok(
            id,
            json!({
                "content": [{"type": "text", "text": e.message}],
                "isError": true,
            }),
        ),
        Err(e) => rpc_error(id, INTERNAL_ERROR, e.message),
    })
}

/// Image `read`: inline ≤ 4 MiB as MCP image content; larger files get path + mime only.
pub fn encode_image_read(abs_path: &Path) -> Result<Vec<Value>, VaneCliError> {
    let meta = std::fs::metadata(abs_path)
        .map_err(|e| VaneCliError::new(format!("stat {}: {e}", abs_path.display())))?;
    let mime = mime_for_path(abs_path);
    let size = meta.len();
    if size > IMAGE_INLINE_MAX_BYTES as u64 {
        let hint = json!({
            "path": abs_path.display().to_string(),
            "mime": mime,
            "bytes": size,
            "hint": "file larger than 4 MiB; not inlined as base64",
        });
        return Ok(vec![json!({"type": "text", "text": pretty(&hint)?})]);
    }
    let bytes = std::fs::read(abs_path)
        .map_err(|e| VaneCliError::new(format!("read {}: {e}", abs_path.display())))?;
    Ok(vec![json!({
        "type": "image",
        "data": base64_encode(&bytes),
        "mimeType": mime,
    })])
}

fn initialize_result(msg: &Value) -> Value {
    let requested = msg
        .pointer("/params/protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or(PROTOCOL_VERSION);
    let version = match requested {
        "2024-11-05" | "2025-03-26" | "2025-06-18" => requested,
        _ => PROTOCOL_VERSION,
    };
    json!({
        "protocolVersion": version,
        "capabilities": {"tools": {}},
        "serverInfo": {
            "name": "vane",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "instructions": "Call list_roots, then search, then read. Do not walk the filesystem. The Vane daemon must be running.",
    })
}

fn tools_list() -> Value {
    json!({
        "tools": [
            {
                "name": "search",
                "description": "Hybrid search over registered document roots. Defaults to all projects. Filter with root (absolute path) and type (extractor name: text or image).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": {"type": "string", "description": "Search query"},
                        "root": {"type": "string", "description": "Optional registered root path"},
                        "type": {"type": "string", "description": "Extractor name: text or image"},
                        "top_k": {"type": "integer", "minimum": 1, "maximum": 50, "default": 8}
                    },
                    "required": ["query"]
                }
            },
            {
                "name": "read",
                "description": "Read a document by id (one chunk) or path (all chunks, ascending). Images ≤ 4 MiB are returned as MCP image content; larger images return path and MIME only.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": {"type": "string", "description": "Document id from search"},
                        "path": {"type": "string", "description": "Relative path within a root"},
                        "root": {"type": "string", "description": "Registered root when path is ambiguous"}
                    }
                }
            },
            {
                "name": "list_roots",
                "description": "List registered roots, project ids, models, and live file counts.",
                "inputSchema": {
                    "type": "object",
                    "properties": {}
                }
            }
        ]
    })
}

fn call_tool(msg: &Value, home: &Path) -> Result<Value, VaneCliError> {
    let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| VaneCliError::new("missing params.name"))?;
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    match name {
        "search" => {
            let result = rpc_call(home, "search", args)?;
            Ok(text_result(&pretty(&result)?))
        }
        "list_roots" => {
            let result = rpc_call(home, "list_roots", json!({}))?;
            Ok(text_result(&pretty(&result)?))
        }
        "read" => wrap_read(rpc_call(home, "read", args)?),
        other => Err(VaneCliError::new(format!("unknown tool: {other}"))),
    }
}

fn wrap_read(result: Value) -> Result<Value, VaneCliError> {
    let chunks = match result {
        Value::Array(a) => a,
        other => vec![other],
    };
    let mut content = Vec::new();
    let mut saw_image = false;
    for chunk in &chunks {
        let modality = chunk
            .get("modality")
            .and_then(Value::as_str)
            .unwrap_or("text");
        if modality == "image" {
            saw_image = true;
            let abs = chunk
                .get("abs_path")
                .and_then(Value::as_str)
                .ok_or_else(|| VaneCliError::new("image read missing abs_path"))?;
            content.extend(encode_image_read(Path::new(abs))?);
        }
    }
    if !saw_image {
        let payload = if chunks.len() == 1 {
            chunks.into_iter().next().unwrap_or(Value::Null)
        } else {
            Value::Array(chunks)
        };
        content.push(json!({"type": "text", "text": pretty(&payload)?}));
    }
    Ok(json!({"content": content, "isError": false}))
}

fn mime_for_path(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => "application/octet-stream",
    }
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        let n = (u32::from(b0) << 16) | (u32::from(b1) << 8) | u32::from(b2);
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((n >> 6) & 63) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(n & 63) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn pretty(v: &Value) -> Result<String, VaneCliError> {
    serde_json::to_string_pretty(v).map_err(|e| VaneCliError::new(format!("encode json: {e}")))
}

fn text_result(text: &str) -> Value {
    json!({
        "content": [{"type": "text", "text": text}],
        "isError": false,
    })
}

fn rpc_ok(id: Option<Value>, result: Value) -> Value {
    let mut resp = json!({
        "jsonrpc": "2.0",
        "result": result,
    });
    resp["id"] = id.unwrap_or(Value::Null);
    resp
}

fn rpc_error(id: Option<Value>, code: i32, message: impl Into<String>) -> Value {
    let mut resp = json!({
        "jsonrpc": "2.0",
        "error": {
            "code": code,
            "message": message.into(),
        },
    });
    resp["id"] = id.unwrap_or(Value::Null);
    resp
}

fn is_stdio_eof(err: &VaneCliError) -> bool {
    err.message == "eof"
        || err.message.starts_with("read mcp header:")
        || err.message.starts_with("read mcp body:")
}

/// MCP client whose config `vane mcp install` knows how to merge.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpClient {
    Claude,
    Cursor,
    Codex,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpInstallTarget {
    pub path: String,
    pub client: String,
    pub action: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpInstallSkip {
    pub path: String,
    pub client: String,
    pub reason: String,
}

/// TTY / piped report for `vane mcp install`. `--dry-run` fills `would_write` only.
#[derive(Debug, Clone, Serialize)]
pub struct McpInstallReport {
    pub ok: bool,
    pub dry_run: bool,
    pub would_write: Vec<McpInstallTarget>,
    pub written: Vec<McpInstallTarget>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<McpInstallSkip>,
}

/// Merge `mcpServers.vane` / `[mcp_servers.vane]` into client configs under `user_home`.
///
/// `user_home` is `$HOME` from the environment (tests pass a fake home). Never creates
/// Claude Desktop dirs. Creates missing Claude/Cursor files; Codex/Grok only if present.
pub fn install_mcp(
    user_home: &Path,
    dry_run: bool,
    client: Option<McpClient>,
) -> Result<McpInstallReport, VaneCliError> {
    if user_home.as_os_str().is_empty() {
        return Err(VaneCliError::new("HOME is empty"));
    }
    let jobs = install_jobs(user_home, client);
    let mut would_write = Vec::new();
    let mut written = Vec::new();
    let mut skipped = Vec::new();

    for job in &jobs {
        match prepare_job(job)? {
            PreparedJob::Skip { reason } => {
                skipped.push(McpInstallSkip {
                    path: job.path.display().to_string(),
                    client: job.client.to_string(),
                    reason,
                });
            }
            PreparedJob::Write { action, bytes } => {
                let target = McpInstallTarget {
                    path: job.path.display().to_string(),
                    client: job.client.to_string(),
                    action: action.to_string(),
                };
                if dry_run {
                    would_write.push(target);
                } else {
                    atomic_write(&job.path, &bytes, job.client)?;
                    written.push(target);
                }
            }
        }
    }

    if matches!(client, Some(McpClient::Codex))
        && would_write.is_empty()
        && written.is_empty()
        && skipped.is_empty()
    {
        skipped.push(McpInstallSkip {
            path: user_home.join(".codex").display().to_string(),
            client: "codex".into(),
            reason: "no existing Codex MCP config (will not create)".into(),
        });
    }

    Ok(McpInstallReport {
        ok: true,
        dry_run,
        would_write,
        written,
        skipped,
    })
}

struct InstallJob {
    client: &'static str,
    path: PathBuf,
    format: ConfigFormat,
    create: bool,
}

#[derive(Clone, Copy)]
enum ConfigFormat {
    Json,
    Toml,
}

enum PreparedJob {
    Skip {
        reason: String,
    },
    Write {
        action: &'static str,
        bytes: Vec<u8>,
    },
}

fn install_jobs(user_home: &Path, client: Option<McpClient>) -> Vec<InstallJob> {
    let all = client.is_none();
    let mut jobs = Vec::new();
    if all || client == Some(McpClient::Claude) {
        jobs.push(InstallJob {
            client: "claude",
            path: user_home.join(".claude.json"),
            format: ConfigFormat::Json,
            create: true,
        });
    }
    if all || client == Some(McpClient::Cursor) {
        jobs.push(InstallJob {
            client: "cursor",
            path: user_home.join(".cursor").join("mcp.json"),
            format: ConfigFormat::Json,
            create: true,
        });
    }
    if all || client == Some(McpClient::Codex) {
        for name in ["mcp.json", "config.json"] {
            let path = user_home.join(".codex").join(name);
            if path.is_file() {
                jobs.push(InstallJob {
                    client: "codex",
                    path,
                    format: ConfigFormat::Json,
                    create: false,
                });
            }
        }
        let toml_path = user_home.join(".codex").join("config.toml");
        if toml_path.is_file() {
            jobs.push(InstallJob {
                client: "codex",
                path: toml_path,
                format: ConfigFormat::Toml,
                create: false,
            });
        }
    }
    if all {
        let grok = user_home.join(".grok").join("config.toml");
        if grok.is_file() {
            jobs.push(InstallJob {
                client: "grok",
                path: grok,
                format: ConfigFormat::Toml,
                create: false,
            });
        }
    }
    jobs
}

fn prepare_job(job: &InstallJob) -> Result<PreparedJob, VaneCliError> {
    let exists = job.path.is_file();
    if !exists && !job.create {
        return Ok(PreparedJob::Skip {
            reason: "not present".into(),
        });
    }
    let action = if exists { "merge" } else { "create" };
    let bytes = match job.format {
        ConfigFormat::Json => {
            let root = if exists {
                load_json_object(&job.path)?
            } else {
                json!({})
            };
            let merged = merge_vane_into_json(root, &job.path)?;
            encode_json_pretty(&merged, &job.path)?
        }
        ConfigFormat::Toml => {
            let root = if exists {
                load_toml_table(&job.path)?
            } else {
                toml::Value::Table(toml::map::Map::new())
            };
            let merged = merge_vane_into_toml(root, &job.path)?;
            encode_toml_pretty(&merged, &job.path)?
        }
    };
    Ok(PreparedJob::Write { action, bytes })
}

fn load_json_object(path: &Path) -> Result<Value, VaneCliError> {
    let text = fs::read_to_string(path)
        .map_err(|e| VaneCliError::new(format!("read {}: {e}", path.display())))?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(json!({}));
    }
    let value: Value = serde_json::from_str(trimmed)
        .map_err(|e| VaneCliError::new(format!("parse {}: {e}", path.display())))?;
    if !value.is_object() {
        return Err(VaneCliError::new(format!(
            "{} root must be a JSON object",
            path.display()
        )));
    }
    Ok(value)
}

fn load_toml_table(path: &Path) -> Result<toml::Value, VaneCliError> {
    let text = fs::read_to_string(path)
        .map_err(|e| VaneCliError::new(format!("read {}: {e}", path.display())))?;
    if text.trim().is_empty() {
        return Ok(toml::Value::Table(toml::map::Map::new()));
    }
    let value: toml::Value = text
        .parse()
        .map_err(|e| VaneCliError::new(format!("parse {}: {e}", path.display())))?;
    if !value.is_table() {
        return Err(VaneCliError::new(format!(
            "{} root must be a TOML table",
            path.display()
        )));
    }
    Ok(value)
}

fn merge_vane_into_json(mut root: Value, path: &Path) -> Result<Value, VaneCliError> {
    let obj = root.as_object_mut().ok_or_else(|| {
        VaneCliError::new(format!("{} root must be a JSON object", path.display()))
    })?;
    let servers = obj.entry("mcpServers").or_insert_with(|| json!({}));
    let servers_obj = servers.as_object_mut().ok_or_else(|| {
        VaneCliError::new(format!(
            "mcpServers in {} must be a JSON object",
            path.display()
        ))
    })?;
    let mut vane = servers_obj
        .get("vane")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    vane.insert("command".into(), json!("vane"));
    vane.insert("args".into(), json!(["mcp"]));
    servers_obj.insert("vane".into(), Value::Object(vane));
    Ok(root)
}

fn merge_vane_into_toml(mut root: toml::Value, path: &Path) -> Result<toml::Value, VaneCliError> {
    let table = root.as_table_mut().ok_or_else(|| {
        VaneCliError::new(format!("{} root must be a TOML table", path.display()))
    })?;
    let servers = table
        .entry("mcp_servers".to_string())
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    let servers_tbl = servers.as_table_mut().ok_or_else(|| {
        VaneCliError::new(format!("mcp_servers in {} must be a table", path.display()))
    })?;
    if !servers_tbl.get("vane").is_some_and(toml::Value::is_table) {
        servers_tbl.insert("vane".into(), toml::Value::Table(toml::map::Map::new()));
    }
    let vane = servers_tbl
        .get_mut("vane")
        .and_then(toml::Value::as_table_mut)
        .ok_or_else(|| {
            VaneCliError::new(format!(
                "mcp_servers.vane in {} must be a table",
                path.display()
            ))
        })?;
    vane.insert("command".into(), toml::Value::String("vane".into()));
    vane.insert(
        "args".into(),
        toml::Value::Array(vec![toml::Value::String("mcp".into())]),
    );
    Ok(root)
}

fn encode_json_pretty(value: &Value, path: &Path) -> Result<Vec<u8>, VaneCliError> {
    let mut body = serde_json::to_vec_pretty(value)
        .map_err(|e| VaneCliError::new(format!("encode {}: {e}", path.display())))?;
    if !body.ends_with(b"\n") {
        body.push(b'\n');
    }
    Ok(body)
}

fn encode_toml_pretty(value: &toml::Value, path: &Path) -> Result<Vec<u8>, VaneCliError> {
    let mut body = toml::to_string_pretty(value)
        .map_err(|e| VaneCliError::new(format!("encode {}: {e}", path.display())))?;
    if !body.ends_with('\n') {
        body.push('\n');
    }
    Ok(body.into_bytes())
}

fn atomic_write(path: &Path, bytes: &[u8], label: &str) -> Result<(), VaneCliError> {
    let dir = path.parent().ok_or_else(|| {
        VaneCliError::new(format!("{label} path has no parent: {}", path.display()))
    })?;
    fs::create_dir_all(dir).map_err(|e| {
        VaneCliError::new(format!("create {} parent {}: {e}", label, dir.display()))
    })?;
    let tmp = dir.join(format!(
        "{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or(label)
    ));
    {
        let mut f = File::create(&tmp).map_err(|e| {
            VaneCliError::new(format!("create {} temp {}: {e}", label, tmp.display()))
        })?;
        f.write_all(bytes).map_err(|e| {
            VaneCliError::new(format!("write {} temp {}: {e}", label, tmp.display()))
        })?;
        f.sync_all().map_err(|e| {
            VaneCliError::new(format!("sync {} temp {}: {e}", label, tmp.display()))
        })?;
    }
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        VaneCliError::new(format!(
            "rename {} -> {}: {e}",
            tmp.display(),
            path.display()
        ))
    })
}
