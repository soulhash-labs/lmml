//! Server tab rendering.

use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::Frame;

use crate::app::{App, ServerStatus};

/// Render the server tab.
pub fn render(area: Rect, app: &App, frame: &mut Frame) {
    let action = match app.server_status {
        ServerStatus::Stopped | ServerStatus::Failed { .. } => "Press s to start server.",
        ServerStatus::Starting { .. } | ServerStatus::Ready { .. } => "Press s to stop server.",
    };
    let model = app
        .selected_server_model()
        .map(|model| model.path.display().to_string())
        .unwrap_or_else(|| "No model selected".to_string());
    let left = vec![
        Line::from(action),
        Line::from(format!("Status: {:?}", app.server_status)),
        Line::from(format!("Model: {model}")),
        Line::from(format!("Binary: {}", app.state.build.binary.display())),
        Line::from(format!("Host: {}", app.state.server.host)),
        Line::from(format!("Port: {}", app.state.server.port)),
        Line::from(format!("Context: {}", app.state.server.ctx_size)),
        Line::from(format!("GPU layers: {}", app.state.server.n_gpu_layers)),
        Line::from(format!("Batch: {}", app.state.server.batch_size)),
        Line::from(format!("Threads: {}", app.state.server.threads)),
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
