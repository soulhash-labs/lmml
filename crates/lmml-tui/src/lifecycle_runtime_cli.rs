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

#[derive(Debug, Clone, PartialEq, Eq)]
struct LiveProcessIdentity {
    backend_executable: PathBuf,
    backend_executable_hash: lmml_substrate::Hash256,
    command_line_hash: lmml_substrate::Hash256,
    context_size: usize,
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
    if let Err(error) = verify_artifact_hash(artifact).await {
        return fail(
            "runtime registration",
            format!("artifact verification failed: {error}"),
        );
    }
    let process_before = match inspect_process(options.pid, &artifact.artifact_path) {
        Ok(process) => process,
        Err(error) => return fail("runtime registration", error),
    };
    if process_before.context_size != options.context_size {
        return fail(
            "runtime registration",
            format!(
                "declared context {} does not match live process context {}",
                options.context_size, process_before.context_size
            ),
        );
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
    if let Err(error) = verify_artifact_hash(artifact).await {
        return fail(
            "runtime registration",
            format!("artifact changed during registration: {error}"),
        );
    }
    let process_after = match inspect_process(options.pid, &artifact.artifact_path) {
        Ok(process) => process,
        Err(error) => return fail("runtime registration", error),
    };
    if process_after != process_before {
        return fail(
            "runtime registration",
            "runtime process identity changed during registration".to_string(),
        );
    }
    let created_at = match OffsetDateTime::now_utc().format(&Rfc3339) {
        Ok(timestamp) => timestamp,
        Err(error) => return fail("runtime registration", error.to_string()),
    };
    let manifest = lmml_substrate::RuntimeManifest {
        schema_version: lmml_substrate::SCHEMA_VERSION,
        runtime_id: options.runtime_id.to_string(),
        pid: options.pid,
        model_lineage_id: artifact.artifact.model_lineage_id.clone(),
        artifact_id: artifact.artifact.artifact_id.clone(),
        artifact_hash: artifact.artifact.artifact_hash.clone(),
        representation: artifact.artifact.representation,
        quantization: artifact.artifact.quantization,
        backend: "llama.cpp".to_string(),
        backend_version: options.backend_version.to_string(),
        backend_executable: process_after.backend_executable,
        backend_executable_hash: process_after.backend_executable_hash,
        command_line_hash: process_after.command_line_hash,
        endpoint: options.endpoint.to_string(),
        context_size: process_after.context_size,
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
    let request = lmml_substrate::ModelRequest {
        model_lineage_id: lineage_id.to_string(),
        purpose: purpose.to_string(),
        representation_preference: vec![lmml_substrate::ModelRepresentation::Gguf],
        required_capabilities: capabilities.iter().copied().map(Into::into).collect(),
    };
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
    let mut rejected = Vec::new();
    for runtime in persisted_runtimes {
        if runtime.model_lineage_id != request.model_lineage_id
            || !request
                .required_capabilities
                .iter()
                .all(|capability| runtime.capabilities.contains(capability))
        {
            continue;
        }
        let Some(artifact) = artifacts
            .iter()
            .find(|artifact| artifact.artifact.artifact_id == runtime.artifact_id)
        else {
            rejected.push(format!(
                "runtime {} references missing artifact {}",
                runtime.runtime_id, runtime.artifact_id
            ));
            continue;
        };
        if let Err(error) = verify_registered_process(&runtime, &artifact.artifact_path) {
            rejected.push(format!("runtime {}: {error}", runtime.runtime_id));
            continue;
        }
        if let Err(error) = verify_artifact_hash(artifact).await {
            rejected.push(format!("runtime {}: {error}", runtime.runtime_id));
            continue;
        }
        if let Err(error) = endpoint_health(&runtime.endpoint).await {
            rejected.push(format!("runtime {}: {error}", runtime.runtime_id));
            continue;
        }
        if let Err(error) = lmml_server::verify_served_model_endpoint(
            &runtime.endpoint,
            &artifact.artifact_path,
            None,
        )
        .await
        {
            rejected.push(format!(
                "runtime {} model identity: {error}",
                runtime.runtime_id
            ));
            continue;
        }
        if let Err(error) = verify_artifact_hash(artifact).await {
            rejected.push(format!("runtime {}: {error}", runtime.runtime_id));
            continue;
        }
        if let Err(error) = verify_registered_process(&runtime, &artifact.artifact_path) {
            rejected.push(format!("runtime {}: {error}", runtime.runtime_id));
            continue;
        }
        runtimes.push(runtime);
    }
    if runtimes.is_empty() && !rejected.is_empty() {
        return fail("runtime lease", rejected.join("; "));
    }
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
                "artifact bytes do not match the admitted hash: {}",
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

fn verify_registered_process(
    runtime: &lmml_substrate::RuntimeManifest,
    artifact: &Path,
) -> Result<(), String> {
    let actual = inspect_process(runtime.pid, artifact)?;
    let expected = LiveProcessIdentity {
        backend_executable: runtime.backend_executable.clone(),
        backend_executable_hash: runtime.backend_executable_hash.clone(),
        command_line_hash: runtime.command_line_hash.clone(),
        context_size: runtime.context_size,
    };
    if process_identity_matches(&expected, &actual) {
        Ok(())
    } else {
        Err("live process identity does not match its runtime manifest".to_string())
    }
}

fn process_identity_matches(expected: &LiveProcessIdentity, actual: &LiveProcessIdentity) -> bool {
    expected == actual
}

#[cfg(target_os = "linux")]
fn inspect_process(pid: u32, artifact: &Path) -> Result<LiveProcessIdentity, String> {
    use std::os::unix::ffi::OsStrExt;

    let proc_root = PathBuf::from(format!("/proc/{pid}"));
    let command_line = std::fs::read(proc_root.join("cmdline"))
        .map_err(|error| format!("could not inspect process {pid}: {error}"))?;
    let arguments: Vec<&[u8]> = command_line
        .split(|byte| *byte == 0)
        .filter(|argument| !argument.is_empty())
        .collect();
    if !command_uses_artifact(&arguments, artifact.as_os_str().as_bytes()) {
        return Err(format!(
            "process {pid} does not execute registered artifact {}",
            artifact.display()
        ));
    }
    let context_size = parse_context_size(&arguments)?;
    let executable_link = proc_root.join("exe");
    let backend_executable = std::fs::read_link(&executable_link)
        .map_err(|error| format!("could not resolve process {pid} executable: {error}"))?;
    if !backend_executable.is_absolute()
        || backend_executable
            .as_os_str()
            .as_bytes()
            .ends_with(b" (deleted)")
    {
        return Err(format!(
            "process {pid} executable is not a stable absolute path: {}",
            backend_executable.display()
        ));
    }
    let backend_executable = backend_executable
        .canonicalize()
        .map_err(|error| format!("could not canonicalize process {pid} executable: {error}"))?;
    let backend_executable_hash = lmml_substrate::sha256_file(&executable_link)
        .map_err(|error| format!("could not hash process {pid} executable: {error}"))?;
    Ok(LiveProcessIdentity {
        backend_executable,
        backend_executable_hash,
        command_line_hash: lmml_substrate::sha256_data(&command_line),
        context_size,
    })
}

#[cfg(not(target_os = "linux"))]
fn inspect_process(_pid: u32, _artifact: &Path) -> Result<LiveProcessIdentity, String> {
    Err("artifact-bound runtime registration currently requires Linux /proc".to_string())
}

fn command_uses_artifact(arguments: &[&[u8]], artifact: &[u8]) -> bool {
    arguments
        .windows(2)
        .any(|pair| matches!(pair[0], b"-m" | b"--model") && pair[1] == artifact)
        || arguments.iter().any(|argument| {
            [b"--model=".as_slice(), b"-m=".as_slice()]
                .iter()
                .any(|prefix| argument.strip_prefix(*prefix) == Some(artifact))
        })
}

fn parse_context_size(arguments: &[&[u8]]) -> Result<usize, String> {
    let mut values = Vec::new();
    for (index, argument) in arguments.iter().enumerate() {
        if matches!(*argument, b"-c" | b"--ctx-size" | b"--context-size") {
            let value = arguments
                .get(index + 1)
                .ok_or_else(|| "runtime context flag has no value".to_string())?;
            values.push(*value);
            continue;
        }
        for prefix in [b"--ctx-size=".as_slice(), b"--context-size=".as_slice()] {
            if let Some(value) = argument.strip_prefix(prefix) {
                values.push(value);
            }
        }
    }
    if values.len() != 1 {
        return Err(format!(
            "runtime must expose exactly one explicit context-size argument; found {}",
            values.len()
        ));
    }
    let value = std::str::from_utf8(values[0])
        .map_err(|_| "runtime context size is not valid UTF-8".to_string())?
        .parse::<usize>()
        .map_err(|_| "runtime context size is not a positive integer".to_string())?;
    if value == 0 {
        return Err("runtime context size must be greater than zero".to_string());
    }
    Ok(value)
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

    #[test]
    fn runtime_arguments_bind_model_and_one_explicit_context() {
        let model = b"/models/qwen38-q8.gguf";
        let arguments = [
            b"llama-server".as_slice(),
            b"--model".as_slice(),
            model,
            b"--ctx-size=16384".as_slice(),
        ];
        assert!(command_uses_artifact(&arguments, model));
        assert_eq!(parse_context_size(&arguments).expect("context"), 16_384);

        let wrong_model = b"/models/other.gguf";
        assert!(!command_uses_artifact(&arguments, wrong_model));
        let duplicate = [
            b"llama-server".as_slice(),
            b"-c".as_slice(),
            b"4096".as_slice(),
            b"--ctx-size".as_slice(),
            b"16384".as_slice(),
        ];
        assert!(parse_context_size(&duplicate).is_err());
    }

    #[test]
    fn runtime_identity_detects_binary_command_and_context_changes() {
        let expected = LiveProcessIdentity {
            backend_executable: PathBuf::from("/usr/bin/llama-server"),
            backend_executable_hash: lmml_substrate::Hash256::parse("a".repeat(64)).expect("hash"),
            command_line_hash: lmml_substrate::Hash256::parse("b".repeat(64)).expect("hash"),
            context_size: 16_384,
        };
        assert!(process_identity_matches(&expected, &expected));

        let mut changed = expected.clone();
        changed.backend_executable_hash =
            lmml_substrate::Hash256::parse("c".repeat(64)).expect("hash");
        assert!(!process_identity_matches(&expected, &changed));
        changed = expected.clone();
        changed.command_line_hash = lmml_substrate::Hash256::parse("d".repeat(64)).expect("hash");
        assert!(!process_identity_matches(&expected, &changed));
        changed = expected.clone();
        changed.context_size = 4096;
        assert!(!process_identity_matches(&expected, &changed));
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn linux_process_inspection_hashes_the_live_executable_and_command() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().expect("tempdir");
        let model = directory.path().join("model.gguf");
        std::fs::write(&model, b"fixture").expect("model fixture");
        let server = directory.path().join("llama-server-fixture");
        std::fs::write(&server, b"#!/bin/sh\nwhile :; do sleep 1; done\n").expect("server fixture");
        let mut permissions = std::fs::metadata(&server)
            .expect("fixture metadata")
            .permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&server, permissions).expect("fixture permissions");
        let mut child = tokio::process::Command::new(&server)
            .arg("--model")
            .arg(&model)
            .args(["--ctx-size", "16384"])
            .kill_on_drop(true)
            .spawn()
            .expect("spawn fixture process");
        let pid = child.id().expect("child pid");

        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
        let identity = loop {
            match inspect_process(pid, &model) {
                Ok(identity) => break identity,
                Err(_) if tokio::time::Instant::now() < deadline => {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
                Err(error) => panic!("inspect live process: {error}"),
            }
        };
        assert_eq!(identity.context_size, 16_384);
        assert!(identity.backend_executable.is_absolute());
        assert_eq!(identity.backend_executable_hash.as_str().len(), 64);
        assert_eq!(identity.command_line_hash.as_str().len(), 64);

        child.start_kill().expect("stop fixture process");
        child.wait().await.expect("wait for fixture process");
    }
}
