//! Application state and action dispatch for the TUI.
//!
//! Rendering lives in `tabs` and widgets. This module owns navigation, modal
//! state, background-task status, and persistent state coordination.

mod settings_state;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::PathBuf;

use lmml_build::{BuildEvent, UpdateCheck};
use lmml_compat::LlamaBinaryCapabilities;
use lmml_detect::{BuildBackend, SystemProfile};
use lmml_models::{DownloadProgress, HfModelResult, HfSearchQuery, ModelEntry, QuantTier};
use lmml_server::ServerHandle;
pub use lmml_server::ServerStatus;
use lmml_state::AppState as PersistentState;

use crate::action::Action;
pub use settings_state::{SettingsField, SettingsKeyResult};

/// Top-level TUI tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    /// Hardware detection tab.
    Detect,
    /// llama.cpp build tab.
    Build,
    /// Local/HF models tab.
    Models,
    /// llama-server lifecycle tab.
    Server,
    /// Settings editor tab.
    Settings,
}

impl Tab {
    /// All tabs in display order.
    pub const ALL: [Tab; 5] = [
        Tab::Detect,
        Tab::Build,
        Tab::Models,
        Tab::Server,
        Tab::Settings,
    ];

    /// User-visible tab title.
    pub fn title(self) -> &'static str {
        match self {
            Tab::Detect => "Detect",
            Tab::Build => "Build",
            Tab::Models => "Models",
            Tab::Server => "Server",
            Tab::Settings => "Settings",
        }
    }

    fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|tab| *tab == self)
            .unwrap_or_default()
    }
}

/// Background and terminal events consumed by the app.
#[derive(Debug, Clone)]
pub enum AppEvent {
    /// Terminal key press.
    Key(KeyEvent),
    /// Terminal resize.
    Resize(u16, u16),
    /// Detection task completed.
    DetectComplete(Box<SystemProfile>),
    /// Build task emitted an event.
    BuildEvent(BuildEvent),
    /// Server status changed.
    ServerStatus(ServerStatus),
    /// Server startup completed.
    ServerStarted(Result<ServerHandle, String>),
    /// Server model-swap restart completed.
    ServerModelSwapComplete {
        /// Model requested for the replacement server.
        model: ModelEntry,
        /// Restart result.
        result: Result<ServerHandle, String>,
    },
    /// llama-server capabilities probe completed.
    ServerCapabilities(Result<LlamaBinaryCapabilities, String>),
    /// Server log line.
    ServerLog(String),
    /// Download progress changed.
    DownloadProgress(DownloadProgress),
    /// Download completed.
    DownloadComplete(Result<ModelEntry, String>),
    /// Model scan completed.
    ModelScanComplete(Vec<ModelEntry>),
    /// Model registry mutation failed.
    ModelRegistryError(String),
    /// Hugging Face search completed.
    HfSearchResults(Vec<HfModelResult>),
    /// Update check completed.
    UpdateCheckResult(UpdateCheck),
}

/// Active user-input modal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Modal {
    /// Path prompt for adding an external model alias.
    AddAlias {
        /// Current path input buffer.
        buffer: String,
        /// Inline validation error.
        error: Option<String>,
    },
    /// Confirmation prompt for deleting a model.
    ConfirmDelete {
        /// Model selected for deletion.
        model: ModelEntry,
    },
    /// Confirmation prompt for restarting server with another model.
    ConfirmModelSwap {
        /// Model selected for serving after restart.
        model: ModelEntry,
    },
    /// Hugging Face search query editor.
    HfSearch {
        /// Focused search field.
        field: HfSearchField,
        /// Keyword query buffer.
        keywords: String,
        /// Optional architecture filter buffer.
        architecture: String,
        /// Optional quantization tier filter.
        quant_filter: Option<QuantTier>,
        /// Inline validation error.
        error: Option<String>,
    },
}

/// Editable fields in the Hugging Face search modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HfSearchField {
    /// Keyword query.
    Keywords,
    /// Architecture filter.
    Architecture,
    /// Quantization tier filter.
    Quant,
}

/// First-run onboarding step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnboardingStep {
    /// Prompt to scan the system.
    Scan,
    /// Hardware summary after detection.
    HardwareSummary,
    /// Confirm or choose backend.
    Backend,
    /// Choose model directory.
    ModelsDir,
    /// Optional starter-model search/download.
    StarterModel,
    /// Configure server port.
    ServerPort,
    /// Final completion screen.
    Done,
}

/// Mutable application state owned by the event loop.
#[derive(Debug)]
pub struct App {
    /// Persisted state loaded from `lmml-state`.
    pub state: PersistentState,
    /// Active top-level tab.
    pub active_tab: Tab,
    /// Whether the application should quit.
    pub should_quit: bool,
    /// Whether the help overlay is visible.
    pub show_help: bool,
    /// Whether first-run onboarding should be shown.
    pub first_run_onboarding: bool,
    /// Current first-run onboarding step.
    pub onboarding_step: OnboardingStep,
    /// Backend selected during onboarding.
    pub onboarding_backend: Option<BuildBackend>,
    /// Model directory input buffer for onboarding.
    pub onboarding_models_dir_buffer: String,
    /// Server port input buffer for onboarding.
    pub onboarding_port_buffer: String,
    /// Inline onboarding validation error.
    pub onboarding_error: Option<String>,
    /// Full detection profile for the current session.
    pub detect_profile: Option<SystemProfile>,
    /// Current server status.
    pub server_status: ServerStatus,
    /// Last probed llama-server capabilities.
    pub server_caps: Option<LlamaBinaryCapabilities>,
    /// Last server capability probe error.
    pub server_caps_error: Option<String>,
    /// Detect tab log lines.
    pub detect_log: Vec<String>,
    /// Build tab log lines.
    pub build_log: Vec<String>,
    /// Whether a build is currently running.
    pub build_running: bool,
    /// Last completed build binary, if any.
    pub build_binary: Option<PathBuf>,
    /// Last build error, if any.
    pub build_error: Option<String>,
    /// Server tab log lines.
    pub server_log: Vec<String>,
    /// Models found by the registry scan.
    pub models: Vec<ModelEntry>,
    /// Selected model list index.
    pub selected_model: usize,
    /// Whether HF search pane is open.
    pub hf_search_open: bool,
    /// Current HF search query.
    pub hf_query: String,
    /// HF search results.
    pub hf_results: Vec<HfModelResult>,
    /// Selected HF result index.
    pub selected_hf_result: usize,
    /// Current download progress.
    pub download_progress: Option<DownloadProgress>,
    /// Last download error.
    pub download_error: Option<String>,
    /// Last update-check result.
    pub update_check: Option<UpdateCheck>,
    /// Selected Settings tab field.
    pub selected_settings_field: SettingsField,
    /// Active inline settings edit buffer.
    pub settings_edit_buffer: Option<String>,
    /// Inline validation error for the Settings tab.
    pub settings_validation_error: Option<String>,
    /// Active modal prompt.
    pub active_modal: Option<Modal>,
    /// Optional state path override used by integration-style tests.
    pub state_save_path: Option<PathBuf>,
    /// Last UI status message.
    pub status_message: String,
    /// Current terminal size.
    pub terminal_size: Option<(u16, u16)>,
}

impl App {
    /// Construct an app using state loaded from disk, falling back to defaults on error.
    pub fn new() -> Self {
        let first_run = !PersistentState::path().exists();
        let state = PersistentState::load().unwrap_or_default();
        Self::new_with_state_and_first_run(state, first_run)
    }

    /// Construct an app with injected state for tests.
    pub fn new_with_state(state: PersistentState) -> Self {
        Self::new_with_state_and_first_run(state, false)
    }

