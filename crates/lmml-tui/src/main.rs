//! lmml TUI binary entry point.

use std::io::{self, stdout, Write};
use std::panic;
use std::path::{Path, PathBuf};
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
    /// Run llama-finetune with lmml path defaults.
    Train {
        /// Training data file. Translated to llama-finetune's --file flag.
        #[arg(long)]
        train_data: PathBuf,
        /// Base model GGUF. Defaults to the selected lmml model.
        #[arg(long, alias = "model-base")]
        model: Option<PathBuf>,
        /// Full fine-tuned GGUF output path passed to llama-finetune.
        #[arg(long, short = 'o')]
        output: Option<PathBuf>,
        /// LoRA adapter output path for custom llama-finetune builds that advertise --lora-out.
        #[arg(long)]
        lora_out: Option<PathBuf>,
        /// Optional checkpoint input for custom llama-finetune builds that advertise --checkpoint-in.
        #[arg(long)]
        checkpoint_in: Option<PathBuf>,
        /// Optional checkpoint output for custom llama-finetune builds that advertise --checkpoint-out.
        #[arg(long)]
        checkpoint_out: Option<PathBuf>,
        /// Optional merged GGUF output produced with llama-export-lora after LoRA training.
        #[arg(long)]
        merge_output: Option<PathBuf>,
        /// Additional llama-finetune arguments after `--`.
        #[arg(last = true)]
        extra_args: Vec<String>,
    },
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
        Some(Command::Train {
            train_data,
            model,
            output,
            lora_out,
            checkpoint_in,
            checkpoint_out,
            merge_output,
            extra_args,
        }) => {
            let code = run_train(TrainRequest {
                train_data,
                model,
                output,
                lora_out,
                checkpoint_in,
                checkpoint_out,
                merge_output,
                extra_args,
            })
            .await;
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

struct TrainRequest {
    train_data: PathBuf,
    model: Option<PathBuf>,
    output: Option<PathBuf>,
    lora_out: Option<PathBuf>,
    checkpoint_in: Option<PathBuf>,
    checkpoint_out: Option<PathBuf>,
    merge_output: Option<PathBuf>,
    extra_args: Vec<String>,
}

async fn run_train(request: TrainRequest) -> i32 {
    let state = match lmml_state::AppState::load_existing_or_default() {
        Ok(state) => state,
        Err(error) => {
            eprintln!("state load failed: {error}");
            return 1;
        }
    };
    let model = request
        .model
        .unwrap_or_else(|| state.model.last_used.clone());
    if model.as_os_str().is_empty() {
        eprintln!("train failed: pass --model or select a model in lmml first");
        return 2;
    }

    let binary = finetune_binary_path(&state.build.binary);
    let capabilities = match detect_finetune_capabilities(&binary).await {
        Ok(capabilities) => capabilities,
        Err(error) => {
            eprintln!("failed to inspect {}: {error}", binary.display());
            return 1;
        }
    };
    let argv = match build_train_argv(
        TrainArgInputs {
            model: &model,
            train_data: &request.train_data,
            output: request.output.as_deref(),
            lora_out: request.lora_out.as_deref(),
            checkpoint_in: request.checkpoint_in.as_deref(),
            checkpoint_out: request.checkpoint_out.as_deref(),
            extra_args: &request.extra_args,
        },
        &capabilities,
    ) {
        Ok(argv) => argv,
        Err(error) => {
            eprintln!("train failed: {error}");
            return 2;
        }
    };
    let status = match tokio::process::Command::new(&binary)
        .args(&argv)
        .status()
        .await
    {
        Ok(status) => status,
        Err(error) => {
            eprintln!("failed to start {}: {error}", binary.display());
            return 1;
        }
    };
    if !status.success() {
        return status.code().unwrap_or(1);
    }

    if let Some(merge_output) = request.merge_output {
        let Some(lora) = request.lora_out.as_deref() else {
            eprintln!("train failed: --merge-output requires --lora-out");
            return 2;
        };
        let export_binary = export_lora_binary_path(&state.build.binary);
        let export_argv = build_export_lora_argv(&model, lora, &merge_output);
        let export_status = match tokio::process::Command::new(&export_binary)
            .args(&export_argv)
            .status()
            .await
        {
            Ok(status) => status,
            Err(error) => {
                eprintln!("failed to start {}: {error}", export_binary.display());
                return 1;
            }
        };
        return export_status.code().unwrap_or(1);
    }

    0
}

fn finetune_binary_path(server_binary: &Path) -> PathBuf {
    sibling_llama_binary_path(server_binary, "llama-finetune")
}

fn export_lora_binary_path(server_binary: &Path) -> PathBuf {
    sibling_llama_binary_path(server_binary, "llama-export-lora")
}

fn sibling_llama_binary_path(server_binary: &Path, binary: &str) -> PathBuf {
    server_binary
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join(binary_name(binary))
}

fn binary_name(base: &str) -> String {
    if cfg!(windows) {
        format!("{base}.exe")
    } else {
        base.to_string()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct FinetuneCapabilities {
    model_base: bool,
    train_data: bool,
    lora_out: bool,
    checkpoint_in: bool,
    checkpoint_out: bool,
}

impl FinetuneCapabilities {
    fn from_help(help: &str) -> Self {
        Self {
            model_base: help.contains("--model-base"),
            train_data: help.contains("--train-data"),
            lora_out: help.contains("--lora-out"),
            checkpoint_in: help.contains("--checkpoint-in"),
            checkpoint_out: help.contains("--checkpoint-out"),
        }
    }
}

async fn detect_finetune_capabilities(binary: &Path) -> io::Result<FinetuneCapabilities> {
    let output = tokio::process::Command::new(binary)
        .arg("--help")
        .output()
        .await?;
    let mut help = String::from_utf8_lossy(&output.stdout).into_owned();
    help.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok(FinetuneCapabilities::from_help(&help))
}

struct TrainArgInputs<'a> {
    model: &'a Path,
    train_data: &'a Path,
    output: Option<&'a Path>,
    lora_out: Option<&'a Path>,
    checkpoint_in: Option<&'a Path>,
    checkpoint_out: Option<&'a Path>,
    extra_args: &'a [String],
}

fn build_train_argv(
    inputs: TrainArgInputs<'_>,
    capabilities: &FinetuneCapabilities,
) -> Result<Vec<String>, String> {
    if inputs.lora_out.is_some() && !capabilities.lora_out {
        return Err("installed llama-finetune does not advertise --lora-out".to_string());
    }
    if inputs.checkpoint_in.is_some() && !capabilities.checkpoint_in {
        return Err("installed llama-finetune does not advertise --checkpoint-in".to_string());
    }
    if inputs.checkpoint_out.is_some() && !capabilities.checkpoint_out {
        return Err("installed llama-finetune does not advertise --checkpoint-out".to_string());
    }

    let model_flag = if capabilities.model_base {
        "--model-base"
    } else {
        "--model"
    };
    let data_flag = if capabilities.train_data {
        "--train-data"
    } else {
        "--file"
    };
    let mut argv = vec![
        model_flag.to_string(),
        inputs.model.to_string_lossy().into_owned(),
        data_flag.to_string(),
        inputs.train_data.to_string_lossy().into_owned(),
    ];
    if let Some(output) = inputs.output {
        argv.push("--output".to_string());
        argv.push(output.to_string_lossy().into_owned());
    }
    if let Some(lora_out) = inputs.lora_out {
        argv.push("--lora-out".to_string());
        argv.push(lora_out.to_string_lossy().into_owned());
    }
    if let Some(checkpoint_in) = inputs.checkpoint_in {
        argv.push("--checkpoint-in".to_string());
        argv.push(checkpoint_in.to_string_lossy().into_owned());
    }
    if let Some(checkpoint_out) = inputs.checkpoint_out {
        argv.push("--checkpoint-out".to_string());
        argv.push(checkpoint_out.to_string_lossy().into_owned());
    }
    argv.extend(inputs.extra_args.iter().cloned());
    Ok(argv)
}

fn build_export_lora_argv(model: &Path, lora: &Path, output: &Path) -> Vec<String> {
    vec![
        "--model".to_string(),
        model.to_string_lossy().into_owned(),
        "--lora".to_string(),
        lora.to_string_lossy().into_owned(),
        "--output".to_string(),
        output.to_string_lossy().into_owned(),
    ]
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

    match &profile.sccache {
        Some(path) => println!("  ✓  sccache active  ·  {}", path.display()),
        None => {
            soft_issues += 1;
            println!("  ⚠  sccache not found");
            println!("     → install sccache for faster repeat llama.cpp builds");
            println!("     → sudo apt install sccache");
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
        lmml_detect::CudaCompatibility::ToolkitTooNew {
            gpu_arch,
            maximum_toolkit,
            found_toolkit,
        } => {
            soft_issues += 1;
            println!("  ⚠  CUDA toolkit {found_toolkit} no longer supports {gpu_arch}");
            println!("     → install CUDA {maximum_toolkit} alongside CUDA 13");
            println!("     → then run: LMML_CUDA_COMPILER=/usr/local/cuda-12.4/bin/nvcc lmml");
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

    if profile.rocm.available {
        let version = profile.rocm.version.as_deref().unwrap_or("version unknown");
        let targets = if profile.rocm.targets.is_empty() {
            "targets auto".to_string()
        } else {
            profile.rocm.targets.join(", ")
        };
        println!("  ✓  ROCm/HIP available  ·  {version}  ·  {targets}");
    } else if profile.rocm.hipconfig_path.is_some() {
        soft_issues += 1;
        println!("  ⚠  ROCm/HIP tooling found, but no supported gfx target was detected");
        if let Some(error) = &profile.rocm.rocminfo_error {
            println!("     → rocminfo: {error}");
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
        println!("  {soft_issues} soft preflight warning(s) found.");
        println!("  lmml can run; fix the warning(s) above for better build/runtime behavior.");
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

#[cfg(test)]
mod train_tests {
    use super::*;

    #[test]
    fn upstream_finetune_maps_to_model_file_and_output_args() {
        let argv = build_train_argv(
            TrainArgInputs {
                model: Path::new("/models/qwen.gguf"),
                train_data: Path::new("/data/train.jsonl"),
                output: Some(Path::new("/out/finetuned.gguf")),
                lora_out: None,
                checkpoint_in: None,
                checkpoint_out: None,
                extra_args: &["--epochs".to_string(), "3".to_string()],
            },
            &FinetuneCapabilities::default(),
        )
        .expect("upstream argv");

        assert_eq!(
            argv,
            vec![
                "--model",
                "/models/qwen.gguf",
                "--file",
                "/data/train.jsonl",
                "--output",
                "/out/finetuned.gguf",
                "--epochs",
                "3",
            ]
        );
    }

    #[test]
    fn custom_lora_flags_require_declared_capabilities() {
        let error = build_train_argv(
            TrainArgInputs {
                model: Path::new("/models/qwen.gguf"),
                train_data: Path::new("/data/train.jsonl"),
                output: None,
                lora_out: Some(Path::new("/out/adapter.bin")),
                checkpoint_in: None,
                checkpoint_out: None,
                extra_args: &[],
            },
            &FinetuneCapabilities::default(),
        )
        .expect_err("missing lora-out capability");

        assert_eq!(
            error,
            "installed llama-finetune does not advertise --lora-out"
        );
    }

    #[test]
    fn custom_finetune_capabilities_use_custom_flag_names() {
        let capabilities = FinetuneCapabilities::from_help(
            "--model-base --train-data --lora-out --checkpoint-in --checkpoint-out",
        );
        let argv = build_train_argv(
            TrainArgInputs {
                model: Path::new("/models/qwen.gguf"),
                train_data: Path::new("/data/train.jsonl"),
                output: None,
                lora_out: Some(Path::new("/out/adapter.bin")),
                checkpoint_in: Some(Path::new("/ckpt/in.bin")),
                checkpoint_out: Some(Path::new("/ckpt/out.bin")),
                extra_args: &["--epochs".to_string(), "3".to_string()],
            },
            &capabilities,
        )
        .expect("custom argv");

        assert_eq!(
            argv,
            vec![
                "--model-base",
                "/models/qwen.gguf",
                "--train-data",
                "/data/train.jsonl",
                "--lora-out",
                "/out/adapter.bin",
                "--checkpoint-in",
                "/ckpt/in.bin",
                "--checkpoint-out",
                "/ckpt/out.bin",
                "--epochs",
                "3",
            ]
        );
    }

    #[test]
    fn export_lora_maps_to_base_lora_and_output_args() {
        let argv = build_export_lora_argv(
            Path::new("/models/qwen-f16.gguf"),
            Path::new("/out/adapter.gguf"),
            Path::new("/out/qwen-finetuned.gguf"),
        );

        assert_eq!(
            argv,
            vec![
                "--model",
                "/models/qwen-f16.gguf",
                "--lora",
                "/out/adapter.gguf",
                "--output",
                "/out/qwen-finetuned.gguf",
            ]
        );
    }

    #[test]
    fn training_binaries_live_next_to_server_binary() {
        assert_eq!(
            finetune_binary_path(Path::new("/lmml/llama.cpp/build/bin/llama-server")),
            PathBuf::from("/lmml/llama.cpp/build/bin").join(binary_name("llama-finetune"))
        );
        assert_eq!(
            export_lora_binary_path(Path::new("/lmml/llama.cpp/build/bin/llama-server")),
            PathBuf::from("/lmml/llama.cpp/build/bin").join(binary_name("llama-export-lora"))
        );
    }
}
