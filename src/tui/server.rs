use crate::app::App;
use crate::server::ServerStatus;
use crate::tui::helpers::{
    panel, COLOR_ACCENT, COLOR_ERROR, COLOR_INFO, COLOR_MUTED, COLOR_SUCCESS, COLOR_WARNING,
};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

pub fn handle_event(key: crossterm::event::KeyEvent, app: &mut App) {
    use crossterm::event::KeyCode;
    if app.state.server_restart_pending {
        match key.code {
            KeyCode::Enter => {
                app.state.server_restart_pending = false;
                app.state.modal_active = false;
                restart_server(app);
            }
            KeyCode::Esc => {
                app.state.server_restart_pending = false;
                app.state.modal_active = false;
                app.state
                    .server_state
                    .log_lines
                    .push("Model swap kept pending; restart later to apply it.".to_string());
            }
            _ => {}
        }
        return;
    }

    if let Some(edit_idx) = app.state.server_edit_field {
        match key.code {
            KeyCode::Esc => {
                app.state.server_edit_field = None;
                app.state.server_edit_buffer.clear();
            }
            KeyCode::Enter => {
                let val = app.state.server_edit_buffer.clone();
                apply_server_field(edit_idx, &val, app);
                app.state.server_edit_field = None;
                app.state.server_edit_buffer.clear();
            }
            KeyCode::Char(c) if c.is_ascii_digit() => {
                app.state.server_edit_buffer.push(c);
            }
            KeyCode::Backspace => {
                app.state.server_edit_buffer.pop();
            }
            _ => {}
        }
        return;
    }

    match key.code {
        KeyCode::Down | KeyCode::Tab => {
            app.state.server_selected_field = (app.state.server_selected_field + 1).min(4);
        }
        KeyCode::Up | KeyCode::BackTab => {
            app.state.server_selected_field = app.state.server_selected_field.saturating_sub(1);
        }
        KeyCode::Char('m') => {
            cycle_model(app);
        }
        KeyCode::Char(' ') | KeyCode::Enter => {
            if matches!(key.code, KeyCode::Enter) {
                app.state.server_edit_field = Some(app.state.server_selected_field);
                app.state.server_edit_buffer =
                    server_field_value(app, app.state.server_selected_field);
                return;
            }
            let status = app.state.server_state.status.clone();
            match status {
                crate::server::ServerStatus::Stopped | crate::server::ServerStatus::Error(_) => {
                    start_server(app);
                }
                crate::server::ServerStatus::Running | crate::server::ServerStatus::Starting => {
                    stop_server(app);
                }
                _ => {}
            }
        }
        _ => {}
    }
}

fn server_field_value(app: &App, idx: usize) -> String {
    let config = &app.state.config.server;
    match idx {
        0 => config.port.to_string(),
        1 => config.context_size.to_string(),
        2 => config.gpu_layers.to_string(),
        3 => config.threads.to_string(),
        4 => config.batch_size.to_string(),
        _ => String::new(),
    }
}

fn apply_server_field(idx: usize, val: &str, app: &mut App) {
    let config = &mut app.state.config.server;
    match idx {
        0 => config.port = val.parse().unwrap_or(8080),
        1 => config.context_size = val.parse().unwrap_or(8192),
        2 => config.gpu_layers = val.parse().unwrap_or(99),
        3 => config.threads = val.parse().unwrap_or(0),
        4 => config.batch_size = val.parse().unwrap_or(512),
        _ => return,
    }
    if crate::app::config::save_config(&app.state.config).is_ok() {
        app.state
            .server_state
            .log_lines
            .push("Server config saved.".to_string());
    }
}

