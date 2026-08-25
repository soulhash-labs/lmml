//! Pending GGUF candidate persistence and backend admission.

use std::fs;
use std::path::{Path, PathBuf};

use super::manifest::{
    absolute_path, artifact_manifest, canonical_source_artifact, ensure_source_artifact, timestamp,
};
use super::{
    default_server_path, publish_no_clobber, read_substrate_manifest, require_admitted_successor,
    tool_version, GgufAdmissionProvider, LlamaServerAdmission,
};

pub(super) struct RecordOptions<'a> {
    pub output_temp: &'a Path,
    pub output: &'a Path,
    pub artifact_id: &'a str,
    pub lineage_id: &'a str,
    pub parent_artifact: &'a str,
    pub quantization: lmml_substrate::QuantizationKind,
    pub artifact_hash: lmml_substrate::Hash256,
    pub source_hash: lmml_substrate::Hash256,
    pub tool_version: &'a str,
    pub command_or_parameters: Vec<String>,
    pub json: bool,
}

pub(super) fn record(options: RecordOptions<'_>, data_root: &Path) -> i32 {
    let candidate = match candidate_manifest(&options) {
        Ok(manifest) => manifest,
        Err(error) => {
            eprintln!("could not create GGUF candidate manifest: {error}");
            return 1;
        }
    };
    if let Err(error) = publish_no_clobber(options.output_temp, options.output) {
        eprintln!("could not publish GGUF candidate: {error}");
        return 1;
    }
    let root = data_root.join("lmml/models/candidates");
    if let Err(error) = lmml_substrate::append_gguf_candidate_manifest(&root, &candidate) {
        eprintln!("could not register GGUF candidate: {error}");
        if let Err(cleanup_error) = fs::remove_file(options.output) {
            eprintln!(
                "could not roll back candidate output {}: {cleanup_error}",
                options.output.display()
            );
        }
        return 1;
    }
    if options.json {
        match serde_json::to_string_pretty(&candidate) {
            Ok(payload) => println!("{payload}"),
            Err(error) => {
                eprintln!("could not serialize GGUF candidate: {error}");
                return 1;
            }
        }
    } else {
        println!(
            "derived pending GGUF {}\ncandidate: {}\nhash: {}\nadmission: deferred",
            options.output.display(),
            candidate.artifact.artifact_id,
            candidate.output_hash
        );
    }
    0
}

