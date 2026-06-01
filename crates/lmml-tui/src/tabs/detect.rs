//! Detect tab rendering.

use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::Frame;

use crate::app::App;

/// Render the hardware detection tab.
pub fn render(area: Rect, app: &App, frame: &mut Frame) {
    let mut left = vec![Line::from("Press d to run hardware detection.")];
    if let Some(profile) = &app.state.system_profile {
        left.push(Line::from(format!("GPUs: {}", profile.gpu_names.len())));
        left.push(Line::from(format!("sccache: {}", profile.sccache)));
        if !profile.gpu_archs.is_empty() {
            left.push(Line::from(format!(
                "CUDA archs: {}",
                profile.gpu_archs.join(", ")
            )));
        }
    }
    let right = if app.detect_log.is_empty() {
        vec![Line::from("Detection log will appear here.")]
    } else {
        app.detect_log.iter().cloned().map(Line::from).collect()
    };
    super::render_two_pane(
        area,
        super::pane("System", left),
        super::pane("Probe Log", right),
        frame,
    );
}
