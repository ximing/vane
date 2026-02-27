use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use vane::cas::Cas;
use vane::extract::extract_image;
use vane::live::{LiveFile, LiveSet};
use vane::project::project_id;

/// 1×1 PNG; MCP `read` inlines files ≤ 4 MiB as image content.
const TINY_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53,
    0xde, 0x00, 0x00, 0x00, 0x0c, 0x49, 0x44, 0x41, 0x54, 0x08, 0xd7, 0x63, 0xf8, 0xcf, 0xc0, 0x00,
    0x00, 0x00, 0x03, 0x00, 0x01, 0x00, 0x05, 0xfe, 0xd4, 0xef, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
    0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

struct TempHome {
    path: PathBuf,
}

fn tempfile_dir() -> TempHome {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    // Unix socket paths are capped (~104 bytes on macOS). Keep the home short.
    let path = std::env::temp_dir().join(format!(
        "vm{}-{n}-{:x}",
        std::process::id(),
        (nanos % 0xfff) as u16
    ));
    fs::create_dir_all(&path).unwrap();
    TempHome { path }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let tmp = std::env::temp_dir();
        if self.path.starts_with(&tmp) && self.path != tmp {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

impl std::ops::Deref for TempHome {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.path
    }
}

fn dirs_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn assert_isolated(home: &Path) {
    assert!(home.starts_with(std::env::temp_dir()));
    assert!(!home.starts_with(dirs_home().join(".vane")));
}

fn fake_user_home(home: &Path) -> PathBuf {
    let fake = home.join("fake-user-home");
    fs::create_dir_all(&fake).unwrap();
    fake
}

fn write_config(home: &Path, project: &Path) {
    let cfg = home.join("config").join("config.toml");
    fs::create_dir_all(cfg.parent().unwrap()).unwrap();
    fs::write(
        &cfg,
        format!(
            r#"
[defaults.embed]
provider = "ollama"
model = "nomic-embed-text"
base_url = "http://127.0.0.1:9"

[[projects]]
path = "{}"
"#,
            project.display()
        ),
    )
    .unwrap();
}

struct DaemonProcess {
    child: Child,
}

impl Drop for DaemonProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn socket_path(home: &Path) -> PathBuf {
    home.join("run").join("vane.sock")
}

fn wait_for_socket(home: &Path, child: &mut Child, timeout: Duration) {
    let sock = socket_path(home);
    let start = Instant::now();
    while start.elapsed() < timeout {
        if let Some(status) = child.try_wait().unwrap() {
            let mut err = String::new();
            if let Some(mut pipe) = child.stderr.take() {
                use std::io::Read;
                let _ = pipe.read_to_string(&mut err);
            }
            panic!("daemon exited early {status}: {err}");
        }
        if std::os::unix::net::UnixStream::connect(&sock).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("daemon socket not ready at {}", sock.display());
}

fn spawn_daemon(home: &Path) -> DaemonProcess {
    let bin = env!("CARGO_BIN_EXE_vane");
    let fake_home = fake_user_home(home);
    let mut child = Command::new(bin)
        .args(["daemon", "--home", home.to_str().expect("utf-8 home")])
        .env("VANE_HOME", home)
        .env("HOME", &fake_home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn vane daemon");
    wait_for_socket(home, &mut child, Duration::from_secs(5));
    DaemonProcess { child }
}

struct McpProcess {
    child: Child,
    stdin: std::process::ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
}

impl Drop for McpProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn spawn_mcp(home: &Path) -> McpProcess {
    let bin = env!("CARGO_BIN_EXE_vane");
    let mut child = Command::new(bin)
        .args(["mcp", "--home", home.to_str().expect("utf-8 home")])
        .env("VANE_HOME", home)
        .env("HOME", fake_user_home(home))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn vane mcp");
    let stdin = child.stdin.take().expect("mcp stdin");
    let stdout = BufReader::new(child.stdout.take().expect("mcp stdout"));
    McpProcess {
        child,
        stdin,
        stdout,
    }
}

fn send_mcp(stdin: &mut impl Write, msg: &Value) {
    vane::mcp::write_message(stdin, msg).expect("write mcp frame");
}

fn recv_mcp(stdout: &mut impl BufRead) -> Value {
    vane::mcp::read_message(stdout).expect("read mcp frame")
}

fn seed_png(home: &Path, project: &Path, rel: &str, bytes: &[u8]) {
    let abs = project.join(rel);
    if let Some(parent) = abs.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&abs, bytes).unwrap();
    let docs = extract_image(rel, bytes).unwrap();
    let pid = project_id(
        &project
            .canonicalize()
            .unwrap_or_else(|_| project.to_path_buf()),
    );
    let cas = Cas::new(home.join("rag").join("cas"));
    let key = format!("ek-{pid}-{rel}");
    cas.put_extract(&key, &docs).unwrap();
    let mut live = LiveSet::load_for_project(home, &pid).unwrap();
    live.files.insert(
        rel.replace('\\', "/"),
        LiveFile {
            content_sha256: "png".into(),
            extract_key: key,
            chunk_count: 1,
        },
    );
    live.save_for_project(home, &pid).unwrap();
}

fn decode_base64(s: &str) -> Vec<u8> {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut vals = [0xFFu8; 256];
    for (i, &c) in TABLE.iter().enumerate() {
        vals[c as usize] = i as u8;
    }
    let cleaned: Vec<u8> = s.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < cleaned.len() {
        let mut n = 0u32;
        let mut pad = 0;
        for k in 0..4 {
            let b = *cleaned.get(i + k).unwrap_or(&b'=');
            if b == b'=' {
                pad += 1;
                continue;
            }
            let v = vals[b as usize];
            assert_ne!(v, 0xFF, "invalid base64 byte {b}");
            n |= u32::from(v) << (18 - 6 * k);
        }
        out.push((n >> 16) as u8);
        if pad < 2 {
            out.push((n >> 8) as u8);
        }
        if pad < 1 {
            out.push(n as u8);
        }
        i += 4;
    }
    out
}

fn tool_names(resp: &Value) -> Vec<String> {
    resp["result"]["tools"]
        .as_array()
        .expect("result.tools")
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn content_length_roundtrip() {
    let tmp = tempfile_dir();
    assert_isolated(&tmp);
    let msg = json!({"jsonrpc":"2.0","id":1,"method":"ping"});
    let mut buf = Vec::new();
    vane::mcp::write_message(&mut buf, &msg).unwrap();
    let framed = String::from_utf8_lossy(&buf);
    assert!(
        framed.starts_with("Content-Length:"),
        "MCP uses Content-Length framing, got {framed:?}"
    );
    assert!(framed.contains("\r\n\r\n"));
    let got = vane::mcp::read_message(&mut BufReader::new(buf.as_slice())).unwrap();
    assert_eq!(got["method"], "ping");
    assert_eq!(got["id"], 1);
}

#[test]
fn initialize_and_tools_list_expose_three_tools() {
    let tmp = tempfile_dir();
    assert_isolated(&tmp);
    let dummy: &Path = &tmp;

    let init = vane::mcp::handle_mcp_message(
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "test", "version": "0"}
            }
        }),
        dummy,
    )
    .expect("initialize is a request");
    assert_eq!(init["id"], 1);
    assert_eq!(init["jsonrpc"], "2.0");
    assert_eq!(init["result"]["serverInfo"]["name"], "vane");
    assert!(init["result"]["capabilities"]["tools"].is_object());

    let listed = vane::mcp::handle_mcp_message(
        &json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list"
        }),
        dummy,
    )
    .expect("tools/list is a request");
    let names = tool_names(&listed);
    assert!(names.contains(&"search".to_string()), "tools={names:?}");
    assert!(names.contains(&"read".to_string()), "tools={names:?}");
    assert!(names.contains(&"list_roots".to_string()), "tools={names:?}");
    assert_eq!(names.len(), 3, "exactly the three MCP tools, got {names:?}");
}