    /// Construct an app with injected state and onboarding flag for tests.
    pub fn new_with_state_and_first_run(
        state: PersistentState,
        first_run_onboarding: bool,
    ) -> Self {
        let onboarding_models_dir_buffer = state.model.models_dir.to_string_lossy().into_owned();
        let onboarding_port_buffer = state.server.port.to_string();
        Self {
            state,
            active_tab: Tab::Detect,
            should_quit: false,
            show_help: false,
            first_run_onboarding,
            onboarding_step: OnboardingStep::Scan,
            onboarding_backend: None,
            onboarding_models_dir_buffer,
            onboarding_port_buffer,
            onboarding_error: None,
            detect_profile: None,
            server_status: ServerStatus::Stopped,
            server_caps: None,
            server_caps_error: None,
            detect_log: Vec::new(),
            build_log: Vec::new(),
            build_running: false,
            build_binary: None,
            build_error: None,
            server_log: Vec::new(),
            models: Vec::new(),
            selected_model: 0,
            hf_search_open: false,
            hf_query: "gguf".to_string(),
            hf_results: Vec::new(),
            selected_hf_result: 0,
            download_progress: None,
            download_error: None,
            update_check: None,
            selected_settings_field: SettingsField::Host,
            settings_edit_buffer: None,
            settings_validation_error: None,
            active_modal: None,
            state_save_path: None,
            status_message: "Ready".to_string(),
            terminal_size: None,
        }
    }

    /// Handle a terminal/background event.
    pub fn handle_event(&mut self, event: AppEvent) -> Option<Action> {
        match event {
            AppEvent::Key(key) => self.handle_key(key),
            AppEvent::Resize(width, height) => {
                self.terminal_size = Some((width, height));
                None
            }
            AppEvent::DetectComplete(profile) => {
                let profile = *profile;
                self.state.system_profile = Some(lmml_state::SystemProfile {
                    cuda_toolkit: match &profile.cuda {
                        lmml_detect::CudaCompatibility::Compatible { .. } => {
                            Some("available".to_string())
                        }
                        lmml_detect::CudaCompatibility::ToolkitTooOld { found_toolkit, .. } => {
                            Some(found_toolkit.clone())
                        }
                        lmml_detect::CudaCompatibility::NoGpu
                        | lmml_detect::CudaCompatibility::NvccMissing => None,
                    },
                    gpu_names: profile.gpus.iter().map(|gpu| gpu.name.clone()).collect(),
                    gpu_archs: profile
                        .gpus
                        .iter()
                        .filter_map(|gpu| gpu.arch.map(ToOwned::to_owned))
                        .collect(),
                    vram_mb: profile.gpus.iter().map(|gpu| gpu.memory_total_mb).collect(),
                    sccache: profile.sccache.is_some(),
                });
                self.detect_log.push(format!(
                    "Detected backend: {:?}",
                    profile.recommended_backend()
                ));
                for warning in profile.warnings() {
                    self.detect_log
                        .push(format!("Warning: {}", warning.message));
                }
                for missing in profile.missing_prerequisites() {
                    self.detect_log
                        .push(format!("Missing {}: {}", missing.name, missing.install));
                }
                self.detect_profile = Some(profile);
                if self.first_run_onboarding {
                    self.onboarding_step = OnboardingStep::HardwareSummary;
                    self.onboarding_backend = self
                        .detect_profile
                        .as_ref()
                        .map(SystemProfile::recommended_backend);
                }
                self.status_message = "Detection complete".to_string();
                self.save_state_after("Detection complete");
                None
            }
            AppEvent::BuildEvent(event) => {
                self.handle_build_event(event);
                None
            }
            AppEvent::ServerStatus(status) => {
                self.server_status = status;
                None
            }
            AppEvent::ServerStarted(result) => {
                match result {
                    Ok(handle) => {
                        self.server_status = handle.status();
                        self.status_message = "Server ready".to_string();
                    }
                    Err(error) => {
                        self.server_status = ServerStatus::Failed {
                            reason: error.clone(),
                        };
                        self.status_message = format!("Server failed: {error}");
                    }
                }
                None
            }
            AppEvent::ServerModelSwapComplete { model, result } => {
                match result {
                    Ok(handle) => {
                        self.server_status = handle.status();
                        self.state.model.last_used = model.path.clone();
                        if let Some(index) = self
                            .models
                            .iter()
                            .position(|entry| entry.path == model.path)
                        {
                            self.selected_model = index;
                        }
                        self.status_message = format!("Server restarted with {}", model.name);
                        self.save_state_after("Model selected");
                    }
                    Err(error) => {
                        self.server_status = ServerStatus::Failed {
                            reason: error.clone(),
                        };
                        self.status_message = format!("Model swap failed: {error}");
                    }
                }
                None
            }
            AppEvent::ServerCapabilities(result) => {
                match result {
                    Ok(caps) => {
                        self.server_caps = Some(caps);
                        self.server_caps_error = None;
                        self.status_message = "Server capabilities probed".to_string();
                    }
                    Err(error) => {
                        self.server_caps = None;
                        self.server_caps_error = Some(error.clone());
                        self.status_message = format!("Capability probe failed: {error}");
                    }
                }
                None
            }
            AppEvent::ServerLog(line) => {
                self.server_log.push(line);
                const MAX_SERVER_LOG_LINES: usize = 500;
                if self.server_log.len() > MAX_SERVER_LOG_LINES {
                    let overflow = self.server_log.len() - MAX_SERVER_LOG_LINES;
                    self.server_log.drain(0..overflow);
                }
                None
            }
            AppEvent::DownloadProgress(progress) => {
                self.download_progress = Some(progress.clone());
                self.status_message = match progress.total_bytes {
                    Some(total) => {
                        format!("Downloading {} / {} bytes", progress.bytes_received, total)
                    }
                    None => format!("Downloading {} bytes", progress.bytes_received),
                };
                None
            }
            AppEvent::DownloadComplete(result) => {
                match result {
                    Ok(model) => {
                        self.download_error = None;
                        self.status_message = format!("Downloaded {}", model.name);
                        self.state.model.last_used = model.path.clone();
                        self.models.push(model);
                        self.save_state_after("Model downloaded");
                    }
                    Err(error) => {
                        self.download_error = Some(error.clone());
                        self.status_message = format!("Download failed: {error}");
                    }
                }
                None
            }
            AppEvent::ModelScanComplete(models) => {
                let count = models.len();
                self.models = models;
                self.selected_model = self.selected_model.min(self.models.len().saturating_sub(1));
                self.status_message = format!("{count} model(s) found");
                None
            }
            AppEvent::ModelRegistryError(error) => {
                self.status_message = error;
                None
            }
            AppEvent::HfSearchResults(results) => {
                let count = results.len();
                self.hf_results = results;
                self.selected_hf_result = self
                    .selected_hf_result
                    .min(self.hf_results.len().saturating_sub(1));
                self.status_message = format!("{count} Hugging Face result(s)");
                None
            }
            AppEvent::UpdateCheckResult(update) => {
                self.update_check = Some(update);
                None
            }
        }
    }

