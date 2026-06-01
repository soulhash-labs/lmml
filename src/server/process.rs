use crate::server::{ServerEvent, ServerMetrics, ServerStatus};
use std::path::Path;
use std::time::{Duration, Instant};
use tokio::process::Child;
use tokio::sync::mpsc;

#[cfg(unix)]
use libc::{SIGKILL, SIGTERM};

/// Manages the llama-server subprocess lifecycle.
pub struct ServerProcess {
    child: Option<Child>,
    start_time: Option<Instant>,
    tx: mpsc::Sender<ServerEvent>,
}

impl ServerProcess {
    pub fn new(tx: mpsc::Sender<ServerEvent>) -> Self {
        ServerProcess {
            child: None,
            start_time: None,
            tx,
        }
    }

    /// Start llama-server with the given configuration.
    #[allow(clippy::too_many_arguments)]
    pub async fn start(
        &mut self,
        binary_path: &Path,
        model_path: &Path,
        port: u16,
        context_size: u32,
        gpu_layers: u32,
        threads: u32,
        batch_size: u32,
        extra_args: &[String],
    ) -> Result<u32, String> {
        if self.child.is_some() {
            return Err("Server is already running.".to_string());
        }

        let _ = self
            .tx
            .send(ServerEvent::StatusChange(ServerStatus::Starting))
            .await;

        let mut args: Vec<String> = vec![
            "-m".into(),
            model_path.to_string_lossy().to_string(),
            "--port".into(),
            port.to_string(),
            "-c".into(),
            context_size.to_string(),
            "-ngl".into(),
            gpu_layers.to_string(),
            "-b".into(),
            batch_size.to_string(),
        ];
        if threads > 0 {
            args.push("-t".into());
            args.push(threads.to_string());
        }
        args.extend(extra_args.iter().cloned());

        let mut child = tokio::process::Command::new(binary_path)
            .args(&args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| {
                format!(
                    "Failed to start llama-server at {} — is it built?\n{e}",
                    binary_path.display()
                )
            })?;

        let pid = child.id().ok_or("Failed to get child process ID")?;
        self.start_time = Some(Instant::now());

        // Spawn log reader
        let tx_log = self.tx.clone();
        let stdout = child.stdout.take().unwrap();
        tokio::spawn(async move {
            use tokio::io::AsyncBufReadExt;
            let reader = tokio::io::BufReader::new(stdout);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = tx_log.send(ServerEvent::LogLine(line)).await;
            }
        });

