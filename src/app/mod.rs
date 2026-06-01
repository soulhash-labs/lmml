//! Core application state and event loop.
//!
//! Defines the [`App`] struct, the [`Screen`] and [`Message`] enums,
//! and the [`update`] dispatcher. All cross-module coordination flows
//! through this module.

pub mod config;
pub mod errors;
pub mod state;

use crate::probe;
use crate::tui;
use color_eyre::Result;
use state::AppState;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::mpsc;

/// The main application struct. Owned by the TUI event loop.
pub struct App {
    /// Runtime state shared with render()
    pub state: AppState,
    /// Set to true to signal the event loop to exit.
    pub quit: bool,

    // --- Background task channels ---
    pub build_rx: mpsc::Receiver<crate::build::BuildEvent>,
    pub download_rx: mpsc::Receiver<crate::models::DownloadEvent>,
    pub server_rx: mpsc::Receiver<crate::server::ServerEvent>,
    pub probe_rx: mpsc::Receiver<crate::probe::ProbeEvent>,

    // --- Transmit ends (cloned to spawned tasks) ---
    pub build_tx: mpsc::Sender<crate::build::BuildEvent>,
    pub download_tx: mpsc::Sender<crate::models::DownloadEvent>,
    pub server_tx: mpsc::Sender<crate::server::ServerEvent>,
    pub probe_tx: mpsc::Sender<crate::probe::ProbeEvent>,

    // --- Server process kill switch ---
    pub server_child: Arc<tokio::sync::Mutex<Option<tokio::process::Child>>>,
    // --- Build cancel flag ---
    pub build_cancel: Arc<std::sync::atomic::AtomicBool>,
    last_vram_poll: Instant,
    last_config_check: SystemTime,
}

/// Top-level screens in the application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Dashboard,
    Models,
    Server,
    Build,
    Settings,
}

/// Messages that can be dispatched to the update loop.
pub enum Message {
    /// Navigate to a screen.
    Navigate(Screen),
    /// A key event was pressed on the current screen.
    KeyEvent(crossterm::event::KeyEvent),
    /// A tick timer fired (for periodic updates).
    Tick,
    /// Quit the application.
    Quit,
}

impl App {
    /// Create a new App with default state and channel setup.
    /// Optionally apply persistent config values to initial state.
    pub fn new() -> (Self, mpsc::Receiver<Message>) {
        let (_msg_tx, msg_rx) = mpsc::channel::<Message>(256);

        let (build_tx, build_rx) = mpsc::channel(256);
        let (download_tx, download_rx) = mpsc::channel(256);
        let (server_tx, server_rx) = mpsc::channel(256);
        let (probe_tx, probe_rx) = mpsc::channel(256);

        let app = App {
            state: AppState::default(),
            quit: false,
            build_rx,
            download_rx,
            server_rx,
            probe_rx,
            build_tx,
            download_tx,
            server_tx,
            probe_tx,
            server_child: Arc::new(tokio::sync::Mutex::new(None)),
            build_cancel: Arc::new(AtomicBool::new(false)),
            last_vram_poll: Instant::now() - Duration::from_secs(5),
            last_config_check: SystemTime::now(),
        };

        (app, msg_rx)
    }

    /// Main update loop — called once per event loop iteration.
    pub fn update(&mut self, msg: Message) -> Result<Option<Screen>> {
        match msg {
            Message::Navigate(screen) => {
                self.state.current_screen = screen;
                Ok(Some(screen))
            }
            Message::KeyEvent(key) => {
                self.dispatch_key(key)?;
                Ok(None)
            }
            Message::Tick => {
                self.drain_channels();
                self.poll_vram_usage();
                self.poll_config_reload();
                self.tick_toast();
                Ok(None)
            }
            Message::Quit => Ok(None),
        }
    }

