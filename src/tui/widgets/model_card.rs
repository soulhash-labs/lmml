use crate::app::state::ModelEntry;
use crate::models::types::format_size;
use crate::tui::helpers::{truncate, COLOR_ACCENT, COLOR_MUTED, COLOR_SUCCESS};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

/// Detail widget for a single local GGUF model.
pub struct ModelCard<'a> {
    pub model: &'a ModelEntry,
}

impl ModelCard<'_> {
    pub fn render(&self, area: Rect, frame: &mut Frame) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let name_width = area.width.saturating_sub(2);
        let loaded = if self.model.is_loaded {
            "loaded"
        } else {
            "available"
        };
        let favorite = if self.model.is_favorite {
            "favorite"
        } else {
            "not favorite"
        };

        let lines = vec![
            Line::from(Span::styled(
                truncate(&self.model.name, name_width),
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(loaded, Style::default().fg(COLOR_SUCCESS))),
            Line::from(Span::styled(favorite, Style::default().fg(COLOR_ACCENT))),
            Line::from(Span::raw(format!("Type:   {}", self.model.model_type))),
            Line::from(Span::raw(format!("Params: {}", self.model.param_count))),
            Line::from(Span::raw(format!("Quant:  {}", self.model.quantization))),
            Line::from(Span::raw(format!(
                "Size:   {}",
                format_size(self.model.size_bytes)
            ))),
            Line::from(Span::styled(
                format!("Path:   {}", self.model.path),
                Style::default().fg(COLOR_MUTED),
            )),
        ];

        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), area);
    }
}
