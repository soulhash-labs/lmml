use crate::app::App;
use crate::tui::helpers::{panel, COLOR_ACCENT, COLOR_MUTED, COLOR_SUCCESS};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

pub fn handle_event(key: crossterm::event::KeyEvent, app: &mut App) {
    use crossterm::event::KeyCode;

    // If search mode is active, capture text input
    if app.state.models_search_active {
        return handle_search_input(key, app);
    }

    match key.code {
        KeyCode::Down => {
            move_selection(&mut app.state, 1);
        }
        KeyCode::Up => {
            move_selection(&mut app.state, -1);
        }
        KeyCode::Char('d') if !app.state.modal_active => {
            app.state.modal_active = true;
            app.state.modal_input.clear();
            app.state.modal_message =
                "Enter model ID (e.g. ggml-org/gemma-3-1b-it-GGUF):".to_string();
        }
        KeyCode::Char('f') if !app.state.modal_active => {
            if let Some(model) = app.state.models.get_mut(app.state.selected_model) {
                model.is_favorite = !model.is_favorite;
            }
        }
        KeyCode::Delete if !app.state.modal_active => {
            if app.state.selected_model < app.state.models.len() {
                app.state.models.remove(app.state.selected_model);
                app.state.selected_model = app.state.selected_model.saturating_sub(1);
            }
        }
        KeyCode::Char('/') if !app.state.modal_active => {
            app.state.models_search_active = true;
            app.state.search_query.clear();
        }
        KeyCode::Char('s') if !app.state.modal_active => {
            use crate::app::state::ModelsSort;
            app.state.models_sort_by = match app.state.models_sort_by {
                ModelsSort::Name => ModelsSort::Size,
                ModelsSort::Size => ModelsSort::Name,
            };
        }
        KeyCode::Esc if app.state.modal_active => {
            app.state.modal_active = false;
            app.state.modal_input.clear();
        }
        KeyCode::Esc if app.state.models_search_active => {
            app.state.models_search_active = false;
            app.state.search_query.clear();
        }
        KeyCode::Enter if app.state.modal_active => {
            let model_id = app.state.modal_input.clone();
            app.state.modal_active = false;
            app.state.modal_input.clear();
            if !model_id.is_empty() {
                let dest_dir = crate::app::config::models_dir();
                let tx = app.download_tx.clone();
                app.state.modal_message = format!("Downloading {model_id}...");
                tokio::spawn(async move {
                    let result =
                        crate::models::download::download_model(&model_id, &dest_dir, tx.clone())
                            .await;
                    if let Err(e) = result {
                        let _ = tx
                            .send(crate::models::DownloadEvent::Complete(Err(e)))
                            .await;
                    }
                });
            }
        }
        KeyCode::Char(c) if app.state.modal_active => {
            app.state.modal_input.push(c);
        }
        KeyCode::Backspace if app.state.modal_active => {
            app.state.modal_input.pop();
        }
        _ => {}
    }
}

/// Handle text input while search mode is active.
fn handle_search_input(key: crossterm::event::KeyEvent, app: &mut App) {
    use crossterm::event::KeyCode;
    match key.code {
        KeyCode::Esc => {
            app.state.models_search_active = false;
            app.state.search_query.clear();
        }
        KeyCode::Enter | KeyCode::Down => {
            app.state.models_search_active = false;
            let list = filtered_models(
                &app.state.models,
                &app.state.search_query,
                app.state.models_sort_by,
            );
            if let Some(m) = list.first() {
                if let Some(pos) = app
                    .state
                    .models
                    .iter()
                    .position(|model| model.path == m.path)
                {
                    app.state.selected_model = pos;
                }
            }
        }
        KeyCode::Char(c) => {
            app.state.search_query.push(c);
        }
        KeyCode::Backspace => {
            app.state.search_query.pop();
        }
        _ => {}
    }
}

