//! Terminal event loop and background task multiplexing.

use std::io;
use std::time::Duration;

use crossterm::event::{self, Event, KeyEventKind};
use lmml_build::{RealBuildRunner, UpdateCheck};
use lmml_server::{ServerHandle, ServerManager};
use tokio::sync::{mpsc, watch};

use crate::action::Action;
use crate::app::{App, AppEvent};

/// Errors returned by the TUI event loop.
#[derive(Debug, thiserror::Error)]
pub enum EventLoopError {
    /// Terminal IO failed.
    #[error("terminal event loop failed: {0}")]
    Io(#[from] io::Error),
}

/// Multiplexes terminal input and background task messages.
pub struct EventLoop {
    app_tx: mpsc::Sender<AppEvent>,
    app_rx: mpsc::Receiver<AppEvent>,
    server_handle: Option<ServerHandle>,
    build_cancel: Option<watch::Sender<bool>>,
    startup_actions_dispatched: bool,
}

impl EventLoop {
    /// Create a new event loop.
    pub fn new() -> Self {
        let (app_tx, app_rx) = mpsc::channel(256);
        Self {
            app_tx,
            app_rx,
            server_handle: None,
            build_cancel: None,
            startup_actions_dispatched: false,
        }
    }

    /// Sender used by background tasks to deliver app events.
    pub fn sender(&self) -> mpsc::Sender<AppEvent> {
        self.app_tx.clone()
    }

    /// Run until the app requests quit.
    #[tracing::instrument(skip(self, app))]
    pub async fn run(&mut self, app: &mut App) -> Result<(), EventLoopError> {
        let mut tick = tokio::time::interval(Duration::from_millis(100));
        loop {
            tick.tick().await;
            self.tick(app).await?;
            if app.should_quit {
                break;
            }
        }
        Ok(())
    }

    /// Process one terminal/background event tick.
    #[tracing::instrument(skip(self, app), level = "trace")]
    pub async fn tick(&mut self, app: &mut App) -> Result<(), EventLoopError> {
        self.dispatch_startup_actions(app).await;
        self.poll_terminal()?;
        self.drain_events(app).await;
        Ok(())
    }

    async fn dispatch_startup_actions(&mut self, app: &mut App) {
        if self.startup_actions_dispatched {
            return;
        }
        self.startup_actions_dispatched = true;
        for action in startup_actions(app) {
            self.dispatch_action(app, action).await;
        }
    }

