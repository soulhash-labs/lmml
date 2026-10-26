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
use serde::de::DeserializeOwned;

use lmml_tui::app::App;
use lmml_tui::event_loop::EventLoop;
use lmml_tui::runtime_cli::{self, RoutingOptions, RoutingSource};
use lmml_tui::tabs;
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;

mod lifecycle_runtime_cli;
mod model_cli;
mod ocr_cli;
mod template;

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
    /// Run OCR on an image with llama-mtmd-cli and Unlimited-OCR defaults.
    Ocr {
        /// Image path to process.
        #[arg(long, short = 'i')]
        image: PathBuf,
        /// Unlimited-OCR language GGUF. Defaults to the selected lmml model.
        #[arg(long)]
        model: Option<PathBuf>,
        /// Multimodal projector GGUF. Defaults to mmproj-unlimited-ocr-F16.gguf beside the model.
        #[arg(long)]
        mmproj: Option<PathBuf>,
        /// OCR prompt. Use "document parsing." for layout/grounding output.
        #[arg(long)]
        prompt: Option<String>,
        /// Context size passed to llama-mtmd-cli. Defaults to the active OCR profile.
        #[arg(long, short = 'c')]
        ctx_size: Option<u32>,
        /// Maximum generated tokens. Defaults to the active OCR profile.
        #[arg(long, short = 'n')]
        predict: Option<u32>,
        /// Allow llama.cpp warmup instead of passing --no-warmup.
        #[arg(long)]
        warmup: bool,
        /// Additional llama-mtmd-cli arguments after `--`.
        #[arg(last = true)]
        extra_args: Vec<String>,
    },
    /// Manage long-running llama-server runtimes for coding harnesses.
    Runtime {
        #[command(subcommand)]
        command: RuntimeCommand,
    },
    /// Import and verify canonical model substrates and their manifests.
    Model {
        #[command(subcommand)]
        command: model_cli::ModelCommand,
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
    /// Serve one exact, signed approval through a private backend gateway.
    ServeApproved {
        /// GGUF model to serve.
        #[arg(long)]
        model: PathBuf,
        /// llama-server executable.
        #[arg(long)]
        binary: PathBuf,
        /// Prompt configuration JSON used for review.
        #[arg(long)]
        prompt: PathBuf,
        /// Runtime configuration JSON used for review.
        #[arg(long)]
        runtime: PathBuf,
        /// Deployment manifest JSON used for review.
        #[arg(long)]
        deployment: PathBuf,
        /// Content-addressed deployment snapshot root.
        #[arg(long)]
        snapshot_root: PathBuf,
        /// Signed approval receipt JSON.
        #[arg(long)]
        receipt: PathBuf,
        /// Pinned approval trust root JSON.
        #[arg(long)]
        trust_root: PathBuf,
        /// Signed revocation state JSON, when a revocation store is configured.
        #[arg(long)]
        revocation_state: Option<PathBuf>,
        /// Private backend port.
        #[arg(long, default_value_t = 1200)]
        backend_port: u16,
        /// Bearer token required by the private backend.
        #[arg(long)]
        backend_api_key: String,
        /// Public gateway port.
        #[arg(long, default_value_t = 8080)]
        port: u16,
        /// Context size passed to llama-server.
        #[arg(long, default_value_t = 4096)]
        context: u32,
        /// GPU layers passed to llama-server.
        #[arg(long, default_value_t = -1)]
        gpu_layers: i32,
        /// llama-server worker threads.
        #[arg(long, default_value_t = 0)]
        threads: usize,
        /// Emit machine-readable launch evidence.
        #[arg(long)]
        json: bool,
    },
    /// Start a managed runtime from one validated approval bundle.
    ServeApprovedBundle {
        /// Approval bundle JSON produced by review tooling.
        #[arg(long)]
        bundle: PathBuf,
        /// Emit machine-readable launch evidence.
        #[arg(long)]
        json: bool,
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
        /// Oh My OpenAgent config path override.
        #[arg(long)]
        openagent_path: Option<PathBuf>,
        /// Update only opencode.json and leave Oh My OpenAgent untouched.
        #[arg(long)]
        opencode_only: bool,
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
    /// Register a live local llama.cpp process against an admitted artifact.
    Register {
        /// Stable runtime identifier.
        #[arg(long)]
        runtime_id: String,
        /// Exact admitted artifact identifier.
        #[arg(long)]
        artifact_id: String,
        /// PID of the local llama.cpp process.
        #[arg(long)]
        pid: u32,
        /// Runtime endpoint granted to clients.
        #[arg(long)]
        endpoint: String,
        /// Exact llama.cpp version or revision.
        #[arg(long)]
        backend_version: String,
        /// Active context size.
        #[arg(long)]
        context_size: usize,
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Request a capability lease without exposing an artifact path.
    Request {
        /// Conceptual model lineage.
        #[arg(long)]
        lineage_id: String,
        /// Workload purpose recorded in the request.
        #[arg(long)]
        purpose: String,
        /// Required runtime capabilities.
        #[arg(long, value_enum, default_value = "text-generation")]
        capability: Vec<lifecycle_runtime_cli::CapabilityArg>,
        /// Stable lease identifier.
        #[arg(long)]
        lease_id: String,
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Inspect a persisted runtime manifest or model lease.
    Inspect {
        /// Runtime or lease manifest path.
        manifest: PathBuf,
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Clone, clap::ValueEnum)]
enum RuntimeConfigTarget {
    /// OpenCode provider config.
    Opencode,
    /// Codex CLI profile config.
    Codex,
    /// DeepSeek Harness custom OpenAI-compatible provider fragment.
    DeepseekHarness,
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
    openagent_path: Option<PathBuf>,
    opencode_only: bool,
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
        Some(Command::Ocr {
            image,
            model,
            mmproj,
            prompt,
            ctx_size,
            predict,
            warmup,
            extra_args,
        }) => {
            let code = ocr_cli::run_ocr(ocr_cli::OcrRequest {
                image,
                model,
                mmproj,
                prompt,
                ctx_size,
                predict,
                warmup,
                extra_args,
            })
            .await;
            std::process::exit(code);
        }
        Some(Command::Runtime { command }) => {
            let code = run_runtime(command).await;
            std::process::exit(code);
        }
        Some(Command::Model { command }) => {
            let code = model_cli::run(command).await;
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
        RuntimeCommand::ServeApproved {
            model,
            binary,
            prompt,
            runtime,
            deployment,
            snapshot_root,
            receipt,
            trust_root,
            revocation_state,
            backend_port,
            backend_api_key,
            port,
            context,
            gpu_layers,
            threads,
            json,
        } => {
            run_runtime_serve_approved(ServeApprovedOptions {
                model,
                binary,
                prompt,
                runtime,
                deployment,
                snapshot_root,
                receipt,
                trust_root,
                revocation_state,
                backend_port,
                backend_api_key,
                port,
                context,
                gpu_layers,
                threads,
                json,
            })
            .await
        }
        RuntimeCommand::ServeApprovedBundle { bundle, json } => {
            let bundle = match lmml_substrate::load_approval_bundle(&bundle) {
                Ok(bundle) => bundle,
                Err(error) => return serve_error("approval bundle", error),
            };
            let backend_api_key = match std::env::var(&bundle.backend_api_key_env) {
                Ok(value) if !value.is_empty() => value,
                Ok(_) => {
                    return serve_error(
                        "approval bundle",
                        format!("{} is empty", bundle.backend_api_key_env),
                    )
                }
                Err(_) => {
                    return serve_error(
                        "approval bundle",
                        format!("{} is not set", bundle.backend_api_key_env),
                    )
                }
            };
            let prompt: lmml_substrate::PromptConfiguration = match read_main_json(&bundle.prompt) {
                Ok(value) => value,
                Err(error) => return serve_error("approval bundle prompt", error),
            };
            let runtime: lmml_substrate::RuntimeConfiguration =
                match read_main_json(&bundle.runtime) {
                    Ok(value) => value,
                    Err(error) => return serve_error("approval bundle runtime", error),
                };
            let deployment: lmml_substrate::DeploymentManifest =
                match read_main_json(&bundle.deployment) {
                    Ok(value) => value,
                    Err(error) => return serve_error("approval bundle deployment", error),
                };
            let model = match resolve_snapshot_file(
                &bundle.snapshot_root,
                &deployment,
                &prompt.artifact_hash,
                "model",
            ) {
                Ok(path) => path,
                Err(error) => return serve_error("approval bundle model", error),
            };
            let binary = match resolve_snapshot_file(
                &bundle.snapshot_root,
                &deployment,
                &runtime.executable_hash,
                "llama-server",
            ) {
                Ok(path) => path,
                Err(error) => return serve_error("approval bundle binary", error),
            };
            run_runtime_serve_approved(ServeApprovedOptions {
                model,
                binary,
                prompt: bundle.prompt,
                runtime: bundle.runtime,
                deployment: bundle.deployment,
                snapshot_root: bundle.snapshot_root,
                receipt: bundle.receipt,
                trust_root: bundle.trust_root,
                revocation_state: bundle.revocation_state,
                backend_port: bundle.backend_port,
                backend_api_key,
                port: bundle.public_port,
                context: bundle.context,
                gpu_layers: bundle.gpu_layers,
                threads: bundle.threads,
                json,
            })
            .await
        }
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
            openagent_path,
            opencode_only,
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
                    openagent_path,
                    opencode_only,
                    rollback,
                    yes,
                    force,
                    routing: RoutingOptions {
                        model: model_source.into(),
                        small_model: small_model_source.into(),
                    },
                },
            )
            .await
        }
        RuntimeCommand::Register {
            runtime_id,
            artifact_id,
            pid,
            endpoint,
            backend_version,
            context_size,
            json,
        } => {
            lifecycle_runtime_cli::register(lifecycle_runtime_cli::RegisterOptions {
                runtime_id: &runtime_id,
                artifact_id: &artifact_id,
                pid,
                endpoint: &endpoint,
                backend_version: &backend_version,
                context_size,
                json,
            })
            .await
        }
        RuntimeCommand::Request {
            lineage_id,
            purpose,
            capability,
            lease_id,
            json,
        } => {
            lifecycle_runtime_cli::request(&lineage_id, &purpose, &capability, &lease_id, json)
                .await
        }
        RuntimeCommand::Inspect { manifest, json } => {
            lifecycle_runtime_cli::inspect(&manifest, json)
        }
    }
}

