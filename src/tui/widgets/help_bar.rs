use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// Footer keybinding help bar.
pub struct HelpBar {
    pub items: Vec<(&'static str, &'static str)>,
}

impl HelpBar {
    pub fn render(&self, area: Rect, frame: &mut Frame) {
        if area.width < 10 {
            return;
        }

        let mut spans = Vec::new();
        for (i, (key, desc)) in self.items.iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw("  "));
            }
            spans.push(Span::styled(
                format!("[{key}]"),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(
                format!(" {desc}"),
                Style::default().fg(Color::DarkGray),
            ));
        }

        let line = Line::from(spans);
        frame.render_widget(Paragraph::new(line), area);
    }
}
