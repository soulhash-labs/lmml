//! Scriptable model-substrate lifecycle commands.
//!
//! This module imports, inspects, and verifies canonical Safetensors manifests.
//! GGUF conversion and artifact admission live in the private [`derive`]
//! module.

mod artifacts;
mod baseline;
mod derive;
mod successor;

use std::path::{Path, PathBuf};

use clap::Subcommand;
use derive::{derive_model, QuantizationArg};
use serde::de::DeserializeOwned;

/// Commands for canonical model substrate registration and inspection.
#[derive(Debug, Subcommand)]
pub(crate) enum ModelCommand {
    /// Import a canonical Safetensors directory and immutably register its manifest.
    Import {
        /// Root directory containing config.json and Safetensors files.
        path: PathBuf,
        /// Stable conceptual lineage identifier.
        #[arg(long)]
        lineage_id: String,
        /// Explicit parent lineage for a successor checkpoint.
        #[arg(long)]
        parent: Option<String>,
        /// Manifest output path. Defaults to LMML's managed data directory.
        #[arg(long)]
        manifest: Option<PathBuf>,
        /// Emit the manifest as JSON on stdout.
        #[arg(long)]
        json: bool,
    },
    /// Inspect a persisted substrate manifest.
    Inspect {
        /// Path to a substrate manifest JSON file.
        manifest: PathBuf,
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Inspect source tokenizer and chat-template assets without executing them.
    InspectTemplate {
        /// Root directory containing tokenizer_config.json and optional template assets.
        source: PathBuf,
        /// Optional deployed GGUF whose embedded templates should be compared.
        #[arg(long)]
        deployed: Option<PathBuf>,
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Verify rendered prompt bytes and token IDs using the exact running llama-server.
    VerifyTemplate {
        /// Base URL of the private llama-server used for the verification.
        #[arg(long)]
        endpoint: String,
        /// Exact llama-server executable used by that instance.
        #[arg(long)]
        binary: PathBuf,
        /// Exact GGUF model loaded by that instance.
        #[arg(long)]
        model: PathBuf,
        /// JSON fixture array containing messages, rendered_prompt_hash, and token_ids.
        #[arg(long)]
        fixtures: PathBuf,
        /// Optional bearer token for a protected llama-server.
        #[arg(long)]
        api_key: Option<String>,
        /// Emit machine-readable evidence.
        #[arg(long)]
        json: bool,
    },
    /// Sign and persist an explicit template/runtime approval receipt.
    ApproveTemplate {
        /// JSON PromptConfiguration produced by inspection/review tooling.
        #[arg(long)]
        prompt: PathBuf,
        /// JSON RuntimeConfiguration for the exact launch.
        #[arg(long)]
        runtime: PathBuf,
        /// JSON DeploymentManifest for the immutable snapshot.
        #[arg(long)]
        deployment: PathBuf,
        /// Operator-pinned trust-root JSON.
        #[arg(long)]
        trust_root: PathBuf,
        /// Operator-held Ed25519 PKCS#8 private key.
        #[arg(long)]
        private_key: PathBuf,
        /// Stable receipt identifier.
        #[arg(long)]
        receipt_id: String,
        /// Monotonic receipt sequence.
        #[arg(long)]
        sequence: u64,
        /// Authorized reviewer identity.
        #[arg(long)]
        reviewer_id: String,
        /// Approval policy version.
        #[arg(long, default_value = "template-approval-v1")]
        policy_version: String,
        /// Validation-suite identity.
        #[arg(long, default_value = "manual-review-v1")]
        validation_suite_id: String,
        /// Receipt storage directory.
        #[arg(long)]
        store_dir: PathBuf,
        /// Protected receipt sequence anchor.
        #[arg(long)]
        anchor: PathBuf,
        /// Content-addressed deployment snapshot root.
        #[arg(long)]
        snapshot_root: Option<PathBuf>,
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
        /// Optional managed bundle output for approved launch.
        #[arg(long)]
        bundle_out: Option<PathBuf>,
        /// Environment variable containing the private backend bearer token.
        #[arg(long, default_value = "LMML_BACKEND_API_KEY")]
        backend_api_key_env: String,
        /// Public gateway port recorded in the bundle.
        #[arg(long, default_value_t = 8080)]
        public_port: u16,
        /// Private backend port recorded in the bundle.
        #[arg(long, default_value_t = 1200)]
        backend_port: u16,
        /// Context size recorded in the bundle.
        #[arg(long, default_value_t = 262_144)]
        context: u32,
        /// GPU layer setting recorded in the bundle.
        #[arg(long, default_value_t = -1)]
        gpu_layers: i32,
        /// Worker thread setting recorded in the bundle.
        #[arg(long, default_value_t = 0)]
        threads: usize,
    },
    /// Rotate the pinned approval trust root through an old-key-signed migration.
    RotateApprovalTrustRoot {
        /// Current operator trust-root JSON.
        #[arg(long)]
        old_root: PathBuf,
        /// Current operator Ed25519 PKCS#8 private key.
        #[arg(long)]
        old_private_key: PathBuf,
        /// Identifier for the replacement key.
        #[arg(long)]
        new_key_id: String,
        /// Replacement trust-root JSON destination.
        #[arg(long)]
        new_root_out: PathBuf,
        /// Replacement private-key destination.
        #[arg(long)]
        new_private_key_out: PathBuf,
        /// Signed migration record destination.
        #[arg(long)]
        migration_out: PathBuf,
        /// Monotonic migration sequence.
        #[arg(long)]
        sequence: u64,
    },
    /// Compute current template/runtime/deployment admission without launching.
    TemplateStatus {
        /// JSON PromptConfiguration.
        #[arg(long)]
        prompt: PathBuf,
        /// JSON RuntimeConfiguration.
        #[arg(long)]
        runtime: PathBuf,
        /// JSON DeploymentManifest.
        #[arg(long)]
        deployment: PathBuf,
        /// Signed approval receipt JSON.
        #[arg(long)]
        receipt: PathBuf,
        /// Pinned approval trust root JSON.
        #[arg(long)]
        trust_root: PathBuf,
        /// Optional signed revocation state JSON.
        #[arg(long)]
        revocation_state: Option<PathBuf>,
        /// Optional content-addressed snapshot root to verify before status.
        #[arg(long)]
        snapshot_root: Option<PathBuf>,
        /// Emit machine-readable output.
        #[arg(long)]
        json: bool,
    },
    /// Re-hash a canonical directory and compare it with its manifest.
    Verify {
        /// Root directory containing the canonical Safetensors files.
        path: PathBuf,
        /// Path to the substrate manifest JSON file.
        #[arg(long)]
        manifest: PathBuf,
        /// Emit a machine-readable result.
        #[arg(long)]
        json: bool,
    },
    /// Show the conceptual model identity recorded by a manifest.
    Lineage {
        /// Path to a substrate manifest JSON file.
        manifest: PathBuf,
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Convert a verified canonical Safetensors substrate into a GGUF artifact.
    Derive {
        /// Canonical substrate manifest JSON.
        #[arg(long)]
        manifest: PathBuf,
        /// Canonical Safetensors directory.
        #[arg(long)]
        source: PathBuf,
        /// New GGUF output path. It must not already exist.
        #[arg(long)]
        output: PathBuf,
        /// Stable derived artifact identifier.
        #[arg(long)]
        artifact_id: String,
        /// GGUF precision or quantization.
        #[arg(long, value_enum)]
        quant: QuantizationArg,
        /// Path to llama.cpp's convert_hf_to_gguf.py.
        #[arg(long)]
        converter: Option<PathBuf>,
        /// Path to llama-quantize for quantized outputs.
        #[arg(long)]
        quantizer: Option<PathBuf>,
        /// Path to llama-server used to prove the GGUF can be opened.
        #[arg(long)]
        server: Option<PathBuf>,
        /// Python executable used for conversion.
        #[arg(long, default_value = "python3")]
        python: String,
        /// Container runtime for full-model conversion, such as docker or podman.
        #[arg(long)]
        rocm_container: Option<PathBuf>,
        /// Image used with --rocm-container. Defaults to LMML's validated ROCm image.
        #[arg(long)]
        rocm_image: Option<String>,
        /// Publish an immutable candidate without loading it in llama.cpp.
        #[arg(long)]
        defer_admission: bool,
        /// Emit the artifact manifest as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Admit a pending GGUF candidate and register it in the runnable artifact DAG.
    AdmitArtifact {
        /// Immutable pending candidate manifest.
        #[arg(long)]
        candidate: PathBuf,
        /// Canonical substrate manifest JSON.
        #[arg(long)]
        manifest: PathBuf,
        /// Canonical Safetensors directory.
        #[arg(long)]
        source: PathBuf,
        /// Path to llama-server used for tensor and backend admission.
        #[arg(long)]
        server: Option<PathBuf>,
        /// Emit the artifact manifest as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Verify a pending GGUF candidate without loading it in a backend.
    VerifyCandidate {
        /// Immutable pending candidate manifest. Repeat to verify a batch.
        #[arg(long)]
        candidate: Vec<PathBuf>,
        /// Canonical substrate manifest JSON.
        #[arg(long)]
        manifest: PathBuf,
        /// Canonical Safetensors directory.
        #[arg(long)]
        source: PathBuf,
        /// Emit a machine-readable verification report.
        #[arg(long)]
        json: bool,
    },
    /// List validated append-only artifact records.
    Artifacts {
        /// Restrict output to one conceptual lineage.
        #[arg(long)]
        lineage_id: Option<String>,
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Inspect and finite-check a LoRA adapter Safetensors payload.
    InspectAdapter {
        /// Adapter Safetensors file.
        path: PathBuf,
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Execute and persist a deterministic pristine baseline.
    Baseline {
        /// Exact admitted GGUF artifact ID.
        #[arg(long)]
        artifact_id: String,
        /// Stable baseline identifier.
        #[arg(long)]
        baseline_id: String,
        /// JSON array of objects containing case_id and prompt.
        #[arg(long)]
        prompts: PathBuf,
        /// llama-cli executable used for deterministic inference.
        #[arg(long)]
        cli: Option<PathBuf>,
        /// Baseline manifest destination.
        #[arg(long)]
        output: Option<PathBuf>,
        /// Maximum generated tokens per case.
        #[arg(long, default_value_t = 32)]
        predict: u32,
        /// GPU layers passed to llama-cli. Use 0 for a CPU baseline.
        #[arg(long, default_value_t = -1)]
        gpu_layers: i32,
        /// Emit the baseline manifest as JSON.
        #[arg(long)]
        json: bool,
        /// Additional llama-cli arguments after `--`.
        #[arg(last = true)]
        extra_args: Vec<String>,
    },
    /// Authorize the trainable inventory before successor optimization starts.
    AuthorizeSuccessorTraining {
        /// Canonical parent substrate manifest.
        #[arg(long)]
        base_manifest: PathBuf,
        /// Proposed training authorization JSON.
        #[arg(long)]
        authorization: PathBuf,
        /// Immutable authorization destination.
        #[arg(long)]
        output: Option<PathBuf>,
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Validate and register a completed training-side adapter report.
    TrainSuccessor {
        /// Canonical parent substrate manifest.
        #[arg(long)]
        base_manifest: PathBuf,
        /// Immutable pre-optimization authorization manifest.
        #[arg(long)]
        authorization: PathBuf,
        /// Training-run JSON emitted by the controlled trainer.
        #[arg(long)]
        report: PathBuf,
        /// Immutable training manifest destination.
        #[arg(long)]
        output: Option<PathBuf>,
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Validate and register a candidate-only successor merge.
    MergeSuccessor {
        /// Canonical parent substrate manifest.
        #[arg(long)]
        base_manifest: PathBuf,
        /// Canonical parent Safetensors directory, used for identity verification.
        #[arg(long)]
        base_source: PathBuf,
        /// Validated training-run manifest.
        #[arg(long)]
        training_manifest: PathBuf,
        /// Pre-optimization authorization consumed by the training run.
        /// Defaults to the managed authorization named by the training manifest.
        #[arg(long)]
        authorization: Option<PathBuf>,
        /// New merged candidate Safetensors directory.
        #[arg(long)]
        candidate: PathBuf,
        /// New conceptual lineage for the candidate successor.
        #[arg(long)]
        successor_lineage_id: String,
        /// Stable candidate merge identifier.
        #[arg(long)]
        candidate_id: String,
        /// Merge delta, dtype, and equivalence report from merge_lora.py.
        #[arg(long)]
        report: PathBuf,
        /// Immutable candidate manifest destination.
        #[arg(long)]
        output: Option<PathBuf>,
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Admit a successor after regression against its parent baseline.
    AdmitSuccessor {
        /// Validated candidate merge manifest.
        #[arg(long)]
        candidate_manifest: PathBuf,
        /// Frozen pristine parent baseline.
        #[arg(long)]
        baseline_manifest: PathBuf,
        /// Successor admission report with regression results.
        #[arg(long)]
        report: PathBuf,
        /// Immutable successor admission destination.
        #[arg(long)]
        output: Option<PathBuf>,
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
}

/// Execute a model-substrate command and return its process exit code.
pub(crate) async fn run(command: ModelCommand) -> i32 {
    match command {
        ModelCommand::Import {
            path,
            lineage_id,
            parent,
            manifest,
            json,
        } => import_model(path, lineage_id, parent, manifest, json),
        ModelCommand::Inspect { manifest, json } => inspect_model(&manifest, json),
        ModelCommand::InspectTemplate {
            source,
            deployed,
            json,
        } => inspect_template(&source, deployed.as_deref(), json).await,
        ModelCommand::VerifyTemplate {
            endpoint,
            binary,
            model,
            fixtures,
            api_key,
            json,
        } => {
            crate::template::verify(
                &endpoint,
                &binary,
                &model,
                &fixtures,
                api_key.as_deref(),
                json,
            )
            .await
        }
        ModelCommand::ApproveTemplate {
            prompt,
            runtime,
            deployment,
            trust_root,
            private_key,
            receipt_id,
            sequence,
            reviewer_id,
            policy_version,
            validation_suite_id,
            store_dir,
            anchor,
            snapshot_root,
            json,
            bundle_out,
            backend_api_key_env,
            public_port,
            backend_port,
            context,
            gpu_layers,
            threads,
        } => approve_template(ApproveTemplateOptions {
            prompt: &prompt,
            runtime: &runtime,
            deployment: &deployment,
            trust_root: &trust_root,
            private_key: &private_key,
            receipt_id: &receipt_id,
            sequence,
            reviewer_id: &reviewer_id,
            policy_version: &policy_version,
            validation_suite_id: &validation_suite_id,
            store_dir: &store_dir,
            anchor: &anchor,
            snapshot_root,
            json,
            bundle_out,
            backend_api_key_env,
            public_port,
            backend_port,
            context,
            gpu_layers,
            threads,
        }),
        ModelCommand::TemplateStatus {
            prompt,
            runtime,
            deployment,
            receipt,
            trust_root,
            revocation_state,
            snapshot_root,
            json,
        } => template_status(TemplateStatusOptions {
            prompt: &prompt,
            runtime: &runtime,
            deployment: &deployment,
            receipt: &receipt,
            trust_root: &trust_root,
            revocation_state: revocation_state.as_deref(),
            snapshot_root: snapshot_root.as_deref(),
            json,
        }),
        ModelCommand::RotateApprovalTrustRoot {
            old_root,
            old_private_key,
            new_key_id,
            new_root_out,
            new_private_key_out,
            migration_out,
            sequence,
        } => rotate_approval_trust_root(
            &old_root,
            &old_private_key,
            &new_key_id,
            &new_root_out,
            &new_private_key_out,
            &migration_out,
            sequence,
        ),
        ModelCommand::Verify {
            path,
            manifest,
            json,
        } => verify_model(&path, &manifest, json),
        ModelCommand::Lineage { manifest, json } => show_lineage(&manifest, json),
        ModelCommand::Derive {
            manifest,
            source,
            output,
            artifact_id,
            quant,
            converter,
            quantizer,
            server,
            python,
            rocm_container,
            rocm_image,
            defer_admission,
            json,
        } => {
            derive_model(
                &manifest,
                &source,
                &output,
                &artifact_id,
                quant,
                converter.as_deref(),
                quantizer.as_deref(),
                server.as_deref(),
                &python,
                rocm_container.as_deref(),
                rocm_image.as_deref(),
                defer_admission,
                json,
            )
            .await
        }
        ModelCommand::AdmitArtifact {
            candidate,
            manifest,
            source,
            server,
            json,
        } => {
            derive::admit_candidate(
                &candidate,
                &manifest,
                &source,
                server.as_deref(),
                json,
                &managed_data_root(),
            )
            .await
        }
        ModelCommand::VerifyCandidate {
            candidate,
            manifest,
            source,
            json,
        } => {
            derive::verify_candidate(&candidate, &manifest, &source, json, &managed_data_root())
                .await
        }
        ModelCommand::Artifacts { lineage_id, json } => {
            artifacts::list(lineage_id.as_deref(), &managed_data_root(), json)
        }
        ModelCommand::InspectAdapter { path, json } => successor::inspect_adapter(&path, json),
        ModelCommand::Baseline {
            artifact_id,
            baseline_id,
            prompts,
            cli,
            output,
            predict,
            gpu_layers,
            json,
            extra_args,
        } => {
            baseline::run(
                baseline::BaselineOptions {
                    artifact_id: &artifact_id,
                    baseline_id: &baseline_id,
                    prompts: &prompts,
                    cli: cli.as_deref(),
                    output: output.as_deref(),
                    predict,
                    gpu_layers,
                    extra_args: &extra_args,
                    json,
                },
                &managed_data_root(),
            )
            .await
        }
        ModelCommand::AuthorizeSuccessorTraining {
            base_manifest,
            authorization,
            output,
            json,
        } => successor::authorize_training(
            &base_manifest,
            &authorization,
            output.as_deref(),
            &managed_data_root(),
            json,
        ),
        ModelCommand::TrainSuccessor {
            base_manifest,
            authorization,
            report,
            output,
            json,
        } => successor::register_training(
            &base_manifest,
            &authorization,
            &report,
            output.as_deref(),
            &managed_data_root(),
            json,
        ),
        ModelCommand::MergeSuccessor {
            base_manifest,
            base_source,
            training_manifest,
            authorization,
            candidate,
            successor_lineage_id,
            candidate_id,
            report,
            output,
            json,
        } => successor::register_candidate(
            successor::CandidateOptions {
                base_manifest: &base_manifest,
                base_source: &base_source,
                training_manifest: &training_manifest,
                authorization_manifest: authorization.as_deref(),
                candidate_path: &candidate,
                successor_lineage_id: &successor_lineage_id,
                candidate_id: &candidate_id,
                report: &report,
                output: output.as_deref(),
                json,
            },
            &managed_data_root(),
        ),
        ModelCommand::AdmitSuccessor {
            candidate_manifest,
            baseline_manifest,
            report,
            output,
            json,
        } => successor::admit(
            &candidate_manifest,
            &baseline_manifest,
            &report,
            output.as_deref(),
            &managed_data_root(),
            json,
        ),
    }
}

fn import_model(
    path: PathBuf,
    lineage_id: String,
    parent: Option<String>,
    manifest_path: Option<PathBuf>,
    json: bool,
) -> i32 {
    let imported = match parent.as_deref() {
        Some(parent) => lmml_substrate::import_successor_safetensors(&path, &lineage_id, parent),
        None => lmml_substrate::import_safetensors_with_progress(&path, &lineage_id, |progress| {
            eprintln!(
                "[model-import] hashed {}/{} shards ({:.2}/{:.2} GiB): {}",
                progress.shards_completed,
                progress.shards_total,
                progress.bytes_hashed as f64 / 1024_f64.powi(3),
                progress.total_bytes as f64 / 1024_f64.powi(3),
                progress.current_shard
            );
        }),
    };
    let imported = match imported {
        Ok(manifest) => manifest,
        Err(error) => {
            eprintln!("model import failed: {error}");
            return 1;
        }
    };
    let manifest_path = manifest_path.unwrap_or_else(|| {
        default_substrate_manifest_path(&managed_data_root(), &imported.model.lineage_id)
    });
    if let Err(error) = lmml_substrate::store_substrate_manifest(&manifest_path, &imported) {
        eprintln!(
            "could not register manifest {}: {error}",
            manifest_path.display()
        );
        return 1;
    }
    let payload = match lmml_substrate::manifest_json(&imported) {
        Ok(payload) => payload,
        Err(error) => {
            eprintln!("could not serialize substrate manifest: {error}");
            return 1;
        }
    };
    if json {
        println!("{payload}");
    } else {
        println!(
            "imported {} as {}\nmanifest: {}\ntensors: {}\nparameters: {}",
            path.display(),
            imported.model.lineage_id,
            manifest_path.display(),
            imported.tensor_count,
            imported.parameter_count
        );
    }
    0
}

fn inspect_model(path: &Path, json: bool) -> i32 {
    let substrate = match read_substrate_manifest(path) {
        Ok(manifest) => manifest,
        Err(error) => {
            eprintln!("model inspect failed: {error}");
            return 1;
        }
    };
    if json {
        match lmml_substrate::manifest_json(&substrate) {
            Ok(payload) => println!("{payload}"),
            Err(error) => {
                eprintln!("could not serialize substrate manifest: {error}");
                return 1;
            }
        }
    } else {
        println!("lineage: {}", substrate.model.lineage_id);
        println!("model: {}", substrate.model.model_name);
        println!("manifest hash: {}", substrate.model.canonical_manifest_hash);
        println!(
            "architecture: {}",
            substrate
                .architecture
                .model_type
                .as_deref()
                .unwrap_or("unknown")
        );
        println!("shards: {}", substrate.shards.len());
        println!("auxiliary files: {}", substrate.auxiliary_files.len());
        println!("tensors: {}", substrate.tensor_count);
        println!("parameters: {}", substrate.parameter_count);
    }
    0
}

async fn inspect_template(source: &Path, deployed: Option<&Path>, json: bool) -> i32 {
    let inspection = match lmml_substrate::inspect_template_source(source) {
        Ok(inspection) => inspection,
        Err(error) => {
            eprintln!("template inspection failed: {error}");
            return 1;
        }
    };
    let deployed_templates = if let Some(deployed) = deployed {
        match lmml_models::parse_gguf_metadata(deployed).await {
            Ok(metadata) => Some(metadata.chat_templates),
            Err(error) => {
                eprintln!("deployed GGUF inspection failed: {error}");
                return 1;
            }
        }
    } else {
        None
    };
    if json {
        let payload = serde_json::json!({
            "source": inspection,
            "deployed_chat_templates": deployed_templates,
            "mode": "inspection_only",
        });
        match serde_json::to_string_pretty(&payload) {
            Ok(payload) => println!("{payload}"),
            Err(error) => {
                eprintln!("could not serialize template inspection: {error}");
                return 1;
            }
        }
    } else {
        println!(
            "source template container: {}",
            inspection.source_template_container_hash
        );
        println!("template variants: {}", inspection.template_hashes.len());
        for (variant, hash) in inspection.template_hashes {
            println!("  {variant}: {hash}");
        }
        println!(
            "external template: {}",
            inspection
                .external_template_hash
                .map_or_else(|| "none".to_string(), |hash| hash.to_string())
        );
        println!(
            "special-token map: {}",
            inspection
                .special_tokens_hash
                .map_or_else(|| "none".to_string(), |hash| hash.to_string())
        );
        println!("mode: inspection_only");
        if let Some(templates) = deployed_templates {
            println!("deployed GGUF templates: {}", templates.len());
            for (name, template) in templates {
                println!(
                    "  {}: {}",
                    name,
                    lmml_substrate::sha256_data(template.as_bytes())
                );
            }
        }
    }
    0
}

struct ApproveTemplateOptions<'a> {
    prompt: &'a Path,
    runtime: &'a Path,
    deployment: &'a Path,
    trust_root: &'a Path,
    private_key: &'a Path,
    receipt_id: &'a str,
    sequence: u64,
    reviewer_id: &'a str,
    policy_version: &'a str,
    validation_suite_id: &'a str,
    store_dir: &'a Path,
    anchor: &'a Path,
    snapshot_root: Option<PathBuf>,
    json: bool,
    bundle_out: Option<PathBuf>,
    backend_api_key_env: String,
    public_port: u16,
    backend_port: u16,
    context: u32,
    gpu_layers: i32,
    threads: usize,
}

fn approve_template(options: ApproveTemplateOptions<'_>) -> i32 {
    let prompt: lmml_substrate::PromptConfiguration = match read_json(options.prompt) {
        Ok(value) => value,
        Err(error) => return model_cli_error("prompt configuration", error),
    };
    let runtime: lmml_substrate::RuntimeConfiguration = match read_json(options.runtime) {
        Ok(value) => value,
        Err(error) => return model_cli_error("runtime configuration", error),
    };
    let deployment: lmml_substrate::DeploymentManifest = match read_json(options.deployment) {
        Ok(value) => value,
        Err(error) => return model_cli_error("deployment manifest", error),
    };
    let trust_root = match lmml_substrate::load_approval_trust_root(options.trust_root) {
        Ok(value) => value,
        Err(error) => return model_cli_error("trust root", error),
    };
    let private_key = match std::fs::read(options.private_key) {
        Ok(value) => value,
        Err(error) => return model_cli_error("private key", error),
    };
    let prompt_hash = match lmml_substrate::prompt_configuration_hash(&prompt) {
        Ok(value) => value,
        Err(error) => return model_cli_error("prompt configuration", error),
    };
    let runtime_hash = match lmml_substrate::runtime_configuration_hash(&runtime) {
        Ok(value) => value,
        Err(error) => return model_cli_error("runtime configuration", error),
    };
    let deployment_hash = match lmml_substrate::deployment_manifest_hash(&deployment) {
        Ok(value) => value,
        Err(error) => return model_cli_error("deployment manifest", error),
    };
    let issued_at = match time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
    {
        Ok(value) => value,
        Err(error) => return model_cli_error("receipt timestamp", error),
    };
    let mut receipt = lmml_substrate::ApprovalReceipt {
        schema_version: 1,
        receipt_id: options.receipt_id.to_string(),
        decision: lmml_substrate::ApprovalDecision::Approved,
        model_lineage_id: prompt.model_lineage_id,
        artifact_hash: prompt.artifact_hash,
        prompt_configuration_hash: prompt_hash,
        runtime_configuration_hash: runtime_hash,
        deployment_manifest_hash: deployment_hash,
        policy_version: options.policy_version.to_string(),
        validation_suite_id: options.validation_suite_id.to_string(),
        reviewer_id: options.reviewer_id.to_string(),
        signing_key_id: trust_root.key_id.clone(),
        sequence: options.sequence,
        issued_at,
        signature_hex: String::new(),
    };
    if let Err(error) = lmml_substrate::sign_approval_receipt(&mut receipt, &private_key) {
        return model_cli_error("approval signing", error);
    }
    let path = match lmml_substrate::store_approval_receipt(
        options.store_dir,
        options.anchor,
        &receipt,
        &trust_root,
    ) {
        Ok(path) => path,
        Err(error) => return model_cli_error("approval storage", error),
    };
    let bundle_path = if let Some(bundle_path) = options.bundle_out.as_ref() {
        let bundle = lmml_substrate::ApprovalBundle {
            schema_version: 1,
            bundle_id: options.receipt_id.to_string(),
            prompt: options.prompt.to_path_buf(),
            runtime: options.runtime.to_path_buf(),
            deployment: options.deployment.to_path_buf(),
            snapshot_root: match options.snapshot_root.as_ref() {
                Some(path) => path.clone(),
                None => {
                    return model_cli_error(
                        "approval bundle",
                        "--snapshot-root is required when --bundle-out is used",
                    )
                }
            },
            receipt: path.clone(),
            trust_root: options.trust_root.to_path_buf(),
            revocation_state: None,
            backend_api_key_env: options.backend_api_key_env.clone(),
            public_port: options.public_port,
            backend_port: options.backend_port,
            context: options.context,
            gpu_layers: options.gpu_layers,
            threads: options.threads,
        };
        if let Err(error) = lmml_substrate::validate_approval_bundle(&bundle) {
            return model_cli_error("approval bundle", error);
        }
        match lmml_substrate::store_approval_bundle(bundle_path, &bundle) {
            Ok(path) => Some(path),
            Err(error) => return model_cli_error("approval bundle", error),
        }
    } else {
        None
    };
    if options.json {
        match serde_json::to_string_pretty(&receipt) {
            Ok(payload) => println!("{payload}"),
            Err(error) => return model_cli_error("approval output", error),
        }
    } else {
        println!("approved receipt: {}", path.display());
        println!("receipt id: {}", receipt.receipt_id);
        println!(
            "prompt configuration: {}",
            receipt.prompt_configuration_hash
        );
        println!(
            "runtime configuration: {}",
            receipt.runtime_configuration_hash
        );
        println!("deployment: {}", receipt.deployment_manifest_hash);
        match bundle_path {
            Some(path) => println!(
                "bundle: {} (launch requires LMML_APPROVAL_ENFORCEMENT=1)",
                path.display()
            ),
            None => println!("mode: inspection_only (no launch bundle created)"),
        }
    }
    0
}

fn rotate_approval_trust_root(
    old_root_path: &Path,
    old_private_key_path: &Path,
    new_key_id: &str,
    new_root_path: &Path,
    new_private_key_path: &Path,
    migration_path: &Path,
    sequence: u64,
) -> i32 {
    let old_root = match lmml_substrate::load_approval_trust_root(old_root_path) {
        Ok(root) => root,
        Err(error) => return model_cli_error("old trust root", error),
    };
    let old_private_key = match std::fs::read(old_private_key_path) {
        Ok(key) => key,
        Err(error) => return model_cli_error("old private key", error),
    };
    let (new_root, new_private_key) =
        match lmml_substrate::generate_approval_trust_root(new_key_id.to_string()) {
            Ok(value) => value,
            Err(error) => return model_cli_error("new trust root", error),
        };
    let issued_at = match time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
    {
        Ok(value) => value,
        Err(error) => return model_cli_error("rotation timestamp", error),
    };
    let mut rotation = lmml_substrate::TrustRootRotation {
        schema_version: 1,
        old_key_id: old_root.key_id.clone(),
        old_key_epoch: old_root.key_epoch,
        new_root: new_root.clone(),
        sequence,
        issued_at,
        signature_hex: String::new(),
    };
    if let Err(error) = lmml_substrate::sign_trust_root_rotation(&mut rotation, &old_private_key) {
        return model_cli_error("rotation signing", error);
    }
    if let Err(error) = write_private_key(new_private_key_path, &new_private_key) {
        return model_cli_error("new private key", error);
    }
    if let Err(error) = lmml_substrate::store_trust_root_rotation(
        new_root_path,
        migration_path,
        &rotation,
        &old_root,
    ) {
        return model_cli_error("trust-root rotation", error);
    }
    println!("new trust root: {}", new_root_path.display());
    println!("new private key: {}", new_private_key_path.display());
    println!("signed migration: {}", migration_path.display());
    0
}

fn write_private_key(path: &Path, key: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    file.write_all(key).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

struct TemplateStatusOptions<'a> {
    prompt: &'a Path,
    runtime: &'a Path,
    deployment: &'a Path,
    receipt: &'a Path,
    trust_root: &'a Path,
    revocation_state: Option<&'a Path>,
    snapshot_root: Option<&'a Path>,
    json: bool,
}

fn template_status(options: TemplateStatusOptions<'_>) -> i32 {
    let prompt: lmml_substrate::PromptConfiguration = match read_json(options.prompt) {
        Ok(value) => value,
        Err(error) => return model_cli_error("prompt configuration", error),
    };
    let runtime: lmml_substrate::RuntimeConfiguration = match read_json(options.runtime) {
        Ok(value) => value,
        Err(error) => return model_cli_error("runtime configuration", error),
    };
    let deployment: lmml_substrate::DeploymentManifest = match read_json(options.deployment) {
        Ok(value) => value,
        Err(error) => return model_cli_error("deployment manifest", error),
    };
    if let Some(snapshot_root) = options.snapshot_root {
        if let Err(error) = lmml_substrate::verify_deployment_snapshot(snapshot_root, &deployment) {
            return model_cli_error("deployment snapshot", error);
        }
    }
    let trust_root = match lmml_substrate::load_approval_trust_root(options.trust_root) {
        Ok(value) => value,
        Err(error) => return model_cli_error("trust root", error),
    };
    let receipt = match lmml_substrate::load_approval_receipt(options.receipt, &trust_root) {
        Ok(value) => value,
        Err(error) => return model_cli_error("approval receipt", error),
    };
    let revoked = match options.revocation_state {
        Some(path) => match lmml_substrate::load_revocation_state(path, &trust_root) {
            Ok(state) => lmml_substrate::is_receipt_revoked(&state, &receipt.receipt_id),
            Err(error) => return model_cli_error("revocation state", error),
        },
        None => false,
    };
    let prompt_hash = match lmml_substrate::prompt_configuration_hash(&prompt) {
        Ok(value) => value,
        Err(error) => return model_cli_error("prompt configuration", error),
    };
    let runtime_hash = match lmml_substrate::runtime_configuration_hash(&runtime) {
        Ok(value) => value,
        Err(error) => return model_cli_error("runtime configuration", error),
    };
    let deployment_hash = match lmml_substrate::deployment_manifest_hash(&deployment) {
        Ok(value) => value,
        Err(error) => return model_cli_error("deployment manifest", error),
    };
    let status = lmml_substrate::compute_admission_status(
        Some(&receipt),
        &prompt_hash,
        &runtime_hash,
        &deployment_hash,
        revoked,
    );
    if options.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&status).expect("admission status serialization")
        );
    } else {
        println!("state: {:?}", status.state);
        println!("reason: {}", status.reason);
        println!("receipt: {}", receipt.receipt_id);
    }
    (status.state != lmml_substrate::AdmissionState::Approved) as i32
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, String> {
    let payload = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    serde_json::from_str(&payload).map_err(|error| error.to_string())
}

fn model_cli_error(context: &str, error: impl std::fmt::Display) -> i32 {
    eprintln!("{context} failed: {error}");
    1
}

fn verify_model(root: &Path, manifest_path: &Path, json: bool) -> i32 {
    let expected = match read_substrate_manifest(manifest_path) {
        Ok(manifest) => manifest,
        Err(error) => {
            eprintln!("model verify failed: {error}");
            return 1;
        }
    };
    match lmml_substrate::verify_safetensors(root, &expected) {
        Ok(()) => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "verified": true,
                        "lineage_id": expected.model.lineage_id,
                    })
                );
            } else {
                println!(
                    "verified: {} ({})",
                    root.display(),
                    expected.model.lineage_id
                );
            }
            0
        }
        Err(error) => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "verified": false,
                        "error": error.to_string(),
                    })
                );
            } else {
                eprintln!("verification failed: {error}");
            }
            1
        }
    }
}

