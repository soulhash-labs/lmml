use crate::app::App;
use crate::tui::helpers::{
    panel, COLOR_ACCENT, COLOR_ERROR, COLOR_INFO, COLOR_MUTED, COLOR_SUCCESS,
};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

pub fn handle_event(key: crossterm::event::KeyEvent, app: &mut App) {
    use crossterm::event::KeyCode;
    match key.code {
        KeyCode::Char('b') | KeyCode::Char('y') => start_build(app),
        KeyCode::Char('n') => {
            if app.state.build_state.complete.is_none() {
                app.state
                    .build_state
                    .log_lines
                    .push("Interrupted build left paused. Press [b] to rebuild.".to_string());
                let _ =
                    crate::app::config::save_state(&crate::app::config::AppStateToml::default());
            }
        }
        KeyCode::Char('c') => {
            if app.state.build_state.is_running {
                app.build_cancel
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                app.state
                    .build_state
                    .log_lines
                    .push("Cancelling build...".to_string());
            }
        }
        KeyCode::Char('r') => {
            // Re-run probe
            app.state.probe_state.log_lines.clear();
            app.state.probe_state.result = None;
            let tx = app.probe_tx.clone();
            tokio::spawn(async move {
                crate::probe::run_all(tx).await;
            });
        }
        _ => {}
    }
}

pub fn start_build(app: &mut App) {
    if app.state.build_state.is_running {
        return;
    }

    app.state.build_state.is_running = true;
    app.state.build_state.complete = None;
    app.state.build_state.log_lines.clear();

    let mut flags = app
        .state
        .probe_state
        .result
        .as_ref()
        .map(|r| r.suggested_cmake_flags.clone())
        .unwrap_or_default();
    apply_backend_override(&mut flags, &app.state.config.build.backend);
    flags.extend(app.state.config.build.extra_cmake_flags.iter().cloned());
    let llama_path = std::path::PathBuf::from(&app.state.config.build.llama_cpp_path);
    let jobs = app.state.config.build.jobs;
    let tx = app.build_tx.clone();
    let cancel_flag = app.build_cancel.clone();
    cancel_flag.store(false, std::sync::atomic::Ordering::Relaxed);

    let _ = crate::app::config::save_state(&crate::app::config::AppStateToml {
        last_session: crate::app::config::LastSession {
            last_model: app
                .state
                .models
                .get(app.state.selected_model)
                .map(|m| m.path.clone())
                .unwrap_or_default(),
            server_was_running: !matches!(
                app.state.server_state.status,
                crate::server::ServerStatus::Stopped | crate::server::ServerStatus::Error(_)
            ),
            build_state: "running".to_string(),
            build_commit: app.state.build_state.commit_hash.clone(),
        },
    });

    tokio::spawn(async move {
        if cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
            let _ = tx
                .send(crate::build::BuildEvent::Complete(Err(
                    "Build cancelled".into()
                )))
                .await;
            return;
        }
        if let Err(e) = crate::build::clone::ensure_repo(&llama_path, tx.clone()).await {
            let _ = tx.send(crate::build::BuildEvent::Complete(Err(e))).await;
            return;
        }
        if cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
            let _ = tx
                .send(crate::build::BuildEvent::Complete(Err(
                    "Build cancelled".into()
                )))
                .await;
            return;
        }
        let result = crate::build::compile::run_build(
            &llama_path,
            &flags,
            jobs,
            tx.clone(),
            Some(cancel_flag),
        )
        .await;
        let _ = tx.send(crate::build::BuildEvent::Complete(result)).await;
    });
}

fn apply_backend_override(flags: &mut Vec<String>, backend: &str) {
    let backend = backend.trim().to_lowercase();
    if backend.is_empty() || backend == "auto" {
        return;
    }

    flags.retain(|flag| {
        !flag.starts_with("-DGGML_CUDA=")
            && !flag.starts_with("-DGGML_HIP=")
            && !flag.starts_with("-DGGML_VULKAN=")
            && !flag.starts_with("-DGGML_METAL=")
    });

    match backend.as_str() {
        "cpu" => {
            flags.push("-DGGML_CUDA=OFF".to_string());
            flags.push("-DGGML_HIP=OFF".to_string());
            flags.push("-DGGML_VULKAN=OFF".to_string());
            flags.push("-DGGML_METAL=OFF".to_string());
        }
        "cuda" => flags.push("-DGGML_CUDA=ON".to_string()),
        "rocm" => flags.push("-DGGML_HIP=ON".to_string()),
        "vulkan" => flags.push("-DGGML_VULKAN=ON".to_string()),
        "metal" => flags.push("-DGGML_METAL=ON".to_string()),
        _ => {}
    }
}

