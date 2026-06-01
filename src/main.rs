//! lmml — Local LLM Manager
//!
//! Turnkey TUI for managing llama.cpp: auto-detect hardware,
//! build from source, manage models, and run the inference server.

#![allow(dead_code)] // The MVP keeps several planned APIs and widgets staged for later phases.

mod app;
mod build;
mod models;
mod probe;
mod server;
mod tui;

use color_eyre::Result;

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let cli = CliArgs::parse();

    if std::env::var("LMML_LOG").is_ok() {
        tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .init();
    }

    // Ensure ~/.lmml/ exists
    app::config::ensure_dirs()?;

    // Load config and prior session state.
    let _config = app::config::load_config()?;
    let saved_state = app::config::load_state()?;
    if cli.diagnose {
        write_diagnostic_dump()?;
        return Ok(());
    }

    // Create the app
    let (mut app, _msg_rx) = app::App::new();
    if let Some(port) = cli.port {
        app.state.config.server.port = port;
    }
    for warning in dev_toolchain_warnings() {
        app.state.build_state.log_lines.push(warning);
    }
    if saved_state.last_session.build_state == "running" {
        app.state.current_screen = app::Screen::Build;
        app.state.build_state.log_lines.push(
            "Build was in progress when lmml last exited. Press [y] to resume or [n] to skip."
                .to_string(),
        );
    } else {
        app.state.build_state.commit_hash = saved_state.last_session.build_commit.clone();
        if saved_state.last_session.build_state == "complete" {
            app.state.build_state.complete = Some(Ok(()));
        }
    }

    // Auto-start probe in background
    let probe_tx = app.probe_tx.clone();
    tokio::spawn(async move {
        probe::run_all(probe_tx).await;
    });

    // Auto-scan models directory
    let models_dir = app::config::models_dir();
    let models =
        tokio::task::spawn_blocking(move || crate::models::local::scan_directory(&models_dir))
            .await
            .unwrap_or_default();
    for m in models {
        app.state.models.push(crate::app::state::ModelEntry {
            name: m.filename.clone(),
            path: m.path.clone(),
            size_bytes: m.size_bytes,
            quantization: m.quantization,
            param_count: m.param_count,
            model_type: m.model_type.clone(),
            is_loaded: false,
            is_favorite: false,
            last_used: String::new(),
        });
    }
    if let Some(model) = cli.model.as_deref() {
        if let Some(idx) = app
            .state
            .models
            .iter()
            .position(|entry| entry.path == model || entry.name == model)
        {
            app.state.selected_model = idx;
        } else {
            app.state.config.general.default_model = model.to_string();
        }
    }
    if cli.build {
        app.state.current_screen = app::Screen::Build;
        tui::build::start_build(&mut app);
    }

    // Initialize TUI and run
    let mut tui = tui::Tui::new()?;
    tui.run(&mut app).await?;

    // Save state on exit.
    let build_state = if app.state.build_state.is_running {
        "running"
    } else if app
        .state
        .build_state
        .complete
        .as_ref()
        .is_some_and(|result| result.is_ok())
    {
        "complete"
    } else if app.state.build_state.complete.is_some() {
        "failed"
    } else {
        "not-started"
    };
    let _ = app::config::save_state(&app::config::AppStateToml {
        last_session: app::config::LastSession {
            last_model: app
                .state
                .models
                .get(app.state.selected_model)
                .map(|m| m.path.clone())
                .unwrap_or_default(),
            server_was_running: !matches!(
                app.state.server_state.status,
                crate::server::ServerStatus::Stopped | crate::server::ServerStatus::Error(_)
            ),
            build_state: build_state.to_string(),
            build_commit: app.state.build_state.commit_hash,
        },
    });

    Ok(())
}

struct CliArgs {
    model: Option<String>,
    port: Option<u16>,
    build: bool,
    diagnose: bool,
}

impl CliArgs {
    fn parse() -> Self {
        let mut args = std::env::args().skip(1);
        let mut cli = CliArgs {
            model: None,
            port: None,
            build: false,
            diagnose: false,
        };

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--model" => cli.model = args.next(),
                "--port" => cli.port = args.next().and_then(|value| value.parse().ok()),
                "--build" => cli.build = true,
                "--diagnose" => cli.diagnose = true,
                _ => {}
            }
        }

        cli
    }
}

fn write_diagnostic_dump() -> Result<()> {
    let path = app::config::config_dir().join("diagnostic.txt");
    let rustc = std::process::Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .unwrap_or_else(|| "rustc unavailable".to_string());
    let config = app::config::load_config()?;
    let state = app::config::load_state()?;
    let content = format!(
        "lmml diagnostic dump\nversion: {}\nrustc: {rustc}\nconfig_dir: {}\nconfig:\n{config:#?}\nstate:\n{state:#?}\n",
        env!("CARGO_PKG_VERSION"),
        app::config::config_dir().display(),
    );
    std::fs::write(&path, content)?;
    Ok(())
}

fn dev_toolchain_warnings() -> Vec<String> {
    let mut warnings = Vec::new();
    if !command_succeeds("rustfmt", &["--version"]) {
        warnings.push("Developer tool missing: rustfmt (cargo fmt will not run).".to_string());
    }
    if !command_succeeds("cargo", &["clippy", "--version"]) {
        warnings.push("Developer tool missing: clippy (cargo clippy will not run).".to_string());
    }
    warnings
}

fn command_succeeds(program: &str, args: &[&str]) -> bool {
    std::process::Command::new(program)
        .args(args)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}
