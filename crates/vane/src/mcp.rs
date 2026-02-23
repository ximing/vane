use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};

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