fn candidate_manifest(
    options: &RecordOptions<'_>,
) -> Result<lmml_substrate::GgufCandidateManifest, String> {
    Ok(lmml_substrate::GgufCandidateManifest {
        schema_version: lmml_substrate::SCHEMA_VERSION,
        artifact: lmml_substrate::ArtifactIdentity {
            artifact_id: options.artifact_id.to_string(),
            model_lineage_id: options.lineage_id.to_string(),
            representation: lmml_substrate::ModelRepresentation::Gguf,
            quantization: Some(options.quantization),
            artifact_hash: options.artifact_hash.clone(),
        },
        parent_artifact: options.parent_artifact.to_string(),
        canonical_model: options.lineage_id.to_string(),
        tool: "llama.cpp".to_string(),
        tool_version: options.tool_version.to_string(),
        command_or_parameters: options.command_or_parameters.clone(),
        source_hashes: vec![options.source_hash.clone()],
        output_hash: options.artifact_hash.clone(),
        artifact_path: absolute_path(options.output)?,
        created_at: timestamp()?,
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn admit(
    candidate_path: &Path,
    substrate_path: &Path,
    source: &Path,
    server: Option<&Path>,
    json: bool,
    data_root: &Path,
) -> i32 {
    admit_with(
        candidate_path,
        substrate_path,
        source,
        server,
        json,
        data_root,
        &LlamaServerAdmission,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn admit_with<A: GgufAdmissionProvider>(
    candidate_path: &Path,
    substrate_path: &Path,
    source: &Path,
    server: Option<&Path>,
    json: bool,
    data_root: &Path,
    admission_provider: &A,
) -> i32 {
    let candidate = match read_candidate(candidate_path) {
        Ok(candidate) => candidate,
        Err(error) => {
            eprintln!("artifact admission refused: {error}");
            return 1;
        }
    };
    let substrate = match read_substrate_manifest(substrate_path) {
        Ok(manifest) => manifest,
        Err(error) => {
            eprintln!("artifact admission refused: {error}");
            return 1;
        }
    };
    if let Err(error) = validate_candidate_inputs(&candidate, &substrate, source, data_root) {
        eprintln!("artifact admission refused: {error}");
        return 1;
    }
    let server_path = server
        .map(PathBuf::from)
        .unwrap_or_else(default_server_path);
    if let Err(error) = admission_provider
        .admit(&candidate.artifact_path, &server_path)
        .await
    {
        eprintln!("artifact admission failed: {error}");
        return 1;
    }
    let server_version = tool_version(&server_path).await;
    let admission = lmml_substrate::ArtifactAdmission {
        backend: "llama.cpp".to_string(),
        backend_version: server_version.clone(),
        checked_at: match timestamp() {
            Ok(value) => value,
            Err(error) => {
                eprintln!("artifact admission failed: {error}");
                return 1;
            }
        },
    };
    let artifact = match artifact_manifest(
        &candidate.artifact.artifact_id,
        &candidate.artifact.model_lineage_id,
        candidate.artifact.representation,
        candidate.artifact.quantization,
        candidate.artifact.artifact_hash.clone(),
        Some(candidate.parent_artifact.clone()),
        &candidate.tool,
        &format!(
            "{}; server_path={}; server_version={server_version}",
            candidate.tool_version,
            server_path.display()
        ),
        candidate.source_hashes.clone(),
        candidate.command_or_parameters.clone(),
        &candidate.artifact_path,
        Some(admission),
    ) {
        Ok(artifact) => artifact,
        Err(error) => {
            eprintln!("artifact admission failed: {error}");
            return 1;
        }
    };
    let source_artifact = match canonical_source_artifact(&substrate, source) {
        Ok(manifest) => manifest,
        Err(error) => {
            eprintln!("artifact admission failed: {error}");
            return 1;
        }
    };
    let artifact_root = data_root.join("lmml/models/artifacts");
    if let Err(error) = ensure_source_artifact(&artifact_root, &source_artifact)
        .and_then(|_| lmml_substrate::append_artifact_manifest(&artifact_root, &artifact))
    {
        eprintln!("could not register admitted artifact: {error}");
        return 1;
    }
    if json {
        match serde_json::to_string_pretty(&artifact) {
            Ok(payload) => println!("{payload}"),
            Err(error) => {
                eprintln!("could not serialize artifact manifest: {error}");
                return 1;
            }
        }
    } else {
        println!(
            "admitted {}\nartifact: {}\nhash: {}",
            artifact.artifact_path.display(),
            artifact.artifact.artifact_id,
            artifact.output_hash
        );
    }
    0
}

fn read_candidate(path: &Path) -> Result<lmml_substrate::GgufCandidateManifest, String> {
    let payload =
        fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    lmml_substrate::parse_gguf_candidate_manifest_json(&payload)
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn validate_candidate_inputs(
    candidate: &lmml_substrate::GgufCandidateManifest,
    substrate: &lmml_substrate::SubstrateManifest,
    source: &Path,
    data_root: &Path,
) -> Result<(), String> {
    require_admitted_successor(substrate, data_root).map_err(|error| error.to_string())?;
    lmml_substrate::verify_safetensors(source, substrate).map_err(|error| error.to_string())?;
    let expected_parent = format!("{}-safetensors", substrate.model.lineage_id);
    if candidate.canonical_model != substrate.model.lineage_id
        || candidate.parent_artifact != expected_parent
        || candidate.source_hashes != vec![substrate.model.canonical_manifest_hash.clone()]
    {
        return Err("candidate does not belong to the verified canonical substrate".to_string());
    }
    let hash =
        lmml_substrate::sha256_file(&candidate.artifact_path).map_err(|error| error.to_string())?;
    if hash != candidate.output_hash {
        return Err("candidate payload hash does not match its immutable record".to_string());
    }
    Ok(())
}
