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
        Line::from("Press / to search Hugging Face. Press D to download selected HF result."),
        Line::from("Press a to add a model alias. Press x to delete selected model."),
        Line::from("Press p to switch selected model runtime profile."),
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

    let right = if app.hf_search_open {
        hf_search_lines(app)
    } else {
        selected_model_lines(app)
    };
    super::render_two_pane(
        area,
        super::pane("Models", left),
        super::pane("Model Details", right),
        frame,
    );
}

fn hf_search_lines(app: &App) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(format!("Query: {}", app.hf_query)),
        Line::from(format!("Results: {}", app.hf_results.len())),
    ];
    if let Some(progress) = &app.download_progress {
        let total = progress
            .total_bytes
            .map(lmml_models::format_size)
            .unwrap_or_else(|| "unknown".to_string());
        lines.push(Line::from(format!(
            "Download: {} / {} (resumed from {})",
            lmml_models::format_size(progress.bytes_received),
            total,
            lmml_models::format_size(progress.resumed_from)
        )));
    }
    if let Some(error) = &app.download_error {
        lines.push(Line::from(format!("Error: {error}")));
    }
    lines.push(Line::from(""));
    for (index, result) in app.hf_results.iter().enumerate() {
        let selected = index == app.selected_hf_result;
        let marker = if selected { "> " } else { "  " };
        lines.push(Line::from(vec![
            Span::styled(
                marker,
                Style::default()
                    .fg(if selected { Color::Cyan } else { Color::Gray })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                "{} / {}  {}  {} downloads",
                result.repo_id,
                result.filename,
                lmml_models::format_size(result.size_bytes),
                result.downloads
            )),
        ]));
    }
    lines
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
    let mut lines = vec![
        Line::from(format!("Name: {}", model.name)),
        Line::from(format!("Path: {}", model.path.display())),
        Line::from(format!(
            "Runtime profile: {}",
            app.state
                .model
                .runtime_profile_for_path(&model.path)
                .map(|profile| profile.name.as_str())
                .unwrap_or("custom/global")
        )),
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
    ];

    let catalog_match = lmml_models::catalog::match_known_model_name(&model.name).or_else(|| {
        model
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(lmml_models::catalog::match_known_model_name)
    });
    if let Some(variant) = catalog_match {
        lines.push(Line::from(""));
        lines.push(Line::styled(
            "Known model family",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
        lines.push(Line::from(format!("Family: {}", variant.family)));
        lines.push(Line::from(format!("Variant: {}", variant.name)));
        lines.push(Line::from(format!(
            "Architecture: {}",
            variant.architecture
        )));
        lines.push(Line::from(format!(
            "Native context: {}",
            variant.context_tokens
        )));
        lines.push(Line::from(format!(
            "Modalities: {}",
            variant.modalities_label()
        )));
        lines.push(Line::from(format!(
            "Implementation: {}",
            variant.implementation_note
        )));
        lines.push(Line::from(format!("Guidance: {}", variant.local_guidance)));
        for note in variant.serving_notes {
            lines.push(Line::from(format!("Note: {note}")));
        }
    }

    lines
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
