use crate::app::config::Config;
use crate::app::App;
use crate::tui::helpers::{panel, COLOR_ACCENT, COLOR_INFO, COLOR_MUTED};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

fn config_fields(config: &Config) -> Vec<(&'static str, &'static str, String)> {
    vec![
        (
            "General",
            "Model dirs",
            format!("{:?}", config.general.model_dirs),
        ),
        (
            "General",
            "Default model",
            config.general.default_model.clone(),
        ),
        ("General", "Theme", config.general.theme.clone()),
        (
            "Build",
            "llama.cpp path",
            config.build.llama_cpp_path.clone(),
        ),
        (
            "Build",
            "Extra flags",
            config.build.extra_cmake_flags.join(" "),
        ),
        ("Build", "Jobs", config.build.jobs.to_string()),
        ("Build", "Backend", config.build.backend.clone()),
        ("Server", "Port", config.server.port.to_string()),
        ("Server", "Context", config.server.context_size.to_string()),
        ("Server", "GPU layers", config.server.gpu_layers.to_string()),
        ("Server", "Threads", config.server.threads.to_string()),
        ("Server", "Batch size", config.server.batch_size.to_string()),
    ]
}

pub fn handle_event(key: crossterm::event::KeyEvent, app: &mut App) {
    use crossterm::event::KeyCode;
    let fields = config_fields(&app.state.config);
    let max_idx = fields.len().saturating_sub(1);

    if let Some(edit_idx) = app.state.settings_edit_field {
        // We're editing a specific field
        match key.code {
            KeyCode::Esc => {
                app.state.settings_edit_field = None;
                app.state.settings_edit_buffer.clear();
            }
            KeyCode::Enter => {
                // Commit edit
                let val = app.state.settings_edit_buffer.clone();
                apply_field(edit_idx, &val, &mut app.state.config);
                app.state.settings_edit_field = None;
                app.state.settings_edit_buffer.clear();
            }
            KeyCode::Char(c) => {
                app.state.settings_edit_buffer.push(c);
            }
            KeyCode::Backspace => {
                app.state.settings_edit_buffer.pop();
            }
            _ => {}
        }
    } else {
        // Navigating fields
        match key.code {
            KeyCode::Tab | KeyCode::Down => {
                let next = app.state.settings_selected.wrapping_add(1).min(max_idx);
                app.state.settings_selected = next;
            }
            KeyCode::BackTab | KeyCode::Up => {
                let prev = app.state.settings_selected.wrapping_sub(1).min(max_idx);
                app.state.settings_selected = prev;
            }
            KeyCode::Enter => {
                // Start editing the selected field
                let idx = app.state.settings_selected.min(max_idx);
                app.state.settings_edit_field = Some(idx);
                app.state.settings_edit_buffer = config_fields(&app.state.config)
                    .get(idx)
                    .map(|(_, _, v)| v.clone())
                    .unwrap_or_default();
            }
            KeyCode::Char('s') => {
                // Save config
                if crate::app::config::save_config(&app.state.config).is_ok() {
                    app.state.modal_message = "✓ Config saved to ~/.lmml/config.toml".to_string();
                } else {
                    app.state.modal_message = "✗ Failed to save config".to_string();
                }
                app.state.modal_active = true;
            }
            KeyCode::Esc if app.state.modal_active => {
                app.state.modal_active = false;
            }
            _ => {}
        }
    }
}

/// Apply an edited value to the in-memory config and persist to disk.
fn apply_field(idx: usize, val: &str, config: &mut Config) {
    match idx {
        0 => config.general.model_dirs = vec![val.to_string()],
        1 => config.general.default_model = val.to_string(),
        2 => {
            // Theme cycling with acceptable values
            let trimmed = val.trim().to_lowercase();
            if matches!(trimmed.as_str(), "auto" | "dark" | "light") {
                config.general.theme = trimmed;
            } else {
                config.general.theme = "auto".to_string();
            }
        }
        3 => config.build.llama_cpp_path = val.to_string(),
        4 => config.build.extra_cmake_flags = val.split_whitespace().map(String::from).collect(),
        5 => config.build.jobs = val.parse().unwrap_or(0),
        6 => {
            let trimmed = val.trim().to_lowercase();
            if matches!(
                trimmed.as_str(),
                "auto" | "cpu" | "cuda" | "rocm" | "vulkan" | "metal"
            ) {
                config.build.backend = trimmed;
            } else {
                config.build.backend = "auto".to_string();
            }
        }
        7 => config.server.port = val.parse().unwrap_or(8080),
        8 => config.server.context_size = val.parse().unwrap_or(8192),
        9 => config.server.gpu_layers = val.parse().unwrap_or(99),
        10 => config.server.threads = val.parse().unwrap_or(0),
        11 => config.server.batch_size = val.parse().unwrap_or(512),
        _ => return,
    }
    let _ = crate::app::config::save_config(config);
}

pub fn render(area: Rect, app: &App, frame: &mut Frame) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Length(6),
            Constraint::Length(8),
            Constraint::Min(3),
        ])
        .split(area);

    let fields = config_fields(&app.state.config);
    render_general(chunks[0], app, frame, 0..3);
    render_build_config(chunks[1], app, frame, 3..7);
    render_server_config(chunks[2], app, frame, 7..fields.len().min(12));
    render_about(chunks[3], app, frame);
}

fn render_fields(
    area: Rect,
    app: &App,
    frame: &mut Frame,
    field_range: std::ops::Range<usize>,
    title: &str,
) {
    let block = panel(Some(title));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let fields = config_fields(&app.state.config);
    let mut lines = Vec::new();

    for i in field_range.clone() {
        if i >= fields.len() {
            break;
        }
        let (_, key, val) = &fields[i];
        let selected = app.state.settings_selected == i;
        let editing = app.state.settings_edit_field == Some(i);
        let prefix = if selected { "▸ " } else { "  " };

        let display = if editing {
            format!("{}: {}", key, app.state.settings_edit_buffer)
        } else {
            format!("{}: {}", key, val)
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
            Style::default().fg(COLOR_INFO)
        };
        lines.push(Line::from(Span::styled(
            format!("{prefix}{display}"),
            style,
        )));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_general(area: Rect, app: &App, frame: &mut Frame, range: std::ops::Range<usize>) {
    render_fields(area, app, frame, range, "General");
}

fn render_build_config(area: Rect, app: &App, frame: &mut Frame, range: std::ops::Range<usize>) {
    render_fields(area, app, frame, range, "Build");
}

fn render_server_config(area: Rect, app: &App, frame: &mut Frame, range: std::ops::Range<usize>) {
    render_fields(area, app, frame, range, "Server Defaults");
}

fn render_about(area: Rect, _app: &App, frame: &mut Frame) {
    let block = panel(Some("About"));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines = vec![
        Line::from(Span::raw(format!("lmml v{}", env!("CARGO_PKG_VERSION")))),
        Line::from(Span::raw("Stack: Rust + ratatui + tokio")),
        Line::from(Span::raw("")),
        Line::from(Span::raw("A turnkey TUI for managing llama.cpp locally.")),
        Line::from(Span::raw("")),
        Line::from(Span::styled(
            "[Tab] Next field  [Enter] Edit  [s] Save  [Esc] Cancel",
            Style::default().fg(COLOR_MUTED),
        )),
    ];

    frame.render_widget(Paragraph::new(lines), inner);
}