    /// Dispatch a user action into local state changes.
    pub fn dispatch(&mut self, action: Action) {
        match action {
            Action::RunDetect => {
                self.detect_log
                    .push("Starting system detection".to_string());
                self.status_message = "Detecting system".to_string();
            }
            Action::StartBuild => {
                self.first_run_onboarding = false;
                self.build_running = true;
                self.build_error = None;
                self.push_build_log("Starting build");
                self.status_message = "Build requested".to_string();
            }
            Action::CleanBuild => {
                self.first_run_onboarding = false;
                self.build_running = true;
                self.build_error = None;
                self.push_build_log("Starting clean build");
                self.status_message = "Clean build requested".to_string();
            }
            Action::CancelBuild => {
                self.push_build_log("Build cancellation requested");
                self.status_message = "Cancelling build".to_string();
            }
            Action::StartServer => {
                self.server_status = ServerStatus::Starting {
                    elapsed: std::time::Duration::ZERO,
                };
                self.status_message = "Starting server".to_string();
            }
            Action::StopServer => {
                self.server_status = ServerStatus::Stopped;
                self.status_message = "Server stopped".to_string();
            }
            Action::ProbeServerCapabilities => {
                self.status_message = "Probing server capabilities".to_string();
            }
            Action::SelectModel(path) => {
                self.state.model.last_used = path;
                self.status_message = "Model selected".to_string();
                self.save_state_after("Model selected");
            }
            Action::ScanModels => {
                self.status_message = "Scanning models".to_string();
            }
            Action::OpenHfSearch => {
                self.hf_search_open = true;
                self.active_modal = Some(Modal::HfSearch {
                    field: HfSearchField::Keywords,
                    keywords: self.hf_query.clone(),
                    architecture: String::new(),
                    quant_filter: None,
                    error: None,
                });
                self.status_message = "HF search opened".to_string();
            }
            Action::SearchHf(query) => {
                self.hf_search_open = true;
                self.hf_query = query.keywords.clone();
                self.active_modal = None;
                self.status_message = format!("Searching: {}", query.keywords);
            }
            Action::DownloadModel(result) => {
                self.download_progress = None;
                self.download_error = None;
                self.status_message = format!("Downloading {}", result.filename);
            }
            Action::DeleteModel(model) => {
                self.active_modal = Some(Modal::ConfirmDelete { model });
                self.status_message = "Confirm model delete".to_string();
            }
            Action::ConfirmDeleteModel(model) => {
                self.active_modal = None;
                self.status_message = format!("Deleting {}", model.name);
            }
            Action::ConfirmModelSwap(model) => {
                self.active_modal = None;
                self.status_message = format!("Restarting server with {}", model.name);
            }
            Action::AddModelAlias => {
                self.active_modal = Some(Modal::AddAlias {
                    buffer: String::new(),
                    error: None,
                });
                self.status_message = "Enter model alias path".to_string();
            }
            Action::ConfirmAddModelAlias(path) => {
                self.active_modal = None;
                self.status_message = format!("Adding alias {}", path.display());
            }
            Action::CheckForUpdate => {
                self.status_message = "Checking for updates".to_string();
            }
            Action::UpdateAndRebuild => {
                self.first_run_onboarding = false;
                self.build_running = true;
                self.build_error = None;
                self.push_build_log("Updating source and starting clean rebuild");
                self.status_message = match self.state.build.track_mode {
                    lmml_state::TrackMode::Main => "Updating main and rebuilding".to_string(),
                    lmml_state::TrackMode::Tag => "Rebuilding pinned ref".to_string(),
                };
            }
            Action::SaveSettings => {
                self.status_message = if self.state.save().is_ok() {
                    "Settings saved".to_string()
                } else {
                    "Settings save failed".to_string()
                };
            }
            Action::ShowHelp => {
                self.show_help = !self.show_help;
                if self.first_run_onboarding {
                    self.first_run_onboarding = false;
                }
            }
            Action::Quit => {
                self.save_state_after("State saved");
                self.should_quit = true;
            }
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> Option<Action> {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Some(Action::Quit);
        }
        if self.active_modal.is_some() {
            return self.handle_modal_key(key);
        }
        if self.first_run_onboarding {
            return self.handle_onboarding_key(key);
        }
        if self.active_tab == Tab::Settings {
            match self.handle_settings_key(key) {
                SettingsKeyResult::Handled(action) => return action,
                SettingsKeyResult::Unhandled => {}
            }
        }

        match key.code {
            KeyCode::Char('1') => {
                self.active_tab = Tab::Detect;
                None
            }
            KeyCode::Char('2') => {
                self.active_tab = Tab::Build;
                None
            }
            KeyCode::Char('3') => {
                self.active_tab = Tab::Models;
                None
            }
            KeyCode::Up if self.active_tab == Tab::Models => {
                if self.hf_search_open && !self.hf_results.is_empty() {
                    self.selected_hf_result = self.selected_hf_result.saturating_sub(1);
                } else {
                    self.select_previous_model();
                }
                None
            }
            KeyCode::Down if self.active_tab == Tab::Models => {
                if self.hf_search_open && !self.hf_results.is_empty() {
                    self.selected_hf_result =
                        (self.selected_hf_result + 1).min(self.hf_results.len() - 1);
                } else {
                    self.select_next_model();
                }
                None
            }
            KeyCode::Enter if self.active_tab == Tab::Models => self
                .models
                .get(self.selected_model)
                .map(|model| Action::SelectModel(model.path.clone())),
            KeyCode::Char('4') => {
                self.active_tab = Tab::Server;
                None
            }
            KeyCode::Char('5') => {
                self.active_tab = Tab::Settings;
                None
            }
            KeyCode::Tab => {
                self.next_tab();
                None
            }
            KeyCode::BackTab => {
                self.previous_tab();
                None
            }
            KeyCode::Char('?') => Some(Action::ShowHelp),
            KeyCode::Char('q') => Some(Action::Quit),
            KeyCode::Enter if self.first_run_onboarding => Some(Action::StartBuild),
            KeyCode::Esc if self.first_run_onboarding => {
                self.first_run_onboarding = false;
                None
            }
            KeyCode::Char('d') => Some(Action::RunDetect),
            KeyCode::Char('b') => Some(Action::StartBuild),
            KeyCode::Char('B') => Some(Action::CleanBuild),
            KeyCode::Char('u') => Some(Action::CheckForUpdate),
            KeyCode::Char('s') => match self.active_tab {
                Tab::Server => match self.server_status {
                    ServerStatus::Stopped | ServerStatus::Failed { .. } => {
                        Some(Action::StartServer)
                    }
                    ServerStatus::Starting { .. } | ServerStatus::Ready { .. } => {
                        Some(Action::StopServer)
                    }
                },
                Tab::Settings => Some(Action::SaveSettings),
                Tab::Detect | Tab::Build | Tab::Models => None,
            },
            KeyCode::Char('m') if self.active_tab == Tab::Server => {
                let next_index = self.next_model_index()?;
                let model = self.models.get(next_index).cloned()?;
                match self.server_status {
                    ServerStatus::Stopped | ServerStatus::Failed { .. } => {
                        self.selected_model = next_index;
                        self.state.model.last_used = model.path.clone();
                        self.status_message = format!("Selected {}", model.name);
                        self.save_state_after("Model selected");
                    }
                    ServerStatus::Starting { .. } | ServerStatus::Ready { .. } => {
                        self.active_modal = Some(Modal::ConfirmModelSwap { model });
                        self.status_message = "Confirm server model swap".to_string();
                    }
                }
                None
            }
            KeyCode::Char('/') if self.active_tab == Tab::Models => Some(Action::OpenHfSearch),
            KeyCode::Char('D') if self.active_tab == Tab::Models => self
                .hf_results
                .get(self.selected_hf_result)
                .cloned()
                .map(Action::DownloadModel),
            KeyCode::Char('a') if self.active_tab == Tab::Models => Some(Action::AddModelAlias),
            KeyCode::Char('x') if self.active_tab == Tab::Models => self
                .models
                .get(self.selected_model)
                .cloned()
                .map(Action::DeleteModel),
            KeyCode::Char('r') if self.active_tab == Tab::Models => Some(Action::ScanModels),
            _ => None,
        }
    }

    fn handle_onboarding_key(&mut self, key: KeyEvent) -> Option<Action> {
        match self.onboarding_step {
            OnboardingStep::Scan => match key.code {
                KeyCode::Enter | KeyCode::Char('d') => Some(Action::RunDetect),
                KeyCode::Esc => {
                    self.first_run_onboarding = false;
                    None
                }
                _ => None,
            },
            OnboardingStep::HardwareSummary => match key.code {
                KeyCode::Enter => {
                    self.onboarding_step = OnboardingStep::Backend;
                    None
                }
                KeyCode::Esc => {
                    self.first_run_onboarding = false;
                    None
                }
                _ => None,
            },
            OnboardingStep::Backend => match key.code {
                KeyCode::Left | KeyCode::Right | KeyCode::Tab => {
                    self.onboarding_backend = Some(next_backend(
                        self.onboarding_backend
                            .clone()
                            .unwrap_or(BuildBackend::CpuFallback),
                    ));
                    None
                }
                KeyCode::Enter => {
                    let backend = self
                        .onboarding_backend
                        .clone()
                        .unwrap_or(BuildBackend::CpuFallback);
                    self.state.build.backend = backend_name(&backend);
                    self.state.build.archs = backend_archs(&backend);
                    self.save_state_after("Backend selected");
                    self.onboarding_step = OnboardingStep::ModelsDir;
                    None
                }
                KeyCode::Esc => {
                    self.first_run_onboarding = false;
                    None
                }
                _ => None,
            },
            OnboardingStep::ModelsDir => match key.code {
                KeyCode::Enter => {
                    let value = self.onboarding_models_dir_buffer.trim();
                    if value.is_empty() {
                        self.onboarding_error = Some("models directory is required".to_string());
                    } else {
                        self.state.model.models_dir = PathBuf::from(value);
                        self.save_state_after("Models directory selected");
                        self.onboarding_error = None;
                        self.onboarding_step = OnboardingStep::StarterModel;
                    }
                    None
                }
                KeyCode::Backspace => {
                    self.onboarding_models_dir_buffer.pop();
                    self.onboarding_error = None;
                    None
                }
                KeyCode::Char(value) => {
                    self.onboarding_models_dir_buffer.push(value);
                    self.onboarding_error = None;
                    None
                }
                KeyCode::Esc => {
                    self.first_run_onboarding = false;
                    None
                }
                _ => None,
            },
            OnboardingStep::StarterModel => match key.code {
                KeyCode::Char('d') | KeyCode::Char('/') => {
                    self.onboarding_step = OnboardingStep::ServerPort;
                    Some(Action::OpenHfSearch)
                }
                KeyCode::Enter => {
                    self.onboarding_step = OnboardingStep::ServerPort;
                    None
                }
                KeyCode::Esc => {
                    self.first_run_onboarding = false;
                    None
                }
                _ => None,
            },
            OnboardingStep::ServerPort => match key.code {
                KeyCode::Enter => match parse_onboarding_port(&self.onboarding_port_buffer) {
                    Ok(port) => {
                        self.state.server.port = port;
                        self.save_state_after("Server port configured");
                        self.onboarding_error = None;
                        self.onboarding_step = OnboardingStep::Done;
                        None
                    }
                    Err(error) => {
                        self.onboarding_error = Some(error);
                        None
                    }
                },
                KeyCode::Backspace => {
                    self.onboarding_port_buffer.pop();
                    self.onboarding_error = None;
                    None
                }
                KeyCode::Char(value) if value.is_ascii_digit() => {
                    self.onboarding_port_buffer.push(value);
                    self.onboarding_error = None;
                    None
                }
                KeyCode::Esc => {
                    self.first_run_onboarding = false;
                    None
                }
                _ => None,
            },
            OnboardingStep::Done => match key.code {
                KeyCode::Enter | KeyCode::Esc => {
                    self.first_run_onboarding = false;
                    self.active_tab = Tab::Build;
                    None
                }
                _ => None,
            },
        }
    }

    fn handle_modal_key(&mut self, key: KeyEvent) -> Option<Action> {
        match self.active_modal.take() {
            Some(Modal::AddAlias {
                mut buffer,
                mut error,
            }) => match key.code {
                KeyCode::Esc => None,
                KeyCode::Enter => {
                    let trimmed = buffer.trim();
                    if trimmed.is_empty() {
                        error = Some("path is required".to_string());
                        self.active_modal = Some(Modal::AddAlias { buffer, error });
                        None
                    } else {
                        Some(Action::ConfirmAddModelAlias(PathBuf::from(trimmed)))
                    }
                }
                KeyCode::Backspace => {
                    buffer.pop();
                    self.active_modal = Some(Modal::AddAlias {
                        buffer,
                        error: None,
                    });
                    None
                }
                KeyCode::Char(value) => {
                    buffer.push(value);
                    self.active_modal = Some(Modal::AddAlias {
                        buffer,
                        error: None,
                    });
                    None
                }
                _ => {
                    self.active_modal = Some(Modal::AddAlias { buffer, error });
                    None
                }
            },
            Some(Modal::ConfirmDelete { model }) => match key.code {
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => None,
                KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                    Some(Action::ConfirmDeleteModel(model))
                }
                _ => {
                    self.active_modal = Some(Modal::ConfirmDelete { model });
                    None
                }
            },
            Some(Modal::ConfirmModelSwap { model }) => match key.code {
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => None,
                KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                    Some(Action::ConfirmModelSwap(model))
                }
                _ => {
                    self.active_modal = Some(Modal::ConfirmModelSwap { model });
                    None
                }
            },
            Some(Modal::HfSearch {
                mut field,
                mut keywords,
                mut architecture,
                mut quant_filter,
                mut error,
            }) => match key.code {
                KeyCode::Esc => None,
                KeyCode::Tab | KeyCode::Down => {
                    field = next_hf_field(field);
                    self.active_modal = Some(Modal::HfSearch {
                        field,
                        keywords,
                        architecture,
                        quant_filter,
                        error,
                    });
                    None
                }
                KeyCode::BackTab | KeyCode::Up => {
                    field = previous_hf_field(field);
                    self.active_modal = Some(Modal::HfSearch {
                        field,
                        keywords,
                        architecture,
                        quant_filter,
                        error,
                    });
                    None
                }
                KeyCode::Left | KeyCode::Right if field == HfSearchField::Quant => {
                    quant_filter = next_quant_filter(quant_filter);
                    self.active_modal = Some(Modal::HfSearch {
                        field,
                        keywords,
                        architecture,
                        quant_filter,
                        error: None,
                    });
                    None
                }
                KeyCode::Backspace => {
                    match field {
                        HfSearchField::Keywords => {
                            keywords.pop();
                        }
                        HfSearchField::Architecture => {
                            architecture.pop();
                        }
                        HfSearchField::Quant => {
                            quant_filter = None;
                        }
                    }
                    self.active_modal = Some(Modal::HfSearch {
                        field,
                        keywords,
                        architecture,
                        quant_filter,
                        error: None,
                    });
                    None
                }
                KeyCode::Char(value) => {
                    match field {
                        HfSearchField::Keywords => keywords.push(value),
                        HfSearchField::Architecture => architecture.push(value),
                        HfSearchField::Quant => {
                            quant_filter = quant_from_char(value).or(quant_filter);
                        }
                    }
                    self.active_modal = Some(Modal::HfSearch {
                        field,
                        keywords,
                        architecture,
                        quant_filter,
                        error: None,
                    });
                    None
                }
                KeyCode::Enter => {
                    if keywords.trim().is_empty() {
                        error = Some("keywords are required".to_string());
                        self.active_modal = Some(Modal::HfSearch {
                            field,
                            keywords,
                            architecture,
                            quant_filter,
                            error,
                        });
                        None
                    } else {
                        Some(Action::SearchHf(HfSearchQuery {
                            keywords: keywords.trim().to_string(),
                            architecture: non_empty_filter(&architecture),
                            quant_filter,
                            max_results: 20,
                        }))
                    }
                }
                _ => {
                    self.active_modal = Some(Modal::HfSearch {
                        field,
                        keywords,
                        architecture,
                        quant_filter,
                        error,
                    });
                    None
                }
            },
            None => None,
        }
    }

    fn handle_build_event(&mut self, event: BuildEvent) {
        match event {
            BuildEvent::Cloning { url } => {
                self.build_running = true;
                self.push_build_log(format!("Cloning {url}"));
            }
            BuildEvent::CmakeConfiguring => {
                self.build_running = true;
                self.push_build_log("Configuring CMake");
            }
            BuildEvent::Compiling { line } => {
                self.build_running = true;
                self.push_build_log(line);
            }
            BuildEvent::Linking => {
                self.build_running = true;
                self.push_build_log("Linking");
            }
            BuildEvent::Completed {
                binary,
                fingerprint,
                backend,
                archs,
                sccache_used,
                ..
            } => {
                self.build_running = false;
                self.state.build.binary = binary;
                self.state.build.commit = fingerprint.commit;
                self.state.build.cmake_hash = lmml_build::hash_to_hex(&fingerprint.cmake_hash);
                self.state.build.backend = backend_name(&backend);
                self.state.build.archs = archs;
                self.state.build.sccache_used = sccache_used;
                self.state.build.last_built = unix_timestamp_string();
                self.build_binary = Some(self.state.build.binary.clone());
                self.build_error = None;
                self.status_message = "Build complete".to_string();
                self.save_state_after("Build complete");
            }
            BuildEvent::Failed {
                last_error,
                log_tail,
            } => {
                self.build_running = false;
                for line in log_tail {
                    self.push_build_log(line);
                }
                self.build_error = Some(last_error.clone());
                self.status_message = format!("Build failed: {last_error}");
            }
            BuildEvent::Cancelled => {
                self.build_running = false;
                self.build_error = Some("cancelled".to_string());
                self.state.build.cmake_hash.clear();
                self.state.build.last_built.clear();
                self.push_build_log("Build cancelled");
                self.status_message = "Build cancelled".to_string();
                self.save_state_after("Build cancelled");
            }
            BuildEvent::Skipped { reason } => {
                self.build_running = false;
                self.build_error = None;
                self.build_binary = Some(self.state.build.binary.clone());
                self.push_build_log(format!("Build skipped: {reason}"));
                self.status_message = "Build up to date".to_string();
            }
        }
    }

    /// Build a `lmml-build` config from current app state.
    pub fn build_config(&self, clean: bool) -> lmml_build::BuildConfig {
        let backend = if self.state.build.backend == "Auto" {
            self.detect_profile
                .as_ref()
                .map(SystemProfile::recommended_backend)
                .unwrap_or(BuildBackend::CpuFallback)
        } else {
            backend_from_state(&self.state.build.backend, &self.state.build.archs)
        };
        let mut config = lmml_build::BuildConfig::new(self.state.build.source_dir.clone(), backend);
        config.clean = clean;
        config.sccache = self
            .detect_profile
            .as_ref()
            .and_then(|profile| profile.sccache.clone());
        if self.state.build.track_mode == lmml_state::TrackMode::Tag
            && !self.state.build.commit.is_empty()
        {
            config.git_ref = Some(self.state.build.commit.clone());
        }
        config
    }

    /// Return the model that should be served, preferring the visible selection.
    pub fn selected_server_model(&self) -> Option<ModelEntry> {
        if let Some(model) = self.models.get(self.selected_model) {
            return Some(model.clone());
        }
        let path = &self.state.model.last_used;
        if path.as_os_str().is_empty() {
            return None;
        }
        Some(ModelEntry {
            path: path.clone(),
            name: path
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or("selected model")
                .to_string(),
            size_bytes: path.metadata().map(|metadata| metadata.len()).unwrap_or(0),
            quant: "unknown".to_string(),
            context_length: None,
            architecture: None,
            aliased: false,
        })
    }

    /// Build a compat server config from persisted settings and model fit.
    pub fn server_config(&self, model: &ModelEntry) -> lmml_compat::ServerConfig {
        let mut n_gpu_layers = self.state.server.n_gpu_layers;
        if n_gpu_layers == -1 {
            n_gpu_layers = self
                .detect_profile
                .as_ref()
                .map(|profile| model.recommended_ngl(&profile.gpus))
                .unwrap_or(0);
        }
        lmml_compat::ServerConfig {
            model: model.path.clone(),
            port: self.state.server.port,
            host: self.state.server.host.clone(),
            ctx_size: self.state.server.ctx_size,
            n_gpu_layers,
            batch_size: self.state.server.batch_size,
            ubatch_size: self.state.server.ubatch_size,
            threads: self.state.server.threads,
            flash_attn: self.state.server.flash_attn,
            mlock: self.state.server.mlock,
            api_key: (!self.state.server.api_key.is_empty())
                .then(|| self.state.server.api_key.clone()),
            chat_template: (!self.state.server.chat_template.is_empty())
                .then(|| self.state.server.chat_template.clone()),
            jinja: self.state.server.jinja,
            extra_args: self.state.server.extra_args.clone(),
        }
    }

    fn push_build_log(&mut self, line: impl Into<String>) {
        self.build_log.push(line.into());
        const MAX_BUILD_LOG_LINES: usize = 500;
        if self.build_log.len() > MAX_BUILD_LOG_LINES {
            let overflow = self.build_log.len() - MAX_BUILD_LOG_LINES;
            self.build_log.drain(0..overflow);
        }
    }

    pub(crate) fn save_state_after(&mut self, success_message: &str) {
        let result = if let Some(path) = &self.state_save_path {
            self.state.save_to_path(path)
        } else {
            self.state.save()
        };
        if let Err(error) = result {
            self.status_message = format!("{success_message}; state save failed: {error}");
        }
    }

    fn next_tab(&mut self) {
        let next = (self.active_tab.index() + 1) % Tab::ALL.len();
        self.active_tab = Tab::ALL[next];
    }

    fn previous_tab(&mut self) {
        let current = self.active_tab.index();
        let previous = if current == 0 {
            Tab::ALL.len() - 1
        } else {
            current - 1
        };
        self.active_tab = Tab::ALL[previous];
    }

    fn select_next_model(&mut self) {
        if !self.models.is_empty() {
            self.selected_model = (self.selected_model + 1).min(self.models.len() - 1);
        }
    }

    fn select_previous_model(&mut self) {
        self.selected_model = self.selected_model.saturating_sub(1);
    }

    fn next_model_index(&self) -> Option<usize> {
        if self.models.is_empty() {
            None
        } else {
            Some((self.selected_model + 1).min(self.models.len() - 1))
        }
    }
}

