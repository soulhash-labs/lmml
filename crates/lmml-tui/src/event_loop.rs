//! Terminal event loop and background task multiplexing.

use std::io;
use std::time::Duration;

use crossterm::event::{self, Event, KeyEventKind};
use lmml_build::{BuildRunner, RealBuildRunner, UpdateCheck};
use lmml_server::{ServerHandle, ServerManager};
use tokio::sync::mpsc;

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
}

impl EventLoop {
    /// Create a new event loop.
    pub fn new() -> Self {
        let (app_tx, app_rx) = mpsc::channel(256);
        Self {
            app_tx,
            app_rx,
            server_handle: None,
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
        self.poll_terminal()?;
        self.drain_events(app).await;
        Ok(())
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
                AppEvent::ServerStatus(lmml_server::ServerStatus::Stopped) => {
                    self.server_handle = None;
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
            Action::StartBuild | Action::CleanBuild | Action::UpdateAndRebuild => {
                let clean = matches!(action, Action::CleanBuild);
                app.dispatch(action);
                let tx = self.app_tx.clone();
                let config = app.build_config(clean);
                tokio::spawn(async move {
                    tracing::debug!("build task started");
                    let runner = RealBuildRunner;
                    let mut build_rx = runner.run(config).await;
                    while let Some(event) = build_rx.recv().await {
                        if tx.send(AppEvent::BuildEvent(event)).await.is_err() {
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
                            let _ignored = tx.send(AppEvent::ServerStarted(Err(reason))).await;
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
                                    let stopped =
                                        matches!(status, lmml_server::ServerStatus::Stopped);
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
                            let _ignored = tx.send(AppEvent::ServerStarted(Ok(handle))).await;
                        }
                        Err(error) => {
                            let _ignored = tx
                                .send(AppEvent::ServerStarted(Err(error.to_string())))
                                .await;
                        }
                    }
                });
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
            Action::Quit => {
                if let Some(handle) = self.server_handle.take() {
                    handle.stop().await;
                }
                app.dispatch(Action::Quit);
            }
            Action::CancelBuild
            | Action::SelectModel(_)
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
