//! llama-server process management.
//!
//! Manages the lifecycle of the llama-server subprocess: start, stop,
//! restart, health checks, and configuration persistence.

pub mod config;
pub mod process;

/// Events sent from the server manager to the TUI.
#[derive(Debug, Clone)]
pub enum ServerEvent {
    LogLine(String),
    StatusChange(ServerStatus),
    Health(ServerMetrics),
}

/// Performance metrics reported by llama-server health endpoints.
#[derive(Debug, Clone, Default)]
pub struct ServerMetrics {
    pub latency_ms: f64,
    pub tok_s: f64,
    pub active_slots: Option<u64>,
    pub kv_cache_used: Option<u64>,
    pub kv_cache_total: Option<u64>,
}

/// Possible server statuses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerStatus {
    Stopped,
    Starting,
    Running,
    Stopping,
    Error(String),
}

/// Runtime server state for the TUI.
#[derive(Debug, Clone)]
pub struct ServerState {
    pub status: ServerStatus,
    pub log_lines: Vec<String>,
    pub health: Option<ServerMetrics>,
    pub pid: Option<u32>,
    pub uptime_secs: u64,
}

impl Default for ServerState {
    fn default() -> Self {
        ServerState {
            status: ServerStatus::Stopped,
            log_lines: Vec::new(),
            health: None,
            pid: None,
            uptime_secs: 0,
        }
    }
}
