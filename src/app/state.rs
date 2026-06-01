//! Runtime session state.
//!
//! [`AppState`] holds everything the TUI renders: current screen,
//! server status, build progress, model list, and hardware probe results.

use crate::app::config::Config;
use crate::app::Screen;
use crate::probe;
use crate::server;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModelsSort {
    #[default]
    Name,
    Size,
}

/// Current download progress.
#[derive(Debug, Default, Clone)]
pub struct DownloadState {
    pub bytes: u64,
    pub total: u64,
    pub speed: f64,
    pub eta_secs: f64,
}

/// Current GPU memory usage reported by vendor tools.
#[derive(Debug, Default, Clone, Copy)]
pub struct VramUsage {
    pub used_mb: u64,
    pub total_mb: u64,
}

/// Single model record displayed in the TUI.
#[derive(Debug, Clone)]
pub struct ModelEntry {
    pub name: String,
    pub path: String,
    pub size_bytes: u64,
    pub quantization: String,
    pub param_count: String,
    pub model_type: String,
    pub is_loaded: bool,
    pub is_favorite: bool,
    pub last_used: String,
}

/// Runtime state for the application.
#[derive(Debug)]
pub struct AppState {
    /// The on-disk config (loaded at startup, mutated via settings screen).
    pub config: Config,
    pub current_screen: Screen,
    pub modal_active: bool,
    pub modal_message: String,
    pub modal_input: String,
    pub toast_message: String,
    pub toast_ticks: u16,
    pub settings_selected: usize,
    pub settings_edit_field: Option<usize>,
    pub settings_edit_buffer: String,
    pub server_selected_field: usize,
    pub server_edit_field: Option<usize>,
    pub server_edit_buffer: String,
    pub server_restart_pending: bool,
    pub models_search_active: bool,
    pub models_sort_by: ModelsSort,

    // --- Build ---
    pub build_state: BuildState,

    // --- Models ---
    pub models: Vec<ModelEntry>,
    pub selected_model: usize,
    pub search_query: String,
    pub download_state: DownloadState,
    pub download_complete: Option<Result<(), String>>,

    // --- Server ---
    pub server_state: server::ServerState,

    // --- Probe ---
    pub probe_state: ProbeState,
    pub vram_usage: Option<VramUsage>,
}

/// Build progress state.
#[derive(Debug)]
pub struct BuildState {
    pub log_lines: Vec<String>,
    pub progress: (u32, u32),
    pub complete: Option<Result<(), String>>,
    pub is_running: bool,
    pub cmake_flags: Vec<String>,
    pub ngl: u32,
    pub commit_hash: String,
}

/// Hardware probe progress state.
#[derive(Debug)]
pub struct ProbeState {
    pub log_lines: Vec<String>,
    pub result: Option<probe::ProbeResult>,
}

impl Default for AppState {
    fn default() -> Self {
        let config = Config::load_or_default();
        AppState {
            config,
            current_screen: Screen::Dashboard,
            modal_active: false,
            modal_message: String::new(),
            modal_input: String::new(),
            toast_message: String::new(),
            toast_ticks: 0,
            settings_selected: 0,
            settings_edit_field: None,
            settings_edit_buffer: String::new(),
            server_selected_field: 0,
            server_edit_field: None,
            server_edit_buffer: String::new(),
            server_restart_pending: false,
            models_search_active: false,
            models_sort_by: ModelsSort::Name,
            build_state: BuildState {
                log_lines: Vec::new(),
                progress: (0, 0),
                complete: None,
                is_running: false,
                cmake_flags: Vec::new(),
                ngl: 0,
                commit_hash: String::new(),
            },
            models: Vec::new(),
            selected_model: 0,
            search_query: String::new(),
            download_state: DownloadState::default(),
            download_complete: None,
            server_state: server::ServerState::default(),
            probe_state: ProbeState {
                log_lines: Vec::new(),
                result: None,
            },
            vram_usage: None,
        }
    }
}