struct ServeApprovedOptions {
    model: PathBuf,
    binary: PathBuf,
    prompt: PathBuf,
    runtime: PathBuf,
    deployment: PathBuf,
    snapshot_root: PathBuf,
    receipt: PathBuf,
    trust_root: PathBuf,
    revocation_state: Option<PathBuf>,
    backend_port: u16,
    backend_api_key: String,
    port: u16,
    context: u32,
    gpu_layers: i32,
    threads: usize,
    json: bool,
}

async fn run_runtime_serve_approved(options: ServeApprovedOptions) -> i32 {
    let prompt: lmml_substrate::PromptConfiguration = match read_main_json(&options.prompt) {
        Ok(value) => value,
        Err(error) => return serve_error("prompt configuration", error),
    };
    let runtime: lmml_substrate::RuntimeConfiguration = match read_main_json(&options.runtime) {
        Ok(value) => value,
        Err(error) => return serve_error("runtime configuration", error),
    };
    let deployment: lmml_substrate::DeploymentManifest = match read_main_json(&options.deployment) {
        Ok(value) => value,
        Err(error) => return serve_error("deployment manifest", error),
    };
    if let Err(error) =
        lmml_substrate::verify_deployment_snapshot(&options.snapshot_root, &deployment)
    {
        return serve_error("deployment snapshot", error);
    }
    let trust_root = match lmml_substrate::load_approval_trust_root(&options.trust_root) {
        Ok(value) => value,
        Err(error) => return serve_error("trust root", error),
    };
    let receipt = match lmml_substrate::load_approval_receipt(&options.receipt, &trust_root) {
        Ok(value) => value,
        Err(error) => return serve_error("approval receipt", error),
    };
    let (revoked, revocation_epoch) = match options.revocation_state.as_deref() {
        Some(path) => match lmml_substrate::load_revocation_state(path, &trust_root) {
            Ok(state) => (
                lmml_substrate::is_receipt_revoked(&state, &receipt.receipt_id),
                state.epoch,
            ),
            Err(error) => return serve_error("revocation state", error),
        },
        None => (false, 0),
    };
    let prompt_hash = match lmml_substrate::prompt_configuration_hash(&prompt) {
        Ok(value) => value,
        Err(error) => return serve_error("prompt configuration", error),
    };
    let runtime_hash = match lmml_substrate::runtime_configuration_hash(&runtime) {
        Ok(value) => value,
        Err(error) => return serve_error("runtime configuration", error),
    };
    let deployment_hash = match lmml_substrate::deployment_manifest_hash(&deployment) {
        Ok(value) => value,
        Err(error) => return serve_error("deployment manifest", error),
    };
    let admission = lmml_substrate::compute_admission_status(
        Some(&receipt),
        &prompt_hash,
        &runtime_hash,
        &deployment_hash,
        revoked,
    );
    if admission.state != lmml_substrate::AdmissionState::Approved {
        return serve_error("runtime admission", admission.reason);
    }
    let model_hash = match lmml_substrate::sha256_file(&options.model) {
        Ok(value) => value,
        Err(error) => return serve_error("model hash", error),
    };
    let supplied_binary_hash = match lmml_substrate::sha256_file(&options.binary) {
        Ok(value) => value,
        Err(error) => return serve_error("binary hash", error),
    };
    if supplied_binary_hash != runtime.executable_hash {
        return serve_error(
            "runtime admission",
            "supplied llama-server is not the executable bound by approval".to_string(),
        );
    }
    if model_hash != prompt.artifact_hash || model_hash != deployment.artifact_hash {
        return serve_error(
            "runtime admission",
            "model hash does not match approval".to_string(),
        );
    }
    let model_path =
        match resolve_snapshot_file(&options.snapshot_root, &deployment, &model_hash, "model") {
            Ok(path) => path,
            Err(error) => return serve_error("deployment snapshot", error),
        };
    let binary_path = match resolve_snapshot_file(
        &options.snapshot_root,
        &deployment,
        &runtime.executable_hash,
        "llama-server",
    ) {
        Ok(path) => path,
        Err(error) => return serve_error("deployment snapshot", error),
    };
    if runtime.prompt_configuration_hash != prompt_hash
        || deployment.prompt_configuration_hash != prompt_hash
        || deployment.runtime_configuration_hash != runtime_hash
    {
        return serve_error(
            "runtime admission",
            "prompt, runtime, and deployment identities are not mutually bound".to_string(),
        );
    }
    let model = match lmml_models::parse_model_file(&model_path, false).await {
        Some(model) => model,
        None => return serve_error("model", "not a readable GGUF model".to_string()),
    };
    let caps = match lmml_compat::LlamaBinaryCapabilities::probe(&binary_path).await {
        Ok(caps) => caps,
        Err(error) => return serve_error("llama-server capability probe", error),
    };
    let server_config = lmml_compat::ServerConfig {
        model: model_path,
        port: options.backend_port,
        host: "127.0.0.1".into(),
        ctx_size: options.context,
        n_gpu_layers: options.gpu_layers,
        batch_size: 512,
        ubatch_size: 512,
        threads: options.threads,
        flash_attn: true,
        mlock: false,
        api_key: Some(options.backend_api_key.clone()),
        chat_template: None,
        jinja: false,
        extra_args: Vec::new(),
    };
    let actual_argv = lmml_compat::build_argv(&server_config, &caps);
    if actual_argv != runtime.resolved_argv {
        return serve_error(
            "runtime admission",
            "resolved llama-server argv differs from the signed runtime configuration".to_string(),
        );
    }
    let executable_hash = match lmml_substrate::sha256_file(&binary_path) {
        Ok(value) => value,
        Err(error) => return serve_error("llama-server hash", error),
    };
    if executable_hash != runtime.executable_hash {
        return serve_error(
            "runtime admission",
            "llama-server hash differs from approval".to_string(),
        );
    }
    if prompt.renderer_id != "llama.cpp/server-native"
        || prompt.renderer_build_hash != executable_hash
    {
        return serve_error(
            "runtime admission",
            "approval does not bind the exact production llama.cpp renderer".to_string(),
        );
    }
    let gateway_admission = lmml_gateway::GatewayAdmission::pending_with_store(
        lmml_substrate::AdmissionStatus {
            state: lmml_substrate::AdmissionState::Blocked,
            reason: "post-load verification pending".into(),
        },
        lmml_gateway::ApprovalStore {
            receipt_path: options.receipt.clone(),
            trust_root_path: options.trust_root.clone(),
            revocation_state_path: options.revocation_state.clone(),
            prompt_hash: prompt_hash.clone(),
            runtime_hash: runtime_hash.clone(),
            deployment_hash: deployment_hash.clone(),
        },
    );
    let attestation_key = match std::env::var("LMML_GATEWAY_ATTESTATION_PRIVATE_KEY_HEX") {
        Ok(value) => match decode_hex(&value) {
            Some(value) => Some(value),
            None => return serve_error("gateway attestation", "invalid private key hex"),
        },
        Err(_) => None,
    };
    if attestation_key.is_none() {
        return serve_error(
            "gateway attestation",
            "LMML_GATEWAY_ATTESTATION_PRIVATE_KEY_HEX is required for an approved runtime",
        );
    }
    let gateway_id = match std::env::var("LMML_GATEWAY_ID") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => return serve_error("gateway attestation", "LMML_GATEWAY_ID is required"),
    };
    let gateway_admission_for_activation = gateway_admission.clone();
    let gateway = match lmml_gateway::GatewayState::new(
        lmml_gateway::GatewayConfig {
            listen: ([127, 0, 0, 1], options.port).into(),
            backend_url: format!("http://127.0.0.1:{}", options.backend_port),
            backend_api_key: Some(options.backend_api_key),
            gateway_id: Some(gateway_id),
            deployment_hash: Some(deployment_hash.to_string()),
            receipt_id: Some(receipt.receipt_id.clone()),
            receipt_sequence: receipt.sequence,
            revocation_epoch,
            attestation_private_key: attestation_key,
            ..lmml_gateway::GatewayConfig::default()
        },
        prompt.request_schema,
        gateway_admission,
    ) {
        Ok(gateway) => gateway,
        Err(error) => return serve_error("gateway", error),
    };
    let (log_tx, mut log_rx) = tokio::sync::mpsc::channel(256);
    tokio::spawn(async move {
        while let Some(line) = log_rx.recv().await {
            tracing::info!(target: "lmml.approved_runtime", "{line}");
        }
    });
    let manager = lmml_server::ServerManager {
        binary: binary_path,
        caps,
    };
    let environment = match approved_environment(&runtime.allowed_environment) {
        Ok(environment) => environment,
        Err(error) => return serve_error("runtime environment", error),
    };
    let handle = match manager
        .start_with_gateway_and_env(lmml_server::GatewayStartOptions {
            model: &model,
            backend_config: &server_config,
            gateway,
            listen: ([127, 0, 0, 1], options.port).into(),
            log_tx,
            startup_timeout: lmml_server::default_startup_timeout(),
            environment: Some(&environment),
        })
        .await
    {
        Ok(handle) => handle,
        Err(error) => return serve_error("approved runtime", error),
    };
    match lmml_substrate::sha256_file(&server_config.model) {
        Ok(hash) if hash == model_hash => {}
        Ok(_) => {
            handle.stop().await;
            return serve_error(
                "runtime admission",
                "model bytes changed before or during backend load".to_string(),
            );
        }
        Err(error) => {
            handle.stop().await;
            return serve_error("runtime admission", error.to_string());
        }
    }
    if let Err(error) = verify_loaded_libraries(&handle, &runtime.loaded_libraries).await {
        handle.stop().await;
        return serve_error("runtime admission", error);
    }
    if let Err(error) = gateway_admission_for_activation
        .activate_if_approved()
        .await
    {
        handle.stop().await;
        return serve_error("runtime admission", error);
    }
    let endpoint = format!("http://127.0.0.1:{}", options.port);
    if options.json {
        let evidence = serde_json::json!({
            "status": "ready",
            "endpoint": &endpoint,
            "pid": handle.pid().await,
            "receipt_id": receipt.receipt_id,
            "receipt_sequence": receipt.sequence,
            "prompt_configuration_hash": prompt_hash,
            "runtime_configuration_hash": runtime_hash,
            "deployment_manifest_hash": deployment_hash,
            "model_hash": model_hash,
            "executable_hash": executable_hash,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&evidence).unwrap_or_default()
        );
    } else {
        println!("approved runtime ready at {endpoint}");
    }
    let _ = tokio::signal::ctrl_c().await;
    handle.stop().await;
    0
}

