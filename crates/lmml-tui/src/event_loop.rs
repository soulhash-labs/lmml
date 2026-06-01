//! Terminal event loop and background task multiplexing.

use std::io;
use std::time::Duration;

use crossterm::event::{self, Event, KeyEventKind};
use lmml_build::{BuildRunner, RealBuildRunner, UpdateCheck};
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
}

impl EventLoop {
    /// Create a new event loop.
    pub fn new() -> Self {
        let (app_tx, app_rx) = mpsc::channel(256);
        Self { app_tx, app_rx }
    }

    /// Sender used by background tasks to deliver app events.
    pub fn sender(&self) -> mpsc::Sender<AppEvent> {
        self.app_tx.clone()
    }

    /// Run until the app requests quit.
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
            if let Some(action) = app.handle_event(event) {
                self.dispatch_action(app, action).await;
            }
        }
    }

    async fn dispatch_action(&self, app: &mut App, action: Action) {
        match action {
            Action::RunDetect => {
                app.dispatch(Action::RunDetect);
                let tx = self.app_tx.clone();
                tokio::spawn(async move {
                    let profile = lmml_detect::SystemProfile::detect().await;
                    let _ignored = tx.send(AppEvent::DetectComplete(Box::new(profile))).await;
                });
            }
            Action::CheckForUpdate => {
                app.dispatch(Action::CheckForUpdate);
                let tx = self.app_tx.clone();
                let source_dir = app.state.build.source_dir.clone();
                tokio::spawn(async move {
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
                    let models = registry.scan().await;
                    let _ignored = tx.send(AppEvent::ModelScanComplete(models)).await;
                });
            }
            Action::StartBuild | Action::CleanBuild | Action::UpdateAndRebuild => {
                let clean = matches!(action, Action::CleanBuild);
                app.dispatch(action);
                let tx = self.app_tx.clone();
                let config = app.build_config(clean);
                tokio::spawn(async move {
                    let runner = RealBuildRunner;
                    let mut build_rx = runner.run(config).await;
                    while let Some(event) = build_rx.recv().await {
                        if tx.send(AppEvent::BuildEvent(event)).await.is_err() {
                            break;
                        }
                    }
                });
            }
            Action::CancelBuild
            | Action::StartServer
            | Action::StopServer
            | Action::SelectModel(_)
            | Action::OpenHfSearch
            | Action::SearchHf(_)
            | Action::DownloadModel(_)
            | Action::DeleteModel(_)
            | Action::AddModelAlias
            | Action::SaveSettings
            | Action::ShowHelp
            | Action::Quit => app.dispatch(action),
        }
    }
}

impl Default for EventLoop {
    fn default() -> Self {
        Self::new()
    }
}
