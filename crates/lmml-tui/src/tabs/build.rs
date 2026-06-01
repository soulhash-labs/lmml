//! Build tab rendering.

use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::Frame;

use crate::app::App;

/// Render the llama.cpp build tab.
pub fn render(area: Rect, app: &App, frame: &mut Frame) {
    let update = app
        .update_check
        .as_ref()
        .map(|update| format!("{update:?}"))
        .unwrap_or_else(|| "not checked".to_string());
    let left = vec![
        Line::from("Press b to build llama.cpp."),
        Line::from("Press u to check for updates."),
        Line::from(format!("Source: {}", app.state.build.source_dir.display())),
        Line::from(format!("Backend: {}", app.state.build.backend)),
        Line::from(format!("Update: {update}")),
    ];
    let right = if app.build_log.is_empty() {
        vec![Line::from("Build output will appear here.")]
    } else {
        app.build_log.iter().cloned().map(Line::from).collect()
    };
    super::render_two_pane(
        area,
        super::pane("Build", left),
        super::pane("Build Log", right),
        frame,
    );
}
