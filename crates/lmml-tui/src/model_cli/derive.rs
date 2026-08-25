//! Verified GGUF derivation and append-only artifact registration.
//!
//! Conversion may run through local llama.cpp tooling or a validated ROCm
//! container. All outputs remain private until metadata and backend admission
//! succeed.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

mod admission;
mod container;

use admission::{GgufAdmissionProvider, LlamaServerAdmission};
use container::{build_converter_command, inspect_rocm_container};

const DEFAULT_ROCM_CONVERSION_IMAGE: &str =
    "rocm/pytorch:rocm7.2.4_ubuntu24.04_py3.12_pytorch_release_2.9.1";

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub(crate) enum QuantizationArg {
    /// Unquantized F16 GGUF.
    F16,
    /// Unquantized BF16 GGUF where supported by the converter.
    Bf16,
    /// Q8_0 GGUF.
    Q8_0,
    /// Q6_K GGUF.
    Q6K,
    /// Q4_K_M GGUF.
    Q4KM,
}

impl QuantizationArg {
    fn kind(self) -> lmml_substrate::QuantizationKind {
        match self {
            Self::F16 => lmml_substrate::QuantizationKind::F16,
            Self::Bf16 => lmml_substrate::QuantizationKind::Bf16,
            Self::Q8_0 => lmml_substrate::QuantizationKind::Q8_0,
            Self::Q6K => lmml_substrate::QuantizationKind::Q6_K,
            Self::Q4KM => lmml_substrate::QuantizationKind::Q4_K_M,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::F16 => "F16",
            Self::Bf16 => "BF16",
            Self::Q8_0 => "Q8_0",
            Self::Q6K => "Q6_K",
            Self::Q4KM => "Q4_K_M",
        }
    }