    fn poll_terminal(&self) -> Result<(), EventLoopError> {
        if event::poll(Duration::from_millis(1))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    let _ignored = self.app_tx.try_send(AppEvent::Key(key));
                }
                Event::Resize(width, height) => {
                    let _ignored = self.app_tx.try_send(AppEvent::Resize(width, height));
                }
                Event::FocusGained
                | Event::FocusLost
                | Event::Key(_)
                | Event::Mouse(_)
                | Event::Paste(_) => {}
            }
        }
        Ok(())
    }

    async fn drain_events(&mut self, app: &mut App) {
        while let Ok(event) = self.app_rx.try_recv() {
            tracing::trace!(event = ?event, "handling app event");
            match &event {
                AppEvent::ServerStarted(Ok(handle)) => {
                    self.server_handle = Some(handle.clone());
                }
                AppEvent::ServerStarted(Err(_)) => {
                    self.server_handle = None;
                }
                AppEvent::ServerModelSwapComplete {
                    result: Ok(handle), ..
                } => {
                    self.server_handle = Some(handle.clone());
                }
                AppEvent::ServerModelSwapComplete { result: Err(_), .. } => {
                    self.server_handle = None;
                }
                AppEvent::ServerStatus(lmml_server::ServerStatus::Stopped) => {
                    self.server_handle = None;
                }
                AppEvent::BuildEvent(
                    lmml_build::BuildEvent::Completed { .. }
                    | lmml_build::BuildEvent::Failed { .. }
                    | lmml_build::BuildEvent::Cancelled
                    | lmml_build::BuildEvent::Skipped { .. },
                ) => {
                    self.build_cancel = None;
                }
                AppEvent::Key(_)
                | AppEvent::Resize(_, _)
                | AppEvent::DetectComplete(_)
                | AppEvent::BuildEvent(_)
                | AppEvent::ServerStatus(_)
                | AppEvent::ServerCapabilities(_)
                | AppEvent::ServerLog(_)
                | AppEvent::DownloadProgress(_)
                | AppEvent::DownloadComplete(_)
                | AppEvent::ModelScanComplete(_)
                | AppEvent::ModelRegistryError(_)
                | AppEvent::HfSearchResults(_)
                | AppEvent::UpdateCheckResult(_) => {}
            }
            if let Some(action) = app.handle_event(event) {
                self.dispatch_action(app, action).await;
            }
        }
    }

    async fn dispatch_action(&mut self, app: &mut App, action: Action) {
        tracing::debug!(action = ?action, "dispatching action");
        match action {
            Action::RunDetect => {
                if app.detect_running {
                    return;
                }
                app.dispatch(Action::RunDetect);
                let tx = self.app_tx.clone();
                tokio::spawn(async move {
                    tracing::debug!("detect task started");
                    let profile = lmml_detect::SystemProfile::detect().await;
                    let _ignored = tx.send(AppEvent::DetectComplete(Box::new(profile))).await;
                });
            }
            Action::CheckForUpdate => {
                app.dispatch(Action::CheckForUpdate);
                let tx = self.app_tx.clone();
                let source_dir = app.state.build.source_dir.clone();
                tokio::spawn(async move {
                    tracing::debug!(source_dir = %source_dir.display(), "update check task started");
                    let update = if source_dir.exists() {
                        lmml_build::check_for_update(&source_dir).await
                    } else {
                        UpdateCheck::Unreachable {
                            reason: "source directory does not exist".to_string(),
                        }
                    };
                    let _ignored = tx.send(AppEvent::UpdateCheckResult(update)).await;
                });
            }
            Action::ScanModels => {
                if app.model_scan_running {
                    return;
                }
                app.dispatch(Action::ScanModels);
                let tx = self.app_tx.clone();
                let registry = lmml_models::ModelRegistry {
                    models_dir: app.state.model.models_dir.clone(),
                    aliases: app.state.model.aliases.clone(),
                };
                tokio::spawn(async move {
                    tracing::debug!("model scan task started");
                    let models = registry.scan().await;
                    let _ignored = tx.send(AppEvent::ModelScanComplete(models)).await;
                });
            }
            Action::SearchHf(query) => {
                app.dispatch(Action::SearchHf(query.clone()));
                let tx = self.app_tx.clone();
                tokio::spawn(async move {
                    tracing::debug!(query = ?query, "huggingface search task started");
                    match lmml_models::search_huggingface(query).await {
                        Ok(results) => {
                            let _ignored = tx.send(AppEvent::HfSearchResults(results)).await;
                        }
                        Err(error) => {
                            let _ignored = tx.send(AppEvent::HfSearchResults(Vec::new())).await;
                            let _ignored = tx
                                .send(AppEvent::DownloadComplete(Err(error.to_string())))
                                .await;
                        }
                    }
                });
            }
            Action::DownloadModel(result) => {
                app.dispatch(Action::DownloadModel(result.clone()));
                let tx = self.app_tx.clone();
                let registry = lmml_models::ModelRegistry {
                    models_dir: app.state.model.models_dir.clone(),
                    aliases: app.state.model.aliases.clone(),
                };
                tokio::spawn(async move {
                    tracing::debug!(url = %result.url, "model download task started");
                    let progress_tx = tx.clone();
                    let downloaded = registry
                        .download(&result.url, move |progress| {
                            let _ignored =
                                progress_tx.try_send(AppEvent::DownloadProgress(progress));
                        })
                        .await
                        .map_err(|error| error.to_string());
                    let _ignored = tx.send(AppEvent::DownloadComplete(downloaded)).await;
                });
            }
            Action::ConfirmAddModelAlias(path) => {
                app.dispatch(Action::ConfirmAddModelAlias(path.clone()));
                let mut registry = lmml_models::ModelRegistry {
                    models_dir: app.state.model.models_dir.clone(),
                    aliases: app.state.model.aliases.clone(),
                };
                match registry.add_alias(path.clone()) {
                    Ok(()) => {
                        app.state.model.aliases = registry.aliases.clone();
                        if let Err(error) = app.state.save() {
                            let _ignored = self
                                .app_tx
                                .send(AppEvent::ModelRegistryError(format!(
                                    "alias added but state save failed: {error}"
                                )))
                                .await;
                        }
                        let tx = self.app_tx.clone();
                        tokio::spawn(async move {
                            let models = registry.scan().await;
                            let _ignored = tx.send(AppEvent::ModelScanComplete(models)).await;
                        });
                    }
                    Err(error) => {
                        let _ignored = self
                            .app_tx
                            .send(AppEvent::ModelRegistryError(format!(
                                "failed to add alias {}: {error}",
                                path.display()
                            )))
                            .await;
                    }
                }
            }
            Action::ConfirmDeleteModel(model) => {
                app.dispatch(Action::ConfirmDeleteModel(model.clone()));
                let registry = lmml_models::ModelRegistry {
                    models_dir: app.state.model.models_dir.clone(),
                    aliases: app.state.model.aliases.clone(),
                };
                match registry.delete(&model) {
                    Ok(()) => {
                        if app.state.model.last_used == model.path {
                            app.state.model.last_used = std::path::PathBuf::new();
                        }
                        if let Err(error) = app.state.save() {
                            let _ignored = self
                                .app_tx
                                .send(AppEvent::ModelRegistryError(format!(
                                    "model deleted but state save failed: {error}"
                                )))
                                .await;
                        }
                        let tx = self.app_tx.clone();
                        tokio::spawn(async move {
                            let models = registry.scan().await;
                            let _ignored = tx.send(AppEvent::ModelScanComplete(models)).await;
                        });
                    }
                    Err(error) => {
                        let _ignored = self
                            .app_tx
                            .send(AppEvent::ModelRegistryError(format!(
                                "failed to delete {}: {error}",
                                model.path.display()
                            )))
                            .await;
                    }
                }
            }
            Action::StartBuild | Action::CleanBuild | Action::UpdateAndRebuild => {
                let update_and_rebuild = matches!(action, Action::UpdateAndRebuild);
                let clean = matches!(action, Action::CleanBuild | Action::UpdateAndRebuild);
                app.dispatch(action);
                let tx = self.app_tx.clone();
                let config = app.build_config(clean);
                let persisted_build = app.state.build.clone();
                let (cancel_tx, cancel_rx) = watch::channel(false);
                self.build_cancel = Some(cancel_tx);
                tokio::spawn(async move {
                    tracing::debug!("build task started");
                    if update_and_rebuild {
                        let update = if config.source_dir.exists() {
                            lmml_build::check_for_update(&config.source_dir).await
                        } else {
                            UpdateCheck::Unreachable {
                                reason: "source directory does not exist; cloning fresh"
                                    .to_string(),
                            }
                        };
                        let _ignored = tx.send(AppEvent::UpdateCheckResult(update)).await;
                    }
                    let runner = RealBuildRunner;
                    if !clean {
                        if let Ok(fingerprint) = current_fingerprint(&config).await {
                            if persisted_fingerprint_matches(
                                &persisted_build,
                                &config,
                                &fingerprint,
                            ) {
                                let _ignored = tx
                                    .send(AppEvent::BuildEvent(lmml_build::BuildEvent::Skipped {
                                        reason: "build fingerprint is up to date".to_string(),
                                    }))
                                    .await;
                                return;
                            }
                        }
                    }
                    let mut build_rx = runner.run_cancellable(config, cancel_rx).await;
                    while let Some(event) = build_rx.recv().await {
                        let terminal = matches!(
                            event,
                            lmml_build::BuildEvent::Completed { .. }
                                | lmml_build::BuildEvent::Failed { .. }
                                | lmml_build::BuildEvent::Cancelled
                                | lmml_build::BuildEvent::Skipped { .. }
                        );
                        if tx.send(AppEvent::BuildEvent(event)).await.is_err() {
                            break;
                        }
                        if terminal {
                            break;
                        }
                    }
                });
            }
            Action::StartServer => {
                app.dispatch(Action::StartServer);
                let tx = self.app_tx.clone();
                let Some(model) = app.selected_server_model() else {
                    let _ignored = tx
                        .send(AppEvent::ServerStarted(Err(
                            "select a model before starting server".to_string(),
                        )))
                        .await;
                    return;
                };
                let config = app.server_config(&model);
                let binary = app.state.build.binary.clone();
                spawn_server_start(tx, binary, model, config, None);
            }
            Action::ProbeServerCapabilities => {
                app.dispatch(Action::ProbeServerCapabilities);
                let tx = self.app_tx.clone();
                let binary = app.state.build.binary.clone();
                tokio::spawn(async move {
                    tracing::debug!(binary = %binary.display(), "server capability probe task started");
                    let result = lmml_compat::LlamaBinaryCapabilities::probe(&binary)
                        .await
                        .map_err(|error| error.to_string());
                    let _ignored = tx.send(AppEvent::ServerCapabilities(result)).await;
                });
            }
            Action::StopServer => {
                app.dispatch(Action::StopServer);
                if let Some(handle) = self.server_handle.take() {
                    handle.stop().await;
                }
                let _ignored = self
                    .app_tx
                    .send(AppEvent::ServerStatus(lmml_server::ServerStatus::Stopped))
                    .await;
            }
            Action::ConfirmModelSwap(model) => {
                app.dispatch(Action::ConfirmModelSwap(model.clone()));
                if let Some(handle) = self.server_handle.take() {
                    handle.stop().await;
                }
                let tx = self.app_tx.clone();
                let config = app.server_config(&model);
                let binary = app.state.build.binary.clone();
                spawn_server_start(tx, binary, model.clone(), config, Some(model));
            }
            Action::Quit => {
                if let Some(handle) = self.server_handle.take() {
                    handle.stop().await;
                }
                app.dispatch(Action::Quit);
            }
            Action::CancelBuild => {
                app.dispatch(Action::CancelBuild);
                if let Some(cancel_tx) = self.build_cancel.take() {
                    let _ignored = cancel_tx.send(true);
                }
            }
            Action::SelectModel(_)
            | Action::OpenHfSearch
            | Action::DeleteModel(_)
            | Action::AddModelAlias
            | Action::SaveSettings
            | Action::ShowHelp => app.dispatch(action),
        }
    }
}