#[test]
fn mcp_child_initialize_and_tools_list() {
    let tmp = tempfile_dir();
    assert_isolated(&tmp);
    let project = tmp.join("proj");
    fs::create_dir_all(&project).unwrap();
    write_config(&tmp, &project);
    let _daemon = spawn_daemon(&tmp);

    let mut mcp = spawn_mcp(&tmp);
    send_mcp(
        &mut mcp.stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "test", "version": "0"}
            }
        }),
    );
    let init = recv_mcp(&mut mcp.stdout);
    assert_eq!(init["result"]["serverInfo"]["name"], "vane");

    send_mcp(
        &mut mcp.stdin,
        &json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
    );
    let listed = recv_mcp(&mut mcp.stdout);
    let names = tool_names(&listed);
    assert!(names.contains(&"search".to_string()), "tools={names:?}");
    assert!(names.contains(&"read".to_string()), "tools={names:?}");
    assert!(names.contains(&"list_roots".to_string()), "tools={names:?}");
}

#[test]
fn read_small_png_returns_image_payload() {
    let tmp = tempfile_dir();
    assert_isolated(&tmp);
    let project = tmp.join("proj");
    fs::create_dir_all(&project).unwrap();
    write_config(&tmp, &project);
    seed_png(&tmp, &project, "photo.png", TINY_PNG);
    let _daemon = spawn_daemon(&tmp);

    let resp = vane::mcp::handle_mcp_message(
        &json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/call",
            "params": {
                "name": "read",
                "arguments": {
                    "path": "photo.png",
                    "root": project.display().to_string()
                }
            }
        }),
        &tmp,
    )
    .expect("read is a request");
    assert!(
        resp.get("error").is_none() || resp["error"].is_null(),
        "read should succeed, got {resp}"
    );
    let content = resp["result"]["content"]
        .as_array()
        .expect("MCP result.content");
    let image = content
        .iter()
        .find(|c| c["type"] == "image")
        .unwrap_or_else(|| panic!("expected image content, got {content:?}"));
    assert_eq!(image["mimeType"], "image/png");
    let data = image["data"].as_str().expect("image.data base64");
    assert_eq!(decode_base64(data), TINY_PNG);
}

