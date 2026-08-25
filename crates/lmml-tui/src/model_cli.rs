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
        /// Emit the artifact manifest as JSON.
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
                json,
            )
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
    }
}
