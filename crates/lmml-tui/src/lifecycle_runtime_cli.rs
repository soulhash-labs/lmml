//! Artifact-bound runtime registration and capability lease commands.

use std::path::{Path, PathBuf};
use std::time::Duration;

use time::{format_description::well_known::Rfc3339, OffsetDateTime};

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub(crate) enum CapabilityArg {
    TextGeneration,
    Logits,
    Embeddings,
    HiddenStateObservation,
    ElevenTapObservation,
}

impl From<CapabilityArg> for lmml_substrate::RuntimeCapability {
    fn from(value: CapabilityArg) -> Self {
        match value {
            CapabilityArg::TextGeneration => Self::TextGeneration,
            CapabilityArg::Logits => Self::Logits,
            CapabilityArg::Embeddings => Self::Embeddings,
            CapabilityArg::HiddenStateObservation => Self::HiddenStateObservation,
            CapabilityArg::ElevenTapObservation => Self::ElevenTapObservation,
        }
    }
}

pub(crate) struct RegisterOptions<'a> {
    pub runtime_id: &'a str,
    pub artifact_id: &'a str,
    pub pid: u32,
    pub endpoint: &'a str,
    pub backend_version: &'a str,
    pub context_size: usize,
    pub json: bool,
}

pub(crate) async fn register(options: RegisterOptions<'_>) -> i32 {
    let data_root = managed_data_root();
    let artifacts =
        match lmml_substrate::load_artifact_manifests(data_root.join("lmml/models/artifacts")) {
            Ok(artifacts) => artifacts,
            Err(error) => return fail("runtime registration", error.to_string()),
        };
    let Some(artifact) = artifacts
        .iter()
        .find(|artifact| artifact.artifact.artifact_id == options.artifact_id)
    else {
        return fail(
            "runtime registration",
            format!("unknown artifact {}", options.artifact_id),
        );
    };
    if artifact.admission.is_none()
        || artifact.artifact.representation != lmml_substrate::ModelRepresentation::Gguf
    {
        return fail(
            "runtime registration",
            "artifact is not an admitted GGUF".to_string(),
        );
    }
    let actual_hash = match lmml_substrate::sha256_file(&artifact.artifact_path) {
        Ok(hash) => hash,
        Err(error) => return fail("runtime registration", error.to_string()),
    };
    if actual_hash != artifact.artifact.artifact_hash {
        return fail(
            "runtime registration",
            "artifact bytes do not match the registered hash".to_string(),
        );
    }
    if let Err(error) = process_executes_artifact(options.pid, &artifact.artifact_path) {
        return fail("runtime registration", error);
    }
    if let Err(error) = endpoint_health(options.endpoint).await {
        return fail("runtime registration", error);
    }
    if let Err(error) =
        lmml_server::verify_served_model_endpoint(options.endpoint, &artifact.artifact_path, None)
            .await
    {
        return fail("runtime registration model identity", error.to_string());
    }
    let created_at = match OffsetDateTime::now_utc().format(&Rfc3339) {
        Ok(timestamp) => timestamp,
        Err(error) => return fail("runtime registration", error.to_string()),
    };
    let manifest = lmml_substrate::RuntimeManifest {
        runtime_id: options.runtime_id.to_string(),
        pid: options.pid,
        model_lineage_id: artifact.artifact.model_lineage_id.clone(),
        artifact_id: artifact.artifact.artifact_id.clone(),
        artifact_hash: artifact.artifact.artifact_hash.clone(),
        representation: artifact.artifact.representation,
        quantization: artifact.artifact.quantization,
        backend: "llama.cpp".to_string(),
        backend_version: options.backend_version.to_string(),
        endpoint: options.endpoint.to_string(),
        context_size: options.context_size,
        capabilities: vec![lmml_substrate::RuntimeCapability::TextGeneration],
        created_at,
    };
    let path = data_root
        .join("lmml/models/runtimes")
        .join(format!("{}.json", manifest.runtime_id));
    if let Err(error) = lmml_substrate::store_runtime_manifest(&path, &manifest) {
        return fail("runtime registration", error.to_string());
    }
    emit(&manifest, &path, options.json, "runtime")
}

pub(crate) async fn request(
    lineage_id: &str,
    purpose: &str,
    capabilities: &[CapabilityArg],
    lease_id: &str,
    json: bool,
) -> i32 {
    let data_root = managed_data_root();
    let artifacts =
        match lmml_substrate::load_artifact_manifests(data_root.join("lmml/models/artifacts")) {
            Ok(artifacts) => artifacts,
            Err(error) => return fail("runtime lease", error.to_string()),
        };
    let persisted_runtimes =
        match lmml_substrate::load_runtime_manifests(data_root.join("lmml/models/runtimes")) {
            Ok(runtimes) => runtimes,
            Err(error) => return fail("runtime lease", error.to_string()),
        };
    let mut runtimes = Vec::new();
    for runtime in persisted_runtimes {
        let Some(artifact) = artifacts
            .iter()
            .find(|artifact| artifact.artifact.artifact_id == runtime.artifact_id)
        else {
            continue;
        };
        if process_executes_artifact(runtime.pid, &artifact.artifact_path).is_ok() {
            if let Err(error) = verify_artifact_hash(artifact).await {
                return fail("runtime lease artifact verification", error);
            }
            if endpoint_health(&runtime.endpoint).await.is_ok() {
                if let Err(error) = lmml_server::verify_served_model_endpoint(
                    &runtime.endpoint,
                    &artifact.artifact_path,
                    None,
                )
                .await
                {
                    return fail("runtime lease model identity", error.to_string());
                }
                runtimes.push(runtime);
            }
        }
    }
    let request = lmml_substrate::ModelRequest {
        model_lineage_id: lineage_id.to_string(),
        purpose: purpose.to_string(),
        representation_preference: vec![lmml_substrate::ModelRepresentation::Gguf],
        required_capabilities: capabilities.iter().copied().map(Into::into).collect(),
    };
    let lease = match lmml_substrate::issue_model_lease(&request, &artifacts, &runtimes, lease_id) {
        Ok(lease) => lease,
        Err(error) => return fail("runtime lease", error.to_string()),
    };
    let path = data_root
        .join("lmml/models/runtimes/leases")
        .join(format!("{}.json", lease.lease_id));
    if let Err(error) = lmml_substrate::store_model_lease(&path, &lease) {
        return fail("runtime lease", error.to_string());
    }
    emit(&lease, &path, json, "lease")
}