#[test]
fn oversized_image_read_returns_path_and_mime_not_base64() {
    let tmp = tempfile_dir();
    assert_isolated(&tmp);
    let png = tmp.join("big.png");
    let mut bytes = TINY_PNG.to_vec();
    bytes.resize(4 * 1024 * 1024 + 1, 0);
    fs::write(&png, &bytes).unwrap();

    let content = vane::mcp::encode_image_read(&png).unwrap();
    assert!(
        content.iter().all(|c| c["type"] != "image"),
        "files > 4 MiB must not be inlined as image content: {content:?}"
    );
    let text = content
        .iter()
        .find(|c| c["type"] == "text")
        .unwrap_or_else(|| panic!("expected text metadata, got {content:?}"));
    let body = text["text"].as_str().unwrap();
    assert!(
        body.contains("image/png") || body.to_ascii_lowercase().contains("mime"),
        "oversize read must mention mime, got {body}"
    );
    let path_s = png.display().to_string();
    assert!(
        body.contains(&path_s) || body.contains("big.png"),
        "oversize read must mention path, got {body}"
    );
    assert!(
        body.len() < 64 * 1024,
        "must not dump the oversized file as base64 (text was {} bytes)",
        body.len()
    );
}

fn vane_server(v: &Value) -> &Value {
    &v["mcpServers"]["vane"]
}

fn run_install_cli(vane_home: &Path, user_home: &Path, args: &[&str]) -> std::process::Output {
    let bin = env!("CARGO_BIN_EXE_vane");
    Command::new(bin)
        .arg("--home")
        .arg(vane_home)
        .args(args)
        .env("VANE_HOME", vane_home)
        .env("HOME", user_home)
        .env("GROK_HOME", user_home.join(".grok"))
        .env("CODEX_HOME", user_home.join(".codex"))
        .output()
        .expect("run vane mcp install")
}

fn stdout_json(output: &std::process::Output) -> Value {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "mcp install failed status={:?} stdout={stdout} stderr={stderr}",
        output.status.code()
    );
    serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
        panic!("piped mcp install must print JSON: {e}; stdout={stdout:?} stderr={stderr:?}")
    })
}

#[test]
fn mcp_install_dry_run_does_not_write_and_lists_claude_cursor() {
    let tmp = tempfile_dir();
    assert_isolated(&tmp);
    let fake = fake_user_home(&tmp);
    let report = vane::mcp::install_mcp(&fake, true, None).expect("dry-run");
    assert!(report.ok);
    assert!(report.dry_run);
    assert!(report.written.is_empty());
    let paths: Vec<&str> = report.would_write.iter().map(|t| t.path.as_str()).collect();
    assert!(
        paths.iter().any(|p| p.ends_with(".claude.json")),
        "would_write should include ~/.claude.json, got {paths:?}"
    );
    assert!(
        paths
            .iter()
            .any(|p| p.ends_with(".cursor/mcp.json") || p.ends_with("mcp.json")),
        "would_write should include ~/.cursor/mcp.json, got {paths:?}"
    );
    assert!(
        !paths.iter().any(|p| p.contains(".codex")),
        "must not invent a Codex file, got {paths:?}"
    );
    assert!(!fake.join(".claude.json").exists());
    assert!(!fake.join(".cursor").exists());
    assert!(!fake.join(".codex").exists());
    assert!(!fake
        .join("Library")
        .join("Application Support")
        .join("Claude")
        .exists());
}

