//! lmml TUI binary entry point.

use std::io::{self, stdout, Write};
use std::panic;
use std::path::PathBuf;
use std::time::Duration;

use clap::{Parser, Subcommand};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use lmml_tui::app::App;
use lmml_tui::event_loop::EventLoop;
use lmml_tui::runtime_cli::{self, RoutingOptions, RoutingSource};
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
    /// Manage long-running llama-server runtimes for coding harnesses.
    Runtime {
        #[command(subcommand)]
        command: RuntimeCommand,
    },
}

#[derive(Debug, Subcommand)]
enum RuntimeCommand {
    /// Start a managed runtime profile.
    Start {
        /// Runtime profile name, such as opencode or opencode-fast.
        profile: String,
        /// Start in the background and return after readiness.
        #[arg(long)]
        detach: bool,
    },
    /// Stop a managed runtime profile.
    Stop {
        /// Runtime profile name, such as opencode or opencode-fast.
        profile: String,
    },
    /// Print managed runtime profile status.
    Status {
        /// Print JSON output. Reserved for the runtime process implementation.
        #[arg(long)]
        json: bool,
    },
    /// Print runtime profile logs.
    Logs {
        /// Runtime profile name, such as opencode or opencode-fast.
        profile: String,
        /// Follow log output.
        #[arg(long)]
        follow: bool,
    },
    /// Print ready-to-paste harness config.
    PrintConfig {
        /// Harness config target.
        target: RuntimeConfigTarget,
    },
    /// Configure an external harness config file.
    Configure {
        /// Harness config target.
        target: RuntimeConfigTarget,
        /// Show planned changes without writing.
        #[arg(long)]
        dry_run: bool,
        /// Config path override.
        #[arg(long)]
        path: Option<PathBuf>,
        /// Restore config from backup.
        #[arg(long)]
        rollback: Option<PathBuf>,
        /// Apply clean changes without prompting.
        #[arg(long)]
        yes: bool,
        /// Allow replacing conflicting lmml-owned provider entries.
        #[arg(long)]
        force: bool,
        /// Source for OpenCode's top-level model routing.
        #[arg(long, default_value = "lmml")]
        model_source: RoutingSourceArg,
        /// Source for OpenCode's top-level small_model routing.
        #[arg(long, default_value = "lmml")]
        small_model_source: RoutingSourceArg,
    },
}