impl Default for EventLoop {
    fn default() -> Self {
        Self::new()
    }
}

fn startup_actions(app: &App) -> Vec<Action> {
    let mut actions = Vec::new();
    if !app.detect_running && app.detect_profile.is_none() {
        actions.push(Action::RunDetect);
    }
    if !app.model_scan_running && app.models.is_empty() {
        actions.push(Action::ScanModels);
    }
    actions
}

async fn current_fingerprint(
    config: &lmml_build::BuildConfig,
) -> Result<lmml_build::BuildFingerprint, lmml_build::BuildError> {
    let commit = lmml_build::current_commit(&config.source_dir).await?;
    let args = lmml_build::cmake_configure_args(config);
    Ok(lmml_build::build_fingerprint(
        commit,
        &args,
        lmml_build::expected_server_binary(&config.source_dir),
    ))
}

fn persisted_fingerprint_matches(
    persisted: &lmml_state::BuildState,
    config: &lmml_build::BuildConfig,
    fingerprint: &lmml_build::BuildFingerprint,
) -> bool {
    !fingerprint.needs_rebuild()
        && persisted.commit == fingerprint.commit
        && persisted.cmake_hash == lmml_build::hash_to_hex(&fingerprint.cmake_hash)
        && persisted.backend == backend_name(&config.backend)
        && persisted.archs == backend_archs(&config.backend)
        && persisted.sccache_used == config.sccache.is_some()
}

