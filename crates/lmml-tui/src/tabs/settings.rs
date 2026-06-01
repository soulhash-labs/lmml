//! Settings tab rendering.

use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::Frame;

use crate::app::App;

/// Render the settings tab.
pub fn render(area: Rect, app: &App, frame: &mut Frame) {
    let left = vec![
        Line::from("Press s to save settings."),
        Line::from(format!(
            "State path: {}",
            lmml_state::AppState::path().display()
        )),
        Line::from(format!("flash_attn: {}", app.state.server.flash_attn)),
        Line::from(format!("mlock: {}", app.state.server.mlock)),
        Line::from(format!("jinja: {}", app.state.server.jinja)),
    ];
    let right = vec![
        Line::from("Unsupported flag warnings will appear here."),
        Line::from("Inline editing lands in a later milestone."),
    ];
    super::render_two_pane(
        area,
        super::pane("Settings", left),
        super::pane("Compatibility", right),
        frame,
    );
}