        let tx_log2 = self.tx.clone();
        let stderr = child.stderr.take().unwrap();
        tokio::spawn(async move {
            use tokio::io::AsyncBufReadExt;
            let reader = tokio::io::BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = tx_log2.send(ServerEvent::LogLine(line)).await;
            }
        });

        // Health check loop
        let tx_health = self.tx.clone();
        let health_port = port;
        let health_pid = pid;
        tokio::spawn(async move {
            // Wait up to 30s for first successful health check
            let deadline = Instant::now() + std::time::Duration::from_secs(30);
            let mut started = false;

            while Instant::now() < deadline {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;

                match check_health(health_port).await {
                    Ok(metrics) => {
                        if !started {
                            let _ = tx_health
                                .send(ServerEvent::StatusChange(ServerStatus::Running))
                                .await;
                            started = true;
                        }
                        let _ = tx_health.send(ServerEvent::Health(metrics)).await;
                        // Once running, check every 5s
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    }
                    Err(_) => {
                        if started {
                            let _ = tx_health
                                .send(ServerEvent::StatusChange(ServerStatus::Error(
                                    "Health check failed".into(),
                                )))
                                .await;
                        }
                    }
                }
            }

            if !started {
                let _ = tx_health
                    .send(ServerEvent::StatusChange(ServerStatus::Error(
                        "Server failed to start within 30s".into(),
                    )))
                    .await;
            }
        });

        self.child = Some(child);
        let _ = self
            .tx
            .send(ServerEvent::LogLine(format!("Server started (pid: {pid})")))
            .await;

        Ok(health_pid)
    }

    /// Stop the server gracefully.
    /// Sends SIGTERM, waits up to 5s for graceful exit, then SIGKILL.
    pub async fn stop(&mut self) -> Result<(), String> {
        let mut child = self.child.take().ok_or("Server is not running")?;
        let _ = self
            .tx
            .send(ServerEvent::StatusChange(ServerStatus::Stopping))
            .await;
        let _ = self
            .tx
            .send(ServerEvent::LogLine(
                "Shutting down server (SIGTERM)...".into(),
            ))
            .await;

        // Phase 1: SIGTERM — ask the process to shut down gracefully
        #[cfg(unix)]
        if let Some(pid) = child.id() {
            // SAFETY: kill with SIGTERM is safe — we own the child process
            unsafe {
                libc::kill(pid as i32, SIGTERM);
            }
        }
        #[cfg(not(unix))]
        let _ = child.kill().await; // fallback: immediate kill on non-Unix

        // Wait up to 5 seconds for graceful exit
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            tokio::time::sleep(Duration::from_millis(200)).await;
            match child.try_wait() {
                Ok(Some(_)) => break, // process exited
                Ok(None) => {
                    if Instant::now() >= deadline {
                        // Phase 2: SIGKILL — force kill
                        #[cfg(unix)]
                        if let Some(pid) = child.id() {
                            unsafe {
                                libc::kill(pid as i32, SIGKILL);
                            }
                        }
                        #[cfg(not(unix))]
                        let _ = child.kill().await;
                        let _ = child.wait().await;
                        let _ = self
                            .tx
                            .send(ServerEvent::LogLine(
                                "Server forcefully killed after 5s grace period".into(),
                            ))
                            .await;
                        break;
                    }
                }
                Err(e) => {
                    let _ = self
                        .tx
                        .send(ServerEvent::LogLine(format!(
                            "Error waiting for server shutdown: {e}"
                        )))
                        .await;
                    // Process went away unexpectedly
                    break;
                }
            }
        }

        self.start_time = None;
        let _ = self
            .tx
            .send(ServerEvent::StatusChange(ServerStatus::Stopped))
            .await;
        Ok(())
    }

    /// Restart with the same config.
    #[allow(clippy::too_many_arguments)]
    pub async fn restart(
        &mut self,
        binary_path: &Path,
        model_path: &Path,
        port: u16,
        context_size: u32,
        gpu_layers: u32,
        threads: u32,
        batch_size: u32,
        extra_args: &[String],
    ) -> Result<u32, String> {
        self.stop().await?;
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        self.start(
            binary_path,
            model_path,
            port,
            context_size,
            gpu_layers,
            threads,
            batch_size,
            extra_args,
        )
        .await
    }

    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().and_then(|c| c.id())
    }

    pub fn uptime(&self) -> u64 {
        self.start_time.map(|t| t.elapsed().as_secs()).unwrap_or(0)
    }
}

/// Check server health via GET /v1/health.
async fn check_health(port: u16) -> Result<ServerMetrics, String> {
    let start = Instant::now();
    let url = format!("http://127.0.0.1:{port}/v1/health");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client.get(&url).send().await.map_err(|e| e.to_string())?;
    let latency = start.elapsed().as_secs_f64() * 1000.0;

    if resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        let value = serde_json::from_str::<serde_json::Value>(&body).ok();
        let tok_s = value
            .as_ref()
            .and_then(|v| {
                first_f64(
                    v,
                    &[
                        "tokens_per_second",
                        "completion_tokens_per_second",
                        "predictions_per_sec",
                    ],
                )
            })
            .unwrap_or(0.0);
        let mut metrics = ServerMetrics {
            latency_ms: latency,
            tok_s,
            active_slots: value.as_ref().and_then(|v| {
                first_u64(
                    v,
                    &["active_slots", "n_slots_processing", "slots_processing"],
                )
            }),
            kv_cache_used: value.as_ref().and_then(|v| {
                first_u64(
                    v,
                    &["kv_cache_used", "kv_cache_used_cells", "kv_cache_tokens"],
                )
            }),
            kv_cache_total: value.as_ref().and_then(|v| {
                first_u64(
                    v,
                    &["kv_cache_total", "kv_cache_total_cells", "kv_cache_size"],
                )
            }),
        };
        if metrics.active_slots.is_none() || metrics.kv_cache_used.is_none() {
            enrich_metrics_from_prometheus(port, &mut metrics).await;
        }
        Ok(metrics)
    } else {
        Err(format!("HTTP {}", resp.status()))
    }
}

