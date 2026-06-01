//! Server tab rendering.

use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::Frame;

use crate::app::App;

/// Render the server tab.
pub fn render(area: Rect, app: &App, frame: &mut Frame) {
    let left = vec![
        Line::from("Press s to start server."),
        Line::from(format!("Status: {:?}", app.server_status)),
        Line::from(format!("Host: {}", app.state.server.host)),
        Line::from(format!("Port: {}", app.state.server.port)),
        Line::from(format!("Context: {}", app.state.server.ctx_size)),
    ];
    let right = if app.server_log.is_empty() {
        vec![Line::from("Server log will appear here.")]
    } else {
        app.server_log.iter().cloned().map(Line::from).collect()
    };
    super::render_two_pane(
        area,
        super::pane("Server", left),
        super::pane("Server Log", right),
        frame,
    );
}