fn cycle_model(app: &mut App) {
    if app.state.models.is_empty() {
        app.state
            .server_state
            .log_lines
            .push("No models available to select.".to_string());
        return;
    }

    let current = app.state.selected_model.min(app.state.models.len() - 1);
    app.state.selected_model = (current + 1) % app.state.models.len();
    let model_name = app.state.models[app.state.selected_model].name.clone();
    app.state
        .server_state
        .log_lines
        .push(format!("Selected model: {model_name}"));

    if matches!(
        app.state.server_state.status,
        ServerStatus::Running | ServerStatus::Starting
    ) {
        app.state.server_restart_pending = true;
        app.state.modal_active = true;
        app.state.modal_message =
            "Model changed. Press Enter to restart the server now, or Esc to keep it running."
                .to_string();
    }
}

fn start_server(app: &mut App) {
    let Some((binary, model_path, config, tx, child_lock)) = server_launch_context(app) else {
        return;
    };

    app.state.server_state.status = crate::server::ServerStatus::Starting;
    tokio::spawn(async move {
        if crate::server::process::is_port_in_use(config.port).await {
            let msg = format!(
                "Port {} is already in use — try a different port in Settings",
                config.port
            );
            let _ = tx.send(crate::server::ServerEvent::LogLine(msg)).await;
            let _ = tx
                .send(crate::server::ServerEvent::StatusChange(
                    crate::server::ServerStatus::Stopped,
                ))
                .await;
            return;
        }

        run_server_loop(binary, model_path, config, tx, child_lock).await;
    });
}

fn stop_server(app: &mut App) {
    let child_lock = app.server_child.clone();
    let tx = app.server_tx.clone();
    tokio::spawn(async move {
        stop_child(child_lock, tx).await;
    });
}

fn restart_server(app: &mut App) {
    let Some((binary, model_path, config, tx, child_lock)) = server_launch_context(app) else {
        return;
    };

    app.state.server_state.status = crate::server::ServerStatus::Stopping;
    tokio::spawn(async move {
        stop_child(child_lock.clone(), tx.clone()).await;
        run_server_loop(binary, model_path, config, tx, child_lock).await;
    });
}

type LaunchContext = (
    std::path::PathBuf,
    String,
    crate::app::config::ServerConfig,
    tokio::sync::mpsc::Sender<crate::server::ServerEvent>,
    std::sync::Arc<tokio::sync::Mutex<Option<tokio::process::Child>>>,
);

fn server_launch_context(app: &mut App) -> Option<LaunchContext> {
    let llama_path = crate::app::config::llama_cpp_dir();
    let binary = llama_path.join("build").join("bin").join("llama-server");
    let model_path = app
        .state
        .models
        .get(app.state.selected_model)
        .map(|m| m.path.clone())
        .unwrap_or_default();
    if model_path.is_empty() {
        app.state
            .server_state
            .log_lines
            .push("No model selected — go to Models screen first.".to_string());
        return None;
    }

    Some((
        binary,
        model_path,
        app.state.config.clone().server,
        app.server_tx.clone(),
        app.server_child.clone(),
    ))
}