async fn enrich_metrics_from_prometheus(port: u16, metrics: &mut ServerMetrics) {
    let url = format!("http://127.0.0.1:{port}/metrics");
    let Ok(resp) = reqwest::Client::new().get(url).send().await else {
        return;
    };
    if !resp.status().is_success() {
        return;
    }
    let Ok(body) = resp.text().await else {
        return;
    };

    metrics.active_slots = metrics.active_slots.or_else(|| {
        first_metric_value(
            &body,
            &[
                "llamacpp_slots_processing",
                "llamacpp_slots_active",
                "llama_slots_processing",
            ],
        )
    });
    metrics.kv_cache_used = metrics.kv_cache_used.or_else(|| {
        first_metric_value(
            &body,
            &[
                "llamacpp_kv_cache_used_cells",
                "llamacpp_kv_cache_tokens",
                "llama_kv_cache_used_cells",
            ],
        )
    });
    metrics.kv_cache_total = metrics.kv_cache_total.or_else(|| {
        first_metric_value(
            &body,
            &[
                "llamacpp_kv_cache_total_cells",
                "llamacpp_kv_cache_size",
                "llama_kv_cache_total_cells",
            ],
        )
    });
}

fn first_metric_value(body: &str, names: &[&str]) -> Option<u64> {
    body.lines()
        .filter(|line| !line.starts_with('#'))
        .find_map(|line| {
            let name_matches = names.iter().any(|name| line.starts_with(name));
            if !name_matches {
                return None;
            }
            line.split_whitespace()
                .last()
                .and_then(|value| value.parse::<f64>().ok())
                .map(|value| value.max(0.0).round() as u64)
        })
}

fn first_f64(value: &serde_json::Value, keys: &[&str]) -> Option<f64> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(|v| v.as_f64()))
}

fn first_u64(value: &serde_json::Value, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(|v| v.as_u64()))
}

/// Check if a port is in use.
pub async fn is_port_in_use(port: u16) -> bool {
    use tokio::net::TcpStream;
    TcpStream::connect(format!("127.0.0.1:{port}"))
        .await
        .is_ok()
}

/// Health check loop — polls /v1/health until the server stops.
/// Sends StatusChange and Health events on the provided channel.
pub async fn run_health_loop(port: u16, tx: mpsc::Sender<ServerEvent>) {
    let deadline = Instant::now() + std::time::Duration::from_secs(30);
    let mut started = false;

    while Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        match check_health(port).await {
            Ok(metrics) => {
                if !started {
                    let _ = tx
                        .send(ServerEvent::StatusChange(ServerStatus::Running))
                        .await;
                    started = true;
                }
                let _ = tx.send(ServerEvent::Health(metrics)).await;
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
            Err(_) => {
                if started {
                    let _ = tx
                        .send(ServerEvent::StatusChange(ServerStatus::Error(
                            "Health check failed".into(),
                        )))
                        .await;
                }
            }
        }
    }

    if !started {
        let _ = tx
            .send(ServerEvent::StatusChange(ServerStatus::Error(
                "Server failed to start within 30s".into(),
            )))
            .await;
    }
}
