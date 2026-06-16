#![cfg(unix)]

use std::fs;
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use lmml_compat::{LlamaBinaryCapabilities, ServerConfig};
use lmml_models::ModelEntry;
use lmml_server::{ServerManager, ServerStatus};
use tokio::sync::mpsc;

#[tokio::test]
async fn stub_ready_reports_ready_within_two_seconds() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let binary = write_stub_server(tempdir.path());
    let port = free_port();
    let manager = ServerManager {
        binary,
        caps: caps(),
    };
    let (log_tx, _log_rx) = mpsc::channel(32);

    let handle = tokio::time::timeout(
        Duration::from_secs(2),
        manager.start_with_timeout(
            &model(tempdir.path()),
            &config(port),
            log_tx,
            Duration::from_secs(2),
        ),
    )
    .await
    .expect("start should finish within two seconds")
    .expect("server should start");

    assert_eq!(
        handle.status(),
        ServerStatus::Ready {
            url: format!("http://127.0.0.1:{port}")
        }
    );
    handle.stop().await;
}

#[tokio::test]
async fn start_creates_missing_slot_save_path() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let binary = write_stub_server(tempdir.path());
    let port = free_port();
    let manager = ServerManager {
        binary,
        caps: caps(),
    };
    let (log_tx, _log_rx) = mpsc::channel(32);
    let slot_save_path = tempdir.path().join("runtime").join("llama-slots");
    let mut config = config(port);
    config.extra_args = vec![
        "--slot-save-path".to_string(),
        slot_save_path.display().to_string(),
    ];

    let handle = tokio::time::timeout(
        Duration::from_secs(2),
        manager.start_with_timeout(
            &model(tempdir.path()),
            &config,
            log_tx,
            Duration::from_secs(2),
        ),
    )
    .await
    .expect("start should finish within two seconds")
    .expect("server should start");

    assert!(slot_save_path.is_dir());
    handle.stop().await;
}

#[tokio::test]
async fn stub_that_never_listens_fails_with_timeout() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let binary = write_stub_server(tempdir.path());
    let port = free_port();
    let manager = ServerManager {
        binary,
        caps: caps(),
    };
    let (log_tx, _log_rx) = mpsc::channel(32);
    let mut config = config(port);
    config.extra_args = vec!["--stub-mode".to_string(), "never".to_string()];

    let error = manager
        .start_with_timeout(
            &model(tempdir.path()),
            &config,
            log_tx,
            Duration::from_millis(800),
        )
        .await
        .expect_err("server should time out");

    assert!(error.to_string().contains("server did not become ready"));
}

#[tokio::test]
async fn occupied_port_fails_before_spawn() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let binary = write_stub_server(tempdir.path());
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind test port");
    let port = listener.local_addr().expect("local addr").port();
    let manager = ServerManager {
        binary,
        caps: caps(),
    };
    let (log_tx, _log_rx) = mpsc::channel(32);

    let error = manager
        .start_with_timeout(
            &model(tempdir.path()),
            &config(port),
            log_tx,
            Duration::from_millis(800),
        )
        .await
        .expect_err("port conflict should fail");

    let message = error.to_string();
    assert!(message.contains(&format!("port {port}")));
    assert!(message.contains("already in use"));
}

fn write_stub_server(dir: &Path) -> PathBuf {
    let path = dir.join("llama-server-stub");
    fs::write(
        &path,
        r#"#!/bin/sh
MODE="ready"
HOST="127.0.0.1"
PORT="8080"
while [ "$#" -gt 0 ]; do
  case "$1" in
    --host) shift; HOST="$1" ;;
    --port) shift; PORT="$1" ;;
    --stub-mode) shift; MODE="$1" ;;
    --model|-m|--ctx-size|--context-size|-c|-ngl|--n-gpu-layers|--batch-size|-b|--threads|-t|--ubatch-size|--micro-batch-size|-ub) shift ;;
  esac
  shift
done
if [ "$MODE" = "never" ]; then
  sleep 60
  exit 0
fi
sleep 0.5
exec python3 -u - "$HOST" "$PORT" <<'PY'
import http.server
import sys

host = sys.argv[1]
port = int(sys.argv[2])

class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path in ("/health", "/v1/health"):
            self.send_response(200)
            self.end_headers()
            self.wfile.write(b"ok")
        else:
            self.send_response(404)
            self.end_headers()
    def log_message(self, format, *args):
        return

http.server.ThreadingHTTPServer((host, port), Handler).serve_forever()
PY
"#,
    )
    .expect("write stub server");
    let mut perms = fs::metadata(&path).expect("stub metadata").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).expect("chmod stub");
    path
}

fn free_port() -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind free port");
    listener.local_addr().expect("local addr").port()
}

fn config(port: u16) -> ServerConfig {
    ServerConfig {
        port,
        host: "127.0.0.1".to_string(),
        model: PathBuf::new(),
        ctx_size: 128,
        n_gpu_layers: 0,
        batch_size: 16,
        ubatch_size: 16,
        threads: 1,
        flash_attn: false,
        mlock: false,
        api_key: None,
        chat_template: None,
        jinja: false,
        extra_args: Vec::new(),
    }
}

fn model(dir: &Path) -> ModelEntry {
    ModelEntry {
        path: dir.join("model.gguf"),
        name: "model".to_string(),
        size_bytes: 0,
        quant: "unknown".to_string(),
        context_length: None,
        architecture: None,
        aliased: false,
    }
}

fn caps() -> LlamaBinaryCapabilities {
    LlamaBinaryCapabilities {
        version: Some("stub".to_string()),
        flash_attn: false,
        flash_attn_requires_value: false,
        mlock: false,
        api_key: false,
        ubatch_size: true,
        chat_template: false,
        jinja: false,
        reranking: false,
        flags: vec![
            "--model".to_string(),
            "--host".to_string(),
            "--port".to_string(),
            "--ctx-size".to_string(),
            "-ngl".to_string(),
            "--batch-size".to_string(),
            "--ubatch-size".to_string(),
            "--threads".to_string(),
        ],
    }
}
