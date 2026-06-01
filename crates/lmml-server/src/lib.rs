//! llama-server lifecycle management for lmml.
//!
//! This crate owns the managed `llama-server` child process. It assembles
//! command-line arguments through `lmml-compat`, streams process logs to the
//! caller, checks port availability before spawning, and waits for HTTP
//! readiness before reporting the server as usable.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use lmml_compat::{LlamaBinaryCapabilities, ServerConfig};
use lmml_models::ModelEntry;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Child;
use tokio::sync::{mpsc, watch, Mutex};

/// Runtime status for a managed llama-server process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerStatus {
    /// No managed process is running.
    Stopped,
    /// The child has spawned and readiness polling is in progress.
    Starting {
        /// Time spent waiting for readiness.
        elapsed: Duration,
    },
    /// The server answered its health endpoint.
    Ready {
        /// HTTP URL callers can use.
        url: String,
    },
    /// Startup or runtime management failed.
    Failed {
        /// Human-readable failure reason.
        reason: String,
    },
}

/// Handle to a managed llama-server child process.
#[derive(Debug, Clone)]
pub struct ServerHandle {
    inner: Arc<ServerInner>,
}

#[derive(Debug)]
struct ServerInner {
    child: Arc<Mutex<Option<Child>>>,
    status_rx: watch::Receiver<ServerStatus>,
    status_tx: watch::Sender<ServerStatus>,
}

impl ServerHandle {
    /// Return the most recent status snapshot.
    pub fn status(&self) -> ServerStatus {
        self.inner.status_rx.borrow().clone()
    }

    /// Subscribe to status changes.
    pub fn subscribe(&self) -> watch::Receiver<ServerStatus> {
        self.inner.status_rx.clone()
    }

    /// Stop the child process, waiting briefly for a graceful exit.
    #[tracing::instrument(skip(self))]
    pub async fn stop(&self) {
        if let Err(error) = stop_child(self.inner.child.clone()).await {
            let _ignored = self.inner.status_tx.send(ServerStatus::Failed {
                reason: error.to_string(),
            });
            return;
        }
        let _ignored = self.inner.status_tx.send(ServerStatus::Stopped);
    }
}

impl Drop for ServerInner {
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.try_lock() {
            if let Some(child) = child.as_mut() {
                let _ignored = child.start_kill();
            }
        }
    }
}

/// Manager used to start llama-server with detected binary capabilities.
#[derive(Debug, Clone)]
pub struct ServerManager {
    /// Path to the `llama-server` binary.
    pub binary: PathBuf,
    /// CLI capabilities detected by `lmml-compat`.
    pub caps: LlamaBinaryCapabilities,
}

impl ServerManager {
    /// Start llama-server for `model` using a stable compat config.
    #[tracing::instrument(skip(self, model, config, log_tx), fields(binary = %self.binary.display(), model = %model.path.display(), host = %config.host, port = config.port))]
    pub async fn start(
        &self,
        model: &ModelEntry,
        config: &ServerConfig,
        log_tx: mpsc::Sender<String>,
    ) -> Result<ServerHandle, ServerError> {
        self.start_with_timeout(model, config, log_tx, Duration::from_secs(30))
            .await
    }

    /// Start llama-server with an explicit readiness timeout.
    #[tracing::instrument(skip(self, model, config, log_tx), fields(binary = %self.binary.display(), model = %model.path.display(), host = %config.host, port = config.port))]
    pub async fn start_with_timeout(
        &self,
        model: &ModelEntry,
        config: &ServerConfig,
        log_tx: mpsc::Sender<String>,
        startup_timeout: Duration,
    ) -> Result<ServerHandle, ServerError> {
        check_port_free(&config.host, config.port).await?;
        tracing::info!("server start requested");

        let mut config = config.clone();
        config.model = model.path.clone();
        let argv = lmml_compat::build_argv(&config, &self.caps);
        let _ignored = log_tx
            .send(format!(
                "Starting {} {}",
                self.binary.display(),
                argv.join(" ")
            ))
            .await;

        let mut child = tokio::process::Command::new(&self.binary)
            .args(&argv)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| ServerError::Spawn {
                binary: self.binary.clone(),
                source,
            })?;

        if let Some(stdout) = child.stdout.take() {
            spawn_log_reader(stdout, log_tx.clone());
        }
        if let Some(stderr) = child.stderr.take() {
            spawn_log_reader(stderr, log_tx.clone());
        }

