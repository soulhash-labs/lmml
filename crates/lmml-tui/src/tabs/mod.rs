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
    use std::path::PathBuf;

    use lmml_build::UpdateCheck;
    use lmml_compat::LlamaBinaryCapabilities;
    use lmml_detect::{
        CmakeInfo, CompilerInfo, CpuFeatures, CudaCompatibility, DiskInfo, GitInfo, GpuInfo,
        MemInfo, MetalSupport, SystemProfile,
    };
    use lmml_models::{DownloadProgress, HfModelResult, ModelEntry};
    use lmml_server::ServerStatus;
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

    #[test]
    fn snapshots_detect_states() {
        let mut app = App::default();
        app.active_tab = Tab::Detect;
        insta::assert_snapshot!("detect_fresh", render_app(&app));

        app.detect_profile = Some(healthy_profile());
        app.detect_log = vec!["Detected backend: Cuda { archs: [\"sm_86\"] }".to_string()];
        insta::assert_snapshot!("detect_complete_all_green", render_app(&app));

        app.detect_profile = Some(missing_prereq_profile());
        app.detect_log = vec!["Missing C++ compiler: sudo apt install build-essential".to_string()];
        insta::assert_snapshot!("detect_missing_prereqs", render_app(&app));
    }

    #[test]
    fn snapshots_build_states() {
        let mut app = App::default();
        app.active_tab = Tab::Build;
        insta::assert_snapshot!("build_idle", render_app(&app));

        app.build_running = true;
        app.build_log = vec![
            "Configuring CMake".to_string(),
            "[ 42%] Building".to_string(),
        ];
        insta::assert_snapshot!("build_running", render_app(&app));

        app.build_running = false;
        app.build_error = Some("cmake failed".to_string());
        app.update_check = Some(UpdateCheck::Available {
            current: "abc".to_string(),
            latest: "def".to_string(),
            commits_behind: 3,
        });
        insta::assert_snapshot!("build_failed_update_available", render_app(&app));
    }

    #[test]
    fn snapshots_models_states() {
        let mut app = App::default();
        app.active_tab = Tab::Models;
        insta::assert_snapshot!("models_empty", render_app(&app));

        app.models = vec![model_entry("mistral-7b-Q4_K_M.gguf", 4_100_000_000)];
        insta::assert_snapshot!("models_populated", render_app(&app));

        app.hf_search_open = true;
        app.hf_query = "mistral gguf".to_string();
        app.hf_results = vec![HfModelResult {
            repo_id: "org/mistral".to_string(),
            filename: "mistral-7b-Q4_K_M.gguf".to_string(),
            size_bytes: 4_100_000_000,
            downloads: 1234,
            url: "https://huggingface.co/org/mistral/resolve/main/mistral.gguf".to_string(),
        }];
        app.download_progress = Some(DownloadProgress {
            bytes_received: 2_000_000_000,
            total_bytes: Some(4_100_000_000),
            resumed_from: 1_000_000_000,
        });
        insta::assert_snapshot!("models_hf_downloading", render_app(&app));
    }

    #[test]
    fn snapshots_server_states() {
        let mut app = App::default();
        app.active_tab = Tab::Server;
        insta::assert_snapshot!("server_stopped", render_app(&app));

        app.models = vec![model_entry("llama.gguf", 2_000_000_000)];
        app.server_status = ServerStatus::Ready {
            url: "http://127.0.0.1:8080".to_string(),
        };
        app.server_log = vec!["server listening".to_string()];
        insta::assert_snapshot!("server_ready", render_app(&app));

        app.server_status = ServerStatus::Failed {
            reason: "port 8080 is already in use".to_string(),
        };
        insta::assert_snapshot!("server_failed_port_conflict", render_app(&app));
    }

    #[test]
    fn snapshots_settings_and_overlays() {
        let mut app = App::default();
        app.active_tab = Tab::Settings;
        insta::assert_snapshot!("settings_default", render_app(&app));

        app.server_caps = Some(LlamaBinaryCapabilities {
            version: Some("llama-server test".to_string()),
            flash_attn: false,
            mlock: false,
            api_key: false,
            ubatch_size: true,
            chat_template: true,
            jinja: true,
            reranking: false,
            flags: vec!["--model".to_string(), "--port".to_string()],
        });
        app.settings_edit_buffer = Some("9090".to_string());
        app.selected_settings_field = crate::app::SettingsField::Port;
        insta::assert_snapshot!("settings_modal_unsupported_flags", render_app(&app));

        app.settings_edit_buffer = None;
        app.show_help = true;
        insta::assert_snapshot!("help_overlay", render_app(&app));

        app.show_help = false;
        app.first_run_onboarding = true;
        insta::assert_snapshot!("first_run_onboarding", render_app(&app));
    }

    fn render_app(app: &App) -> String {
        let backend = TestBackend::new(110, 34);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        terminal
            .draw(|frame| render(frame.area(), app, frame))
            .expect("render snapshot");
        normalize_snapshot(terminal.backend().to_string())
    }

    fn normalize_snapshot(mut rendered: String) -> String {
        if let Some(home) = std::env::var_os("HOME").and_then(|home| home.into_string().ok()) {
            rendered = rendered.replace(&home, "$HOME");
        }
        if let Some(userprofile) =
            std::env::var_os("USERPROFILE").and_then(|home| home.into_string().ok())
        {
            rendered = rendered.replace(&userprofile, "$USERPROFILE");
        }
        rendered
    }

    fn healthy_profile() -> SystemProfile {
        SystemProfile {
            compiler: Some(CompilerInfo {
                path: PathBuf::from("/usr/bin/g++"),
                version: "g++ 15.2.0".to_string(),
                cpp17_ok: true,
                cpp17_error: None,
            }),
            cmake: Some(CmakeInfo {
                path: PathBuf::from("/usr/bin/cmake"),
                version: "4.2.3".to_string(),
                meets_minimum: true,
            }),
            git: Some(GitInfo {
                path: PathBuf::from("/usr/bin/git"),
                version: "2.45.0".to_string(),
                meets_minimum: true,
            }),
            cuda: CudaCompatibility::Compatible {
                archs: vec!["sm_86"],
            },
            gpus: vec![GpuInfo {
                name: "NVIDIA RTX 3090".to_string(),
                memory_total_mb: 24_576,
                compute_cap: "8.6".to_string(),
                arch: Some("sm_86"),
            }],
            sccache: Some(PathBuf::from("/usr/bin/sccache")),
            metal: MetalSupport {
                available: false,
                displays: Vec::new(),
            },
            cpu: CpuFeatures {
                model: "Ryzen".to_string(),
                cores: 8,
                threads: 16,
                avx: true,
                avx2: true,
                avx512: false,
                neon: false,
                features: vec!["avx2".to_string()],
            },
            memory: MemInfo {
                total_mb: 65_536,
                available_mb: 32_768,
            },
            disk: DiskInfo {
                available_bytes: 200 * 1024 * 1024 * 1024,
                path: PathBuf::from("/home/angelo/repos/lmml"),
            },
        }
    }

    fn missing_prereq_profile() -> SystemProfile {
        SystemProfile {
            compiler: None,
            cmake: Some(CmakeInfo {
                path: PathBuf::from("/usr/bin/cmake"),
                version: "3.10.0".to_string(),
                meets_minimum: false,
            }),
            git: None,
            cuda: CudaCompatibility::NvccMissing,
            gpus: Vec::new(),
            sccache: None,
            metal: MetalSupport {
                available: false,
                displays: Vec::new(),
            },
            cpu: CpuFeatures {
                model: "generic".to_string(),
                cores: 2,
                threads: 4,
                avx: true,
                avx2: false,
                avx512: false,
                neon: false,
                features: Vec::new(),
            },
            memory: MemInfo {
                total_mb: 4096,
                available_mb: 1024,
            },
            disk: DiskInfo {
                available_bytes: 1024 * 1024 * 1024,
                path: PathBuf::from("/tmp"),
            },
        }
    }

    fn model_entry(path: &str, size_bytes: u64) -> ModelEntry {
        ModelEntry {
            path: PathBuf::from(path),
            name: path.to_string(),
            size_bytes,
            quant: "Q4_K_M".to_string(),
            context_length: Some(4096),
            architecture: Some("llama".to_string()),
            aliased: false,
        }
    }
}