fn decode_hex(value: &str) -> Option<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return None;
    }
    (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&value[index..index + 2], 16).ok())
        .collect()
}

fn approved_environment(
    entries: &[lmml_substrate::EnvironmentIdentity],
) -> Result<std::collections::BTreeMap<String, String>, String> {
    let mut environment = std::collections::BTreeMap::new();
    for entry in entries {
        if !entry.secret_presence_only && entry.value_hash.is_none() {
            return Err(format!(
                "non-secret environment entry has no value hash: {}",
                entry.name
            ));
        }
        let value = std::env::var(&entry.name).map_err(|_| {
            format!(
                "approved environment variable is unavailable: {}",
                entry.name
            )
        })?;
        if !entry.secret_presence_only {
            let Some(expected_hash) = entry.value_hash.as_ref() else {
                return Err(format!("environment hash is missing: {}", entry.name));
            };
            let actual_hash = lmml_substrate::sha256_data(value.as_bytes());
            if &actual_hash != expected_hash {
                return Err(format!(
                    "approved environment variable changed: {}",
                    entry.name
                ));
            }
        }
        environment.insert(entry.name.clone(), value);
    }
    Ok(environment)
}

fn resolve_snapshot_file(
    snapshot_root: &Path,
    deployment: &lmml_substrate::DeploymentManifest,
    expected_hash: &lmml_substrate::Hash256,
    label: &str,
) -> Result<PathBuf, String> {
    let matches: Vec<_> = deployment
        .files
        .iter()
        .filter(|identity| &identity.sha256 == expected_hash)
        .collect();
    let identity = match matches.as_slice() {
        [identity] => identity,
        [] => {
            return Err(format!(
                "deployment has no {label} matching the approved hash"
            ))
        }
        _ => return Err(format!("deployment has multiple files matching {label}")),
    };
    let path = snapshot_root.join(&identity.path);
    if !path.is_file() {
        return Err(format!("approved {label} is missing: {}", path.display()));
    }
    Ok(path)
}