async fn stop_child(
    child_lock: std::sync::Arc<tokio::sync::Mutex<Option<tokio::process::Child>>>,
    tx: tokio::sync::mpsc::Sender<crate::server::ServerEvent>,
) {
    let _ = tx
        .send(crate::server::ServerEvent::LogLine(
            "Stopping server...".into(),
        ))
        .await;
    let _ = tx
        .send(crate::server::ServerEvent::StatusChange(
            crate::server::ServerStatus::Stopping,
        ))
        .await;

    let mut guard = child_lock.lock().await;
    if let Some(mut child) = guard.take() {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
    drop(guard);

    let _ = tx
        .send(crate::server::ServerEvent::LogLine(
            "Server stopped.".into(),
        ))
        .await;
    let _ = tx
        .send(crate::server::ServerEvent::StatusChange(
            crate::server::ServerStatus::Stopped,
        ))
        .await;
}

async fn run_server_loop(
    binary: std::path::PathBuf,
    model_path: String,
    config: crate::app::config::ServerConfig,
    tx: tokio::sync::mpsc::Sender<crate::server::ServerEvent>,
    child_lock: std::sync::Arc<tokio::sync::Mutex<Option<tokio::process::Child>>>,
) {
    let _ = tx
        .send(crate::server::ServerEvent::LogLine(format!(
            "Starting server on port {}...",
            config.port
        )))
        .await;

    let mut backoff = 1u64;
    let max_backoff: u64 = 30;

    loop {
        let mut args: Vec<String> = vec![
            "-m".into(),
            model_path.clone(),
            "--port".into(),
            config.port.to_string(),
            "-c".into(),
            config.context_size.to_string(),
            "-ngl".into(),
            config.gpu_layers.to_string(),
            "-b".into(),
            config.batch_size.to_string(),
        ];
        if config.threads > 0 {
            args.push("-t".into());
            args.push(config.threads.to_string());
        }
        args.extend(config.extra_args.iter().cloned());

        match tokio::process::Command::new(&binary)
            .args(&args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(child) => {
                let pid = child.id().unwrap_or(0);
                let _ = tx
                    .send(crate::server::ServerEvent::LogLine(format!(
                        "Server started (pid: {pid})"
                    )))
                    .await;
                *child_lock.lock().await = Some(child);
            }
            Err(e) => {
                let _ = tx
                    .send(crate::server::ServerEvent::LogLine(format!(
                        "Failed to start server: {e}"
                    )))
                    .await;
                let _ = tx
                    .send(crate::server::ServerEvent::StatusChange(
                        crate::server::ServerStatus::Stopped,
                    ))
                    .await;
                break;
            }
        }

        let _ = tx
            .send(crate::server::ServerEvent::StatusChange(
                crate::server::ServerStatus::Running,
            ))
            .await;

        crate::server::process::run_health_loop(config.port, tx.clone()).await;

        if child_lock.lock().await.is_none() {
            let _ = tx
                .send(crate::server::ServerEvent::StatusChange(
                    crate::server::ServerStatus::Stopped,
                ))
                .await;
            break;
        }

        let _ = tx
            .send(crate::server::ServerEvent::LogLine(format!(
                "Server crashed — restarting in {backoff}s..."
            )))
            .await;
        tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
        backoff = (backoff * 2).min(max_backoff);
    }
}

pub fn render(area: Rect, app: &App, frame: &mut Frame) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(8),
            Constraint::Length(6),
            Constraint::Min(5),
        ])
        .split(area);

    render_status(chunks[0], app, frame);
    render_config(chunks[1], app, frame);
    render_performance(chunks[2], app, frame);
    render_log(chunks[3], app, frame);

    if app.state.server_restart_pending {
        let modal_area = centered_rect(60, 25, area);
        let block = Block::default()
            .title(" Restart Server ")
            .borders(Borders::ALL)
            .style(Style::default());
        let inner = block.inner(modal_area);
        frame.render_widget(block, modal_area);
        frame.render_widget(
            Paragraph::new(app.state.modal_message.clone()).wrap(Wrap { trim: true }),
            inner,
        );
    }
}

fn render_status(area: Rect, app: &App, frame: &mut Frame) {
    let server = &app.state.server_state;
    let (status_text, status_color) = match &server.status {
        ServerStatus::Running => ("● Running  ", COLOR_SUCCESS),
        ServerStatus::Starting => ("◉ Starting...  ", COLOR_WARNING),
        ServerStatus::Stopped => ("○ Stopped  ", COLOR_MUTED),
        ServerStatus::Stopping => ("◉ Stopping...  ", COLOR_WARNING),
        ServerStatus::Error(_e) => ("● Error  ", COLOR_ERROR),
    };

    let mut spans = vec![Span::styled(
        status_text,
        Style::default()
            .fg(status_color)
            .add_modifier(Modifier::BOLD),
    )];

    if let Some(pid) = server.pid {
        spans.push(Span::styled(
            format!("pid: {pid}  "),
            Style::default().fg(COLOR_MUTED),
        ));
    }
    if server.uptime_secs > 0 {
        let mins = server.uptime_secs / 60;
        let secs = server.uptime_secs % 60;
        spans.push(Span::styled(
            format!("uptime: {mins}m{secs}s  "),
            Style::default().fg(COLOR_INFO),
        ));
    }
    if let Some(metrics) = &server.health {
        spans.push(Span::styled(
            format!(
                "⚡ {:.1} tok/s  ({:.0}ms)",
                metrics.tok_s, metrics.latency_ms
            ),
            Style::default().fg(COLOR_ACCENT),
        ));
    }

    let block = panel(Some("Server Status"));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(Line::from(spans)), inner);
}