fn backend_from_state(backend: &str, archs: &[String]) -> BuildBackend {
    match backend {
        "Cuda" => BuildBackend::Cuda {
            archs: archs
                .iter()
                .filter_map(|arch| owned_arch_to_static(arch))
                .collect(),
        },
        "Metal" => BuildBackend::Metal,
        "CpuAvx2" => BuildBackend::CpuAvx2,
        "CpuAvx" => BuildBackend::CpuAvx,
        "CpuFallback" => BuildBackend::CpuFallback,
        _ => BuildBackend::CpuFallback,
    }
}

fn backend_name(backend: &BuildBackend) -> String {
    match backend {
        BuildBackend::Cuda { .. } => "Cuda",
        BuildBackend::Metal => "Metal",
        BuildBackend::CpuAvx2 => "CpuAvx2",
        BuildBackend::CpuAvx => "CpuAvx",
        BuildBackend::CpuFallback => "CpuFallback",
    }
    .to_string()
}

fn backend_archs(backend: &BuildBackend) -> Vec<String> {
    match backend {
        BuildBackend::Cuda { archs } => archs.iter().map(|arch| (*arch).to_string()).collect(),
        BuildBackend::Metal
        | BuildBackend::CpuAvx2
        | BuildBackend::CpuAvx
        | BuildBackend::CpuFallback => Vec::new(),
    }
}