fn backend_name(backend: &lmml_detect::BuildBackend) -> String {
    match backend {
        lmml_detect::BuildBackend::Cuda { .. } => "Cuda",
        lmml_detect::BuildBackend::Metal => "Metal",
        lmml_detect::BuildBackend::Vulkan => "Vulkan",
        lmml_detect::BuildBackend::CpuAvx2 => "CpuAvx2",
        lmml_detect::BuildBackend::CpuAvx => "CpuAvx",
        lmml_detect::BuildBackend::CpuFallback => "CpuFallback",
    }
    .to_string()
}

fn backend_archs(backend: &lmml_detect::BuildBackend) -> Vec<String> {
    match backend {
        lmml_detect::BuildBackend::Cuda { archs } => {
            archs.iter().map(|arch| (*arch).to_string()).collect()
        }
        lmml_detect::BuildBackend::Metal
        | lmml_detect::BuildBackend::Vulkan
        | lmml_detect::BuildBackend::CpuAvx2
        | lmml_detect::BuildBackend::CpuAvx
        | lmml_detect::BuildBackend::CpuFallback => Vec::new(),
    }
}

fn spawn_server_start(
    tx: mpsc::Sender<AppEvent>,
    binary: std::path::PathBuf,
    model: lmml_models::ModelEntry,
    config: lmml_compat::ServerConfig,
    swap_model: Option<lmml_models::ModelEntry>,
) {
    tokio::spawn(async move {
        tracing::debug!(binary = %binary.display(), "server start task started");
        let (log_tx, mut log_rx) = mpsc::channel(256);
        let log_app_tx = tx.clone();
        tokio::spawn(async move {
            while let Some(line) = log_rx.recv().await {
                if log_app_tx.send(AppEvent::ServerLog(line)).await.is_err() {
                    break;
                }
            }
        });

        let caps = match lmml_compat::LlamaBinaryCapabilities::probe(&binary).await {
            Ok(caps) => {
                let _ignored = tx
                    .send(AppEvent::ServerCapabilities(Ok(caps.clone())))
                    .await;
                caps
            }
            Err(error) => {
                let reason = error.to_string();
                let _ignored = tx
                    .send(AppEvent::ServerCapabilities(Err(reason.clone())))
                    .await;
                send_server_start_result(&tx, swap_model, Err(reason)).await;
                return;
            }
        };
        let manager = ServerManager { binary, caps };
        match manager.start(&model, &config, log_tx).await {
            Ok(handle) => {
                let mut status_rx = handle.subscribe();
                let status_tx = tx.clone();
                tokio::spawn(async move {
                    while status_rx.changed().await.is_ok() {
                        let status = status_rx.borrow().clone();
                        let stopped = matches!(status, lmml_server::ServerStatus::Stopped);
                        if status_tx
                            .send(AppEvent::ServerStatus(status))
                            .await
                            .is_err()
                        {
                            break;
                        }
                        if stopped {
                            break;
                        }
                    }
                });
                send_server_start_result(&tx, swap_model, Ok(handle)).await;
            }
            Err(error) => {
                send_server_start_result(&tx, swap_model, Err(error.to_string())).await;
            }
        }
    });
}

