//! Ratatui TUI infrastructure.
//!
//! Terminal initialization, event loop, and screen rendering dispatch.

pub mod build;
pub mod dashboard;
pub mod helpers;
pub mod models;
pub mod server;
pub mod settings;
pub mod widgets;

use crate::app::{App, Screen};
use color_eyre::Result;
use crossterm::event::{Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Terminal;
use std::io::stdout;
use std::time::Duration;

/// TUI wrapper that owns terminal state.
pub struct Tui {
    terminal: Terminal<CrosstermBackend<std::io::Stdout>>,
}

impl Tui {
    /// Initialize the terminal into alternate screen mode.
    pub fn new() -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let terminal = Terminal::new(CrosstermBackend::new(stdout))?;
        Ok(Tui { terminal })
    }

    /// Run the main event loop. Returns when the user quits.
    pub async fn run(&mut self, app: &mut App) -> Result<()> {
        let mut tick_interval = tokio::time::interval(Duration::from_millis(100));

        loop {
            tick_interval.tick().await;

            // Check for crossterm events
            if crossterm::event::poll(Duration::from_millis(10))? {
                if let Event::Key(key) = crossterm::event::read()? {
                    if key.kind == KeyEventKind::Press {
                        app.update(crate::app::Message::KeyEvent(key))?;
                    }
                }
            }

            // Tick for background channel draining
            app.update(crate::app::Message::Tick)?;

            // Check for quit (set by dispatch_key via 'q')
            if app.quit {
                break;
            }

            // Render
            self.terminal.draw(|frame| {
                let area = frame.area();
                render_screen(area, app, frame);
            })?;
        }

        Ok(())
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
    }
}

/// Dispatch rendering to the current screen's render function.
fn render_screen(area: Rect, app: &App, frame: &mut ratatui::Frame) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(area);

    let main_area = layout[0];
    let help_area = layout[1];

    match app.state.current_screen {
        Screen::Dashboard => dashboard::render(main_area, app, frame),
        Screen::Models => models::render(main_area, app, frame),
        Screen::Server => server::render(main_area, app, frame),
        Screen::Build => build::render(main_area, app, frame),
        Screen::Settings => settings::render(main_area, app, frame),
    }

    // Screen-specific help bar
    let help_items: Vec<(&'static str, &'static str)> = match app.state.current_screen {
        Screen::Dashboard => vec![
            ("1", "Dashboard"),
            ("2", "Models"),
            ("3", "Server"),
            ("4", "Build"),
            ("5", "Settings"),
            ("q", "Quit"),
        ],
        Screen::Models => vec![
            ("↑↓", "Select"),
            ("/", "Search"),
            ("d", "Download"),
            ("f", "Favorite"),
            ("Del", "Delete"),
            ("q", "Quit"),
        ],
        Screen::Server => vec![
            ("↑↓", "Field"),
            ("Enter", "Edit"),
            ("m", "Model"),
            ("Space", "Start/Stop"),
            ("q", "Quit"),
        ],
        Screen::Build => vec![
            ("b", "Build"),
            ("c", "Cancel"),
            ("r", "Re-detect"),
            ("q", "Quit"),
        ],
        Screen::Settings => vec![
            ("Tab", "Next"),
            ("Enter", "Edit"),
            ("s", "Save"),
            ("q", "Quit"),
        ],
    };
    let help = widgets::help_bar::HelpBar { items: help_items };
    help.render(help_area, frame);
    render_toast(area, app, frame);
}

fn render_toast(area: Rect, app: &App, frame: &mut ratatui::Frame) {
    if app.state.toast_ticks == 0 || app.state.toast_message.is_empty() || area.width < 24 {
        return;
    }

    let width = (app.state.toast_message.len() as u16 + 4)
        .min(area.width.saturating_sub(2))
        .max(24);
    let toast_area = Rect {
        x: area.x + area.width.saturating_sub(width + 1),
        y: area.y + 1,
        width,
        height: 3,
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .style(Style::default().fg(Color::Cyan).bg(Color::Black));
    frame.render_widget(
        Paragraph::new(app.state.toast_message.clone())
            .alignment(Alignment::Center)
            .block(block),
        toast_area,
    );
}