        let (status_tx, status_rx) = watch::channel(ServerStatus::Starting {
            elapsed: Duration::ZERO,
        });
        let started_at = Instant::now();
        match wait_for_ready(&config.host, config.port, startup_timeout).await {
            Ok(url) => {
                tracing::info!(url = %url, "server ready");
                let _ignored = status_tx.send(ServerStatus::Ready { url });
            }
            Err(error) => {
                tracing::error!(error = %error, "server startup failed");
                let child = Arc::new(Mutex::new(Some(child)));
                let _ignored = stop_child(child).await;
                let reason = error.to_string();
                let _ignored = status_tx.send(ServerStatus::Failed {
                    reason: reason.clone(),
                });
                return Err(ServerError::Startup {
                    elapsed: started_at.elapsed(),
                    reason,
                });
            }
        }

        let child = Arc::new(Mutex::new(Some(child)));
        spawn_runtime_monitor(
            child.clone(),
            status_tx.clone(),
            config.host.clone(),
            config.port,
            log_tx,
        );

        Ok(ServerHandle {
            inner: Arc::new(ServerInner {
                child,
                status_rx,
                status_tx,
            }),
        })
    }
}

/// Check whether a host/port can be bound before spawning llama-server.
#[tracing::instrument]
pub async fn check_port_free(host: &str, port: u16) -> Result<(), ServerError> {
    match tokio::net::TcpListener::bind((host, port)).await {
        Ok(listener) => {
            drop(listener);
            Ok(())
        }
        Err(source) => Err(ServerError::PortInUse { port, source }),
    }
}

/// Poll the llama-server health endpoint until it returns a success response.
#[tracing::instrument]
pub async fn wait_for_ready(
    host: &str,
    port: u16,
    timeout: Duration,
) -> Result<String, ServerError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(ServerError::HttpClient)?;
    wait_for_ready_with(host, port, timeout, move |url| {
        let client = client.clone();
        async move {
            client
                .get(url)
                .send()
                .await
                .map(|response| response.status().is_success())
        }
    })
    .await
}

async fn wait_for_ready_with<F, Fut>(
    host: &str,
    port: u16,
    timeout: Duration,
    mut check: F,
) -> Result<String, ServerError>
where
    F: FnMut(String) -> Fut,
    Fut: std::future::Future<Output = Result<bool, reqwest::Error>>,
{
    let deadline = Instant::now() + timeout;
    let urls = health_urls(host, port);
    loop {
        if Instant::now() > deadline {
            return Err(ServerError::StartupTimeout { timeout });
        }
        for url in &urls {
            if check(url.clone()).await.unwrap_or(false) {
                return Ok(base_url(host, port));
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

async fn check_health_once(
    client: &reqwest::Client,
    host: &str,
    port: u16,
) -> Result<bool, reqwest::Error> {
    for url in health_urls(host, port) {
        if client.get(url).send().await?.status().is_success() {
            return Ok(true);
        }
    }
    Ok(false)
}

fn spawn_runtime_monitor(
    child: Arc<Mutex<Option<Child>>>,
    status_tx: watch::Sender<ServerStatus>,
    host: String,
    port: u16,
    log_tx: mpsc::Sender<String>,
) {
    tokio::spawn(async move {
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
        {
            Ok(client) => client,
            Err(error) => {
                let _ignored = status_tx.send(ServerStatus::Failed {
                    reason: format!("failed to create health-check HTTP client: {error}"),
                });
                return;
            }
        };
        let mut interval = tokio::time::interval(Duration::from_secs(2));
        loop {
            interval.tick().await;
            if status_tx.receiver_count() == 0 {
                break;
            }
            {
                let mut guard = child.lock().await;
                let Some(child) = guard.as_mut() else {
                    break;
                };
                match child.try_wait() {
                    Ok(Some(status)) => {
                        let reason = format!("llama-server exited with {status}");
                        let _ignored = log_tx.send(reason.clone()).await;
                        let _finished = guard.take();
                        let _ignored = status_tx.send(ServerStatus::Failed { reason });
                        break;
                    }
                    Ok(None) => {}
                    Err(error) => {
                        let reason = format!("failed to inspect llama-server process: {error}");
                        let _ignored = status_tx.send(ServerStatus::Failed { reason });
                        break;
                    }
                }
            }
            match check_health_once(&client, &host, port).await {
                Ok(true) => {}
                Ok(false) => {
                    let reason = "llama-server health check failed".to_string();
                    let _ignored = status_tx.send(ServerStatus::Failed { reason });
                    break;
                }
                Err(error) => {
                    let reason = format!("llama-server health check failed: {error}");
                    let _ignored = status_tx.send(ServerStatus::Failed { reason });
                    break;
                }
            }
        }
    });
}

fn spawn_log_reader<R>(reader: R, log_tx: mpsc::Sender<String>)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    if log_tx.send(line).await.is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    let _ignored = log_tx
                        .send(format!("server log read failed: {error}"))
                        .await;
                    break;
                }
            }
        }
    });
}