    /// Drain all background channels into state.
    fn drain_channels(&mut self) {
        // Build events
        while let Ok(event) = self.build_rx.try_recv() {
            match event {
                crate::build::BuildEvent::Line(line) => {
                    self.state.build_state.log_lines.push(line);
                }
                crate::build::BuildEvent::Progress { current, total } => {
                    self.state.build_state.progress = (current, total);
                }
                crate::build::BuildEvent::Complete(result) => {
                    self.set_toast(if result.is_ok() {
                        "Build complete".to_string()
                    } else {
                        "Build failed".to_string()
                    });
                    self.state.build_state.complete = Some(result);
                    self.state.build_state.is_running = false;
                    let build_state = if self
                        .state
                        .build_state
                        .complete
                        .as_ref()
                        .is_some_and(|result| result.is_ok())
                    {
                        "complete"
                    } else {
                        "failed"
                    };
                    let _ = config::save_state(&config::AppStateToml {
                        last_session: config::LastSession {
                            last_model: self
                                .state
                                .models
                                .get(self.state.selected_model)
                                .map(|m| m.path.clone())
                                .unwrap_or_default(),
                            server_was_running: !matches!(
                                self.state.server_state.status,
                                crate::server::ServerStatus::Stopped
                                    | crate::server::ServerStatus::Error(_)
                            ),
                            build_state: build_state.to_string(),
                            build_commit: self.state.build_state.commit_hash.clone(),
                        },
                    });
                }
                crate::build::BuildEvent::CommitHash(hash) => {
                    self.state.build_state.commit_hash = hash;
                }
            }
        }

        // Download events
        while let Ok(event) = self.download_rx.try_recv() {
            match event {
                crate::models::DownloadEvent::Progress {
                    bytes,
                    total,
                    speed,
                    eta_secs,
                } => {
                    self.state.download_state = crate::app::state::DownloadState {
                        bytes,
                        total,
                        speed,
                        eta_secs,
                    };
                }
                crate::models::DownloadEvent::Complete(result) => {
                    match &result {
                        Ok(()) => {
                            self.set_toast("Download complete".to_string());
                            self.state.modal_message =
                                "Download complete. The model list will refresh on next scan."
                                    .to_string();
                        }
                        Err(e) => {
                            self.set_toast("Download interrupted".to_string());
                            self.state.modal_message = format!(
                                "Download interrupted. Retry the same model to resume from the .part file.\n{e}"
                            );
                        }
                    }
                    self.state.modal_active = true;
                    self.state.download_complete = Some(result);
                }
            }
        }

        // Server events
        while let Ok(event) = self.server_rx.try_recv() {
            match event {
                crate::server::ServerEvent::LogLine(line) => {
                    self.state.server_state.log_lines.push(line);
                }
                crate::server::ServerEvent::StatusChange(status) => {
                    match &status {
                        crate::server::ServerStatus::Running => {
                            self.set_toast("Server running".to_string());
                        }
                        crate::server::ServerStatus::Stopped => {
                            self.set_toast("Server stopped".to_string());
                        }
                        crate::server::ServerStatus::Error(e) => {
                            self.set_toast(format!("Server error: {e}"));
                        }
                        crate::server::ServerStatus::Starting
                        | crate::server::ServerStatus::Stopping => {}
                    }
                    self.state.server_state.status = status;
                }
                crate::server::ServerEvent::Health(metrics) => {
                    let _ = config::append_metric_sample(&metrics);
                    self.state.server_state.health = Some(metrics);
                }
            }
        }

        // Probe events
        while let Ok(event) = self.probe_rx.try_recv() {
            match event {
                probe::ProbeEvent::Line(line) => {
                    self.state.probe_state.log_lines.push(line);
                }
                probe::ProbeEvent::Complete(result) => match *result {
                    Ok(r) => self.state.probe_state.result = Some(r),
                    Err(e) => {
                        self.state
                            .probe_state
                            .log_lines
                            .push(format!("Probe failed: {e}"));
                    }
                },
            }
        }
    }

    fn poll_vram_usage(&mut self) {
        if self.last_vram_poll.elapsed() < Duration::from_secs(5) {
            return;
        }
        self.last_vram_poll = Instant::now();

        if !matches!(
            self.state.probe_state.result.as_ref().map(|r| &r.cuda),
            Some(probe::CudaProbe::Found { .. })
        ) {
            self.state.vram_usage = None;
            return;
        }

        self.state.vram_usage = detect_vram_usage();
    }

    fn poll_config_reload(&mut self) {
        if !config::check_config_reload(&mut self.last_config_check) {
            return;
        }

        match config::load_config() {
            Ok(config) => {
                self.state.config = config;
                self.state
                    .server_state
                    .log_lines
                    .push("Config reloaded from disk.".to_string());
                self.set_toast("Config reloaded".to_string());
            }
            Err(e) => {
                self.state
                    .server_state
                    .log_lines
                    .push(format!("Config reload failed: {e}"));
            }
        }
    }

    fn set_toast(&mut self, message: String) {
        self.state.toast_message = message;
        self.state.toast_ticks = 30;
    }

    fn tick_toast(&mut self) {
        if self.state.toast_ticks > 0 {
            self.state.toast_ticks -= 1;
            if self.state.toast_ticks == 0 {
                self.state.toast_message.clear();
            }
        }
    }

