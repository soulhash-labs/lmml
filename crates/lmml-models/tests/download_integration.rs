use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;

use lmml_models::{DownloadError, ModelRegistry};

#[tokio::test]
async fn full_download_succeeds_from_local_http_server() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let fixture = fixture_gguf();
    let (url, handle) = serve(FixtureServer::Full(fixture.clone()), 1);
    let registry = ModelRegistry {
        models_dir: tempdir.path().join("models"),
        aliases: Vec::new(),
    };

    let model = registry
        .download(&url, |_progress| {})
        .await
        .expect("download model");

    handle.join().expect("server thread");
    assert_eq!(model.name, "Mistral Test");
    assert_eq!(
        std::fs::read(model.path).expect("downloaded bytes"),
        fixture
    );
}

#[tokio::test]
async fn interrupted_download_resumes_with_range_request() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let fixture = fixture_gguf();
    let split_at = fixture.len() / 2;
    let (url, handle) = serve(FixtureServer::Resume(fixture.clone(), split_at), 2);
    let registry = ModelRegistry {
        models_dir: tempdir.path().join("models"),
        aliases: Vec::new(),
    };

    let first = registry.download(&url, |_progress| {}).await;
    assert!(first.is_err());
    let part = registry.models_dir.join("model-Q4_K_M.gguf.part");
    assert_eq!(
        std::fs::metadata(&part).expect("partial metadata").len(),
        split_at as u64
    );

    let model = registry
        .download(&url, |_progress| {})
        .await
        .expect("resumed download");

    handle.join().expect("server thread");
    assert_eq!(
        std::fs::read(model.path).expect("downloaded bytes"),
        fixture
    );
}

#[tokio::test]
async fn download_404_returns_clear_error() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let (url, handle) = serve(FixtureServer::NotFound, 1);
    let registry = ModelRegistry {
        models_dir: tempdir.path().join("models"),
        aliases: Vec::new(),
    };

    let error = registry
        .download(&url, |_progress| {})
        .await
        .expect_err("404 should fail");

    handle.join().expect("server thread");
    assert!(matches!(error, DownloadError::Status(404)));
    assert_eq!(error.to_string(), "download returned HTTP status 404");
}

enum FixtureServer {
    Full(Vec<u8>),
    Resume(Vec<u8>, usize),
    NotFound,
}

fn serve(server: FixtureServer, requests: usize) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind local server");
    let port = listener.local_addr().expect("local addr").port();
    let server = Arc::new(server);
    let handle = thread::spawn(move || {
        for index in 0..requests {
            let (mut stream, _addr) = listener.accept().expect("accept request");
            let request = read_request(&mut stream);
            match server.as_ref() {
                FixtureServer::Full(body) => write_response(&mut stream, 200, body, None),
                FixtureServer::Resume(body, split_at) if index == 0 => {
                    write_partial_close(&mut stream, body, *split_at);
                }
                FixtureServer::Resume(body, split_at) => {
                    assert!(request
                        .to_ascii_lowercase()
                        .contains(&format!("range: bytes={split_at}-")));
                    write_response(&mut stream, 206, &body[*split_at..], Some(body.len()));
                }
                FixtureServer::NotFound => {
                    write_raw(
                        &mut stream,
                        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n",
                    );
                }
            }
        }
    });
    (format!("http://127.0.0.1:{port}/model-Q4_K_M.gguf"), handle)
}

fn read_request(stream: &mut TcpStream) -> String {
    let mut buffer = [0_u8; 4096];
    let size = stream.read(&mut buffer).expect("read request");
    String::from_utf8_lossy(&buffer[..size]).into_owned()
}

fn write_response(stream: &mut TcpStream, status: u16, body: &[u8], full_len: Option<usize>) {
    let status_text = match status {
        200 => "OK",
        206 => "Partial Content",
        _ => "OK",
    };
    let mut header = format!(
        "HTTP/1.1 {status} {status_text}\r\nContent-Length: {}\r\n",
        body.len()
    );
    if let Some(total) = full_len {
        let start = total - body.len();
        header.push_str(&format!(
            "Content-Range: bytes {start}-{}/{}\r\n",
            total - 1,
            total
        ));
    }
    header.push_str("\r\n");
    write_raw(stream, &header);
    stream.write_all(body).expect("write body");
    stream.flush().expect("flush body");
}

fn write_partial_close(stream: &mut TcpStream, body: &[u8], split_at: usize) {
    write_raw(
        stream,
        &format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()),
    );
    stream
        .write_all(&body[..split_at])
        .expect("write partial body");
    stream.flush().expect("flush partial body");
}

fn write_raw(stream: &mut TcpStream, text: &str) {
    stream.write_all(text.as_bytes()).expect("write response");
}

fn fixture_gguf() -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"GGUF");
    bytes.extend_from_slice(&3_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u64.to_le_bytes());
    bytes.extend_from_slice(&5_u64.to_le_bytes());
    write_kv_string(&mut bytes, "general.name", "Mistral Test");
    write_kv_string(&mut bytes, "general.architecture", "llama");
    write_kv_u32(&mut bytes, "llama.context_length", 4096);
    write_kv_u32(&mut bytes, "llama.embedding_length", 4096);
    write_kv_u32(&mut bytes, "llama.block_count", 32);
    write_string(&mut bytes, "blk.0.attn_q.weight");
    bytes.extend_from_slice(&2_u32.to_le_bytes());
    bytes.extend_from_slice(&4096_u64.to_le_bytes());
    bytes.extend_from_slice(&4096_u64.to_le_bytes());
    bytes.extend_from_slice(&12_u32.to_le_bytes());
    bytes.extend_from_slice(&0_u64.to_le_bytes());
    bytes
}

fn write_kv_string(bytes: &mut Vec<u8>, key: &str, value: &str) {
    write_string(bytes, key);
    bytes.extend_from_slice(&8_u32.to_le_bytes());
    write_string(bytes, value);
}

fn write_kv_u32(bytes: &mut Vec<u8>, key: &str, value: u32) {
    write_string(bytes, key);
    bytes.extend_from_slice(&4_u32.to_le_bytes());
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn write_string(bytes: &mut Vec<u8>, value: &str) {
    bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
    bytes.extend_from_slice(value.as_bytes());
}