#[derive(Debug, Clone, clap::ValueEnum)]
enum RuntimeConfigTarget {
    /// OpenCode provider config.
    Opencode,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum RoutingSourceArg {
    /// Preserve an existing key and do not create it when missing.
    Existing,
    /// Set the key to lmml's local managed model.
    Lmml,
    /// Do not touch this key.
    None,
}

struct RuntimeConfigureArgs {
    target: RuntimeConfigTarget,
    dry_run: bool,
    path: Option<PathBuf>,
    rollback: Option<PathBuf>,
    yes: bool,
    force: bool,
    routing: RoutingOptions,
}

impl From<RoutingSourceArg> for RoutingSource {
    fn from(value: RoutingSourceArg) -> Self {
        match value {
            RoutingSourceArg::Existing => Self::Existing,
            RoutingSourceArg::Lmml => Self::Lmml,
            RoutingSourceArg::None => Self::None,
        }
    }
}

struct LoggingGuards {
    _primary: tracing_appender::non_blocking::WorkerGuard,
    _rolling: tracing_appender::non_blocking::WorkerGuard,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Doctor) => {
            let code = run_doctor().await;
            std::process::exit(code);
        }
        Some(Command::Smoke) => {
            let code = run_smoke().await;
            std::process::exit(code);
        }
        Some(Command::Runtime { command }) => {
            let code = run_runtime(command).await;
            std::process::exit(code);
        }
        None => {}
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

async fn run_runtime(command: RuntimeCommand) -> i32 {
    match command {
        RuntimeCommand::Start { profile, detach } => run_runtime_start(&profile, detach).await,
        RuntimeCommand::Stop { profile } => run_runtime_stop(&profile).await,
        RuntimeCommand::Status { json } => {
            let mut state = match lmml_state::AppState::load_existing_or_default() {
                Ok(state) => state,
                Err(error) => {
                    eprintln!("state load failed: {error}");
                    return 1;
                }
            };
            runtime_cli::reconcile_runtime_state(&mut state);
            run_runtime_status(&state, json)
        }
        RuntimeCommand::Logs { profile, follow } => run_runtime_logs(&profile, follow).await,
        RuntimeCommand::PrintConfig { target } => {
            let state = match lmml_state::AppState::load_existing_or_default() {
                Ok(state) => state,
                Err(error) => {
                    eprintln!("state load failed: {error}");
                    return 1;
                }
            };
            run_runtime_print_config(&state, target)
        }
        RuntimeCommand::Configure {
            target,
            dry_run,
            path,
            rollback,
            yes,
            force,
            model_source,
            small_model_source,
        } => {
            let state = match lmml_state::AppState::load_existing_or_default() {
                Ok(state) => state,
                Err(error) => {
                    eprintln!("state load failed: {error}");
                    return 1;
                }
            };
            run_runtime_configure(
                &state,
                RuntimeConfigureArgs {
                    target,
                    dry_run,
                    path,
                    rollback,
                    yes,
                    force,
                    routing: RoutingOptions {
                        model: model_source.into(),
                        small_model: small_model_source.into(),
                    },
                },
            )
        }
    }
}

async fn run_runtime_start(profile: &str, detach: bool) -> i32 {
    if !detach {
        eprintln!(
            "runtime start currently runs detached; pass --detach to acknowledge this behavior"
        );
        return 2;
    }
    let mut state = match lmml_state::AppState::load() {
        Ok(state) => state,
        Err(error) => {
            eprintln!("state load failed: {error}");
            return 1;
        }
    };
    match runtime_cli::start_profile(&mut state, profile, runtime_startup_timeout()).await {
        Ok(started) => {
            println!(
                "{} ready at {} (pid {}, log {})",
                started.profile,
                started.url,
                started.pid,
                started.log_path.display()
            );
            0
        }
        Err(error) => {
            eprintln!("runtime start failed: {error}");
            1
        }
    }
}

fn runtime_startup_timeout() -> Duration {
    std::env::var("LMML_RUNTIME_STARTUP_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or_else(|| Duration::from_secs(60))
}

async fn run_runtime_stop(profile: &str) -> i32 {
    let mut state = match lmml_state::AppState::load() {
        Ok(state) => state,
        Err(error) => {
            eprintln!("state load failed: {error}");
            return 1;
        }
    };
    match runtime_cli::stop_profile(&mut state, profile).await {
        Ok(stopped) => {
            println!("{}: {}", stopped.profile, stopped.message);
            0
        }
        Err(error) => {
            eprintln!("runtime stop failed: {error}");
            1
        }
    }
}

async fn run_runtime_logs(profile: &str, follow: bool) -> i32 {
    if !runtime_cli::is_known_profile(profile) {
        eprintln!("unknown runtime profile `{profile}`");
        return 2;
    }
    let path = runtime_cli::runtime_log_path(profile);
    if !follow {
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                print!("{content}");
                return 0;
            }
            Err(error) => {
                eprintln!("failed to read {}: {error}", path.display());
                return 1;
            }
        }
    }

