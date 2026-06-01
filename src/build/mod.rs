//! Build pipeline for llama.cpp.
//!
//! Clones llama.cpp from GitHub, configures with cmake using flags
//! from the probe engine, then compiles with streaming output.

pub mod clone;
pub mod compile;

/// Events sent from the build pipeline to the TUI.
#[derive(Debug, Clone)]
pub enum BuildEvent {
    /// A line of cmake/make output.
    Line(String),
    /// Build progress (current, total) parsed from output.
    Progress { current: u32, total: u32 },
    /// Build completed successfully or with an error.
    Complete(Result<(), String>),
    /// Git commit hash after clone/pull.
    CommitHash(String),
}
