#![cfg(unix)]

use std::fs;
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Stdio};
use std::time::Duration;

use pretty_assertions::assert_eq;
use reqwest::StatusCode;
use serde_json::{json, Value};
use tokio::process::{Child, Command};

const API_KEY: &str = "integration-secret";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

#[tokio::test]
async fn lmml_node_and_router_proxy_infer_to_stub_llama_server() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let model_dir = tempdir.path().join("models");
    fs::create_dir_all(&model_dir).expect("model dir");
    let mut llama_server = start_stub_llama_server(tempdir.path()).await;
    let llama_url = format!("http://127.0.0.1:{}", llama_server.port);

    let node_port = free_port();
    let router_port = free_port();
    let node_url = format!("http://127.0.0.1:{node_port}");
    let router_url = format!("http://127.0.0.1:{router_port}");

    let mut node = spawn_lmml_node(node_port, &llama_url, &model_dir);
    wait_for_json(&format!("{node_url}/v1/health"))
        .await
        .expect("lmml-node health");

    let mut router = spawn_lmml_router(router_port, &node_url);
    wait_for_json(&format!("{router_url}/v1/health"))
        .await
        .expect("lmml-router health");

    let client = reqwest::Client::new();
    let response = client
        .post(format!("{router_url}/v1/infer"))
        .bearer_auth(API_KEY)
        .header("x-lmml-request-id", "chain-req-1")
        .json(&json!({
            "request_id": "chain-req-1",
            "task_type": "general",
            "model": "stub-model",
            "prompt": "route this",
            "metadata": {}
        }))
        .send()
        .await
        .expect("router infer response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("x-lmml-request-id")
            .and_then(|value| value.to_str().ok()),
        Some("chain-req-1")
    );
    let body = response.json::<Value>().await.expect("infer json");

    assert_eq!(body["request_id"], "chain-req-1");
    assert_eq!(body["output"], "stubbed route this");
    assert_eq!(body["finish_reason"], "stop");

    let capabilities = client
        .get(format!("{router_url}/v1/capabilities"))
        .bearer_auth(API_KEY)
        .send()
        .await
        .expect("router capabilities")
        .json::<Value>()
        .await
        .expect("capabilities json");
    assert_eq!(capabilities["roles"][0], "router");
    assert_eq!(capabilities["extra"]["ready_upstream_count"], 1);

    router.stop().await;
    node.stop().await;
    llama_server.child.stop().await;
}

struct StubServer {
    port: u16,
    child: ChildGuard,
}

struct ChildGuard {
    child: Child,
}

impl ChildGuard {
    fn new(child: Child) -> Self {
        Self { child }
    }

    async fn stop(&mut self) {
        if let Ok(Some(_status)) = self.child.try_wait() {
            return;
        }
        let _ignored = self.child.start_kill();
        let _ignored = tokio::time::timeout(Duration::from_secs(2), self.child.wait()).await;
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Ok(Some(_status)) = self.child.try_wait() {
            return;
        }
        let _ignored = self.child.start_kill();
    }
}

async fn start_stub_llama_server(dir: &Path) -> StubServer {
    let port = free_port();
    let script = write_stub_llama_server(dir);
    let child = Command::new(script)
        .arg(port.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn stub llama-server");
    let url = format!("http://127.0.0.1:{port}/v1/health");
    wait_for_text(&url).await.expect("stub health");
    StubServer {
        port,
        child: ChildGuard::new(child),
    }
}

fn write_stub_llama_server(dir: &Path) -> PathBuf {
    let path = dir.join("stub-llama-server.py");
    fs::write(
        &path,
        r#"#!/usr/bin/env python3
import json
import http.server
import sys

port = int(sys.argv[1])

class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path in ("/health", "/v1/health"):
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(b'{"status":"ok"}')
            return
        self.send_response(404)
        self.end_headers()

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        body = json.loads(self.rfile.read(length).decode("utf-8") or "{}")
        if self.path == "/v1/chat/completions":
            user_content = ""
            for message in body.get("messages", []):
                if message.get("role") == "user":
                    user_content = message.get("content", "")
            payload = {
                "id": "chatcmpl-stub",
                "model": body.get("model", "stub-model"),
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": f"stubbed {user_content}",
                    },
                    "finish_reason": "stop",
                }],
                "usage": {
                    "prompt_tokens": 3,
                    "completion_tokens": 2,
                },
            }
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(json.dumps(payload).encode("utf-8"))
            return
        self.send_response(404)
        self.end_headers()

    def log_message(self, format, *args):
        return

