//! Error types with human-friendly Display impls.
//!
//! Every error includes a fix suggestion where possible.

use std::fmt;

/// Top-level application error.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Config error: {0}")]
    Config(String),

    #[error("Probe error: {0}")]
    Probe(String),

    #[error("Build error: {0}")]
    Build(String),

    #[error("Model error: {0}")]
    Model(String),

    #[error("Server error: {0}")]
    Server(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Build-specific errors.
#[derive(Debug)]
pub enum BuildError {
    GitCloneFailed { detail: String },
    CmakeFailed { code: Option<i32>, stderr: String },
    BuildFailed { code: Option<i32>, stderr: String },
    VerificationFailed { binary: String },
    CcacheMissing,
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BuildError::GitCloneFailed { detail } => {
                write!(
                    f,
                    "Failed to clone llama.cpp — {detail}\n\
                     Check your internet connection and try again."
                )
            }
            BuildError::CmakeFailed { code, stderr } => {
                write!(
                    f,
                    "cmake configuration failed (exit code: {code:?}) — {stderr}\n\
                     See the build log above for details. Try fixing any missing dependencies."
                )
            }
            BuildError::BuildFailed { code, stderr } => {
                write!(
                    f,
                    "cmake --build failed (exit code: {code:?}) — {stderr}\n\
                     Check the build log for compiler errors."
                )
            }
            BuildError::VerificationFailed { binary } => {
                write!(
                    f,
                    "Build completed but {binary} failed to run or reported an error.\n\
                     Try a clean rebuild."
                )
            }
            BuildError::CcacheMissing => {
                write!(
                    f,
                    "ccache not found — install with: sudo apt install ccache (Linux) or brew install ccache (macOS)"
                )
            }
        }
    }
}
