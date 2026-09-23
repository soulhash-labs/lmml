//! GGUF metadata and llama-server admission checks.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::time::{sleep, timeout};

use super::{default_server_path, drain_process_lines};

pub(super) trait GgufAdmissionProvider {
    fn admit<'a>(
        &'a self,
        artifact: &'a Path,
        server: &'a Path,
    ) -> impl Future<Output = Result<(), String>> + Send + 'a;
}

pub(super) struct LlamaServerAdmission;

impl GgufAdmissionProvider for LlamaServerAdmission {
    async fn admit(&self, artifact: &Path, server: &Path) -> Result<(), String> {
        validate_derived_gguf(artifact, Some(server)).await
    }
}

async fn validate_derived_gguf(artifact: &Path, server: Option<&Path>) -> Result<(), String> {
    validate_gguf_structure(artifact).await?;
    let server = server
        .map(PathBuf::from)
        .unwrap_or_else(default_server_path);
    if !server.is_file() {
        return Err(format!("llama-server not found: {}", server.display()));
    }
    admit_with_server(artifact, &server).await
}

pub(super) async fn validate_gguf_structure(artifact: &Path) -> Result<(), String> {
    lmml_substrate::validate_gguf(artifact).map_err(|error| error.to_string())?;
    let metadata = lmml_models::parse_gguf_metadata(artifact)
        .await
        .map_err(|error| format!("GGUF metadata inspection failed: {error}"))?;
    if metadata.tensor_count == 0 || metadata.architecture.is_none() {
        return Err("GGUF metadata is incomplete: tensor count or architecture missing".into());
    }
    Ok(())
}

async fn admit_with_server(artifact: &Path, server: &Path) -> Result<(), String> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))
        .map_err(|error| format!("could not allocate validation port: {error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    drop(listener);
    let arguments = admission_args(artifact, port);
    let mut child = Command::new(server)
        .args(&arguments)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("could not launch llama-server: {error}"))?;
    let stdout = child
        .stdout
        .take()
        .map(|stream| tokio::spawn(drain_process_lines(stream, "[llama-server]".to_string())));
    let stderr = child
        .stderr
        .take()
        .map(|stream| tokio::spawn(drain_process_lines(stream, "[llama-server]".to_string())));
    let deadline = lmml_server::default_startup_timeout();
    let result = timeout(deadline, async {
        loop {
            if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
                return Err(format!("llama-server exited while opening GGUF: {status}"));
            }
            if let Ok(mut stream) = tokio::net::TcpStream::connect(("127.0.0.1", port)).await {
                let request = format!(
                    "GET /health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
                );
                if stream.write_all(request.as_bytes()).await.is_ok() {
                    let mut response = Vec::new();
                    if stream.read_to_end(&mut response).await.is_ok()
                        && response.starts_with(b"HTTP/1.1 200")
                    {
                        return lmml_server::verify_served_model("127.0.0.1", port, artifact, None)
                            .await
                            .map(|_| ())
                            .map_err(|error| {
                                format!("llama-server opened a different GGUF: {error}")
                            });
                    }
                }
            }
            sleep(Duration::from_millis(250)).await;
        }
    })
    .await;
    if result.is_err() {
        let _ = child.kill().await;
        let _ = child.wait().await;
        return Err(format!(
            "llama-server did not open the GGUF within {} seconds",
            deadline.as_secs()
        ));
    }
    let result = result.unwrap();
    let _ = child.kill().await;
    let _ = child.wait().await;
    if let Some(task) = stdout {
        let _ = task.await;
    }
    if let Some(task) = stderr {
        let _ = task.await;
    }
    result
}

fn admission_args(artifact: &Path, port: u16) -> Vec<String> {
    vec![
        "-m".to_string(),
        artifact.to_string_lossy().into_owned(),
        "--host".to_string(),
        "127.0.0.1".to_string(),
        "--port".to_string(),
        port.to_string(),
        "--check-tensors".to_string(),
        "-c".to_string(),
        "1".to_string(),
        "-ngl".to_string(),
        "0".to_string(),
        "--no-warmup".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_admission_is_cpu_only_and_checks_tensors() {
        let arguments = admission_args(Path::new("/models/qwen.gguf"), 1234);
        assert!(arguments.windows(2).any(|pair| pair == ["-ngl", "0"]));
        assert!(arguments
            .iter()
            .any(|argument| argument == "--check-tensors"));
        assert!(arguments
            .windows(2)
            .any(|pair| pair == ["-m", "/models/qwen.gguf"]));
    }
}