    fn converter_dtype(self) -> &'static str {
        match self {
            Self::F16 | Self::Q8_0 | Self::Q6K | Self::Q4KM => "f16",
            Self::Bf16 => "bf16",
        }
    }

    fn is_quantized(self) -> bool {
        matches!(self, Self::Q8_0 | Self::Q6K | Self::Q4KM)
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn derive_model(
    manifest_path: &Path,
    source: &Path,
    output: &Path,
    artifact_id: &str,
    quant: QuantizationArg,
    converter: Option<&Path>,
    quantizer: Option<&Path>,
    server: Option<&Path>,
    python: &str,
    rocm_container: Option<&Path>,
    rocm_image: Option<&str>,
    json: bool,
) -> i32 {
    derive_model_with(
        manifest_path,
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
        &managed_data_root(),
        &LlamaServerAdmission,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn derive_model_with<A: GgufAdmissionProvider>(
    manifest_path: &Path,
    source: &Path,
    output: &Path,
    artifact_id: &str,
    quant: QuantizationArg,
    converter: Option<&Path>,
    quantizer: Option<&Path>,
    server: Option<&Path>,
    python: &str,
    rocm_container: Option<&Path>,
    rocm_image: Option<&str>,
    json: bool,
    data_root: &Path,
    admission: &A,
) -> i32 {
    let substrate = match read_substrate_manifest(manifest_path) {
        Ok(manifest) => manifest,
        Err(error) => {
            eprintln!("model derive failed: {error}");
            return 1;
        }
    };
    if let Err(error) = require_admitted_successor(&substrate, data_root) {
        eprintln!("model derive refused: {error}");
        return 1;
    }
    if output.exists() {
        eprintln!(
            "model derive refused: output already exists: {}",
            output.display()
        );
        return 1;
    }
    if let Err(error) = lmml_substrate::validate_identifier(artifact_id) {
        eprintln!("model derive refused: invalid artifact ID: {error}");
        return 1;
    }
    let converter = converter
        .map(PathBuf::from)
        .unwrap_or_else(default_converter_path);
    if !converter.is_file() {
        eprintln!(
            "model derive refused: converter not found: {}",
            converter.display()
        );
        return 1;
    }
    if rocm_container.is_none() && rocm_image.is_some() {
        eprintln!("model derive refused: --rocm-image requires --rocm-container");
        return 1;
    }
    if let Some(runtime) = rocm_container {
        if !runtime.is_file() {
            eprintln!(
                "model derive refused: ROCm container runtime not found: {}",
                runtime.display()
            );
            return 1;
        }
    }
    let rocm_image = rocm_container.map(|_| rocm_image.unwrap_or(DEFAULT_ROCM_CONVERSION_IMAGE));
    let quantizer = quantizer
        .map(PathBuf::from)
        .unwrap_or_else(default_quantizer_path);
    if quant.is_quantized() && !quantizer.is_file() {
        eprintln!(
            "model derive refused: quantizer not found: {}",
            quantizer.display()
        );
        return 1;
    }
    if quant.is_quantized() && !quantizer_supports(&quantizer, quant).await {
        eprintln!(
            "model derive refused: installed llama-quantize does not advertise {}",
            quant.label()
        );
        return 1;
    }
    let container_provenance = match (rocm_container, rocm_image) {
        (Some(runtime), Some(image)) => {
            match inspect_rocm_container(runtime, image, python, &converter).await {
                Ok(provenance) => Some(provenance),
                Err(error) => {
                    eprintln!("model derive refused: ROCm container preflight failed: {error}");
                    return 1;
                }
            }
        }
        (None, None) => None,
        _ => {
            eprintln!("model derive refused: incomplete ROCm container configuration");
            return 1;
        }
    };
    if let Err(error) = lmml_substrate::verify_safetensors(source, &substrate) {
        eprintln!("model derive refused: canonical substrate verification failed: {error}");
        return 1;
    }
    if let Some(parent) = output.parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            eprintln!(
                "could not create output directory {}: {error}",
                parent.display()
            );
            return 1;
        }
    }

    let temporary_directory = match tempfile::Builder::new()
        .prefix(".lmml-derive-")
        .tempdir_in(output.parent().unwrap_or_else(|| Path::new(".")))
    {
        Ok(directory) => directory,
        Err(error) => {
            eprintln!("could not create private derivation directory: {error}");
            return 1;
        }
    };
    let output_temp = temporary_directory.path().join("output.gguf");
    if let Err(error) = fs::File::create(&output_temp) {
        eprintln!("could not create temporary GGUF output: {error}");
        return 1;
    }
    let intermediate = if quant.is_quantized() {
        let path = temporary_directory.path().join("intermediate.gguf");
        if let Err(error) = fs::File::create(&path) {
            eprintln!("could not create temporary conversion output: {error}");
            return 1;
        }
        Some(path)
    } else {
        None
    };
    let conversion_output = intermediate.as_deref().unwrap_or(&output_temp);
    let (converter_program, converter_args) = match build_converter_command(
        python,
        &converter,
        source,
        conversion_output,
        quant,
        rocm_container,
        rocm_image,
    ) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("model derive refused: {error}");
            return 1;
        }
    };
    let conversion = run_streamed(&converter_program, &converter_args, "[llama.cpp convert]").await;
    match conversion {
        Ok(result) if result.success => {}
        Ok(result) => {
            eprintln!("llama.cpp conversion failed: {}", result.tail.join(" | "));
            return 1;
        }
        Err(error) => {
            eprintln!("could not execute converter {python}: {error}");
            return 1;
        }
    }
    if !conversion_output.is_file() {
        eprintln!("converter did not produce {}", conversion_output.display());
        return 1;
    }

    let mut command_parameters = vec![
        "converter-program".into(),
        converter_program.to_string_lossy().into_owned(),
    ];
    command_parameters.extend(converter_args);
    if let Some(intermediate) = &intermediate {
        let quantizer_args = build_quantizer_args(intermediate, &output_temp, quant);
        let quantized = run_streamed(&quantizer, &quantizer_args, "[llama.cpp quantize]").await;
        match quantized {
            Ok(result) if result.success => {
                command_parameters.push("quantizer".into());
                command_parameters.extend(quantizer_args);
            }
            Ok(result) => {
                eprintln!("llama-quantize failed: {}", result.tail.join(" | "));
                return 1;
            }
            Err(error) => {
                eprintln!(
                    "could not execute quantizer {}: {error}",
                    quantizer.display()
                );
                return 1;
            }
        }
    }

    let server_path = server
        .map(PathBuf::from)
        .unwrap_or_else(default_server_path);
    if let Err(error) = admission.admit(&output_temp, &server_path).await {
        eprintln!("model derive failed post-conversion validation: {error}");
        return 1;
    }
    let artifact_hash = match lmml_substrate::sha256_file(&output_temp) {
        Ok(hash) => hash,
        Err(error) => {
            eprintln!("could not hash derived artifact: {error}");
            return 1;
        }
    };
    let source_artifact_id = format!("{}-safetensors", substrate.model.lineage_id);
    let source_artifact = match artifact_manifest(
        &source_artifact_id,
        &substrate.model.lineage_id,
        lmml_substrate::ModelRepresentation::Safetensors,
        None,
        substrate.model.canonical_manifest_hash.clone(),
        None,
        "lmml-canonical",
        "2",
        vec![substrate.model.canonical_manifest_hash.clone()],
        vec!["canonical_manifest".into()],
        source,
        None,
    ) {
        Ok(manifest) => manifest,
        Err(error) => {
            eprintln!("could not create source artifact manifest: {error}");
            return 1;
        }
    };
    let host_converter_revision =
        git_revision(converter.parent().unwrap_or_else(|| Path::new("."))).await;
    let quantizer_provenance = if quant.is_quantized() {
        let version = tool_version(&quantizer).await;
        let version = if version == "unknown" {
            format!("llama.cpp-{host_converter_revision}")
        } else {
            version
        };
        format!(
            "quantizer_path={}; quantizer_version={}",
            quantizer.display(),
            version
        )
    } else {
        "quantizer=not-used".into()
    };
    let converter_provenance = if let Some(provenance) = container_provenance {
        format!(
            "converter_runtime={}; container_runtime_version={}; converter_image={}; converter_image_identity={}; container_environment={}; host_converter_revision={}",
            provenance.runtime_path.display(),
            provenance.runtime_version,
            provenance.image,
            provenance.image_identity,
            provenance.environment,
            host_converter_revision,
        )
    } else {
        format!(
            "converter_path={}; converter_revision={}",
            converter.display(),
            host_converter_revision
        )
    };
    let python_provenance = if rocm_container.is_some() {
        "python=container-environment".to_string()
    } else {
        format!(
            "python={}; python_version={}",
            python,
            command_version(python).await
        )
    };
    let server_version = tool_version(&server_path).await;
    let tool_version = format!(
        "{}; {}; {}; server_path={}; server_version={}",
        converter_provenance,
        quantizer_provenance,
        python_provenance,
        server_path.display(),
        server_version,
    );
    let admitted_at = match timestamp() {
        Ok(timestamp) => timestamp,
        Err(error) => {
            eprintln!("could not create admission timestamp: {error}");
            return 1;
        }
    };
    let derived = match artifact_manifest(
        artifact_id,
        &substrate.model.lineage_id,
        lmml_substrate::ModelRepresentation::Gguf,
        Some(quant.kind()),
        artifact_hash.clone(),
        Some(source_artifact_id),
        "llama.cpp",
        &tool_version,
        vec![substrate.model.canonical_manifest_hash.clone()],
        command_parameters,
        output,
        Some(lmml_substrate::ArtifactAdmission {
            backend: "llama.cpp".to_string(),
            backend_version: server_version,
            checked_at: admitted_at,
        }),
    ) {
        Ok(manifest) => manifest,
        Err(error) => {
            eprintln!("could not create derived artifact manifest: {error}");
            return 1;
        }
    };
    let artifact_root = data_root.join("lmml/models/artifacts");
    if let Err(error) = publish_no_clobber(&output_temp, output) {
        eprintln!("could not publish derived artifact: {error}");
        return 1;
    }
    if let Err(error) = ensure_source_artifact(&artifact_root, &source_artifact)
        .and_then(|_| lmml_substrate::append_artifact_manifest(&artifact_root, &derived))
    {
        eprintln!("could not register derived artifact: {error}");
        if let Err(cleanup_error) = fs::remove_file(output) {
            eprintln!(
                "could not roll back derived output {}: {cleanup_error}",
                output.display()
            );
        }
        return 1;
    }
    if json {
        match serde_json::to_string_pretty(&derived) {
            Ok(payload) => println!("{payload}"),
            Err(error) => {
                eprintln!("could not serialize artifact manifest: {error}");
                return 1;
            }
        }
    } else {
        println!(
            "derived {} ({})\nartifact: {}\nhash: {}",
            output.display(),
            quant.label(),
            derived.artifact.artifact_id,
            derived.artifact.artifact_hash
        );
    }
    0
}