http.server.ThreadingHTTPServer(("127.0.0.1", port), Handler).serve_forever()
"#,
    )
    .expect("write stub llama-server");
    let mut perms = fs::metadata(&path).expect("metadata").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).expect("chmod stub");
    path
}

fn spawn_lmml_node(port: u16, llama_url: &str, model_dir: &Path) -> ChildGuard {
    let child = Command::new(lmml_node_binary())
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .arg("--node-id")
        .arg("integration-node")
        .arg("--node-name")
        .arg("Integration Node")
        .arg("--llama-url")
        .arg(llama_url)
        .arg("--model-dir")
        .arg(model_dir)
        .arg("--api-key")
        .arg(API_KEY)
        .arg("--skip-probe")
        .env("RUST_LOG", "warn")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn lmml-node");
    ChildGuard::new(child)
}

fn spawn_lmml_router(port: u16, node_url: &str) -> ChildGuard {
    let child = Command::new(lmml_router_binary())
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .arg("--router-id")
        .arg("integration-router")
        .arg("--router-name")
        .arg("Integration Router")
        .arg("--api-key")
        .arg(API_KEY)
        .arg("--upstream")
        .arg(format!("node={node_url}"))
        .arg("--upstream-key")
        .arg(format!("node={API_KEY}"))
        .arg("--discovery-timeout-ms")
        .arg("500")
        .env("RUST_LOG", "warn")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn lmml-router");
    ChildGuard::new(child)
}

async fn wait_for_json(url: &str) -> Result<Value, reqwest::Error> {
    let client = reqwest::Client::new();
    let deadline = tokio::time::Instant::now() + REQUEST_TIMEOUT;
    loop {
        let result = client
            .get(url)
            .timeout(Duration::from_millis(250))
            .send()
            .await;
        if let Ok(response) = result {
            if response.status().is_success() {
                return response.json::<Value>().await;
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return client.get(url).send().await?.json::<Value>().await;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn wait_for_text(url: &str) -> Result<String, reqwest::Error> {
    let client = reqwest::Client::new();
    let deadline = tokio::time::Instant::now() + REQUEST_TIMEOUT;
    loop {
        let result = client
            .get(url)
            .timeout(Duration::from_millis(250))
            .send()
            .await;
        if let Ok(response) = result {
            if response.status().is_success() {
                return response.text().await;
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return client.get(url).send().await?.text().await;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn lmml_node_binary() -> PathBuf {
    std::env::var_os("CARGO_BIN_EXE_lmml-node")
        .map(PathBuf::from)
        .unwrap_or_else(|| fallback_binary("lmml-node"))
}

fn lmml_router_binary() -> PathBuf {
    std::env::var_os("CARGO_BIN_EXE_lmml-router")
        .map(PathBuf::from)
        .unwrap_or_else(|| fallback_binary("lmml-router"))
}

fn fallback_binary(name: &str) -> PathBuf {
    let mut path = std::env::current_exe().expect("current exe");
    while path.file_name().and_then(|name| name.to_str()) != Some("target") {
        assert!(path.pop(), "current test executable is not under target/");
    }
    path.push("debug");
    path.push(name);
    let package = match name {
        "lmml-node" => "lmml-node",
        "lmml-router" => "lmml-router",
        _ => name,
    };
    let status = StdCommand::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .arg("build")
        .arg("-p")
        .arg(package)
        .arg("--bin")
        .arg(name)
        .status()
        .expect("build integration binary");
    assert!(status.success(), "failed to build {name}");
    path
}

fn free_port() -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind free port");
    listener.local_addr().expect("local addr").port()
}
