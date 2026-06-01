use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, BorderType, Borders};

pub const COLOR_SUCCESS: Color = Color::Green;
pub const COLOR_WARNING: Color = Color::Yellow;
pub const COLOR_ERROR: Color = Color::Red;
pub const COLOR_INFO: Color = Color::Cyan;
pub const COLOR_MUTED: Color = Color::DarkGray;
pub const COLOR_ACCENT: Color = Color::Magenta;

pub fn status_style(ready: bool) -> Style {
    if ready {
        Style::default()
            .fg(COLOR_SUCCESS)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(COLOR_MUTED)
    }
}

pub fn panel<'a>(title: Option<&'a str>) -> Block<'a> {
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded);
    if let Some(t) = title {
        block = block.title(format!(" {t} "));
    }
    block
}

pub fn centered_line<'a>(text: &'a str, width: u16) -> Line<'a> {
    let text_width = text.len() as u16;
    if text_width >= width {
        return Line::raw(text.to_string());
    }
    let padding = ((width - text_width) / 2) as usize;
    let line = format!("{:padding$}{text}", "");
    Line::raw(line)
}

pub fn truncate(s: &str, max_width: u16) -> String {
    if s.len() as u16 <= max_width {
        s.to_string()
    } else {
        let max = max_width.saturating_sub(3) as usize;
        format!("{}...", &s[..max])
    }
}