#[test]
fn mcp_install_creates_claude_and_cursor_and_preserves_other_servers() {
    let tmp = tempfile_dir();
    assert_isolated(&tmp);
    let fake = fake_user_home(&tmp);
    let claude = fake.join(".claude.json");
    fs::write(
        &claude,
        r#"{
  "theme": "dark",
  "mcpServers": {
    "github": { "command": "npx", "args": ["-y", "github"] }
  }
}
"#,
    )
    .unwrap();

    let report = vane::mcp::install_mcp(&fake, false, None).expect("install");
    assert!(report.ok);
    assert!(!report.dry_run);
    assert!(report.would_write.is_empty());
    assert!(report.written.iter().any(|t| t.client == "claude"));
    assert!(report.written.iter().any(|t| t.client == "cursor"));

    let claude_v: Value = serde_json::from_str(&fs::read_to_string(&claude).unwrap()).unwrap();
    assert_eq!(claude_v["theme"], "dark");
    assert_eq!(claude_v["mcpServers"]["github"]["command"], "npx");
    assert_eq!(vane_server(&claude_v)["command"], "vane");
    assert_eq!(vane_server(&claude_v)["args"], json!(["mcp"]));

    let cursor = fake.join(".cursor").join("mcp.json");
    assert!(cursor.is_file(), "should create ~/.cursor/mcp.json");
    let cursor_v: Value = serde_json::from_str(&fs::read_to_string(&cursor).unwrap()).unwrap();
    assert_eq!(vane_server(&cursor_v)["command"], "vane");
    assert_eq!(vane_server(&cursor_v)["args"], json!(["mcp"]));
    assert_eq!(cursor_v["mcpServers"].as_object().unwrap().len(), 1);

    assert!(!fake.join(".codex").exists());
    assert!(!fake
        .join("Library")
        .join("Application Support")
        .join("Claude")
        .exists());
    let raw = fs::read_to_string(&claude).unwrap();
    assert!(!raw.to_ascii_lowercase().contains("api_key"));
}

#[test]
fn mcp_install_keeps_extra_vane_keys_and_updates_command() {
    let tmp = tempfile_dir();
    assert_isolated(&tmp);
    let fake = fake_user_home(&tmp);
    fs::write(
        fake.join(".claude.json"),
        r#"{
  "mcpServers": {
    "vane": {
      "command": "old-vane",
      "args": ["nope"],
      "env": { "FOO": "bar" }
    }
  }
}
"#,
    )
    .unwrap();
    vane::mcp::install_mcp(&fake, false, Some(vane::mcp::McpClient::Claude)).unwrap();
    let v: Value =
        serde_json::from_str(&fs::read_to_string(fake.join(".claude.json")).unwrap()).unwrap();
    assert_eq!(vane_server(&v)["command"], "vane");
    assert_eq!(vane_server(&v)["args"], json!(["mcp"]));
    assert_eq!(vane_server(&v)["env"]["FOO"], "bar");
}

#[test]
fn mcp_install_client_flag_limits_targets() {
    let tmp = tempfile_dir();
    assert_isolated(&tmp);
    let fake = fake_user_home(&tmp);
    vane::mcp::install_mcp(&fake, false, Some(vane::mcp::McpClient::Claude)).unwrap();
    assert!(fake.join(".claude.json").is_file());
    assert!(!fake.join(".cursor").exists());
}

