use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use vane::config::EmbedConfig;
use vane::dirty::DirtyQueue;
use vane::embed::{embed_model_id, ollama_embedder, openai_embedder, Embedder, MockEmbedder};

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
    let path = std::env::temp_dir().join(format!(
        "vane-embed-test-{}-{}-{}",
        std::process::id(),
        nanos,
        n
    ));
    std::fs::create_dir_all(&path).unwrap();
    // Isolation: never touch the user's real ~/.vane.
    std::env::set_var("VANE_HOME", &path);
    TempHome { path }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let tmp = std::env::temp_dir();
        if self.path.starts_with(&tmp) && self.path != tmp {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

impl std::ops::Deref for TempHome {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.path
    }
}

struct FakeServer {
    base_url: String,
    hits: Arc<AtomicUsize>,
}

impl FakeServer {
    fn spawn(kind: FakeKind) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake embed server");
        let addr = listener.local_addr().expect("local_addr");
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_thread = Arc::clone(&hits);
        thread::spawn(move || {
            listener.set_nonblocking(false).ok();
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                handle_client(stream, kind, &hits_thread);
            }
        });
        Self {
            base_url: format!("http://{addr}"),
            hits,
        }
    }

    fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }
}

#[derive(Clone, Copy)]
enum FakeKind {
    Ollama,
    OpenAi,
}

fn handle_client(mut stream: TcpStream, kind: FakeKind, hits: &AtomicUsize) {
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(2))).ok();
    let mut buf = Vec::new();
    let mut tmp = [0u8; 2048];
    let headers_end;
    loop {
        let n = match stream.read(&mut tmp) {
            Ok(0) => return,
            Ok(n) => n,
            Err(_) => return,
        };
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = find_double_crlf(&buf) {
            headers_end = pos;
            break;
        }
        if buf.len() > 64 * 1024 {
            return;
        }
    }
    let header_text = String::from_utf8_lossy(&buf[..headers_end]);
    let content_len = header_text
        .lines()
        .find_map(|line| {
            let (k, v) = line.split_once(':')?;
            if k.eq_ignore_ascii_case("content-length") {
                v.trim().parse::<usize>().ok()
            } else {
                None
            }
        })
        .unwrap_or(0);
    while buf.len().saturating_sub(headers_end) < content_len {
        let n = match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };
        buf.extend_from_slice(&tmp[..n]);
    }
    let body = &buf[headers_end..buf.len().min(headers_end + content_len)];
    hits.fetch_add(1, Ordering::SeqCst);
    let payload = match kind {
        FakeKind::Ollama => r#"{"embedding":[0.1,0.2]}"#.to_string(),
        FakeKind::OpenAi => openai_payload(body),
    };
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
        payload.len()
    );
    let _ = stream.write_all(resp.as_bytes());
    let _ = stream.flush();
}

fn openai_payload(body: &[u8]) -> String {
    // Response shape from the plan; repeat one vector per input so a 65-text
    // batch can succeed while still counting HTTP calls.
    let n = parse_openai_input_len(body).unwrap_or(1);
    let items = vec![r#"{"embedding":[1.0,0.0,0.0]}"#; n].join(",");
    format!(r#"{{"data":[{items}]}}"#)
}

fn parse_openai_input_len(body: &[u8]) -> Option<usize> {
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    v.get("input")?.as_array().map(|a| a.len())
}

fn find_double_crlf(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
}

#[test]
fn mock_fail_makes_embed_error() {
    let _tmp = tempfile_dir();
    let emb = MockEmbedder {
        dim: 4,
        fail: true,
        calls: Arc::new(Mutex::new(Vec::new())),
    };
    let err = emb
        .embed(&[String::from("hello")])
        .expect_err("fail=true must error");
    assert!(!err.message.is_empty());
}

#[test]
fn ollama_probe_dim_from_fake_http() {
    let _tmp = tempfile_dir();
    let server = FakeServer::spawn(FakeKind::Ollama);
    let cfg = EmbedConfig {
        provider: "ollama".into(),
        model: "nomic-embed-text".into(),
        base_url: server.base_url.clone(),
        api_key: None,
    };
    let emb = ollama_embedder(&cfg);
    assert_eq!(emb.probe_dim().unwrap(), 2);
    let vecs = emb.embed(&[String::from("hi")]).unwrap();
    assert_eq!(vecs, vec![vec![0.1, 0.2]]);
}

#[test]
fn openai_compat_batches_65_texts_into_two_http_calls() {
    let _tmp = tempfile_dir();
    let server = FakeServer::spawn(FakeKind::OpenAi);
    let cfg = EmbedConfig {
        provider: "openai_compat".into(),
        model: "text-embedding-3-small".into(),
        base_url: server.base_url.clone(),
        api_key: None,
    };
    let emb = openai_embedder(&cfg);
    assert_eq!(emb.probe_dim().unwrap(), 3);
    let probe_hits = server.hits();
    let texts: Vec<String> = (0..65).map(|i| format!("t{i}")).collect();
    let vecs = emb.embed(&texts).unwrap();
    assert_eq!(vecs.len(), 65);
    assert_eq!(vecs[0], vec![1.0, 0.0, 0.0]);
    assert_eq!(server.hits() - probe_hits, 2);
}

#[test]
fn dirty_queue_first_retry_then_doubles_until_60s() {
    let _tmp = tempfile_dir();
    let mut q = DirtyQueue::new();
    q.push("proj", "docs/a.md");
    assert!(q.pop_due(0).is_empty(), "first retry is at +1s");
    let due = q.pop_due(1);
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].project_id, "proj");
    assert_eq!(due[0].path, "docs/a.md");

    // delays after the first 1s retry: 2, 4, 8, 16, 32, 60, 60
    let mut t = 1u64;
    let mut delay = 2u64;
    for _ in 0..7 {
        assert!(
            q.pop_due(t + delay - 1).is_empty(),
            "not due before +{delay}s (t={t})"
        );
        t += delay;
        let due = q.pop_due(t);
        assert_eq!(due.len(), 1, "due at t={t} delay={delay}");
        delay = delay.saturating_mul(2).min(60);
    }
}

#[test]
fn embed_model_id_is_provider_model_dim() {
    assert_eq!(
        embed_model_id("ollama", "nomic-embed-text", 768),
        "ollama:nomic-embed-text:768"
    );
}
