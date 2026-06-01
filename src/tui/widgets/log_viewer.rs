use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

/// A scrollable log output viewer.
pub struct LogViewer {
    pub lines: Vec<String>,
    pub scroll_offset: usize,
}

impl LogViewer {
    pub fn render(&self, area: Rect, frame: &mut Frame) {
        if area.height < 2 {
            return;
        }

        let visible_lines = (area.height as usize).saturating_sub(2);
        let offset = self
            .scroll_offset
            .min(self.lines.len().saturating_sub(visible_lines));

        let displayed: Vec<Line> = self
            .lines
            .iter()
            .skip(offset)
            .take(visible_lines)
            .map(|line| {
                let (style, text) = if line.contains("error") || line.contains("Error") {
                    (Style::default().fg(Color::Red), line.clone())
                } else if line.contains("warning") || line.contains("Warning") {
                    (Style::default().fg(Color::Yellow), line.clone())
                } else {
                    (Style::default().fg(Color::Reset), line.clone())
                };
                Line::from(Span::styled(text, style))
            })
            .collect();

        let block = Block::default().borders(Borders::ALL).title(" Log ");

        let inner = block.inner(area);
        frame.render_widget(block, area);

        if !displayed.is_empty() {
            let para = Paragraph::new(displayed);
            frame.render_widget(para, inner);
        }
    }

    pub fn scroll_up(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_sub(3);
    }

    pub fn scroll_down(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_add(3);
    }

    pub fn scroll_to_bottom(&mut self) {
        self.scroll_offset = usize::MAX;
    }
}