#[test]
fn mcp_install_codex_only_if_present() {
    let tmp = tempfile_dir();
    assert_isolated(&tmp);
    let fake = fake_user_home(&tmp);
    vane::mcp::install_mcp(&fake, false, Some(vane::mcp::McpClient::Codex)).unwrap();
    assert!(!fake.join(".codex").exists(), "must not create ~/.codex");

    let json_path = fake.join(".codex").join("mcp.json");
    fs::create_dir_all(json_path.parent().unwrap()).unwrap();
    fs::write(
        &json_path,
        r#"{"mcpServers":{"other":{"command":"echo","args":["hi"]}}}"#,
    )
    .unwrap();
    let toml_path = fake.join(".codex").join("config.toml");
    fs::write(
        &toml_path,
        "[mcp_servers.other]\ncommand = \"echo\"\nargs = [\"hi\"]\n",
    )
    .unwrap();

    let report =
        vane::mcp::install_mcp(&fake, false, Some(vane::mcp::McpClient::Codex)).expect("codex");
    assert!(report.written.iter().any(|t| t.path.ends_with("mcp.json")));
    assert!(report
        .written
        .iter()
        .any(|t| t.path.ends_with("config.toml")));

    let json: Value = serde_json::from_str(&fs::read_to_string(&json_path).unwrap()).unwrap();
    assert_eq!(json["mcpServers"]["other"]["command"], "echo");
    assert_eq!(json["mcpServers"]["vane"]["command"], "vane");
    assert_eq!(json["mcpServers"]["vane"]["args"], json!(["mcp"]));

    let toml_body = fs::read_to_string(&toml_path).unwrap();
    assert!(
        toml_body.contains("echo"),
        "must keep the other Codex server, got {toml_body}"
    );
    assert!(
        toml_body.contains("[mcp_servers.vane]"),
        "must merge [mcp_servers.vane], got {toml_body}"
    );
    assert!(
        toml_body.contains("command = \"vane\""),
        "vane command, got {toml_body}"
    );
    assert!(
        toml_body.contains("\"mcp\""),
        "vane args [mcp], got {toml_body}"
    );
}

#[test]
fn mcp_install_grok_toml_if_present_on_all() {
    let tmp = tempfile_dir();
    assert_isolated(&tmp);
    let fake = fake_user_home(&tmp);
    let grok = fake.join(".grok").join("config.toml");
    fs::create_dir_all(grok.parent().unwrap()).unwrap();
    fs::write(
        &grok,
        "[mcp_servers.fs]\ncommand = \"npx\"\nargs = [\"-y\", \"fs\"]\n",
    )
    .unwrap();
    vane::mcp::install_mcp(&fake, false, None).unwrap();
    let grok_body = fs::read_to_string(&grok).unwrap();
    assert!(
        grok_body.contains("npx"),
        "must keep the other Grok server, got {grok_body}"
    );
    assert!(
        grok_body.contains("[mcp_servers.vane]"),
        "must merge [mcp_servers.vane], got {grok_body}"
    );
    assert!(
        grok_body.contains("command = \"vane\""),
        "vane command, got {grok_body}"
    );
}

#[test]
fn mcp_install_invalid_json_does_not_clobber() {
    let tmp = tempfile_dir();
    assert_isolated(&tmp);
    let fake = fake_user_home(&tmp);
    let claude = fake.join(".claude.json");
    fs::write(&claude, "{not-json").unwrap();
    let err = vane::mcp::install_mcp(&fake, false, Some(vane::mcp::McpClient::Claude)).unwrap_err();
    assert!(
        err.message.contains("parse") || err.message.contains("JSON"),
        "invalid JSON must fail closed, got {}",
        err.message
    );
    assert_eq!(fs::read_to_string(&claude).unwrap(), "{not-json");
}

#[test]
fn mcp_install_cli_dry_run_json_and_client_limit() {
    let tmp = tempfile_dir();
    assert_isolated(&tmp);
    let fake = fake_user_home(&tmp);

    let dry = run_install_cli(&tmp, &fake, &["mcp", "install", "--dry-run"]);
    let v = stdout_json(&dry);
    assert_eq!(v["ok"], true);
    assert_eq!(v["dry_run"], true);
    let would = v["would_write"].as_array().expect("would_write");
    assert!(
        would
            .iter()
            .any(|t| t["path"].as_str().unwrap().ends_with(".claude.json")),
        "would_write={would:?}"
    );
    assert!(!fake.join(".claude.json").exists());
    assert!(!fake.join(".cursor").exists());

    let only = run_install_cli(&tmp, &fake, &["mcp", "install", "--client", "claude"]);
    let wrote = stdout_json(&only);
    assert_eq!(wrote["dry_run"], false);
    assert!(fake.join(".claude.json").is_file());
    assert!(!fake.join(".cursor").exists());
    let body: Value =
        serde_json::from_str(&fs::read_to_string(fake.join(".claude.json")).unwrap()).unwrap();
    assert_eq!(vane_server(&body)["command"], "vane");
    assert_eq!(vane_server(&body)["args"], json!(["mcp"]));
}

#[test]
fn mcp_install_cli_does_not_need_init() {
    let tmp = tempfile_dir();
    assert_isolated(&tmp);
    let fake = fake_user_home(&tmp);
    assert!(!tmp.join("config").join("config.toml").exists());
    let out = run_install_cli(&tmp, &fake, &["mcp", "install", "--dry-run"]);
    let v = stdout_json(&out);
    assert_eq!(v["ok"], true);
}
