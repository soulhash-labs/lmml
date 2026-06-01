//! Models tab rendering.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::Frame;

use crate::app::App;

/// Render the models tab.
pub fn render(area: Rect, app: &App, frame: &mut Frame) {
    let mut left = vec![
        Line::from("Press r to scan local models."),
        Line::from("Press / to search Hugging Face."),
        Line::from("Press a to add a model alias."),
        Line::from(format!(
            "Models dir: {}",
            app.state.model.models_dir.display()
        )),
        Line::from(""),
    ];
    if app.models.is_empty() {
        left.push(Line::from("No models found."));
    } else {
        for (index, model) in app.models.iter().enumerate() {
            let selected = index == app.selected_model;
            let marker = if selected { "> " } else { "  " };
            let fit = model
                .vram_fit(
                    &app.detect_profile
                        .as_ref()
                        .map(|profile| profile.gpus.clone())
                        .unwrap_or_default(),
                )
                .label();
            left.push(Line::from(vec![
                Span::styled(
                    marker,
                    Style::default()
                        .fg(if selected { Color::Cyan } else { Color::Gray })
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!(
                    "{}  {}  {}  {fit}",
                    model.name,
                    lmml_models::format_size(model.size_bytes),
                    model.quant
                )),
            ]));
        }
    }

    let right = selected_model_lines(app);
    super::render_two_pane(
        area,
        super::pane("Models", left),
        super::pane("Model Details", right),
        frame,
    );
}

fn selected_model_lines(app: &App) -> Vec<Line<'static>> {
    let Some(model) = app.models.get(app.selected_model) else {
        return vec![Line::from(format!(
            "Last model: {}",
            app.state.model.last_used.display()
        ))];
    };
    let gpus = app
        .detect_profile
        .as_ref()
        .map(|profile| profile.gpus.clone())
        .unwrap_or_default();
    let fit = model.vram_fit(&gpus);
    vec![
        Line::from(format!("Name: {}", model.name)),
        Line::from(format!("Path: {}", model.path.display())),
        Line::from(format!(
            "Size: {}",
            lmml_models::format_size(model.size_bytes)
        )),
        Line::from(format!("Quant: {}", model.quant)),
        Line::from(format!(
            "Architecture: {}",
            model.architecture.as_deref().unwrap_or("unknown")
        )),
        Line::from(format!(
            "Context: {}",
            model
                .context_length
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        )),
        Line::from(format!("Alias: {}", model.aliased)),
        Line::from(format!("VRAM: {}", fit.label())),
        Line::from(format!("Recommended ngl: {}", model.recommended_ngl(&gpus))),
    ]
}

trait VramFitLabel {
    fn label(&self) -> String;
}

impl VramFitLabel for lmml_models::VramFit {
    fn label(&self) -> String {
        match self {
            lmml_models::VramFit::Full { vram_free_mb, .. } => {
                format!("fits ({} MB free)", vram_free_mb)
            }
            lmml_models::VramFit::Partial {
                recommended_ngl, ..
            } => format!("partial ({recommended_ngl} layers)"),
            lmml_models::VramFit::TooLarge { model_mb, vram_mb } => {
                format!("too large ({model_mb} MB model / {vram_mb} MB VRAM)")
            }
            lmml_models::VramFit::CpuOnly => "CPU only".to_string(),
        }
    }
}
