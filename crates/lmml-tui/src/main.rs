//! lmml TUI binary entry point.

use std::io::{self, stdout};
use std::panic;

use clap::{Parser, Subcommand};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use lmml_tui::app::App;
use lmml_tui::event_loop::EventLoop;
use lmml_tui::tabs;
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(
    name = "lmml",
    version,
    about = "Terminal UI for managing llama.cpp locally"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run system preflight checks outside the TUI.
    Doctor,
    /// Run a short headless startup check for install smoke tests.
    Smoke,
}

struct LoggingGuards {
    _primary: tracing_appender::non_blocking::WorkerGuard,
    _rolling: tracing_appender::non_blocking::WorkerGuard,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    if matches!(cli.command, Some(Command::Doctor)) {
        let code = run_doctor().await;
        std::process::exit(code);
    }
    if matches!(cli.command, Some(Command::Smoke)) {
        let code = run_smoke().await;
        std::process::exit(code);
    }

    let log_guard = init_logging()?;
    install_panic_hook();
    tracing::info!(log_path = %lmml_state::AppState::log_path().display(), "lmml starting");

    let mut terminal = init_terminal()?;
    let result = run(&mut terminal).await;
    restore_terminal()?;
    if let Err(error) = &result {
        eprintln!("lmml exited with error: {error}");
        eprintln!("Log file: {}", lmml_state::AppState::log_path().display());
    }
    drop(log_guard);
    result
}

async fn run_smoke() -> i32 {
    println!("lmml smoke — startup check");
    match lmml_state::AppState::load() {
        Ok(_) => {}
        Err(error) => {
            eprintln!("state load failed: {error}");
            return 1;
        }
    }
    let profile = lmml_detect::SystemProfile::detect().await;
    if !profile.missing_prerequisites().is_empty() {
        eprintln!("hard prerequisites are missing; run `lmml doctor` for details");
        return 1;
    }
    println!("ok");
    0
}

async fn run_doctor() -> i32 {
    let profile = lmml_detect::SystemProfile::detect().await;
    let mut hard_issues = 0;
    let mut soft_issues = 0;

    println!("lmml doctor — system preflight check");
    println!("─────────────────────────────────────");

    match &profile.compiler {
        Some(compiler) if compiler.cpp17_ok => {
            println!("  ✓  {}", concise_tool_line("compiler", &compiler.version));
        }
        Some(compiler) => {
            hard_issues += 1;
            println!("  ✗  C++17 compiler probe failed");
            if let Some(error) = &compiler.cpp17_error {
                println!("     → {error}");
            }
        }
        None => {
            hard_issues += 1;
            println!("  ✗  gcc or clang not found");
            println!("     → sudo apt install build-essential");
        }
    }

    match &profile.cmake {
        Some(cmake) if cmake.meets_minimum => println!("  ✓  cmake {}", cmake.version),
        Some(cmake) => {
            hard_issues += 1;
            println!("  ✗  cmake {} found; 3.21 required", cmake.version);
            println!("     → sudo apt install cmake");
        }
        None => {
            hard_issues += 1;
            println!("  ✗  cmake not found");
            println!("     → sudo apt install cmake");
        }
    }

    match &profile.git {
        Some(git) if git.meets_minimum => println!("  ✓  git {}", git.version),
        Some(git) => {
            hard_issues += 1;
            println!("  ✗  git {} found; 2.28 required", git.version);
            println!("     → sudo apt install git");
        }
        None => {
            hard_issues += 1;
            println!("  ✗  git not found");
            println!("     → sudo apt install git");
        }
    }

    match &profile.cuda {
        lmml_detect::CudaCompatibility::Compatible { archs } => {
            let gpu = profile
                .gpus
                .first()
                .map(|gpu| gpu.name.as_str())
                .unwrap_or("CUDA GPU");
            println!("  ✓  CUDA available  ·  {gpu}  ·  {}", archs.join(", "));
        }
        lmml_detect::CudaCompatibility::ToolkitTooOld {
            gpu_arch,
            minimum_toolkit,
            found_toolkit,
        } => {
            soft_issues += 1;
            println!("  ✗  CUDA toolkit {found_toolkit} too old for {gpu_arch}");
            println!("     → install CUDA >= {minimum_toolkit}");
        }
        lmml_detect::CudaCompatibility::NoGpu => {
            soft_issues += 1;
            println!("  ✗  CUDA GPU not detected");
            println!("     → run `lmml` to proceed in CPU-only mode");
        }
        lmml_detect::CudaCompatibility::NvccMissing => {
            soft_issues += 1;
            println!("  ✗  nvcc not found");
            println!("     → sudo apt install nvidia-cuda-toolkit");
        }
    }

    let disk_gb = profile.disk.available_bytes / 1024 / 1024 / 1024;
    if profile.disk.require(4 * 1024 * 1024 * 1024).is_ok() {
        println!("  ✓  disk: {disk_gb} GB available");
    } else {
        hard_issues += 1;
        println!("  ✗  disk: {disk_gb} GB available");
        println!("     → free at least 4 GB for llama.cpp source and build artifacts");
    }

    println!();
    if hard_issues == 0 && soft_issues == 0 {
        println!("  No issues found. Run `lmml` to launch the TUI.");
        0
    } else if hard_issues == 0 {
        println!("  {soft_issues} issue(s) found. Run `lmml` to proceed in CPU-only mode,");
        println!("  or fix the issues above for GPU acceleration.");
        0
    } else {
        println!("  {hard_issues} hard prerequisite issue(s) found.");
        println!("  Fix the issues above before first use.");
        1
    }
}

fn concise_tool_line(fallback: &str, version: &str) -> String {
    let first = version.lines().next().unwrap_or(fallback).trim();
    if first.is_empty() {
        fallback.to_string()
    } else {
        first.to_string()
    }
}

#[tracing::instrument(skip(terminal))]
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

fn init_logging() -> io::Result<LoggingGuards> {
    let log_path = lmml_state::AppState::log_path();
    let log_dir = log_path
        .parent()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| ".".into());
    std::fs::create_dir_all(&log_dir)?;
    let primary_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    let rolling_appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("lmml")
        .filename_suffix("log")
        .max_log_files(7)
        .build(&log_dir)
        .map_err(io::Error::other)?;
    let (primary_writer, primary_guard) = tracing_appender::non_blocking(primary_file);
    let (rolling_writer, rolling_guard) = tracing_appender::non_blocking(rolling_appender);
    let primary_filter =
        EnvFilter::try_from_env("LMML_LOG").unwrap_or_else(|_| EnvFilter::new("debug"));
    let rolling_filter =
        EnvFilter::try_from_env("LMML_LOG").unwrap_or_else(|_| EnvFilter::new("debug"));

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(primary_writer)
                .with_ansi(false)
                .with_target(true)
                .with_filter(primary_filter),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(rolling_writer)
                .with_ansi(false)
                .with_target(true)
                .with_filter(rolling_filter),
        )
        .init();
    Ok(LoggingGuards {
        _primary: primary_guard,
        _rolling: rolling_guard,
    })
}

fn install_panic_hook() {
    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let _ignored = restore_terminal();
        eprintln!("lmml crashed: {info}");
        eprintln!("Log file: {}", lmml_state::AppState::log_path().display());
        default_hook(info);
    }));
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
