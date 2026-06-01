//! Application state and action dispatch for the TUI.
//!
//! Rendering lives in `tabs` and widgets. This module owns navigation, modal
//! state, background-task status, and persistent state coordination.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::PathBuf;

use lmml_build::{BuildEvent, UpdateCheck};
use lmml_detect::{BuildBackend, SystemProfile};
use lmml_models::{DownloadProgress, HfModelResult, ModelEntry};
use lmml_state::AppState as PersistentState;

use crate::action::Action;

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

/// Runtime server status for the Milestone 5 shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerStatus {
    /// No managed server is running.
    Stopped,
    /// Start has been requested.
    Starting,
    /// Server is ready at a URL.
    Ready { url: String },
    /// Server failed.
    Failed { reason: String },
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
    /// Server log line.
    ServerLog(String),
    /// Download progress changed.
    DownloadProgress(DownloadProgress),
    /// Download completed.
    DownloadComplete(Result<ModelEntry, String>),
    /// Model scan completed.
    ModelScanComplete(Vec<ModelEntry>),
    /// Hugging Face search completed.
    HfSearchResults(Vec<HfModelResult>),
    /// Update check completed.
    UpdateCheckResult(UpdateCheck),
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
    /// Full detection profile for the current session.
    pub detect_profile: Option<SystemProfile>,
    /// Current server status.
    pub server_status: ServerStatus,
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
        Self {
            state,
            active_tab: Tab::Detect,
            should_quit: false,
            show_help: false,
            first_run_onboarding,
            detect_profile: None,
            server_status: ServerStatus::Stopped,
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
                self.first_run_onboarding = false;
                self.status_message = "Detection complete".to_string();
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
            AppEvent::ServerLog(line) => {
                self.server_log.push(line);
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
                        self.models.push(model);
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
                self.server_status = ServerStatus::Starting;
                self.status_message = "Starting server".to_string();
            }
            Action::StopServer => {
                self.server_status = ServerStatus::Stopped;
                self.status_message = "Server stopped".to_string();
            }
            Action::SelectModel(path) => {
                self.state.model.last_used = path;
                self.status_message = "Model selected".to_string();
            }
            Action::ScanModels => {
                self.status_message = "Scanning models".to_string();
            }
            Action::OpenHfSearch => {
                self.hf_search_open = true;
                self.status_message = "HF search opened".to_string();
            }
            Action::SearchHf(query) => {
                self.hf_search_open = true;
                self.hf_query = query.keywords.clone();
                self.status_message = format!("Searching: {}", query.keywords);
            }
            Action::DownloadModel(result) => {
                self.download_progress = None;
                self.download_error = None;
                self.status_message = format!("Downloading {}", result.filename);
            }
            Action::DeleteModel(model) => {
                self.status_message = format!("Delete requested for {}", model.name);
            }
            Action::AddModelAlias => {
                self.status_message = "Add alias requested".to_string();
            }
            Action::CheckForUpdate => {
                self.status_message = "Checking for updates".to_string();
            }
            Action::UpdateAndRebuild => {
                self.status_message = "Update and rebuild requested".to_string();
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
                self.should_quit = true;
            }
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> Option<Action> {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Some(Action::Quit);
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
                Tab::Server => Some(Action::StartServer),
                Tab::Settings => Some(Action::SaveSettings),
                Tab::Detect | Tab::Build | Tab::Models => None,
            },
            KeyCode::Char('/') => Some(Action::SearchHf(lmml_models::HfSearchQuery {
                keywords: self.hf_query.clone(),
                architecture: None,
                quant_filter: None,
                max_results: 20,
            })),
            KeyCode::Char('D') if self.active_tab == Tab::Models => self
                .hf_results
                .get(self.selected_hf_result)
                .cloned()
                .map(Action::DownloadModel),
            KeyCode::Char('a') => Some(Action::AddModelAlias),
            KeyCode::Char('r') if self.active_tab == Tab::Models => Some(Action::ScanModels),
            _ => None,
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
            BuildEvent::Completed { binary, .. } => {
                self.build_running = false;
                self.state.build.binary = binary;
                self.build_binary = Some(self.state.build.binary.clone());
                self.build_error = None;
                self.status_message = "Build complete".to_string();
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
        }
    }

    /// Build a `lmml-build` config from current app state.
    pub fn build_config(&self, clean: bool) -> lmml_build::BuildConfig {
        let backend = self
            .detect_profile
            .as_ref()
            .map(SystemProfile::recommended_backend)
            .unwrap_or_else(|| {
                backend_from_state(&self.state.build.backend, &self.state.build.archs)
            });
        let mut config = lmml_build::BuildConfig::new(self.state.build.source_dir.clone(), backend);
        config.clean = clean;
        config.sccache = self
            .detect_profile
            .as_ref()
            .and_then(|profile| profile.sccache.clone());
        config
    }

    fn push_build_log(&mut self, line: impl Into<String>) {
        self.build_log.push(line.into());
        const MAX_BUILD_LOG_LINES: usize = 500;
        if self.build_log.len() > MAX_BUILD_LOG_LINES {
            let overflow = self.build_log.len() - MAX_BUILD_LOG_LINES;
            self.build_log.drain(0..overflow);
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
    fn first_run_enter_starts_build_and_esc_dismisses() {
        let mut app = App::new_with_state_and_first_run(PersistentState::default(), true);
        let action = app.handle_event(AppEvent::Key(KeyEvent::from(KeyCode::Enter)));
        assert_eq!(action, Some(Action::StartBuild));
        app.dispatch(action.expect("start build action"));
        assert!(!app.first_run_onboarding);

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
}