fn next_backend(current: BuildBackend) -> BuildBackend {
    match current {
        BuildBackend::Cuda { .. } => BuildBackend::Metal,
        BuildBackend::Metal => BuildBackend::CpuAvx2,
        BuildBackend::CpuAvx2 => BuildBackend::CpuAvx,
        BuildBackend::CpuAvx => BuildBackend::CpuFallback,
        BuildBackend::CpuFallback => BuildBackend::Cuda { archs: Vec::new() },
    }
}

fn parse_onboarding_port(value: &str) -> Result<u16, String> {
    match value.trim().parse::<u16>() {
        Ok(0) => Err("port must be between 1 and 65535".to_string()),
        Ok(port) => Ok(port),
        Err(_error) => Err("port must be between 1 and 65535".to_string()),
    }
}

fn unix_timestamp_string() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

fn owned_arch_to_static(arch: &str) -> Option<&'static str> {
    match arch {
        "sm_37" => Some("sm_37"),
        "sm_50" => Some("sm_50"),
        "sm_52" => Some("sm_52"),
        "sm_53" => Some("sm_53"),
        "sm_60" => Some("sm_60"),
        "sm_61" => Some("sm_61"),
        "sm_62" => Some("sm_62"),
        "sm_70" => Some("sm_70"),
        "sm_72" => Some("sm_72"),
        "sm_75" => Some("sm_75"),
        "sm_80" => Some("sm_80"),
        "sm_86" => Some("sm_86"),
        "sm_87" => Some("sm_87"),
        "sm_89" => Some("sm_89"),
        "sm_90" => Some("sm_90"),
        "sm_90a" => Some("sm_90a"),
        "sm_100" => Some("sm_100"),
        "sm_100a" => Some("sm_100a"),
        _ => None,
    }
}