    /// Dispatch a key event to the current screen handler.
    fn dispatch_key(&mut self, key: crossterm::event::KeyEvent) -> Result<()> {
        // Global keybindings
        use crossterm::event::KeyCode;

        match key.code {
            KeyCode::Char('q') if !self.state.modal_active => {
                self.quit = true;
                return Ok(());
            }
            KeyCode::Char('1') => {
                self.state.current_screen = Screen::Dashboard;
                return Ok(());
            }
            KeyCode::Char('d') if self.state.current_screen != Screen::Models => {
                self.state.current_screen = Screen::Dashboard;
                return Ok(());
            }
            KeyCode::Char('2') => {
                self.state.current_screen = Screen::Models;
                return Ok(());
            }
            KeyCode::Char('m') if self.state.current_screen != Screen::Server => {
                self.state.current_screen = Screen::Models;
                return Ok(());
            }
            KeyCode::Char('3') => {
                self.state.current_screen = Screen::Server;
                return Ok(());
            }
            KeyCode::Char('s')
                if !matches!(
                    self.state.current_screen,
                    Screen::Models | Screen::Server | Screen::Settings
                ) =>
            {
                self.state.current_screen = Screen::Server;
                return Ok(());
            }
            KeyCode::Char('4') => {
                self.state.current_screen = Screen::Build;
                return Ok(());
            }
            KeyCode::Char('b') if self.state.current_screen != Screen::Build => {
                self.state.current_screen = Screen::Build;
                return Ok(());
            }
            KeyCode::Char('5') | KeyCode::Char('S') => {
                self.state.current_screen = Screen::Settings;
                return Ok(());
            }
            KeyCode::Tab => {
                let screens = &[
                    Screen::Dashboard,
                    Screen::Models,
                    Screen::Server,
                    Screen::Build,
                    Screen::Settings,
                ];
                let idx = screens
                    .iter()
                    .position(|s| *s == self.state.current_screen)
                    .unwrap_or(0);
                self.state.current_screen = screens[(idx + 1) % screens.len()];
                return Ok(());
            }
            _ => {}
        }

        // Delegate to screen-specific handler
        match self.state.current_screen {
            Screen::Dashboard => tui::dashboard::handle_event(key, self),
            Screen::Models => tui::models::handle_event(key, self),
            Screen::Server => tui::server::handle_event(key, self),
            Screen::Build => tui::build::handle_event(key, self),
            Screen::Settings => tui::settings::handle_event(key, self),
        }

        Ok(())
    }
}

fn detect_vram_usage() -> Option<state::VramUsage> {
    detect_nvidia_vram_usage()
        .or_else(detect_rocm_vram_usage)
        .or_else(detect_macos_unified_memory)
}

fn detect_nvidia_vram_usage() -> Option<state::VramUsage> {
    let output = std::process::Command::new("nvidia-smi")
        .args([
            "--query-gpu=memory.used,memory.total",
            "--format=csv,noheader,nounits",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let first = stdout.lines().next()?;
    let mut parts = first.split(',').map(str::trim);
    let used_mb = parts.next()?.parse().ok()?;
    let total_mb = parts.next()?.parse().ok()?;
    Some(state::VramUsage { used_mb, total_mb })
}

fn detect_rocm_vram_usage() -> Option<state::VramUsage> {
    let output = std::process::Command::new("rocm-smi")
        .args(["--showmeminfo", "vram"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let used_mb = find_first_number_after(&stdout, "used")?;
    let total_mb = find_first_number_after(&stdout, "total")?;
    Some(state::VramUsage { used_mb, total_mb })
}

#[cfg(target_os = "macos")]
fn detect_macos_unified_memory() -> Option<state::VramUsage> {
    let mut sys = sysinfo::System::new_all();
    sys.refresh_memory();
    Some(state::VramUsage {
        used_mb: sys.used_memory() / 1024 / 1024,
        total_mb: sys.total_memory() / 1024 / 1024,
    })
}

#[cfg(not(target_os = "macos"))]
fn detect_macos_unified_memory() -> Option<state::VramUsage> {
    None
}

fn find_first_number_after(text: &str, needle: &str) -> Option<u64> {
    text.lines()
        .find(|line| line.to_lowercase().contains(needle))
        .and_then(|line| {
            line.split(|c: char| !c.is_ascii_digit())
                .find(|part| !part.is_empty())
        })
        .and_then(|part| part.parse().ok())
}