async fn send_server_start_result(
    tx: &mpsc::Sender<AppEvent>,
    swap_model: Option<lmml_models::ModelEntry>,
    result: Result<ServerHandle, String>,
) {
    let event = match swap_model {
        Some(model) => AppEvent::ServerModelSwapComplete { model, result },
        None => AppEvent::ServerStarted(result),
    };
    let _ignored = tx.send(event).await;
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use lmml_detect::BuildBackend;

    use super::*;

    #[test]
    fn startup_actions_schedule_detect_and_model_scan_once() {
        let mut app = App::default();
        assert_eq!(
            startup_actions(&app),
            vec![Action::RunDetect, Action::ScanModels]
        );

        app.detect_running = true;
        assert_eq!(startup_actions(&app), vec![Action::ScanModels]);

        app.detect_running = false;
        app.model_scan_running = true;
        assert_eq!(startup_actions(&app), vec![Action::RunDetect]);
    }

    #[tokio::test]
    async fn startup_dispatch_sets_in_flight_flags_and_is_one_shot() {
        let mut app = App::default();
        let mut event_loop = EventLoop::new();

        event_loop.dispatch_startup_actions(&mut app).await;

        assert!(app.detect_running);
        assert!(app.model_scan_running);
        assert_eq!(app.status_message, "Scanning models");

        app.detect_running = false;
        app.model_scan_running = false;
        event_loop.dispatch_startup_actions(&mut app).await;

        assert!(!app.detect_running);
        assert!(!app.model_scan_running);
    }

    #[test]
    fn persisted_fingerprint_skips_when_unchanged_and_rebuilds_on_flag_change() {
        let binary = std::env::current_exe().expect("current test executable");
        let mut config =
            lmml_build::BuildConfig::new(PathBuf::from("/tmp/lmml-src"), BuildBackend::CpuFallback);
        let args = lmml_build::cmake_configure_args(&config);
        let fingerprint = lmml_build::build_fingerprint("abc123", &args, binary);
        let mut persisted = lmml_state::BuildState {
            commit: "abc123".to_string(),
            cmake_hash: lmml_build::hash_to_hex(&fingerprint.cmake_hash),
            backend: "CpuFallback".to_string(),
            binary: fingerprint.binary.clone(),
            ..lmml_state::BuildState::default()
        };

        assert!(persisted_fingerprint_matches(
            &persisted,
            &config,
            &fingerprint
        ));

        config
            .extra_cmake_flags
            .push("-DGGML_NATIVE=ON".to_string());
        let changed_args = lmml_build::cmake_configure_args(&config);
        let changed =
            lmml_build::build_fingerprint("abc123", &changed_args, fingerprint.binary.clone());
        persisted.cmake_hash = lmml_build::hash_to_hex(&fingerprint.cmake_hash);

        assert!(!persisted_fingerprint_matches(
            &persisted, &config, &changed
        ));
    }

    #[test]
    fn persisted_fingerprint_rebuilds_when_backend_or_sccache_changes() {
        let binary = std::env::current_exe().expect("current test executable");
        let config = lmml_build::BuildConfig::new(
            PathBuf::from("/tmp/lmml-src"),
            BuildBackend::Cuda {
                archs: vec!["sm_86"],
            },
        );
        let args = lmml_build::cmake_configure_args(&config);
        let fingerprint = lmml_build::build_fingerprint("abc123", &args, binary);
        let mut persisted = lmml_state::BuildState {
            commit: "abc123".to_string(),
            cmake_hash: lmml_build::hash_to_hex(&fingerprint.cmake_hash),
            backend: "CpuFallback".to_string(),
            binary: fingerprint.binary.clone(),
            ..lmml_state::BuildState::default()
        };

        assert!(!persisted_fingerprint_matches(
            &persisted,
            &config,
            &fingerprint
        ));

        persisted.backend = "Cuda".to_string();
        persisted.archs = vec!["sm_86".to_string()];
        persisted.sccache_used = true;

        assert!(!persisted_fingerprint_matches(
            &persisted,
            &config,
            &fingerprint
        ));
    }
}