fn next_hf_field(field: HfSearchField) -> HfSearchField {
    match field {
        HfSearchField::Keywords => HfSearchField::Architecture,
        HfSearchField::Architecture => HfSearchField::Quant,
        HfSearchField::Quant => HfSearchField::Keywords,
    }
}

fn previous_hf_field(field: HfSearchField) -> HfSearchField {
    match field {
        HfSearchField::Keywords => HfSearchField::Quant,
        HfSearchField::Architecture => HfSearchField::Keywords,
        HfSearchField::Quant => HfSearchField::Architecture,
    }
}

fn next_quant_filter(current: Option<QuantTier>) -> Option<QuantTier> {
    match current {
        None => Some(QuantTier::Q4),
        Some(QuantTier::Q4) => Some(QuantTier::Q5),
        Some(QuantTier::Q5) => Some(QuantTier::Q6),
        Some(QuantTier::Q6) => Some(QuantTier::Q8),
        Some(QuantTier::Q8) => Some(QuantTier::F16),
        Some(QuantTier::F16) => Some(QuantTier::F32),
        Some(QuantTier::F32) => None,
    }
}

fn quant_from_char(value: char) -> Option<QuantTier> {
    match value.to_ascii_lowercase() {
        '4' => Some(QuantTier::Q4),
        '5' => Some(QuantTier::Q5),
        '6' => Some(QuantTier::Q6),
        '8' => Some(QuantTier::Q8),
        'h' => Some(QuantTier::F16),
        'f' => Some(QuantTier::F32),
        _ => None,
    }
}

