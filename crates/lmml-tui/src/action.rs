//! User and background actions handled by the TUI core.

use std::path::PathBuf;

/// Action emitted by key handlers and dispatched by the event loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Re-run system detection.
    RunDetect,
    /// Start a llama.cpp build.
    StartBuild,
    /// Start a clean llama.cpp build.
    CleanBuild,
    /// Cancel the running build.
    CancelBuild,
    /// Start llama-server.
    StartServer,
    /// Stop llama-server.
    StopServer,
    /// Probe llama-server flag capabilities.
    ProbeServerCapabilities,
    /// Select a local model path.
    SelectModel(PathBuf),
    /// Scan local model directories.
    ScanModels,
    /// Open the Hugging Face search pane.
    OpenHfSearch,
    /// Search Hugging Face.
    SearchHf(lmml_models::HfSearchQuery),
    /// Download a Hugging Face result.
    DownloadModel(lmml_models::HfModelResult),
    /// Delete a local model.
    DeleteModel(lmml_models::ModelEntry),
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
