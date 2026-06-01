//! First-run onboarding modal.

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

/// Render the first-run onboarding modal.
pub fn render(area: Rect, frame: &mut Frame) {
    let area = centered_rect(58, 48, area);
    frame.render_widget(Clear, area);
    let lines = vec![
        Line::from(Span::styled(
            "Welcome to lmml",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("Scan your system, build llama.cpp for this hardware,"),
        Line::from("then manage local GGUF models and the server here."),
        Line::from(""),
        Line::from("Press d to scan prerequisites."),
        Line::from("Press Enter to go straight to Build."),
        Line::from("Press Esc to dismiss this guide."),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .block(Block::default().title("First run").borders(Borders::ALL)),
        area,
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