fn non_empty_filter(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

impl Default for App {
    fn default() -> Self {
        Self::new_with_state(PersistentState::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEvent;

    #[test]
    fn number_keys_select_tabs() {
        let mut app = App::default();
        app.handle_event(AppEvent::Key(KeyEvent::from(KeyCode::Char('4'))));
        assert_eq!(app.active_tab, Tab::Server);
    }

    #[test]
    fn tab_cycles_forward_and_backward() {
        let mut app = App::default();
        app.handle_event(AppEvent::Key(KeyEvent::from(KeyCode::Tab)));
        assert_eq!(app.active_tab, Tab::Build);
        app.handle_event(AppEvent::Key(KeyEvent::from(KeyCode::BackTab)));
        assert_eq!(app.active_tab, Tab::Detect);
    }

    #[test]
    fn help_and_quit_are_actions() {
        let mut app = App::default();
        let help = app.handle_event(AppEvent::Key(KeyEvent::from(KeyCode::Char('?'))));
        assert_eq!(help, Some(Action::ShowHelp));
        app.dispatch(help.expect("help action"));
        assert!(app.show_help);

        let quit = app.handle_event(AppEvent::Key(KeyEvent::from(KeyCode::Char('q'))));
        assert_eq!(quit, Some(Action::Quit));
        app.dispatch(quit.expect("quit action"));
        assert!(app.should_quit);
    }

    #[test]
    fn build_events_update_status_and_cap_log() {
        let mut app = App::default();
        for index in 0..505 {
            app.handle_event(AppEvent::BuildEvent(BuildEvent::Compiling {
                line: format!("line {index}"),
            }));
        }
        assert_eq!(app.build_log.len(), 500);
        assert!(app.build_running);

        app.handle_event(AppEvent::BuildEvent(BuildEvent::Failed {
            last_error: "cmake failed".to_string(),
            log_tail: vec!["tail".to_string()],
        }));
        assert!(!app.build_running);
        assert_eq!(app.build_error, Some("cmake failed".to_string()));

        let tempdir = tempfile::tempdir().expect("tempdir");
        app.state_save_path = Some(tempdir.path().join("state.toml"));
        app.state.build.cmake_hash = "stale".to_string();
        app.handle_event(AppEvent::BuildEvent(BuildEvent::Cancelled));
        assert!(!app.build_running);
        assert_eq!(app.build_error, Some("cancelled".to_string()));
        assert!(app.state.build.cmake_hash.is_empty());
    }

    #[test]
    fn build_config_uses_persisted_backend_and_clean_flag() {
        let mut app = App::default();
        app.state.build.backend = "Cuda".to_string();
        app.state.build.archs = vec!["sm_86".to_string()];
        let config = app.build_config(true);
        assert!(config.clean);
        assert_eq!(
            config.backend,
            BuildBackend::Cuda {
                archs: vec!["sm_86"]
            }
        );
    }

    #[test]
    fn build_config_auto_uses_detection_but_explicit_backend_wins() {
        let mut app = App::default();
        app.detect_profile = Some(cuda_profile());

        app.state.build.backend = "Auto".to_string();
        let config = app.build_config(false);
        assert_eq!(
            config.backend,
            BuildBackend::Cuda {
                archs: vec!["sm_86"]
            }
        );

        app.state.build.backend = "CpuFallback".to_string();
        let config = app.build_config(false);
        assert_eq!(config.backend, BuildBackend::CpuFallback);
    }

    #[test]
    fn first_run_enter_starts_build_and_esc_dismisses() {
        let mut app = App::new_with_state_and_first_run(PersistentState::default(), true);
        let action = app.handle_event(AppEvent::Key(KeyEvent::from(KeyCode::Enter)));
        assert_eq!(action, Some(Action::RunDetect));
        app.handle_event(AppEvent::DetectComplete(Box::new(SystemProfile {
            compiler: None,
            cmake: None,
            git: None,
            cuda: lmml_detect::CudaCompatibility::NoGpu,
            gpus: Vec::new(),
            sccache: None,
            metal: lmml_detect::MetalSupport {
                available: false,
                displays: Vec::new(),
            },
            cpu: lmml_detect::CpuFeatures {
                model: String::new(),
                cores: 1,
                threads: 1,
                avx: false,
                avx2: false,
                avx512: false,
                neon: false,
                features: Vec::new(),
            },
            memory: lmml_detect::MemInfo {
                total_mb: 1024,
                available_mb: 512,
            },
            disk: lmml_detect::DiskInfo {
                available_bytes: 8 * 1024 * 1024 * 1024,
                path: PathBuf::from("."),
            },
        })));
        assert_eq!(app.onboarding_step, OnboardingStep::HardwareSummary);
        assert!(app.first_run_onboarding);

        let mut app = App::new_with_state_and_first_run(PersistentState::default(), true);
        assert_eq!(
            app.handle_event(AppEvent::Key(KeyEvent::from(KeyCode::Esc))),
            None
        );
        assert!(!app.first_run_onboarding);
    }

    #[test]
    fn model_scan_and_selection_update_state() {
        let mut app = App::default();
        app.handle_event(AppEvent::ModelScanComplete(vec![
            model_entry("a.gguf"),
            model_entry("b.gguf"),
        ]));
        assert_eq!(app.models.len(), 2);
        app.active_tab = Tab::Models;
        app.handle_event(AppEvent::Key(KeyEvent::from(KeyCode::Down)));
        assert_eq!(app.selected_model, 1);
        let action = app.handle_event(AppEvent::Key(KeyEvent::from(KeyCode::Enter)));
        assert_eq!(action, Some(Action::SelectModel(PathBuf::from("b.gguf"))));
    }

    #[test]
    fn hf_results_and_download_progress_update_state() {
        let mut app = App::default();
        app.handle_event(AppEvent::HfSearchResults(vec![HfModelResult {
            repo_id: "org/model".to_string(),
            filename: "model-Q4_K_M.gguf".to_string(),
            size_bytes: 10,
            downloads: 5,
            url: "https://example.test/model-Q4_K_M.gguf".to_string(),
        }]));
        assert_eq!(app.hf_results.len(), 1);
        app.active_tab = Tab::Models;
        let action = app.handle_event(AppEvent::Key(KeyEvent::from(KeyCode::Char('D'))));
        assert!(matches!(action, Some(Action::DownloadModel(_))));

        app.handle_event(AppEvent::DownloadProgress(DownloadProgress {
            bytes_received: 5,
            total_bytes: Some(10),
            resumed_from: 2,
        }));
        assert_eq!(
            app.download_progress
                .as_ref()
                .map(|progress| progress.resumed_from),
            Some(2)
        );
    }

    #[test]
    fn alias_modal_collects_path_and_delete_requires_confirmation() {
        let mut app = App::default();
        app.active_tab = Tab::Models;

        let action = app.handle_event(AppEvent::Key(KeyEvent::from(KeyCode::Char('a'))));
        assert_eq!(action, Some(Action::AddModelAlias));
        app.dispatch(action.expect("alias action"));
        assert!(matches!(app.active_modal, Some(Modal::AddAlias { .. })));

        for value in "/tmp/model.gguf".chars() {
            app.handle_event(AppEvent::Key(KeyEvent::from(KeyCode::Char(value))));
        }
        let action = app.handle_event(AppEvent::Key(KeyEvent::from(KeyCode::Enter)));
        assert_eq!(
            action,
            Some(Action::ConfirmAddModelAlias(PathBuf::from(
                "/tmp/model.gguf"
            )))
        );

        app.models = vec![model_entry("delete-me.gguf")];
        let action = app.handle_event(AppEvent::Key(KeyEvent::from(KeyCode::Char('x'))));
        assert!(matches!(action, Some(Action::DeleteModel(_))));
        app.dispatch(action.expect("delete action"));
        assert!(matches!(
            app.active_modal,
            Some(Modal::ConfirmDelete { .. })
        ));
        let action = app.handle_event(AppEvent::Key(KeyEvent::from(KeyCode::Char('y'))));
        assert!(matches!(action, Some(Action::ConfirmDeleteModel(_))));
    }

    #[test]
    fn hf_search_modal_builds_filtered_query() {
        let mut app = App::default();
        app.active_tab = Tab::Models;
        let action = app.handle_event(AppEvent::Key(KeyEvent::from(KeyCode::Char('/'))));
        assert_eq!(action, Some(Action::OpenHfSearch));
        app.dispatch(action.expect("open hf search"));

        for _ in 0.."gguf".len() {
            app.handle_event(AppEvent::Key(KeyEvent::from(KeyCode::Backspace)));
        }
        for value in "mistral".chars() {
            app.handle_event(AppEvent::Key(KeyEvent::from(KeyCode::Char(value))));
        }
        app.handle_event(AppEvent::Key(KeyEvent::from(KeyCode::Tab)));
        for value in "mistral".chars() {
            app.handle_event(AppEvent::Key(KeyEvent::from(KeyCode::Char(value))));
        }
        app.handle_event(AppEvent::Key(KeyEvent::from(KeyCode::Tab)));
        app.handle_event(AppEvent::Key(KeyEvent::from(KeyCode::Char('4'))));

        let action = app.handle_event(AppEvent::Key(KeyEvent::from(KeyCode::Enter)));
        assert_eq!(
            action,
            Some(Action::SearchHf(HfSearchQuery {
                keywords: "mistral".to_string(),
                architecture: Some("mistral".to_string()),
                quant_filter: Some(QuantTier::Q4),
                max_results: 20,
            }))
        );
    }

    #[test]
    fn server_config_auto_ngl_uses_detected_vram_fit() {
        let mut app = App::default();
        app.detect_profile = Some(SystemProfile {
            compiler: None,
            cmake: None,
            git: None,
            cuda: lmml_detect::CudaCompatibility::NoGpu,
            gpus: vec![lmml_detect::GpuInfo {
                name: "RTX".to_string(),
                memory_total_mb: 8_192,
                compute_cap: "8.6".to_string(),
                arch: Some("sm_86"),
            }],
            sccache: None,
            metal: lmml_detect::MetalSupport {
                available: false,
                displays: Vec::new(),
            },
            cpu: lmml_detect::CpuFeatures {
                model: String::new(),
                cores: 4,
                threads: 8,
                avx: false,
                avx2: false,
                avx512: false,
                neon: false,
                features: Vec::new(),
            },
            memory: lmml_detect::MemInfo {
                total_mb: 16,
                available_mb: 8,
            },
            disk: lmml_detect::DiskInfo {
                available_bytes: 8,
                path: PathBuf::from("."),
            },
        });
        let model = ModelEntry {
            size_bytes: 1024 * 1024 * 1024,
            ..model_entry("small.gguf")
        };

        assert_eq!(app.server_config(&model).n_gpu_layers, -1);
    }

    #[test]
    fn server_key_toggles_start_and_stop() {
        let mut app = App::default();
        app.active_tab = Tab::Server;
        assert_eq!(
            app.handle_event(AppEvent::Key(KeyEvent::from(KeyCode::Char('s')))),
            Some(Action::StartServer)
        );
        app.server_status = ServerStatus::Ready {
            url: "http://127.0.0.1:8080".to_string(),
        };
        assert_eq!(
            app.handle_event(AppEvent::Key(KeyEvent::from(KeyCode::Char('s')))),
            Some(Action::StopServer)
        );
    }

    #[test]
    fn server_model_swap_updates_selection_when_stopped() {
        let mut app = App::default();
        let tempdir = tempfile::tempdir().expect("tempdir");
        app.state_save_path = Some(tempdir.path().join("state.toml"));
        app.active_tab = Tab::Server;
        app.models = vec![model_entry("a.gguf"), model_entry("b.gguf")];
        app.selected_model = 0;

        let action = app.handle_event(AppEvent::Key(KeyEvent::from(KeyCode::Char('m'))));

        assert_eq!(action, None);
        assert_eq!(app.selected_model, 1);
        assert_eq!(app.state.model.last_used, PathBuf::from("b.gguf"));
        assert!(app.active_modal.is_none());
    }

    #[test]
    fn server_model_swap_running_cancel_keeps_selection() {
        let mut app = App::default();
        app.active_tab = Tab::Server;
        app.models = vec![model_entry("a.gguf"), model_entry("b.gguf")];
        app.selected_model = 0;
        app.state.model.last_used = PathBuf::from("a.gguf");
        app.server_status = ServerStatus::Ready {
            url: "http://127.0.0.1:8080".to_string(),
        };

        let action = app.handle_event(AppEvent::Key(KeyEvent::from(KeyCode::Char('m'))));

        assert_eq!(action, None);
        assert_eq!(app.selected_model, 0);
        assert_eq!(app.state.model.last_used, PathBuf::from("a.gguf"));
        assert!(matches!(
            app.active_modal,
            Some(Modal::ConfirmModelSwap { .. })
        ));

        let action = app.handle_event(AppEvent::Key(KeyEvent::from(KeyCode::Esc)));
        assert_eq!(action, None);
        assert!(app.active_modal.is_none());
        assert_eq!(app.selected_model, 0);
        assert_eq!(app.state.model.last_used, PathBuf::from("a.gguf"));
    }

    #[test]
    fn server_model_swap_running_confirm_emits_restart_action() {
        let mut app = App::default();
        app.active_tab = Tab::Server;
        app.models = vec![model_entry("a.gguf"), model_entry("b.gguf")];
        app.selected_model = 0;
        app.state.model.last_used = PathBuf::from("a.gguf");
        app.server_status = ServerStatus::Ready {
            url: "http://127.0.0.1:8080".to_string(),
        };

        app.handle_event(AppEvent::Key(KeyEvent::from(KeyCode::Char('m'))));
        let action = app.handle_event(AppEvent::Key(KeyEvent::from(KeyCode::Char('y'))));

        assert_eq!(
            action,
            Some(Action::ConfirmModelSwap(model_entry("b.gguf")))
        );
        assert_eq!(app.state.model.last_used, PathBuf::from("a.gguf"));
    }

    #[test]
    fn settings_modal_edits_and_toggles_server_config() {
        let mut app = App::default();
        app.active_tab = Tab::Settings;
        app.selected_settings_field = SettingsField::Port;

        assert_eq!(
            app.handle_event(AppEvent::Key(KeyEvent::from(KeyCode::Char('e')))),
            None
        );
        assert_eq!(app.settings_edit_buffer.as_deref(), Some("8080"));
        for key in [
            KeyCode::Backspace,
            KeyCode::Backspace,
            KeyCode::Backspace,
            KeyCode::Backspace,
            KeyCode::Char('1'),
            KeyCode::Char('2'),
            KeyCode::Char('0'),
            KeyCode::Char('0'),
        ] {
            app.handle_event(AppEvent::Key(KeyEvent::from(key)));
        }
        app.handle_event(AppEvent::Key(KeyEvent::from(KeyCode::Enter)));
        assert_eq!(app.state.server.port, 1200);
        assert_eq!(app.active_tab, Tab::Settings);
        assert!(app.settings_edit_buffer.is_none());

        app.selected_settings_field = SettingsField::FlashAttn;
        app.handle_event(AppEvent::Key(KeyEvent::from(KeyCode::Char(' '))));
        assert!(!app.state.server.flash_attn);
    }

    #[test]
    fn settings_probe_key_and_unsupported_warnings_work() {
        let mut app = App::default();
        app.active_tab = Tab::Settings;
        assert_eq!(
            app.handle_event(AppEvent::Key(KeyEvent::from(KeyCode::Char('p')))),
            Some(Action::ProbeServerCapabilities)
        );

        app.state.server.flash_attn = true;
        app.state.server.api_key = "secret".to_string();
        app.handle_event(AppEvent::ServerCapabilities(Ok(LlamaBinaryCapabilities {
            version: Some("test".to_string()),
            flash_attn: false,
            mlock: true,
            api_key: false,
            ubatch_size: true,
            chat_template: true,
            jinja: true,
            reranking: false,
            flags: vec!["--model".to_string(), "--port".to_string()],
        })));
        assert_eq!(
            app.server_compat_warnings(),
            vec![
                "--flash-attn not available in this llama-server build".to_string(),
                "--api-key not available in this llama-server build".to_string(),
            ]
        );
    }

    #[test]
    fn significant_events_persist_and_reload_state() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let state_path = tempdir.path().join("state.toml");
        let mut app = App::default();
        app.state_save_path = Some(state_path.clone());
        app.active_tab = Tab::Models;

        let model_path = tempdir.path().join("model.gguf");
        app.handle_event(AppEvent::DownloadComplete(Ok(ModelEntry {
            path: model_path.clone(),
            name: "model".to_string(),
            size_bytes: 4,
            quant: "Q4".to_string(),
            context_length: None,
            architecture: None,
            aliased: false,
        })));
        app.active_tab = Tab::Settings;
        app.selected_settings_field = SettingsField::Port;
        app.settings_edit_buffer = Some("9091".to_string());
        app.handle_event(AppEvent::Key(KeyEvent::from(KeyCode::Enter)));
        app.dispatch(Action::Quit);

        let reloaded =
            PersistentState::load_from_path(&state_path).expect("reload persisted state");
        assert_eq!(reloaded.model.last_used, model_path);
        assert_eq!(reloaded.server.port, 9091);
    }

    fn model_entry(path: &str) -> ModelEntry {
        ModelEntry {
            path: PathBuf::from(path),
            name: path.to_string(),
            size_bytes: 1024,
            quant: "Q4_K_M".to_string(),
            context_length: Some(4096),
            architecture: Some("llama".to_string()),
            aliased: false,
        }
    }

    fn cuda_profile() -> SystemProfile {
        SystemProfile {
            compiler: None,
            cmake: None,
            git: None,
            cuda: lmml_detect::CudaCompatibility::Compatible {
                archs: vec!["sm_86"],
            },
            gpus: vec![lmml_detect::GpuInfo {
                name: "RTX".to_string(),
                memory_total_mb: 8_192,
                compute_cap: "8.6".to_string(),
                arch: Some("sm_86"),
            }],
            sccache: None,
            metal: lmml_detect::MetalSupport {
                available: false,
                displays: Vec::new(),
            },
            cpu: lmml_detect::CpuFeatures {
                model: String::new(),
                cores: 4,
                threads: 8,
                avx: false,
                avx2: false,
                avx512: false,
                neon: false,
                features: Vec::new(),
            },
            memory: lmml_detect::MemInfo {
                total_mb: 16,
                available_mb: 8,
            },
            disk: lmml_detect::DiskInfo {
                available_bytes: 8,
                path: PathBuf::from("."),
            },
        }
    }
}