async fn stop_child(child: Arc<Mutex<Option<Child>>>) -> Result<(), ServerError> {
    let Some(mut child) = child.lock().await.take() else {
        return Ok(());
    };

    #[cfg(unix)]
    if let Some(pid) = child.id() {
        // SAFETY: the pid belongs to the child process owned by this handle.
        unsafe {
            libc::kill(pid as i32, libc::SIGTERM);
        }
    }
    #[cfg(not(unix))]
    child.start_kill().map_err(ServerError::Kill)?;

    let wait = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
    match wait {
        Ok(Ok(_status)) => Ok(()),
        Ok(Err(source)) => Err(ServerError::Wait { source }),
        Err(_elapsed) => {
            child.start_kill().map_err(ServerError::Kill)?;
            child
                .wait()
                .await
                .map_err(|source| ServerError::Wait { source })?;
            Ok(())
        }
    }
}

fn health_urls(host: &str, port: u16) -> Vec<String> {
    let base = base_url(host, port);
    vec![format!("{base}/health"), format!("{base}/v1/health")]
}

fn base_url(host: &str, port: u16) -> String {
    let host = connect_host(host);
    if host.contains(':') && !host.starts_with('[') {
        return format!("http://[{host}]:{port}");
    }
    match format!("{host}:{port}").parse::<SocketAddr>() {
        Ok(SocketAddr::V6(_)) => format!("http://[{host}]:{port}"),
        Ok(SocketAddr::V4(_)) | Err(_) => format!("http://{host}:{port}"),
    }
}

fn connect_host(host: &str) -> &str {
    match host {
        "0.0.0.0" | "::" => "127.0.0.1",
        other => other,
    }
}

/// Errors returned by server lifecycle management.
#[derive(Debug, Error)]
pub enum ServerError {
    /// The configured port is already bound or unavailable.
    #[error("llama-server failed to start — port {port} is already in use")]
    PortInUse {
        /// Conflicting port.
        port: u16,
        /// OS bind error.
        #[source]
        source: std::io::Error,
    },
    /// The process could not be spawned.
    #[error("failed to start llama-server at {binary}: {source}")]
    Spawn {
        /// Binary path.
        binary: PathBuf,
        /// Source IO error.
        #[source]
        source: std::io::Error,
    },
    /// Reqwest client construction failed.
    #[error("failed to create health-check HTTP client: {0}")]
    HttpClient(#[source] reqwest::Error),
    /// Readiness polling timed out.
    #[error("server did not become ready within {timeout:?}")]
    StartupTimeout {
        /// Configured timeout.
        timeout: Duration,
    },
    /// Startup failed after the process had been spawned.
    #[error("server startup failed after {elapsed:?}: {reason}")]
    Startup {
        /// Elapsed startup time.
        elapsed: Duration,
        /// Failure reason.
        reason: String,
    },
    /// Killing the child failed.
    #[error("failed to kill server process: {0}")]
    Kill(#[source] std::io::Error),
    /// Waiting for the child failed.
    #[error("failed to wait for server process: {source}")]
    Wait {
        /// Source IO error.
        #[source]
        source: std::io::Error,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn health_urls_try_current_and_legacy_paths() {
        assert_eq!(
            health_urls("0.0.0.0", 8080),
            vec![
                "http://127.0.0.1:8080/health".to_string(),
                "http://127.0.0.1:8080/v1/health".to_string(),
            ]
        );
    }

    #[test]
    fn ipv6_base_url_is_bracketed() {
        assert_eq!(base_url("::1", 8080), "http://[::1]:8080");
    }

    #[tokio::test]
    async fn readiness_uses_first_successful_health_endpoint() {
        let mut calls = Vec::new();
        let url = wait_for_ready_with("127.0.0.1", 8080, Duration::from_secs(1), |url| {
            calls.push(url.clone());
            async move { Ok(url.ends_with("/v1/health")) }
        })
        .await
        .expect("ready");

        assert_eq!(url, "http://127.0.0.1:8080");
        assert_eq!(
            calls,
            vec![
                "http://127.0.0.1:8080/health".to_string(),
                "http://127.0.0.1:8080/v1/health".to_string(),
            ]
        );
    }
}