/// Move the model selection by `delta` steps through the filtered list.
fn move_selection(state: &mut crate::app::state::AppState, delta: isize) {
    let list = filtered_models(&state.models, &state.search_query, state.models_sort_by);
    if list.is_empty() {
        return;
    }
    // Find current position in the filtered list
    let current = list
        .iter()
        .position(|m| {
            state
                .models
                .get(state.selected_model)
                .is_some_and(|sel| m.path == sel.path)
        })
        .unwrap_or(0);
    // Compute new position with wrapping
    let len = list.len() as isize;
    let new_idx = ((current as isize + delta).rem_euclid(len)) as usize;
    if let Some(m) = list.get(new_idx) {
        if let Some(pos) = state.models.iter().position(|model| model.path == m.path) {
            state.selected_model = pos;
        }
    }
}

fn filtered_models<'a>(
    models: &'a [crate::app::state::ModelEntry],
    query: &str,
    sort_by: crate::app::state::ModelsSort,
) -> Vec<&'a crate::app::state::ModelEntry> {
    let q = query.to_lowercase();
    let filters = SearchFilters::parse(&q);
    let mut result: Vec<_> = if q.is_empty() {
        models.iter().collect()
    } else {
        models.iter().filter(|m| filters.matches(m)).collect()
    };
    result.sort_by_key(|m| {
        let fav = if m.is_favorite { 0u8 } else { 1u8 };
        let sort = match sort_by {
            crate::app::state::ModelsSort::Name => m.name.clone(),
            crate::app::state::ModelsSort::Size => format!("{:020}", m.size_bytes),
        };
        (fav, sort)
    });
    result
}

struct SearchFilters {
    text: Vec<String>,
    quant: Option<String>,
    model_type: Option<String>,
    min_size: Option<u64>,
    max_size: Option<u64>,
}

impl SearchFilters {
    fn parse(query: &str) -> Self {
        let mut filters = SearchFilters {
            text: Vec::new(),
            quant: None,
            model_type: None,
            min_size: None,
            max_size: None,
        };

        for token in query.split_whitespace() {
            if let Some(quant) = token.strip_prefix("quant:") {
                filters.quant = Some(quant.to_string());
            } else if let Some(model_type) = token.strip_prefix("type:") {
                filters.model_type = Some(model_type.to_string());
            } else if let Some(size) = token.strip_prefix("size>") {
                filters.min_size = parse_size_filter(size);
            } else if let Some(size) = token.strip_prefix("size<") {
                filters.max_size = parse_size_filter(size);
            } else {
                filters.text.push(token.to_string());
            }
        }

        filters
    }

    fn matches(&self, model: &crate::app::state::ModelEntry) -> bool {
        let name = model.name.to_lowercase();
        let quant = model.quantization.to_lowercase();
        let model_type = model.model_type.to_lowercase();

        if let Some(filter) = &self.quant {
            if !quant.contains(filter) {
                return false;
            }
        }
        if let Some(filter) = &self.model_type {
            if !model_type.contains(filter) {
                return false;
            }
        }
        if let Some(min_size) = self.min_size {
            if model.size_bytes < min_size {
                return false;
            }
        }
        if let Some(max_size) = self.max_size {
            if model.size_bytes > max_size {
                return false;
            }
        }

        self.text
            .iter()
            .all(|term| name.contains(term) || quant.contains(term) || model_type.contains(term))
    }
}

fn parse_size_filter(value: &str) -> Option<u64> {
    let digits: String = value
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let suffix = value[digits.len()..].trim();
    let amount = digits.parse::<f64>().ok()?;
    let multiplier = match suffix {
        "gb" | "g" => 1024_f64.powi(3),
        "mb" | "m" => 1024_f64.powi(2),
        "kb" | "k" => 1024_f64,
        "" | "b" => 1.0,
        _ => return None,
    };
    Some((amount * multiplier) as u64)
}

