//! Model management for lmml.
//!
//! Scans local directories for .gguf files, parses model metadata,
//! and downloads models from HuggingFace with progress reporting.

pub mod download;
pub mod local;
pub mod types;

/// Events sent from download operations to the TUI.
#[derive(Debug, Clone)]
pub enum DownloadEvent {
    Progress {
        bytes: u64,
        total: u64,
        speed: f64,
        eta_secs: f64,
    },
    Complete(Result<(), String>),
}
