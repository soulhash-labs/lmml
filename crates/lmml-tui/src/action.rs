//! User and background actions handled by the TUI core.

use std::path::PathBuf;

/// Action emitted by key handlers and dispatched by the event loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Re-run system detection.
    RunDetect,
    /// Start a llama.cpp build.
    StartBuild,
    /// Cancel the running build.
    CancelBuild,
    /// Start llama-server.
    StartServer,
    /// Stop llama-server.
    StopServer,
    /// Select a local model path.
    SelectModel(PathBuf),
    /// Open the Hugging Face search pane.
    OpenHfSearch,
    /// Search Hugging Face.
    SearchHf(HfSearchQuery),
    /// Download a Hugging Face result.
    DownloadModel(HfModelResult),
    /// Delete a local model.
    DeleteModel(ModelEntry),
    /// Add an external model alias.
    AddModelAlias,
    /// Check llama.cpp for updates.
    CheckForUpdate,
    /// Update llama.cpp and rebuild.
    UpdateAndRebuild,
    /// Save settings to persistent state.
    SaveSettings,
    /// Toggle the help overlay.
    ShowHelp,
    /// Quit the application.
    Quit,
}

/// Minimal HF search query placeholder for the Milestone 5 TUI skeleton.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HfSearchQuery {
    /// User-entered keywords.
    pub keywords: String,
}

/// Minimal HF result placeholder until `lmml-models` lands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HfModelResult {
    /// Repository id.
    pub repo_id: String,
    /// GGUF filename.
    pub filename: String,
    /// Download URL.
    pub url: String,
}

/// Minimal model record placeholder until `lmml-models` owns the type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelEntry {
    /// Model path.
    pub path: PathBuf,
    /// Display name.
    pub name: String,
}
