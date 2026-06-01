use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// A progress bar widget showing ▰▰▰▱▱ style fill.
pub struct ProgressBar {
    pub current: u32,
    pub total: u32,
    pub label: String,
}

impl ProgressBar {
    pub fn render(&self, area: Rect, frame: &mut Frame) {
        if area.width < 10 || self.total == 0 {
            return;
        }

        let pct = (self.current as f64 / self.total as f64 * 100.0) as u32;
        let bar_width = (area.width as usize).saturating_sub(12);
        let filled = (bar_width as f64 * pct as f64 / 100.0) as usize;
        let empty = bar_width.saturating_sub(filled);

        let mut bar = String::with_capacity(bar_width + 10);
        bar.push('[');
        for _ in 0..filled {
            bar.push('▰');
        }
        for _ in 0..empty {
            bar.push('▱');
        }
        bar.push(']');
        bar.push_str(&format!(" {pct}%"));

        let color = if pct < 50 {
            Color::Yellow
        } else if pct < 90 {
            Color::Cyan
        } else {
            Color::Green
        };

        let line = Line::from(vec![
            Span::styled(self.label.clone(), Style::default().fg(Color::DarkGray)),
            Span::raw(" "),
            Span::styled(bar, Style::default().fg(color)),
        ]);

        frame.render_widget(Paragraph::new(line), area);
    }
}
