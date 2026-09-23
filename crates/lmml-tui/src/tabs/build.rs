//! Build tab rendering.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::Frame;

use crate::app::App;

/// Render the llama.cpp build tab.
pub fn render(area: Rect, app: &App, frame: &mut Frame) {
    let flavor = app.state.build.selected_build_flavor();
    let (source_dir, binary, commit, backend, requested_ref) = match flavor {
        lmml_compat::LlamaRuntimeFlavor::Upstream => (
            &app.state.build.source_dir,
            &app.state.build.binary,
            app.state.build.commit.as_str(),
            app.state.build.backend.as_str(),
            "tracking policy",
        ),
        lmml_compat::LlamaRuntimeFlavor::Prism => (
            &app.state.build.prism.source_dir,
            &app.state.build.prism.binary,
            app.state.build.prism.commit.as_str(),
            app.state.build.prism.backend.as_str(),
            app.state.build.prism.requested_ref.as_str(),
        ),
    };
    let update = app
        .update_check
        .as_ref()
        .map(|update| format!("{update:?}"))
        .unwrap_or_else(|| "not checked".to_string());
    let sccache = app
        .detect_profile
        .as_ref()
        .and_then(|profile| profile.sccache.as_ref())
        .map(|path| format!("active ({})", path.display()))
        .unwrap_or_else(|| {
            if app.state.build.sccache_used {
                "used in last build".to_string()
            } else {
                "not active".to_string()
            }
        });
    let (build_archs, verification) = match flavor {
        lmml_compat::LlamaRuntimeFlavor::Upstream => (
            app.state.build.archs.as_slice(),
            "not required for upstream".to_string(),
        ),
        lmml_compat::LlamaRuntimeFlavor::Prism => {
            let prism = &app.state.build.prism;
            let gaps = crate::runtime_cli::prism_attestation_gaps(prism);
            let verification = if gaps.is_empty() {
                let targets = match prism.backend.as_str() {
                    "Cuda" => &prism.verified_cuda_targets,
                    "Rocm" => &prism.verified_rocm_targets,
                    _ => &prism.archs,
                };
                format!(
                    "v{}; tensors {:?}; {} {:?}; bytes bound",
                    prism.verification_version,
                    prism.verified_prism_tensor_types,
                    prism.backend,
                    targets
                )
            } else {
                format!("not verified ({}); clean rebuild required", gaps.join(", "))
            };
            (prism.archs.as_slice(), verification)
        }
    };
    let status = if app.build_running {
        status_line("BUILDING", Color::Yellow)
    } else if let Some(error) = &app.build_error {
        status_line(format!("FAILED: {error}"), Color::Red)
    } else if let Some(binary) = &app.build_binary {
        status_line(format!("READY: {}", binary.display()), Color::Green)
    } else {
        status_line("IDLE", Color::Gray)
    };
    let left = vec![
        status,
        Line::from("Press b to build llama.cpp."),
        Line::from("Press B for a clean build."),
        Line::from("Press u to check for updates."),
        Line::from("Set build_runtime in Settings before building."),
        Line::from(format!("Runtime: {flavor}")),
        Line::from(format!("Repository: {}", flavor.repository_url())),
        Line::from(format!("Requested ref: {requested_ref}")),
        Line::from(format!(
            "Resolved commit: {}",
            if commit.is_empty() {
                "not built"
            } else {
                commit
            }
        )),
        Line::from(format!("Source: {}", source_dir.display())),
        Line::from(format!("Binary: {}", binary.display())),
        Line::from(format!("Backend: {backend}")),
        Line::from(format!(
            "Targets: {}",
            if build_archs.is_empty() {
                "automatic".to_string()
            } else {
                build_archs.join(", ")
            }
        )),
        Line::from(format!("Verification: {verification}")),
        Line::from(format!("sccache: {sccache}")),
        Line::from(format!("Update: {update}")),
    ];
    let visible_log_lines = usize::from(area.height.saturating_sub(2));
    let right = if app.build_log.is_empty() {
        vec![Line::from("Build output will appear here.")]
    } else {
        app.build_log
            .iter()
            .skip(app.build_log.len().saturating_sub(visible_log_lines))
            .cloned()
            .map(Line::from)
            .collect()
    };
    super::render_two_pane(
        area,
        super::pane("Build", left),
        super::pane("Build Log", right),
        frame,
    );
}

fn status_line(text: impl Into<String>, color: Color) -> Line<'static> {
    Line::from(vec![Span::styled(
        text.into(),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )])
}