fn publish_no_clobber(source: &Path, destination: &Path) -> Result<(), String> {
    fs::hard_link(source, destination).map_err(|error| {
        if destination.exists() {
            format!("destination already exists: {}", destination.display())
        } else {
            format!("atomic publication failed: {error}")
        }
    })?;
    if let Err(error) = fs::remove_file(source) {
        let rollback = fs::remove_file(destination)
            .map_err(|rollback| format!("; destination rollback also failed: {rollback}"))
            .err()
            .unwrap_or_default();
        return Err(format!(
            "could not remove temporary publication link: {error}{rollback}"
        ));
    }
    Ok(())
}

fn build_quantizer_args(
    intermediate: &Path,
    output_temp: &Path,
    quant: QuantizationArg,
) -> Vec<String> {
    vec![
        intermediate.to_string_lossy().into_owned(),
        output_temp.to_string_lossy().into_owned(),
        quant.label().into(),
    ]
}

fn ensure_source_artifact(
    root: &Path,
    expected: &lmml_substrate::ArtifactManifest,
) -> Result<PathBuf, lmml_substrate::SubstrateError> {
    let path = root.join(format!("{}.json", expected.artifact.artifact_id));
    match read_matching_source_artifact(&path, expected)? {
        Some(path) => Ok(path),
        None => match lmml_substrate::append_artifact_manifest(root, expected) {
            Ok(path) => Ok(path),
            Err(lmml_substrate::SubstrateError::ArtifactConflict(_)) => {
                read_matching_source_artifact(&path, expected)?
                    .ok_or(lmml_substrate::SubstrateError::ArtifactConflict(path))
            }
            Err(error) => Err(error),
        },
    }
}