async fn verify_loaded_libraries(
    handle: &lmml_server::ServerHandle,
    declared: &[lmml_substrate::LibraryIdentity],
) -> Result<(), String> {
    if declared.is_empty() {
        return Err("runtime approval declares no loader closure".into());
    }
    let pid = handle
        .pid()
        .await
        .ok_or_else(|| "managed backend exited before library verification".to_string())?;
    #[cfg(target_os = "linux")]
    {
        let maps = std::fs::read_to_string(format!("/proc/{pid}/maps"))
            .map_err(|error| format!("read loaded-library map: {error}"))?;
        let mut observed = std::collections::BTreeSet::new();
        for line in maps.lines() {
            let Some(path) = line.split_whitespace().last() else {
                continue;
            };
            if path.starts_with('/') && path.contains(".so") {
                observed.insert(std::path::PathBuf::from(path));
            }
        }
        for path in &observed {
            let identity = declared.iter().find(|identity| identity.path == *path);
            let Some(identity) = identity else {
                return Err(format!("unlisted loaded library: {}", path.display()));
            };
            let actual = lmml_substrate::sha256_file(path)
                .map_err(|error| format!("hash loaded library {}: {error}", path.display()))?;
            if actual != identity.sha256 {
                return Err(format!("loaded library hash changed: {}", path.display()));
            }
        }
        for identity in declared.iter().filter(|identity| identity.required) {
            if !observed.contains(&identity.path) {
                return Err(format!(
                    "required declared library was not loaded: {}",
                    identity.path.display()
                ));
            }
        }
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (pid, declared);
        Err("loaded-library admission is not implemented on this platform".into())
    }
}