fn show_lineage(path: &Path, json: bool) -> i32 {
    let substrate = match read_substrate_manifest(path) {
        Ok(manifest) => manifest,
        Err(error) => {
            eprintln!("model lineage failed: {error}");
            return 1;
        }
    };
    if json {
        println!(
            "{}",
            serde_json::json!({
                "lineage_id": substrate.model.lineage_id,
                "model_name": substrate.model.model_name,
                "parent": substrate.model.parent,
                "canonical_manifest_hash": substrate.model.canonical_manifest_hash,
            })
        );
    } else {
        println!("{}", substrate.model.lineage_id);
        if let Some(parent) = substrate.model.parent {
            println!("parent: {parent}");
        }
    }
    0
}

pub(super) fn read_substrate_manifest(
    path: &Path,
) -> Result<lmml_substrate::SubstrateManifest, String> {
    let payload =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    lmml_substrate::parse_manifest_json(&payload)
        .map_err(|error| format!("{}: {error}", path.display()))
}

pub(super) fn managed_data_root() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn default_substrate_manifest_path(data_root: &Path, lineage_id: &str) -> PathBuf {
    data_root
        .join("lmml/models/manifests")
        .join(format!("{lineage_id}.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_manifest_path_is_derived_from_lineage() {
        assert_eq!(
            default_substrate_manifest_path(Path::new("/data"), "qwen38-successor-1"),
            PathBuf::from("/data/lmml/models/manifests/qwen38-successor-1.json")
        );
    }

    #[test]
    fn successor_merge_uses_explicit_transformers_dtype() {
        let script = include_str!("../../../scripts/merge_lora.py");
        assert!(script.contains("dtype=dtype"));
        assert!(!script.contains("torch_dtype="));
        assert!(script.contains("merge_and_unload(safe_merge=True)"));
        assert!(script.contains("successor output and report must remain outside"));
        assert!(script.contains("output_path.parent.mkdir(parents=True, exist_ok=True)"));
        assert!(script.contains("merge report must remain outside the successor output"));
    }
}