fn render_config(area: Rect, app: &App, frame: &mut Frame) {
    let block = panel(Some("Configuration"));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let config = &app.state.config.server;
    let model_path = if app.state.models.is_empty() {
        "No model selected".to_string()
    } else {
        app.state.models[app.state.selected_model].name.clone()
    };

    let labels = [
        ("Port", config.port.to_string()),
        ("Context", config.context_size.to_string()),
        ("GPU layers", config.gpu_layers.to_string()),
        (
            "Threads",
            if config.threads == 0 {
                "auto".to_string()
            } else {
                config.threads.to_string()
            },
        ),
        ("Batch", config.batch_size.to_string()),
    ];
    let mut lines = Vec::with_capacity(labels.len() + 1);
    for (idx, (label, value)) in labels.iter().enumerate() {
        let selected = app.state.server_selected_field == idx;
        let editing = app.state.server_edit_field == Some(idx);
        let prefix = if selected { "▸ " } else { "  " };
        let display_value = if editing {
            app.state.server_edit_buffer.clone()
        } else {
            value.clone()
        };
        let style = if editing {
            Style::default()
                .fg(COLOR_ACCENT)
                .add_modifier(Modifier::SLOW_BLINK)
        } else if selected {
            Style::default()
                .fg(COLOR_ACCENT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(
            format!("{prefix}{label}: {display_value}"),
            style,
        )));
    }
    lines.push(Line::from(Span::raw(format!("  Model: {model_path}"))));

    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_performance(area: Rect, app: &App, frame: &mut Frame) {
    let block = panel(Some("Performance"));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(metrics) = &app.state.server_state.health else {
        frame.render_widget(
            Paragraph::new("No metrics yet. Start the server to collect health data.")
                .style(Style::default().fg(COLOR_MUTED)),
            inner,
        );
        return;
    };

    let active_slots = metrics
        .active_slots
        .map(|v| v.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let kv_cache = match (metrics.kv_cache_used, metrics.kv_cache_total) {
        (Some(used), Some(total)) if total > 0 => {
            format!(
                "{used} / {total} cells ({:.0}%)",
                used as f64 / total as f64 * 100.0
            )
        }
        (Some(used), _) => format!("{used} used"),
        _ => "unknown".to_string(),
    };

    let lines = vec![
        Line::from(Span::styled(
            format!("Tokens/sec:   {:.1}", metrics.tok_s),
            Style::default().fg(COLOR_INFO),
        )),
        Line::from(Span::raw(format!(
            "Latency:      {:.0} ms",
            metrics.latency_ms
        ))),
        Line::from(Span::raw(format!("Active slots: {active_slots}"))),
        Line::from(Span::raw(format!("KV cache:     {kv_cache}"))),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_log(area: Rect, app: &App, frame: &mut Frame) {
    let mut log_viewer = crate::tui::widgets::log_viewer::LogViewer {
        lines: app.state.server_state.log_lines.clone(),
        scroll_offset: usize::MAX,
    };
    log_viewer.scroll_to_bottom();
    log_viewer.render(area, frame);
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let x_pad = area.width.saturating_mul(100 - percent_x) / 200;
    let y_pad = area.height.saturating_mul(100 - percent_y) / 200;

    Rect {
        x: area.x + x_pad,
        y: area.y + y_pad,
        width: area.width.saturating_sub(x_pad * 2),
        height: area.height.saturating_sub(y_pad * 2),
    }
}