fn read_matching_source_artifact(
    path: &Path,
    expected: &lmml_substrate::ArtifactManifest,
) -> Result<Option<PathBuf>, lmml_substrate::SubstrateError> {
    let payload = match fs::read(path) {
        Ok(payload) => payload,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(lmml_substrate::SubstrateError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    let payload = String::from_utf8(payload).map_err(|error| {
        lmml_substrate::SubstrateError::InvalidArtifactManifest(format!(
            "artifact manifest is not UTF-8: {error}"
        ))
    })?;
    let existing = lmml_substrate::parse_artifact_manifest_json(&payload)?;
    let matches = existing.schema_version == expected.schema_version
        && existing.artifact == expected.artifact
        && existing.parent_artifact == expected.parent_artifact
        && existing.canonical_model == expected.canonical_model
        && existing.tool == expected.tool
        && existing.tool_version == expected.tool_version
        && existing.command_or_parameters == expected.command_or_parameters
        && existing.source_hashes == expected.source_hashes
        && existing.output_hash == expected.output_hash
        && existing.artifact_path == expected.artifact_path
        && existing.admission == expected.admission;
    if matches {
        Ok(Some(path.to_path_buf()))
    } else {
        Err(lmml_substrate::SubstrateError::ArtifactConflict(
            path.to_path_buf(),
        ))
    }
}

fn require_admitted_successor(
    substrate: &lmml_substrate::SubstrateManifest,
    data_root: &Path,
) -> Result<(), lmml_substrate::SubstrateError> {
    let Some(parent) = substrate.model.parent.as_deref() else {
        return Ok(());
    };
    let path = data_root
        .join("lmml/models/successors")
        .join(&substrate.model.lineage_id)
        .join("successor_manifest.json");
    let payload = fs::read_to_string(&path).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            lmml_substrate::SubstrateError::SuccessorNotAdmitted(substrate.model.lineage_id.clone())
        } else {
            lmml_substrate::SubstrateError::Io {
                path: path.clone(),
                source,
            }
        }
    })?;
    let admission = lmml_substrate::parse_successor_manifest_json(&payload)?;
    if admission.successor_lineage_id != substrate.model.lineage_id
        || admission.parent_lineage_id != parent
        || admission.successor_hash != substrate.model.canonical_manifest_hash
    {
        return Err(lmml_substrate::SubstrateError::SuccessorNotAdmitted(
            substrate.model.lineage_id.clone(),
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn artifact_manifest(
    artifact_id: &str,
    lineage_id: &str,
    representation: lmml_substrate::ModelRepresentation,
    quantization: Option<lmml_substrate::QuantizationKind>,
    artifact_hash: lmml_substrate::Hash256,
    parent_artifact: Option<String>,
    tool: &str,
    tool_version: &str,
    source_hashes: Vec<lmml_substrate::Hash256>,
    command_or_parameters: Vec<String>,
    artifact_path: &Path,
    admission: Option<lmml_substrate::ArtifactAdmission>,
) -> Result<lmml_substrate::ArtifactManifest, String> {
    Ok(lmml_substrate::ArtifactManifest {
        schema_version: lmml_substrate::SCHEMA_VERSION,
        artifact: lmml_substrate::ArtifactIdentity {
            artifact_id: artifact_id.to_string(),
            model_lineage_id: lineage_id.to_string(),
            representation,
            quantization,
            artifact_hash: artifact_hash.clone(),
        },
        parent_artifact,
        canonical_model: lineage_id.to_string(),
        tool: tool.to_string(),
        tool_version: tool_version.to_string(),
        command_or_parameters,
        source_hashes,
        output_hash: artifact_hash,
        artifact_path: absolute_path(artifact_path)?,
        admission,
        created_at: timestamp()?,
    })
}

fn absolute_path(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        std::env::current_dir()
            .map(|root| root.join(path))
            .map_err(|error| format!("could not resolve {}: {error}", path.display()))
    }
}

fn timestamp() -> Result<String, String> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|error| error.to_string())
}