pub fn render(area: Rect, app: &App, frame: &mut Frame) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);

    render_model_list(chunks[0], app, frame);
    render_model_detail(chunks[1], app, frame);

    // Download progress overlay
    if app.state.modal_active {
        let modal_area = centered_rect(60, 30, area);
        let block = Block::default()
            .title(" Download Model ")
            .borders(Borders::ALL)
            .style(Style::default().bg(Color::Black));
        let inner = block.inner(modal_area);

        let input_display = if app.state.modal_input.is_empty() {
            app.state.modal_message.clone()
        } else {
            format!("{}\n> {}", app.state.modal_message, app.state.modal_input)
        };
        frame.render_widget(block, modal_area);
        frame.render_widget(
            Paragraph::new(input_display).wrap(Wrap { trim: true }),
            inner,
        );

        if app.state.download_state.total > 0 {
            let pb_area = Rect {
                x: inner.x,
                y: inner.y + inner.height.saturating_sub(1),
                width: inner.width,
                height: 1,
            };
            let speed_str =
                crate::models::types::format_size(app.state.download_state.speed as u64);
            let eta = app.state.download_state.eta_secs;
            let label = if eta > 0.0 {
                let eta_mins = (eta / 60.0) as u64;
                let eta_secs = (eta % 60.0) as u64;
                format!("{speed_str}/s — ETA: {eta_mins}m{eta_secs}s")
            } else {
                format!("{speed_str}/s")
            };
            let pb = crate::tui::widgets::progress_bar::ProgressBar {
                current: app.state.download_state.bytes as u32,
                total: app.state.download_state.total as u32,
                label,
            };
            pb.render(pb_area, frame);
        }
    }
}

fn render_model_list(area: Rect, app: &App, frame: &mut Frame) {
    let block = panel(Some("Models"));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Search bar
    let search_text = if app.state.models_search_active {
        format!("/{}_", app.state.search_query)
    } else if !app.state.search_query.is_empty() {
        format!("/{}", app.state.search_query)
    } else {
        String::new()
    };
    let search_line = Line::from(Span::styled(
        if search_text.is_empty() {
            "Press [/] to search".to_string()
        } else {
            search_text
        },
        Style::default().fg(COLOR_MUTED),
    ));

    let list = filtered_models(
        &app.state.models,
        &app.state.search_query,
        app.state.models_sort_by,
    );
    // Disk usage summary
    let total_size: u64 = app.state.models.iter().map(|m| m.size_bytes).sum();
    let disk_line = Line::from(Span::styled(
        format!(
            "{} models — {}",
            app.state.models.len(),
            crate::models::types::format_size(total_size)
        ),
        Style::default().fg(COLOR_MUTED),
    ));

    if list.is_empty() {
        frame.render_widget(
            Paragraph::new(vec![
                search_line,
                disk_line,
                Line::from(Span::styled(
                    if app.state.search_query.is_empty() {
                        "No .gguf models found. Press [d] to download from HuggingFace."
                    } else {
                        "No models match your search."
                    },
                    Style::default().fg(COLOR_MUTED),
                )),
            ]),
            inner,
        );
        return;
    }

    let visible = (inner.height as usize).saturating_sub(2);
    let mut lines = Vec::with_capacity(visible + 1);

    lines.push(search_line);

    // Find selected model position in filtered list
    let selected_in_filtered = list
        .iter()
        .position(|m| {
            app.state
                .models
                .get(app.state.selected_model)
                .is_some_and(|sel| m.path == sel.path)
        })
        .unwrap_or(0);

    for (i, model) in list.iter().enumerate().take(visible) {
        let selected = i == selected_in_filtered;
        let prefix = if selected { "▸ " } else { "  " };
        let star = if model.is_favorite { "★ " } else { "  " };
        let size = crate::models::types::format_size(model.size_bytes);

        let style = if selected {
            Style::default()
                .fg(COLOR_ACCENT)
                .add_modifier(Modifier::BOLD)
        } else if model.is_loaded {
            Style::default().fg(COLOR_SUCCESS)
        } else {
            Style::default()
        };

        let name = crate::tui::helpers::truncate(&model.name, inner.width.saturating_sub(20));
        lines.push(Line::from(vec![
            Span::styled(prefix, style),
            Span::styled(star, Style::default().fg(Color::Yellow)),
            Span::styled(name, style),
            Span::raw("  "),
            Span::styled(size, Style::default().fg(COLOR_MUTED)),
        ]));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_model_detail(area: Rect, app: &App, frame: &mut Frame) {
    let block = panel(Some("Details"));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if app.state.models.is_empty() {
        return;
    }

    if let Some(model) = app.state.models.get(app.state.selected_model) {
        crate::tui::widgets::model_card::ModelCard { model }.render(inner, frame);
    }
}

/// Helper: create a centered rect within a larger rect.
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
