//! Help overlay widget.

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

/// Render the global keybinding help overlay.
pub fn render(area: Rect, frame: &mut Frame) {
    let area = centered_rect(62, 70, area);
    frame.render_widget(Clear, area);
    let lines = vec![
        Line::from("1-5        switch tabs"),
        Line::from("Tab        next tab"),
        Line::from("Shift+Tab  previous tab"),
        Line::from("d          run detect"),
        Line::from("b          start build"),
        Line::from("B          clean build"),
        Line::from("u          check for update"),
        Line::from("s          start server / save settings"),
        Line::from("/          open HF search"),
        Line::from("a          add model alias"),
        Line::from("?          close help"),
        Line::from("q/Ctrl+C   quit"),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Left)
            .block(Block::default().title("Help").borders(Borders::ALL))
            .style(Style::default().fg(Color::White)),
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