pub fn render(area: Rect, app: &App, frame: &mut Frame) {
    let has_progress = app.state.build_state.is_running && app.state.build_state.progress.1 > 0;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(8),
            Constraint::Length(if has_progress { 4 } else { 3 }),
            Constraint::Min(3),
        ])
        .split(area);

    render_probe_results(chunks[0], app, frame);
    render_build_controls(chunks[1], app, frame, has_progress);
    render_build_log(chunks[2], app, frame);
}

fn render_probe_results(area: Rect, app: &App, frame: &mut Frame) {
    let block = panel(Some("Hardware Detection"));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let probe = &app.state.probe_state;
    let mut lines: Vec<Line> = probe
        .log_lines
        .iter()
        .map(|l| {
            let color = if l.starts_with('✓') {
                COLOR_SUCCESS
            } else if l.starts_with('○') {
                COLOR_MUTED
            } else if l.starts_with('⚠') || l.starts_with('✗') {
                COLOR_ERROR
            } else {
                COLOR_INFO
            };
            Line::from(Span::styled(l.clone(), Style::default().fg(color)))
        })
        .collect();

    if probe.result.is_none() && lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "Hardware detection will run on startup. Press [r] to re-run.",
            Style::default().fg(COLOR_MUTED),
        )));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_build_controls(area: Rect, app: &App, frame: &mut Frame, has_progress: bool) {
    let block = panel(Some("Build"));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines = Vec::new();

    if let Some(flags) = app
        .state
        .probe_state
        .result
        .as_ref()
        .map(|r| &r.suggested_cmake_flags)
    {
        let flags_str = flags.join(" ");
        lines.push(Line::from(Span::styled(
            format!("cmake -B build {flags_str}"),
            Style::default().fg(COLOR_ACCENT),
        )));
    }

    // Progress bar — place in a sub-area above text if active
    if has_progress {
        let (cur, total) = app.state.build_state.progress;
        let pb_area = Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: 1,
        };
        let pb = crate::tui::widgets::progress_bar::ProgressBar {
            current: cur,
            total,
            label: format!("{cur}/{total}"),
        };
        pb.render(pb_area, frame);
    }

    // Last build summary
    if let Some(ref complete) = app.state.build_state.complete {
        match complete {
            Ok(_) => lines.push(Line::from(Span::styled(
                "✓ Last build: succeeded",
                Style::default().fg(COLOR_SUCCESS),
            ))),
            Err(e) => {
                lines.push(Line::from(Span::styled(
                    format!("✗ Last build: {e}"),
                    Style::default().fg(crate::tui::helpers::COLOR_ERROR),
                )));
                let failed_lines: Vec<_> = app
                    .state
                    .build_state
                    .log_lines
                    .iter()
                    .rev()
                    .take(20)
                    .rev()
                    .cloned()
                    .collect();
                for l in &failed_lines {
                    lines.push(Line::from(Span::styled(
                        format!("  {l}"),
                        Style::default().fg(crate::tui::helpers::COLOR_ERROR),
                    )));
                }
            }
        }
    }

    if !app.state.build_state.is_running {
        let action = if app.state.build_state.complete.is_some() {
            "Press [b] to rebuild  |  [r] re-detect hardware"
        } else {
            "Press [b] to build  |  [r] re-detect hardware"
        };
        lines.push(Line::from(Span::styled(
            action,
            Style::default().fg(COLOR_INFO),
        )));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_build_log(area: Rect, app: &App, frame: &mut Frame) {
    let mut log_viewer = crate::tui::widgets::log_viewer::LogViewer {
        lines: app.state.build_state.log_lines.clone(),
        scroll_offset: usize::MAX,
    };
    log_viewer.scroll_to_bottom();
    log_viewer.render(area, frame);
}
