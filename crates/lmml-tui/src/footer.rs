//! Footer rendering for global status and key hints.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::{App, ServerStatus};

/// Render the global footer.
pub fn render(area: Rect, app: &App, frame: &mut Frame) {
    if area.height == 0 {
        return;
    }

    let server = match &app.server_status {
        ServerStatus::Stopped => "Server: stopped".to_string(),
        ServerStatus::Starting { elapsed } => format!("Server: starting {}s", elapsed.as_secs()),
        ServerStatus::Ready { url } => format!("Server: ready {url}"),
        ServerStatus::Failed { reason } => format!("Server: failed {reason}"),
    };

    let line = Line::from(vec![
        Span::styled(
            "lmml",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(server, Style::default().fg(Color::Cyan)),
        Span::raw("  "),
        Span::styled(&app.status_message, Style::default().fg(Color::Gray)),
        Span::raw("  [?] Help  [q] Quit"),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}