fn read_main_json<T: DeserializeOwned>(path: &Path) -> Result<T, String> {
    let payload = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    serde_json::from_str(&payload).map_err(|error| error.to_string())
}

fn serve_error(context: &str, error: impl std::fmt::Display) -> i32 {
    eprintln!("{context} failed: {error}");
    1
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
        RuntimeConfigTarget::Codex => match runtime_cli::render_codex_config(state) {
            Ok(rendered) => {
                println!("{rendered}");
                0
            }
            Err(error) => {
                eprintln!("failed to render Codex config: {error}");
                1
            }
        },
        RuntimeConfigTarget::DeepseekHarness => {
            match runtime_cli::render_deepseek_harness_config(state) {
                Ok(rendered) => {
                    println!("{rendered}");
                    0
                }
                Err(error) => {
                    eprintln!("failed to render DeepSeek Harness config: {error}");
                    1
                }
            }
        }
    }
}

async fn run_runtime_configure(state: &lmml_state::AppState, args: RuntimeConfigureArgs) -> i32 {
    match args.target {
        RuntimeConfigTarget::Opencode => {
            let path = args
                .path
                .unwrap_or_else(runtime_cli::default_opencode_config_path);
            let openagent_path = args
                .openagent_path
                .unwrap_or_else(runtime_cli::default_openagent_config_path);
            let sync_openagent = !args.opencode_only && openagent_path.is_file();
            if let Some(backup) = args.rollback {
                if !args.opencode_only {
                    eprintln!("rollback restores one file at a time; pass --opencode-only for the OpenCode backup and restore the printed Oh My OpenAgent backup separately");
                    return 2;
                }
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
            if sync_openagent && args.routing != RoutingOptions::default() {
                eprintln!("Oh My OpenAgent synchronization requires --model-source lmml and --small-model-source lmml; use --opencode-only for custom routing");
                return 2;
            }
            let (plan, openagent_diff) = if !sync_openagent {
                match runtime_cli::plan_opencode_configure(state, &path, args.routing, args.force) {
                    Ok(plan) => (plan, None),
                    Err(error) => {
                        eprintln!("configure plan failed: {error}");
                        return 1;
                    }
                }
            } else {
                match runtime_cli::plan_opencode_handoff(state, &path, &openagent_path, args.force)
                {
                    Ok(handoff) => (handoff.opencode, Some(handoff.openagent_diff)),
                    Err(error) => {
                        eprintln!("configure plan failed: {error}");
                        return 1;
                    }
                }
            };
            println!("OpenCode config: {}", plan.path.display());
            print_diff(&plan.diff);
            if let Some(diff) = &openagent_diff {
                println!("Oh My OpenAgent config: {}", openagent_path.display());
                print_diff(diff);
            } else if !args.opencode_only {
                println!(
                    "Oh My OpenAgent config not found at {}; updating OpenCode only",
                    openagent_path.display()
                );
            }
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
            if args.routing.model == RoutingSource::Lmml
                || args.routing.small_model == RoutingSource::Lmml
            {
                let (host, port, model, api_key) = if state.runtime.mode
                    == lmml_state::OpenCodeRuntimeMode::SingleServer
                    && !state.model.last_used.as_os_str().is_empty()
                {
                    (
                        state.server.host.as_str(),
                        state.server.port,
                        state.model.last_used.as_path(),
                        (!state.server.api_key.is_empty()).then_some(state.server.api_key.as_str()),
                    )
                } else {
                    let profile = &state.runtime.opencode;
                    (
                        profile.host.as_str(),
                        profile.port,
                        profile.model.as_path(),
                        None,
                    )
                };
                if runtime_cli::approval_enforcement_enabled() {
                    if let Err(error) = verify_approval_gateway_endpoint(host, port, api_key).await
                    {
                        eprintln!(
                            "refusing OpenCode routing because approval enforcement requires the LMML gateway: {error}"
                        );
                        return 1;
                    }
                }
                match lmml_server::verify_served_model(host, port, model, api_key).await {
                    Ok(reported) => {
                        println!("verified live llama-server model: {reported}");
                    }
                    Err(error) => {
                        eprintln!(
                            "refusing to update OpenCode routing because the selected server/model is not healthy: {error}"
                        );
                        return 1;
                    }
                }
                let expected_context = match runtime_cli::opencode_context_per_slot(state) {
                    Ok(context) => context,
                    Err(error) => {
                        eprintln!("failed to resolve OpenCode context capacity: {error}");
                        return 1;
                    }
                };
                match lmml_server::verify_context_capacity(host, port, expected_context, api_key)
                    .await
                {
                    Ok(properties) => println!(
                        "verified live llama-server context: {} tokens per slot across {} slot(s)",
                        properties.context_per_slot, properties.total_slots
                    ),
                    Err(error) => {
                        eprintln!(
                            "refusing to publish OpenCode context metadata because the live server does not match LMML: {error}"
                        );
                        return 1;
                    }
                }
            }
            if !args.yes && !confirm("Apply OpenCode config changes? [y/N] ") {
                println!("aborted; no files written");
                return 2;
            }
            if !sync_openagent {
                match runtime_cli::apply_opencode_configure(state, &path, args.routing, args.force)
                {
                    Ok(applied) => {
                        println!("updated {}", applied.path.display());
                        println!("backup: {}", applied.backup_path.display());
                        println!(
                            "rollback: lmml runtime configure opencode --opencode-only --path {} --rollback {}",
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
            } else {
                match runtime_cli::apply_opencode_handoff(state, &path, &openagent_path, args.force)
                {
                    Ok(applied) => {
                        println!("updated {}", applied.opencode_path.display());
                        println!("backup: {}", applied.opencode_backup.display());
                        println!("updated {}", applied.openagent_path.display());
                        println!("backup: {}", applied.openagent_backup.display());
                        0
                    }
                    Err(error) => {
                        eprintln!("configure apply failed: {error}");
                        1
                    }
                }
            }
        }
        RuntimeConfigTarget::Codex => {
            eprintln!("automatic Codex config writes are not supported; use `lmml runtime print-config codex` and review the TOML before adding it to ~/.codex/lmml.config.toml");
            2
        }
        RuntimeConfigTarget::DeepseekHarness => {
            eprintln!("automatic DeepSeek Harness config writes are not supported; use `lmml runtime print-config deepseek-harness` and review the YAML before adding it to $DSH_HOME/settings.yaml");
            2
        }
    }
}

async fn verify_approval_gateway_endpoint(
    host: &str,
    port: u16,
    api_key: Option<&str>,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|error| error.to_string())?;
    let mut request = client.get(format!("http://{host}:{port}/health"));
    if let Some(api_key) = api_key.filter(|value| !value.is_empty()) {
        request = request.bearer_auth(api_key);
    }
    let response = request.send().await.map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("gateway health returned {}", response.status()));
    }
    if response
        .headers()
        .get("x-lmml-gateway")
        .and_then(|value| value.to_str().ok())
        != Some("approved")
    {
        return Err("endpoint did not identify as an approved LMML gateway".into());
    }
    Ok(())
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
    println!("model-aware runtime selection: supported");
    println!("runtime policy switching: supported");
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

    let cuda_block = cuda_doctor_block(&profile);
    soft_issues += cuda_block.soft_issues;
    for line in cuda_block.lines {
        println!("{line}");
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
        println!("  ⚠  ROCm/HIP tooling found, but HIP backend is not build-ready");
        if let Some(error) = &profile.rocm.hip_cmake_error {
            println!("     → {error}");
        } else if let Some(error) = &profile.rocm.rocminfo_error {
            println!("     → rocminfo: {error}");
        } else {
            println!("     → rocminfo did not report a supported gfx target");
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

#[derive(Debug, Default, PartialEq, Eq)]
struct DoctorBlock {
    soft_issues: i32,
    lines: Vec<String>,
}

fn cuda_doctor_block(profile: &lmml_detect::SystemProfile) -> DoctorBlock {
    let mut block = DoctorBlock::default();
    let non_cuda_backend_available =
        profile.rocm.available || profile.vulkan.available || profile.metal.available;

    match &profile.cuda {
        lmml_detect::CudaCompatibility::Compatible { archs } => {
            let gpu = profile
                .gpus
                .first()
                .map(|gpu| gpu.name.as_str())
                .unwrap_or("CUDA GPU");
            block.lines.push(format!(
                "  ✓  CUDA available  ·  {gpu}  ·  {}",
                archs.join(", ")
            ));
        }
        lmml_detect::CudaCompatibility::ToolkitTooOld {
            gpu_arch,
            minimum_toolkit,
            found_toolkit,
        } => {
            block.soft_issues += 1;
            block.lines.push(format!(
                "  ⚠  CUDA toolkit {found_toolkit} too old for {gpu_arch}"
            ));
            block
                .lines
                .push(format!("     → install CUDA >= {minimum_toolkit}"));
        }
        lmml_detect::CudaCompatibility::ToolkitTooNew {
            gpu_arch,
            maximum_toolkit,
            found_toolkit,
        } => {
            block.soft_issues += 1;
            block.lines.push(format!(
                "  ⚠  CUDA toolkit {found_toolkit} no longer supports {gpu_arch}"
            ));
            block.lines.push(format!(
                "     → install CUDA {maximum_toolkit} alongside CUDA 13"
            ));
            block.lines.push(
                "     → then run: LMML_CUDA_COMPILER=/usr/local/cuda-12.4/bin/nvcc lmml"
                    .to_string(),
            );
        }
        lmml_detect::CudaCompatibility::NoGpu => {
            if non_cuda_backend_available {
                return block;
            }
            block.soft_issues += 1;
            block
                .lines
                .push("  ⚠  GPU acceleration backend not confirmed".to_string());
            if let Some(error) = &profile.gpu_probe_error {
                block
                    .lines
                    .push(format!("     → CUDA probe: nvidia-smi: {error}"));
            } else {
                block
                    .lines
                    .push("     → CUDA probe: no NVIDIA GPU reported by nvidia-smi".to_string());
            }
            block.lines.push(
                "     → configure CUDA, ROCm/HIP, Vulkan, or run CPU-only mode intentionally"
                    .to_string(),
            );
        }
        lmml_detect::CudaCompatibility::NvccMissing => {
            if non_cuda_backend_available {
                return block;
            }
            block.soft_issues += 1;
            if profile.gpus.is_empty() {
                block
                    .lines
                    .push("  ⚠  GPU acceleration backend not confirmed".to_string());
                block.lines.push(
                    "     → CUDA toolkit not found and no ROCm/HIP, Vulkan, or Metal backend was detected"
                        .to_string(),
                );
                block.lines.push(
                    "     → configure a GPU backend or run CPU-only mode intentionally".to_string(),
                );
            } else {
                block
                    .lines
                    .push("  ⚠  NVIDIA GPU detected but nvcc not found".to_string());
                block.lines.push(
                    "     → install the CUDA toolkit for NVIDIA GPU acceleration".to_string(),
                );
            }
        }
    }

    block
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
mod doctor_tests {
    use super::*;

    fn profile_without_gpu_backend() -> lmml_detect::SystemProfile {
        let mut profile = lmml_detect::SystemProfile::skipped_probe();
        profile.cuda = lmml_detect::CudaCompatibility::NoGpu;
        profile.gpu_probe_error = Some("No such file or directory (os error 2)".to_string());
        profile
    }

    #[test]
    fn cuda_no_gpu_is_silent_when_rocm_is_available() {
        let mut profile = profile_without_gpu_backend();
        profile.rocm.available = true;
        profile.rocm.targets = vec!["gfx1201".to_string()];

        assert_eq!(cuda_doctor_block(&profile), DoctorBlock::default());
    }

    #[test]
    fn cuda_no_gpu_reports_generic_warning_without_any_gpu_backend() {
        let profile = profile_without_gpu_backend();
        let block = cuda_doctor_block(&profile);

        assert_eq!(block.soft_issues, 1);
        assert!(block
            .lines
            .iter()
            .any(|line| line.contains("GPU acceleration backend not confirmed")));
        assert!(!block
            .lines
            .iter()
            .any(|line| line.contains("NVIDIA driver/GPU probe failed")));
    }

    #[test]
    fn missing_nvcc_is_silent_when_vulkan_is_available() {
        let mut profile = lmml_detect::SystemProfile::skipped_probe();
        profile.vulkan.available = true;
        profile.vulkan.devices = vec!["AMD Radeon AI PRO R9700".to_string()];

        assert_eq!(cuda_doctor_block(&profile), DoctorBlock::default());
    }
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
