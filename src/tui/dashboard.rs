use crate::app::App;
use crate::server::ServerStatus;
use crate::tui::helpers::{
    panel, status_style, COLOR_ERROR, COLOR_INFO, COLOR_MUTED, COLOR_SUCCESS, COLOR_WARNING,
};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, Paragraph};
use ratatui::Frame;

pub fn handle_event(_key: crossterm::event::KeyEvent, _app: &mut App) {}

pub fn render(area: Rect, app: &App, frame: &mut Frame) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(9),
            Constraint::Length(8),
            Constraint::Min(5),
        ])
        .split(area);

    render_system_summary(chunks[0], app, frame);
    render_status_panels(chunks[1], app, frame);
    render_models_preview(chunks[2], app, frame);
}

fn render_system_summary(area: Rect, app: &App, frame: &mut Frame) {
    let block = panel(Some("System Overview"));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let probe = &app.state.probe_state;
    let mut lines = Vec::new();
    let y_offset = inner.y;

    if let Some(ref result) = probe.result {
        let gpu_status = match &result.cuda {
            crate::probe::CudaProbe::Found {
                gpu_name, vram_gb, ..
            } => {
                format!("● NVIDIA CUDA — {gpu_name} ({vram_gb} GB VRAM)")
            }
            crate::probe::CudaProbe::NotFound => "○ No NVIDIA GPU detected".to_string(),
            crate::probe::CudaProbe::Error(_) => "⚠ CUDA probe error".to_string(),
        };
        lines.push(Line::from(Span::styled(gpu_status, status_style(true))));
        if let crate::probe::CudaProbe::Found { vram_gb, .. } = result.cuda {
            let vram_detail = app.state.vram_usage.map_or_else(
                || format!("○ VRAM total: {vram_gb} GB"),
                |usage| {
                    format!(
                        "○ VRAM: {:.1} / {:.1} GB",
                        usage.used_mb as f64 / 1024.0,
                        usage.total_mb as f64 / 1024.0
                    )
                },
            );
            lines.push(Line::from(Span::styled(
                vram_detail,
                Style::default().fg(COLOR_INFO),
            )));
        }

        let cpu = &result.cpu;
        lines.push(Line::from(Span::styled(
            format!("● CPU: {}  —  {}C/{}T", cpu.model, cpu.cores, cpu.threads),
            Style::default().fg(COLOR_INFO),
        )));

        // RAM usage bar using sysinfo
        let mut sys = sysinfo::System::new_all();
        sys.refresh_memory();
        let used_mb = sys.used_memory() / (1024 * 1024);
        let total_mb = sys.total_memory() / (1024 * 1024);
        let ratio = if total_mb > 0 {
            used_mb as f64 / total_mb as f64
        } else {
            0.0
        };
        lines.push(Line::from(Span::styled(
            format!(
                "● RAM: {:.1} / {:.1} GB ({:.0}%)",
                used_mb as f64 / 1024.0,
                total_mb as f64 / 1024.0,
                ratio * 100.0
            ),
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "Running hardware detection...",
            Style::default().fg(Color::Yellow),
        )));
    }

    // Build status
    if let Some(ref complete) = app.state.build_state.complete {
        match complete {
            Ok(_) => {
                let hash = &app.state.build_state.commit_hash;
                let hash_suffix = if hash.is_empty() {
                    String::new()
                } else {
                    format!(" ({hash})")
                };
                lines.push(Line::from(Span::styled(
                    format!("✓ Build: completed{hash_suffix}"),
                    Style::default().fg(COLOR_SUCCESS),
                )));
            }
            Err(_) => lines.push(Line::from(Span::styled(
                "✗ Build: failed",
                Style::default().fg(COLOR_ERROR),
            ))),
        }
    }

    let line_count = lines.len();
    frame.render_widget(Paragraph::new(lines), inner);

    // Reusable Gauge style for memory bar
    let gauge_area = Rect {
        x: area.x + 1,
        y: y_offset + line_count as u16,
        width: area.width.saturating_sub(2),
        height: 1,
    };
    if gauge_area.width > 10 && probe.result.is_some() {
        let mut sys = sysinfo::System::new_all();
        sys.refresh_memory();
        let used_mb = sys.used_memory() / (1024 * 1024);
        let total_mb = sys.total_memory() / (1024 * 1024);
        let ratio = if total_mb > 0 {
            used_mb as f64 / total_mb as f64
        } else {
            0.0
        };
        let color = if ratio > 0.9 {
            Color::Red
        } else if ratio > 0.7 {
            Color::Yellow
        } else {
            Color::Green
        };
        frame.render_widget(
            Gauge::default()
                .gauge_style(Style::default().fg(color).bg(Color::DarkGray))
                .percent((ratio * 100.0) as u16)
                .label(format!("RAM: {:.0}%", ratio * 100.0)),
            gauge_area,
        );
    }

    if let Some(crate::probe::ProbeResult {
        cuda: crate::probe::CudaProbe::Found { vram_gb, .. },
        ..
    }) = probe.result.as_ref()
    {
        let vram_area = Rect {
            x: area.x + 1,
            y: gauge_area.y.saturating_add(1),
            width: area.width.saturating_sub(2),
            height: 1,
        };
        if vram_area.width > 10 {
            let (percent, label) =
                app.state
                    .vram_usage
                    .map_or((100, format!("VRAM total: {vram_gb} GB")), |usage| {
                        let percent = if usage.total_mb > 0 {
                            ((usage.used_mb as f64 / usage.total_mb as f64) * 100.0) as u16
                        } else {
                            0
                        };
                        (
                            percent.min(100),
                            format!(
                                "VRAM: {:.0}%",
                                if usage.total_mb > 0 {
                                    usage.used_mb as f64 / usage.total_mb as f64 * 100.0
                                } else {
                                    0.0
                                }
                            ),
                        )
                    });
            let color = if percent > 90 {
                Color::Red
            } else if percent > 70 {
                Color::Yellow
            } else {
                COLOR_INFO
            };
            frame.render_widget(
                Gauge::default()
                    .gauge_style(Style::default().fg(color).bg(Color::DarkGray))
                    .percent(percent)
                    .label(label),
                vram_area,
            );
        }
    }
}

