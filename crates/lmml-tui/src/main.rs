//! lmml TUI binary entry point.

use std::io::{self, stdout};

use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use lmml_tui::app::App;
use lmml_tui::event_loop::EventLoop;
use lmml_tui::tabs;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut terminal = init_terminal()?;
    let result = run(&mut terminal).await;
    restore_terminal()?;
    result
}

async fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut app = App::new();
    let mut event_loop = EventLoop::new();
    let mut frame_tick = tokio::time::interval(std::time::Duration::from_millis(16));

    loop {
        frame_tick.tick().await;
        event_loop.tick(&mut app).await?;
        terminal.draw(|frame| tabs::render(frame.area(), &app, frame))?;
        if app.should_quit {
            break;
        }
    }
    Ok(())
}

fn init_terminal() -> io::Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen)?;
    Terminal::new(CrosstermBackend::new(stdout))
}

fn restore_terminal() -> io::Result<()> {
    disable_raw_mode()?;
    execute!(stdout(), LeaveAlternateScreen)
}