async fn verify_artifact_hash(artifact: &lmml_substrate::ArtifactManifest) -> Result<(), String> {
    let path = artifact.artifact_path.clone();
    let expected = artifact.artifact.artifact_hash.clone();
    tokio::task::spawn_blocking(move || {
        let actual = lmml_substrate::sha256_file(&path).map_err(|error| error.to_string())?;
        if actual == expected {
            Ok(())
        } else {
            Err(format!(
                "artifact bytes changed after runtime registration: {}",
                path.display()
            ))
        }
    })
    .await
    .map_err(|error| format!("artifact hash task failed: {error}"))?
}

pub(crate) fn inspect(path: &Path, json: bool) -> i32 {
    let payload = match std::fs::read_to_string(path) {
        Ok(payload) => payload,
        Err(error) => return fail("runtime inspect", format!("{}: {error}", path.display())),
    };
    if let Ok(runtime) = lmml_substrate::parse_runtime_manifest_json(&payload) {
        return emit(&runtime, path, json, "runtime");
    }
    match lmml_substrate::parse_model_lease_json(&payload) {
        Ok(lease) => emit(&lease, path, json, "lease"),
        Err(error) => fail("runtime inspect", error.to_string()),
    }
}

#[cfg(target_os = "linux")]
fn process_executes_artifact(pid: u32, artifact: &Path) -> Result<(), String> {
    let command_line = std::fs::read(format!("/proc/{pid}/cmdline"))
        .map_err(|error| format!("could not inspect process {pid}: {error}"))?;
    let arguments: Vec<&[u8]> = command_line
        .split(|byte| *byte == 0)
        .filter(|argument| !argument.is_empty())
        .collect();
    let artifact_bytes = artifact.as_os_str().as_encoded_bytes();
    if arguments.contains(&artifact_bytes) {
        Ok(())
    } else {
        Err(format!(
            "process {pid} does not execute registered artifact {}",
            artifact.display()
        ))
    }
}

#[cfg(not(target_os = "linux"))]
fn process_executes_artifact(_pid: u32, _artifact: &Path) -> Result<(), String> {
    Err("artifact-bound runtime registration currently requires Linux /proc".to_string())
}

fn emit(value: &impl serde::Serialize, path: &Path, json: bool, label: &str) -> i32 {
    if json {
        match serde_json::to_string_pretty(value) {
            Ok(payload) => println!("{payload}"),
            Err(error) => return fail(label, error.to_string()),
        }
    } else {
        println!("{label} registered\nmanifest: {}", path.display());
    }
    0
}

fn fail(operation: &str, error: String) -> i32 {
    eprintln!("{operation} failed: {error}");
    1
}

fn managed_data_root() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."))
}

async fn endpoint_health(endpoint: &str) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .map_err(|error| format!("could not create health client: {error}"))?;
    let base = endpoint.trim_end_matches('/');
    let mut errors = Vec::new();
    for path in ["/health", "/v1/health"] {
        match client.get(format!("{base}{path}")).send().await {
            Ok(response) if response.status().is_success() => return Ok(()),
            Ok(response) => errors.push(format!("{path}: HTTP {}", response.status())),
            Err(error) => errors.push(format!("{path}: {error}")),
        }
    }
    Err(format!(
        "runtime endpoint is not ready at {endpoint}: {}",
        errors.join("; ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn runtime_lease_rechecks_artifact_bytes() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("model.gguf");
        std::fs::write(&path, b"admitted").expect("artifact");
        let admitted_hash = lmml_substrate::sha256_file(&path).expect("hash");
        let artifact = lmml_substrate::ArtifactManifest {
            schema_version: lmml_substrate::SCHEMA_VERSION,
            artifact: lmml_substrate::ArtifactIdentity {
                artifact_id: "artifact-1".into(),
                model_lineage_id: "lineage-1".into(),
                representation: lmml_substrate::ModelRepresentation::Gguf,
                quantization: Some(lmml_substrate::QuantizationKind::Q8_0),
                artifact_hash: admitted_hash.clone(),
            },
            parent_artifact: Some("lineage-1-safetensors".into()),
            canonical_model: "lineage-1".into(),
            tool: "llama.cpp".into(),
            tool_version: "test".into(),
            command_or_parameters: vec!["Q8_0".into()],
            source_hashes: vec![lmml_substrate::Hash256::parse("a".repeat(64)).expect("hash")],
            output_hash: admitted_hash,
            artifact_path: path.clone(),
            admission: Some(lmml_substrate::ArtifactAdmission {
                backend: "llama.cpp".into(),
                backend_version: "test".into(),
                checked_at: "2026-08-25T00:00:00Z".into(),
            }),
            created_at: "2026-08-25T00:00:00Z".into(),
        };

        verify_artifact_hash(&artifact)
            .await
            .expect("matching artifact");
        std::fs::write(path, b"replaced").expect("replace artifact");
        assert!(verify_artifact_hash(&artifact).await.is_err());
    }
}