fn render_status_panels(area: Rect, app: &App, frame: &mut Frame) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    // Build status panel
    let build_block = panel(Some("Build"));
    let build_inner = build_block.inner(chunks[0]);
    frame.render_widget(build_block, chunks[0]);

    let build_status = if app
        .state
        .build_state
        .complete
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .is_some()
    {
        "✓ Built"
    } else if app.state.build_state.is_running {
        "◉ Building..."
    } else {
        "○ Not built"
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            build_status,
            status_style(build_status.starts_with("✓")),
        )))
        .block(Block::default().borders(Borders::NONE)),
        build_inner,
    );

    // Server status panel
    let srv_block = panel(Some("Server"));
    let srv_inner = srv_block.inner(chunks[1]);
    frame.render_widget(srv_block, chunks[1]);

    let server = &app.state.server_state;
    let (status_text, status_color) = match &server.status {
        ServerStatus::Running => ("● Running", COLOR_SUCCESS),
        ServerStatus::Starting => ("◉ Starting...", COLOR_WARNING),
        ServerStatus::Stopped => ("○ Stopped", COLOR_MUTED),
        ServerStatus::Stopping => ("◉ Stopping...", COLOR_WARNING),
        ServerStatus::Error(_e) => ("● Error", COLOR_ERROR),
    };

    let mut srv_lines = vec![Line::from(Span::styled(
        status_text,
        Style::default().fg(status_color),
    ))];

    if let Some(metrics) = &server.health {
        if metrics.tok_s > 0.0 {
            srv_lines.push(Line::from(Span::styled(
                format!("⚡ {:.1} tok/s", metrics.tok_s),
                Style::default().fg(COLOR_INFO),
            )));
        }
    }

    frame.render_widget(Paragraph::new(srv_lines), srv_inner);
}

fn render_models_preview(area: Rect, app: &App, frame: &mut Frame) {
    let block = panel(Some("Models"));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if app.state.models.is_empty() {
        frame.render_widget(
            Paragraph::new("No models found — press [2] to manage models")
                .style(Style::default().fg(COLOR_MUTED)),
            inner,
        );
        return;
    }

    let preview_count = (inner.height as usize)
        .saturating_sub(1)
        .min(app.state.models.len());
    let mut lines = Vec::with_capacity(preview_count);

    for model in app.state.models.iter().take(preview_count) {
        let star = if model.is_favorite { "★" } else { " " };
        let loaded = if model.is_loaded { " ✓" } else { "" };
        let name = if model.name.len() > 40 {
            format!("{}...", &model.name[..37])
        } else {
            model.name.clone()
        };
        lines.push(Line::from(vec![
            Span::styled(star, Style::default().fg(Color::Yellow)),
            Span::raw(" "),
            Span::styled(name, Style::default().fg(Color::White)),
            Span::raw(loaded.to_string()),
        ]));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}
