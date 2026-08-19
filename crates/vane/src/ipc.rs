use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::VaneCliError;

pub const PARSE_ERROR: i32 = -32700;
pub const INVALID_REQUEST: i32 = -32600;
pub const METHOD_NOT_FOUND: i32 = -32601;
pub const INVALID_PARAMS: i32 = -32602;
pub const INTERNAL_ERROR: i32 = -32603;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RpcRequest {
    pub id: String,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RpcResponse {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl RpcResponse {
    pub fn ok(id: impl Into<String>, result: Value) -> Self {
        Self {
            id: id.into(),
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: impl Into<String>, code: i32, message: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            result: None,
            error: Some(RpcError {
                code,
                message: message.into(),
            }),
        }
    }
}

pub fn parse_request(line: &str) -> Result<RpcRequest, VaneCliError> {
    serde_json::from_str(line.trim()).map_err(|e| VaneCliError::new(format!("invalid rpc: {e}")))
}

pub fn encode_response(resp: &RpcResponse) -> Result<String, VaneCliError> {
    serde_json::to_string(resp).map_err(|e| VaneCliError::new(format!("encode rpc: {e}")))
}

fn rpc_timeout(method: &str) -> Duration {
    match method {
        "rebuild" | "set_model" | "gc" | "reload_config" | "add_root" => Duration::from_secs(600),
        _ => Duration::from_secs(30),
    }
}

pub fn rpc_call(home: &Path, method: &str, params: Value) -> Result<Value, VaneCliError> {
    let sock = crate::daemon::socket_path(home);
    let mut stream = UnixStream::connect(&sock).map_err(|_| {
        VaneCliError::new("daemon is not running; run `vane start` or check the user service")
    })?;
    let timeout = rpc_timeout(method);
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));
    let req = RpcRequest {
        id: "1".into(),
        method: method.to_string(),
        params,
    };
    let encoded =
        serde_json::to_string(&req).map_err(|e| VaneCliError::new(format!("encode rpc: {e}")))?;
    writeln!(stream, "{encoded}").map_err(|e| VaneCliError::new(format!("write rpc: {e}")))?;
    stream
        .flush()
        .map_err(|e| VaneCliError::new(format!("flush rpc: {e}")))?;
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .map_err(|e| VaneCliError::new(format!("read rpc: {e}")))?;
    if line.trim().is_empty() {
        return Err(VaneCliError::new("empty rpc response"));
    }
    let resp: RpcResponse = serde_json::from_str(line.trim())
        .map_err(|e| VaneCliError::new(format!("decode rpc: {e}")))?;
    if let Some(err) = resp.error {
        return Err(VaneCliError::new(err.message));
    }
    resp.result
        .ok_or_else(|| VaneCliError::new("rpc response missing result"))
}
