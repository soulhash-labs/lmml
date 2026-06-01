use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::Span;

use ratatui::Frame;

/// A colored status pill indicator.
pub struct StatusBadge {
    pub label: String,
    pub status: Status,
}

pub enum Status {
    Ready,
    Busy,
    Error,
    Muted,
}

impl StatusBadge {
    pub fn render(&self, area: Rect, frame: &mut Frame) {
        if area.width < self.label.len() as u16 + 4 {
            return;
        }

        let color = match self.status {
            Status::Ready => Color::Green,
            Status::Busy => Color::Yellow,
            Status::Error => Color::Red,
            Status::Muted => Color::DarkGray,
        };

        let style = Style::default().fg(color).bg(Color::Reset);
        let symbol = match self.status {
            Status::Ready => "●",
            Status::Busy => "◉",
            Status::Error => "●",
            Status::Muted => "○",
        };

        let span = Span::styled(format!(" {symbol} {} ", self.label), style);
        frame.render_widget(span, area);
    }
}
