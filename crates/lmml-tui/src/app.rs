//! Application state and action dispatch for the TUI.
//!
//! Rendering lives in `tabs` and widgets. This module owns navigation, modal
//! state, background-task status, and persistent state coordination.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use lmml_build::{BuildEvent, UpdateCheck};
use lmml_detect::SystemProfile;
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

/// Download progress placeholder until `lmml-models` lands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadProgress {
    /// Bytes received.
    pub bytes_received: u64,
    /// Total bytes, if known.
    pub total_bytes: Option<u64>,
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
    /// Model scan completed.
    ModelScanComplete(Vec<crate::action::ModelEntry>),
    /// Hugging Face search completed.
    HfSearchResults(Vec<crate::action::HfModelResult>),
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
    /// Current server status.
    pub server_status: ServerStatus,
    /// Detect tab log lines.
    pub detect_log: Vec<String>,
    /// Build tab log lines.
    pub build_log: Vec<String>,
    /// Server tab log lines.
    pub server_log: Vec<String>,
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
        let state = PersistentState::load().unwrap_or_default();
        Self::new_with_state(state)
    }

    /// Construct an app with injected state for tests.
    pub fn new_with_state(state: PersistentState) -> Self {
        Self {
            state,
            active_tab: Tab::Detect,
            should_quit: false,
            show_help: false,
            server_status: ServerStatus::Stopped,
            detect_log: Vec::new(),
            build_log: Vec::new(),
            server_log: Vec::new(),
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
                self.status_message = match progress.total_bytes {
                    Some(total) => {
                        format!("Downloading {} / {} bytes", progress.bytes_received, total)
                    }
                    None => format!("Downloading {} bytes", progress.bytes_received),
                };
                None
            }
            AppEvent::ModelScanComplete(models) => {
                self.status_message = format!("{} model(s) found", models.len());
                None
            }
            AppEvent::HfSearchResults(results) => {
                self.status_message = format!("{} Hugging Face result(s)", results.len());
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
                self.build_log.push("Starting build".to_string());
                self.status_message = "Build requested".to_string();
            }
            Action::CancelBuild => {
                self.build_log
                    .push("Build cancellation requested".to_string());
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
            Action::OpenHfSearch => {
                self.status_message = "HF search opened".to_string();
            }
            Action::SearchHf(query) => {
                self.status_message = format!("Searching: {}", query.keywords);
            }
            Action::DownloadModel(result) => {
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
            KeyCode::Char('d') => Some(Action::RunDetect),
            KeyCode::Char('b') => Some(Action::StartBuild),
            KeyCode::Char('u') => Some(Action::CheckForUpdate),
            KeyCode::Char('s') => match self.active_tab {
                Tab::Server => Some(Action::StartServer),
                Tab::Settings => Some(Action::SaveSettings),
                Tab::Detect | Tab::Build | Tab::Models => None,
            },
            KeyCode::Char('/') => Some(Action::OpenHfSearch),
            KeyCode::Char('a') => Some(Action::AddModelAlias),
            _ => None,
        }
    }

    fn handle_build_event(&mut self, event: BuildEvent) {
        match event {
            BuildEvent::Cloning { url } => {
                self.build_log.push(format!("Cloning {url}"));
            }
            BuildEvent::CmakeConfiguring => {
                self.build_log.push("Configuring CMake".to_string());
            }
            BuildEvent::Compiling { line } => {
                self.build_log.push(line);
            }
            BuildEvent::Linking => {
                self.build_log.push("Linking".to_string());
            }
            BuildEvent::Completed { binary, .. } => {
                self.state.build.binary = binary;
                self.status_message = "Build complete".to_string();
            }
            BuildEvent::Failed {
                last_error,
                log_tail,
            } => {
                self.build_log.extend(log_tail);
                self.status_message = format!("Build failed: {last_error}");
            }
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
}
