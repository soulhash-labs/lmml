//! First-run onboarding modal.

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::app::{App, OnboardingStep};

/// Render the first-run onboarding modal.
pub fn render(area: Rect, app: &App, frame: &mut Frame) {
    let area = centered_rect(58, 48, area);
    frame.render_widget(Clear, area);
    let mut lines = onboarding_lines(app);
    if let Some(error) = &app.onboarding_error {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("Error: {error}"),
            Style::default().fg(Color::Red),
        )));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .block(Block::default().title("First run").borders(Borders::ALL)),
        area,
    );
}

fn onboarding_lines(app: &App) -> Vec<Line<'static>> {
    let title = Line::from(Span::styled(
        "Welcome to lmml",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ));
    match app.onboarding_step {
        OnboardingStep::Scan => vec![
            title,
            Line::from(""),
            Line::from("Step 1 of 7: scan this system."),
            Line::from(""),
            Line::from("Press Enter or d to scan prerequisites."),
            Line::from("Press Esc to leave onboarding."),
        ],
        OnboardingStep::HardwareSummary => {
            let backend = app
                .detect_profile
                .as_ref()
                .map(|profile| format!("{:?}", profile.recommended_backend()))
                .unwrap_or_else(|| "unknown".to_string());
            vec![
                title,
                Line::from(""),
                Line::from("Step 2 of 7: detected hardware."),
                Line::from(format!("Recommended backend: {backend}")),
                Line::from(format!(
                    "GPUs: {}",
                    app.detect_profile
                        .as_ref()
                        .map(|profile| profile.gpus.len())
                        .unwrap_or_default()
                )),
                Line::from(""),
                Line::from("Press Enter to confirm backend."),
            ]
        }
        OnboardingStep::Backend => vec![
            title,
            Line::from(""),
            Line::from("Step 3 of 7: choose backend."),
            Line::from(format!(
                "Selected: {:?}",
                app.onboarding_backend
                    .clone()
                    .unwrap_or(lmml_detect::BuildBackend::CpuFallback)
            )),
            Line::from(""),
            Line::from("Left/Right cycles. Enter confirms."),
        ],
        OnboardingStep::ModelsDir => vec![
            title,
            Line::from(""),
            Line::from("Step 4 of 7: choose models directory."),
            Line::from(""),
            Line::from(app.onboarding_models_dir_buffer.clone()),
            Line::from(""),
            Line::from("Type path. Enter confirms."),
        ],
        OnboardingStep::StarterModel => vec![
            title,
            Line::from(""),
            Line::from("Step 5 of 7: starter model."),
            Line::from(""),
            Line::from("Press / or d to search Hugging Face."),
            Line::from("Press Enter to skip."),
        ],
        OnboardingStep::ServerPort => vec![
            title,
            Line::from(""),
            Line::from("Step 6 of 7: server port."),
            Line::from(""),
            Line::from(app.onboarding_port_buffer.clone()),
            Line::from(""),
            Line::from("Type port. Enter confirms."),
        ],
        OnboardingStep::Done => vec![
            title,
            Line::from(""),
            Line::from("Step 7 of 7: done."),
            Line::from(""),
            Line::from("lmml is ready for build, models, and server management."),
            Line::from("Press Enter to open Build."),
        ],
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1]);
    horizontal[1]
}
