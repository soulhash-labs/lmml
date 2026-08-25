//! Artifact-manifest construction and source-anchor persistence.

use std::fs;
use std::path::{Path, PathBuf};

use time::{format_description::well_known::Rfc3339, OffsetDateTime};

pub(super) fn canonical_source_artifact(
    substrate: &lmml_substrate::SubstrateManifest,
    source: &Path,
) -> Result<lmml_substrate::ArtifactManifest, String> {
    artifact_manifest(
        &format!("{}-safetensors", substrate.model.lineage_id),
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
    )
}

pub(super) fn ensure_source_artifact(
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

#[allow(clippy::too_many_arguments)]
pub(super) fn artifact_manifest(
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

pub(super) fn absolute_path(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        std::env::current_dir()
            .map(|root| root.join(path))
            .map_err(|error| format!("could not resolve {}: {error}", path.display()))
    }
}

pub(super) fn timestamp() -> Result<String, String> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|error| error.to_string())
}