struct ProcessResult {
    success: bool,
    tail: Vec<String>,
}

async fn run_streamed(
    program: impl AsRef<Path>,
    args: &[String],
    prefix: &str,
) -> Result<ProcessResult, String> {
    let mut child = Command::new(program.as_ref())
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("{}: {error}", program.as_ref().display()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "converter stdout pipe unavailable".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "converter stderr pipe unavailable".to_string())?;
    let stdout_task = tokio::spawn(drain_process_lines(stdout, prefix.to_string()));
    let stderr_task = tokio::spawn(drain_process_lines(stderr, prefix.to_string()));
    let status = child
        .wait()
        .await
        .map_err(|error| format!("failed waiting for {}: {error}", program.as_ref().display()))?;
    let mut tail = stdout_task.await.map_err(|error| error.to_string())?;
    tail.extend(stderr_task.await.map_err(|error| error.to_string())?);
    Ok(ProcessResult {
        success: status.success(),
        tail,
    })
}

async fn drain_process_lines<R>(reader: R, prefix: String) -> Vec<String>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut lines = BufReader::new(reader).lines();
    let mut tail = std::collections::VecDeque::with_capacity(64);
    while let Ok(Some(line)) = lines.next_line().await {
        eprintln!("{prefix} {line}");
        if tail.len() == 64 {
            tail.pop_front();
        }
        tail.push_back(line);
    }
    tail.into_iter().collect()
}

fn default_quantizer_path() -> PathBuf {
    managed_data_root().join("lmml/llama.cpp/build/bin/llama-quantize")
}

fn default_converter_path() -> PathBuf {
    managed_data_root().join("lmml/llama.cpp/convert_hf_to_gguf.py")
}

fn default_server_path() -> PathBuf {
    managed_data_root().join("lmml/llama.cpp/build/bin/llama-server")
}

async fn tool_version(path: &Path) -> String {
    Command::new(path)
        .arg("--version")
        .output()
        .await
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|version| version.trim().to_string())
        .filter(|version| !version.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

async fn quantizer_supports(path: &Path, quant: QuantizationArg) -> bool {
    Command::new(path)
        .arg("--help")
        .output()
        .await
        .ok()
        .map(|output| {
            let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
            text.push_str(&String::from_utf8_lossy(&output.stderr));
            text.contains(quant.label())
        })
        .unwrap_or(false)
}

async fn command_version(command: &str) -> String {
    Command::new(command)
        .arg("--version")
        .output()
        .await
        .ok()
        .filter(|output| output.status.success())
        .map(|output| {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let value = if stdout.trim().is_empty() {
                stderr
            } else {
                stdout
            };
            value.trim().to_string()
        })
        .filter(|version| !version.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

async fn git_revision(root: &Path) -> String {
    Command::new("git")
        .args(["-C", &root.to_string_lossy(), "rev-parse", "HEAD"])
        .output()
        .await
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|revision| revision.trim().to_string())
        .filter(|revision| !revision.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

fn read_substrate_manifest(path: &Path) -> Result<lmml_substrate::SubstrateManifest, String> {
    let payload =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    lmml_substrate::parse_manifest_json(&payload)
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn managed_data_root() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests;