    let mut offset = std::fs::metadata(&path)
        .map(|metadata| metadata.len() as usize)
        .unwrap_or(0);
    loop {
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                if content.len() > offset {
                    print!("{}", &content[offset..]);
                    let _ignored = io::stdout().flush();
                    offset = content.len();
                }
            }
            Err(error) => {
                if error.kind() != std::io::ErrorKind::NotFound {
                    eprintln!("failed to read {}: {error}", path.display());
                    return 1;
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

fn run_runtime_status(state: &lmml_state::AppState, json: bool) -> i32 {
    if json {
        eprintln!("runtime status --json will be available when runtime process state is wired");
        return 2;
    }
    println!("{}", runtime_cli::render_status(state));
    0
}

fn run_runtime_print_config(state: &lmml_state::AppState, target: RuntimeConfigTarget) -> i32 {
    match target {
        RuntimeConfigTarget::Opencode => {
            for warning in runtime_cli::opencode_config_warnings(state) {
                eprintln!("{warning}");
            }
            match runtime_cli::render_opencode_config(state) {
                Ok(rendered) => {
                    println!("{rendered}");
                    0
                }
                Err(error) => {
                    eprintln!("failed to render OpenCode config: {error}");
                    1
                }
            }
        }
    }
}

fn run_runtime_configure(state: &lmml_state::AppState, args: RuntimeConfigureArgs) -> i32 {
    match args.target {
        RuntimeConfigTarget::Opencode => {
            let path = args
                .path
                .unwrap_or_else(runtime_cli::default_opencode_config_path);
            if let Some(backup) = args.rollback {
                return match runtime_cli::rollback_opencode_config(&backup, &path) {
                    Ok(()) => {
                        println!("restored {} from {}", path.display(), backup.display());
                        0
                    }
                    Err(error) => {
                        eprintln!("rollback failed: {error}");
                        1
                    }
                };
            }
            let plan = match runtime_cli::plan_opencode_configure(
                state,
                &path,
                args.routing,
                args.force,
            ) {
                Ok(plan) => plan,
                Err(error) => {
                    eprintln!("configure plan failed: {error}");
                    return 1;
                }
            };
            println!("OpenCode config: {}", plan.path.display());
            print_diff(&plan.diff);
            print_routing(&plan.routing);
            if plan.has_provider_conflicts && !args.force {
                eprintln!("conflicting lmml-owned provider entries found; rerun with --force after reviewing the diff");
                return 2;
            }
            if plan.has_routing_conflicts {
                eprintln!("OpenCode top-level routing will be changed to lmml local values.");
            }
            if args.dry_run {
                println!("dry run only; no files written");
                return 0;
            }
            if !args.yes && !confirm("Apply OpenCode config changes? [y/N] ") {
                println!("aborted; no files written");
                return 2;
            }
            match runtime_cli::apply_opencode_configure(state, &path, args.routing, args.force) {
                Ok(applied) => {
                    println!("updated {}", applied.path.display());
                    println!("backup: {}", applied.backup_path.display());
                    println!(
                        "rollback: lmml runtime configure opencode --path {} --rollback {}",
                        applied.path.display(),
                        applied.backup_path.display()
                    );
                    0
                }
                Err(error) => {
                    eprintln!("configure apply failed: {error}");
                    1
                }
            }
        }
    }
}

fn print_routing(routing: &runtime_cli::RoutingPlan) {
    println!("Routing:");
    print_routing_decision(&routing.model);
    print_routing_decision(&routing.small_model);
}

fn print_routing_decision(decision: &runtime_cli::RoutingDecision) {
    match decision.source {
        RoutingSource::Existing => match &decision.existing {
            Some(existing) => println!("  {}: preserve existing {existing}", decision.key),
            None => println!(
                "  {}: preserve existing; key missing, no write",
                decision.key
            ),
        },
        RoutingSource::None => println!("  {}: not touched", decision.key),
        RoutingSource::Lmml => {
            if decision.conflict {
                let existing = decision.existing.as_deref().unwrap_or("<missing>");
                println!(
                    "  {}: conflict, current {existing}, requested {}",
                    decision.key, decision.lmml
                );
            } else {
                println!("  {}: set to {}", decision.key, decision.lmml);
            }
        }
    }
}

fn print_diff(diff: &[String]) {
    if diff.is_empty() {
        println!("No changes needed.");
    } else {
        println!("Planned changes:");
        for line in diff {
            println!("  {line}");
        }
    }
}

fn confirm(prompt: &str) -> bool {
    print!("{prompt}");
    let _ignored = io::stdout().flush();
    let mut answer = String::new();
    if io::stdin().read_line(&mut answer).is_err() {
        return false;
    }
    matches!(answer.trim(), "y" | "Y" | "yes" | "YES")
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
            println!("  ⚠  CUDA toolkit {found_toolkit} too old for {gpu_arch}");
            println!("     → install CUDA >= {minimum_toolkit}");
        }
        lmml_detect::CudaCompatibility::NoGpu => {
            soft_issues += 1;
            if let Some(error) = &profile.gpu_probe_error {
                println!("  ⚠  NVIDIA driver/GPU probe failed");
                println!("     → nvidia-smi: {error}");
                println!("     → check that the NVIDIA driver is installed, loaded, and matches the running kernel");
            } else {
                println!("  ⚠  CUDA GPU not detected");
                println!("     → run `lmml` to proceed in CPU-only mode");
            }
        }
        lmml_detect::CudaCompatibility::NvccMissing => {
            soft_issues += 1;
            println!("  ⚠  nvcc not found");
            println!("     → install the CUDA toolkit only if you want NVIDIA GPU acceleration");
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
        println!("  {soft_issues} GPU acceleration warning(s) found.");
        println!(
            "  lmml can run in CPU-only mode; fix the warning(s) above to enable GPU acceleration."
        );
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
