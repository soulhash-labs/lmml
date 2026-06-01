//! Tab routing and top-level layout.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Tabs};
use ratatui::Frame;

use crate::app::{App, Tab};

pub mod build;
pub mod detect;
pub mod models;
pub mod server;
pub mod settings;

/// Render the full application shell.
pub fn render(area: Rect, app: &App, frame: &mut Frame) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);

    render_tab_bar(layout[0], app, frame);
    match app.active_tab {
        Tab::Detect => detect::render(layout[1], app, frame),
        Tab::Build => build::render(layout[1], app, frame),
        Tab::Models => models::render(layout[1], app, frame),
        Tab::Server => server::render(layout[1], app, frame),
        Tab::Settings => settings::render(layout[1], app, frame),
    }
    crate::footer::render(layout[2], app, frame);
    if app.first_run_onboarding {
        crate::widgets::onboarding::render(area, frame);
    }
    if app.show_help {
        crate::widgets::help_overlay::render(area, frame);
    }
}

fn render_tab_bar(area: Rect, app: &App, frame: &mut Frame) {
    let titles = Tab::ALL
        .iter()
        .enumerate()
        .map(|(index, tab)| {
            Line::from(vec![
                Span::styled(
                    format!("{}", index + 1),
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!(" {}", tab.title())),
            ])
        })
        .collect::<Vec<_>>();
    let selected = Tab::ALL
        .iter()
        .position(|tab| *tab == app.active_tab)
        .unwrap_or_default();
    frame.render_widget(
        Tabs::new(titles)
            .select(selected)
            .block(Block::default().title("lmml").borders(Borders::ALL))
            .highlight_style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        area,
    );
}

/// Render a standard two-pane tab.
fn render_two_pane(area: Rect, left: Paragraph<'_>, right: Paragraph<'_>, frame: &mut Frame) {
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(area);
    frame.render_widget(left, panes[0]);
    frame.render_widget(right, panes[1]);
}

/// Create a bordered paragraph from lines.
fn pane<'a>(title: &'a str, lines: Vec<Line<'a>>) -> Paragraph<'a> {
    Paragraph::new(lines).block(Block::default().title(title).borders(Borders::ALL))
}

#[cfg(test)]
mod tests {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use super::*;

    #[test]
    fn renders_each_tab_and_help_overlay() {
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut app = App::default();

        for tab in Tab::ALL {
            app.active_tab = tab;
            terminal
                .draw(|frame| render(frame.area(), &app, frame))
                .expect("render tab");
        }

        app.show_help = true;
        terminal
            .draw(|frame| render(frame.area(), &app, frame))
            .expect("render help overlay");

        app.show_help = false;
        app.first_run_onboarding = true;
        terminal
            .draw(|frame| render(frame.area(), &app, frame))
            .expect("render onboarding");
    }
}
