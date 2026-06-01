//! Models tab rendering.

use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::Frame;

use crate::app::App;

/// Render the models tab.
pub fn render(area: Rect, app: &App, frame: &mut Frame) {
    let left = vec![
        Line::from("Press / to search Hugging Face."),
        Line::from("Press a to add a model alias."),
        Line::from(format!(
            "Models dir: {}",
            app.state.model.models_dir.display()
        )),
    ];
    let right = vec![Line::from(format!(
        "Last model: {}",
        app.state.model.last_used.display()
    ))];
    super::render_two_pane(
        area,
        super::pane("Models", left),
        super::pane("Model Details", right),
        frame,
    );
}
