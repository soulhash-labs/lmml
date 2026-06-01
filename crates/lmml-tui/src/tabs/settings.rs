//! Settings tab rendering.

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::app::{App, SettingsField};

/// Render the settings tab.
pub fn render(area: Rect, app: &App, frame: &mut Frame) {
    let mut left = vec![
        Line::from("Up/Down select. e edits. Space toggles. s saves. p probes flags."),
        Line::from(format!(
            "State path: {}",
            lmml_state::AppState::path().display()
        )),
        Line::from(""),
    ];
    for field in SettingsField::ALL {
        left.push(settings_line(app, field));
    }

    let mut right = vec![Line::from(format!(
        "Binary: {}",
        app.state.build.binary.display()
    ))];
    if let Some(caps) = &app.server_caps {
        right.push(Line::from(format!(
            "Version: {}",
            caps.version.as_deref().unwrap_or("unknown")
        )));
        right.push(Line::from(format!("Flags parsed: {}", caps.flags.len())));
    } else if let Some(error) = &app.server_caps_error {
        right.push(Line::from(format!("Probe failed: {error}")));
    } else {
        right.push(Line::from("Press p to probe llama-server capabilities."));
    }
    right.push(Line::from(""));
    let warnings = app.server_compat_warnings();
    if warnings.is_empty() {
        right.push(Line::from("No unsupported flag warnings."));
    } else {
        right.push(Line::from("Unsupported settings:"));
        for warning in warnings {
            right.push(Line::from(format!("! {warning}")));
        }
    }

    super::render_two_pane(
        area,
        super::pane("Settings", left),
        super::pane("Compatibility", right),
        frame,
    );

    if let Some(buffer) = &app.settings_edit_buffer {
        render_edit_modal(area, app.selected_settings_field, buffer, frame);
    }
}

fn settings_line(app: &App, field: SettingsField) -> Line<'static> {
    let selected = field == app.selected_settings_field;
    let marker = if selected { "> " } else { "  " };
    let action = if field.is_bool() { "Space" } else { "e" };
    Line::from(vec![
        Span::styled(
            marker,
            Style::default()
                .fg(if selected { Color::Cyan } else { Color::Gray })
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{:<14}", field.label()),
            Style::default().fg(if selected { Color::White } else { Color::Gray }),
        ),
        Span::raw(format!("{}  [{}]", app.settings_field_value(field), action)),
    ])
}

fn render_edit_modal(area: Rect, field: SettingsField, buffer: &str, frame: &mut Frame) {
    let modal = centered_rect(58, 28, area);
    frame.render_widget(Clear, modal);
    let lines = vec![
        Line::from(format!("{}:", field.label())),
        Line::from(""),
        Line::from(buffer.to_string()),
        Line::from(""),
        Line::from("Enter applies. Esc cancels."),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Left)
            .block(Block::default().title("Edit Setting").borders(Borders::ALL))
            .style(Style::default().fg(Color::White)),
        modal,
    );
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1]);
    horizontal[1]
}
