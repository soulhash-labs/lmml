//! Immutable substrate and artifact manifest persistence.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::Value;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use crate::{
    read_required, sha256_bytes, tokenizer_hash_from_records, ArtifactManifest, Hash256,
    ModelRepresentation, SubstrateError, SubstrateManifest, SCHEMA_VERSION,
};

/// Serialize a manifest in stable, human-readable JSON.
pub fn manifest_json(manifest: &SubstrateManifest) -> Result<String, SubstrateError> {
    validate_substrate_manifest(manifest)?;
    serde_json::to_string_pretty(manifest).map_err(SubstrateError::Serialize)
}

/// Parse and validate a versioned substrate manifest.
pub fn parse_manifest_json(payload: &str) -> Result<SubstrateManifest, SubstrateError> {
    let value: Value = serde_json::from_str(payload).map_err(SubstrateError::ManifestJson)?;
    let version = value
        .get("schema_version")
        .and_then(Value::as_u64)
        .and_then(|version| u32::try_from(version).ok())
        .ok_or(SubstrateError::MissingSchemaVersion)?;
    validate_schema_version(version)?;
    let manifest: SubstrateManifest =
        serde_json::from_value(value).map_err(SubstrateError::ManifestJson)?;
    validate_substrate_manifest(&manifest)?;
    Ok(manifest)
}

/// Parse and validate a versioned artifact manifest.
pub fn parse_artifact_manifest_json(payload: &str) -> Result<ArtifactManifest, SubstrateError> {
    let manifest: ArtifactManifest =
        serde_json::from_str(payload).map_err(SubstrateError::ManifestJson)?;
    validate_artifact_manifest(&manifest)?;
    Ok(manifest)
}

/// Persist a canonical manifest without replacing an existing lineage record.
pub fn store_substrate_manifest(
    path: impl AsRef<Path>,
    manifest: &SubstrateManifest,
) -> Result<PathBuf, SubstrateError> {
    validate_substrate_manifest(manifest)?;
    let path = path.as_ref();
    let payload = serde_json::to_vec_pretty(manifest).map_err(SubstrateError::Serialize)?;
    persist_immutable(path, &payload, |path| {
        SubstrateError::ManifestConflict(path)
    })?;
    Ok(path.to_path_buf())
}

/// Append an artifact manifest to a managed lineage directory.
///
/// An existing artifact ID may be written again only when its serialized
/// record is identical. A different record for the same ID is rejected.
pub fn append_artifact_manifest(
    root: impl AsRef<Path>,
    manifest: &ArtifactManifest,
) -> Result<PathBuf, SubstrateError> {
    validate_artifact_manifest(manifest)?;
    let root = root.as_ref();
    fs::create_dir_all(root).map_err(|source| SubstrateError::Io {
        path: root.to_path_buf(),
        source,
    })?;
    let path = root.join(format!("{}.json", manifest.artifact.artifact_id));
    let payload = serde_json::to_vec_pretty(manifest).map_err(SubstrateError::Serialize)?;
    persist_immutable(&path, &payload, SubstrateError::ArtifactConflict)?;
    Ok(path)
}

fn manifest_hash(manifest: &SubstrateManifest) -> Result<Hash256, SubstrateError> {
    let mut value = manifest.clone();
    value.model.canonical_manifest_hash = zero_hash();
    let bytes = serde_json::to_vec(&value).map_err(SubstrateError::Serialize)?;
    Ok(sha256_bytes(&bytes))
}

pub(crate) fn seal_manifest(manifest: &mut SubstrateManifest) -> Result<(), SubstrateError> {
    manifest.model.canonical_manifest_hash = manifest_hash(manifest)?;
    validate_substrate_manifest(manifest)
}

pub(crate) fn validate_substrate_manifest(
    manifest: &SubstrateManifest,
) -> Result<(), SubstrateError> {
    validate_schema_version(manifest.schema_version)?;
    validate_identifier(&manifest.model.lineage_id)?;
    if let Some(parent) = &manifest.model.parent {
        validate_identifier(parent)?;
    }
    if manifest.shards.is_empty() || manifest.tensors.is_empty() {
        return Err(SubstrateError::InvalidSubstrateManifest(
            "manifest must contain shards and tensors".to_string(),
        ));
    }
    if manifest.tensor_count != manifest.tensors.len() as u64 {
        return Err(SubstrateError::InvalidSubstrateManifest(
            "tensor_count does not match tensor records".to_string(),
        ));
    }
    let parameter_count = manifest
        .tensors
        .iter()
        .try_fold(0_u64, |total, tensor| {
            let tensor_count = tensor
                .shape
                .iter()
                .try_fold(1_u64, |count, dimension| count.checked_mul(*dimension))?;
            (tensor_count == tensor.parameter_count)
                .then_some(())
                .and_then(|()| total.checked_add(tensor_count))
        })
        .ok_or_else(|| {
            SubstrateError::InvalidSubstrateManifest(
                "tensor parameter counts are inconsistent or overflow".to_string(),
            )
        })?;
    if parameter_count != manifest.parameter_count {
        return Err(SubstrateError::InvalidSubstrateManifest(
            "parameter_count does not match tensor records".to_string(),
        ));
    }
    if !is_strictly_sorted_unique(manifest.tensors.iter().map(|tensor| tensor.name.as_str())) {
        return Err(SubstrateError::InvalidSubstrateManifest(
            "tensor records must be uniquely sorted by name".to_string(),
        ));
    }
    if !is_strictly_sorted_unique(manifest.shards.iter().map(|shard| shard.path.as_str())) {
        return Err(SubstrateError::InvalidSubstrateManifest(
            "shard records must be uniquely sorted by path".to_string(),
        ));
    }
    if !is_strictly_sorted_unique(
        manifest
            .auxiliary_files
            .iter()
            .map(|file| file.path.as_str()),
    ) {
        return Err(SubstrateError::InvalidSubstrateManifest(
            "auxiliary file records must be uniquely sorted by path".to_string(),
        ));
    }
    for shard in &manifest.shards {
        if !is_strictly_sorted_unique(shard.tensor_names.iter().map(String::as_str)) {
            return Err(SubstrateError::InvalidSubstrateManifest(format!(
                "tensor names for shard {} must be uniquely sorted",
                shard.path
            )));
        }
        let recorded: Vec<&str> = manifest
            .tensors
            .iter()
            .filter(|tensor| tensor.shard == shard.path)
            .map(|tensor| tensor.name.as_str())
            .collect();
        let indexed: Vec<&str> = shard.tensor_names.iter().map(String::as_str).collect();
        if recorded != indexed {
            return Err(SubstrateError::InvalidSubstrateManifest(format!(
                "tensor inventory does not match shard {}",
                shard.path
            )));
        }
    }
    if manifest.tensors.iter().any(|tensor| {
        !manifest
            .shards
            .iter()
            .any(|shard| shard.path == tensor.shard)
    }) {
        return Err(SubstrateError::InvalidSubstrateManifest(
            "tensor references an unknown shard".to_string(),
        ));
    }
    let tokenizer_hash =
        tokenizer_hash_from_records(&manifest.auxiliary_files).ok_or_else(|| {
            SubstrateError::InvalidSubstrateManifest(
                "manifest has no tokenizer asset records".to_string(),
            )
        })?;
    if tokenizer_hash != manifest.tokenizer_hash {
        return Err(SubstrateError::InvalidSubstrateManifest(
            "tokenizer_hash does not match auxiliary records".to_string(),
        ));
    }
    let expected = manifest_hash(manifest)?;
    if manifest.model.canonical_manifest_hash != expected {
        return Err(SubstrateError::ManifestMismatch);
    }
    Ok(())
}

pub(crate) fn validate_artifact_manifest(
    manifest: &ArtifactManifest,
) -> Result<(), SubstrateError> {
    validate_schema_version(manifest.schema_version)?;
    validate_identifier(&manifest.artifact.artifact_id)?;
    validate_identifier(&manifest.artifact.model_lineage_id)?;
    if let Some(parent) = &manifest.parent_artifact {
        validate_identifier(parent)?;
        if parent == &manifest.artifact.artifact_id {
            return Err(SubstrateError::InvalidArtifactManifest(
                "artifact cannot be its own parent".to_string(),
            ));
        }
    }
    if manifest.canonical_model != manifest.artifact.model_lineage_id {
        return Err(SubstrateError::InvalidArtifactManifest(
            "canonical_model must match artifact.model_lineage_id".to_string(),
        ));
    }
    if manifest.output_hash != manifest.artifact.artifact_hash {
        return Err(SubstrateError::InvalidArtifactManifest(
            "output_hash must match artifact.artifact_hash".to_string(),
        ));
    }
    if manifest.source_hashes.is_empty() {
        return Err(SubstrateError::InvalidArtifactManifest(
            "derived artifacts require at least one source hash".to_string(),
        ));
    }
    if manifest.tool.trim().is_empty() || manifest.tool_version.trim().is_empty() {
        return Err(SubstrateError::InvalidArtifactManifest(
            "tool, tool_version, and created_at are required".to_string(),
        ));
    }
    OffsetDateTime::parse(&manifest.created_at, &Rfc3339).map_err(|_| {
        SubstrateError::InvalidArtifactManifest("created_at must be RFC3339".to_string())
    })?;
    if !manifest.artifact_path.is_absolute() {
        return Err(SubstrateError::InvalidArtifactManifest(
            "artifact_path must be absolute".to_string(),
        ));
    }
    if let Some(admission) = &manifest.admission {
        if admission.backend.trim().is_empty() || admission.backend_version.trim().is_empty() {
            return Err(SubstrateError::InvalidArtifactManifest(
                "admission backend and version are required".to_string(),
            ));
        }
        OffsetDateTime::parse(&admission.checked_at, &Rfc3339).map_err(|_| {
            SubstrateError::InvalidArtifactManifest(
                "admission.checked_at must be RFC3339".to_string(),
            )
        })?;
    }
    if manifest.artifact.representation == ModelRepresentation::Gguf && manifest.admission.is_none()
    {
        return Err(SubstrateError::InvalidArtifactManifest(
            "GGUF artifacts require backend admission evidence".to_string(),
        ));
    }
    if manifest.artifact.representation == ModelRepresentation::Safetensors
        && manifest.artifact.quantization.is_some()
    {
        return Err(SubstrateError::InvalidArtifactManifest(
            "Safetensors artifacts cannot declare GGUF quantization".to_string(),
        ));
    }
    Ok(())
}

fn is_strictly_sorted_unique<'a>(values: impl Iterator<Item = &'a str>) -> bool {
    let mut previous = None;
    for value in values {
        if previous.is_some_and(|previous| previous >= value) {
            return false;
        }
        previous = Some(value);
    }
    true
}

pub(crate) fn validate_schema_version(version: u32) -> Result<(), SubstrateError> {
    if version == SCHEMA_VERSION {
        Ok(())
    } else {
        Err(SubstrateError::UnsupportedSchemaVersion {
            expected: SCHEMA_VERSION,
            actual: version,
        })
    }
}

pub fn validate_identifier(identifier: &str) -> Result<(), SubstrateError> {
    if !identifier.is_empty()
        && identifier != "."
        && identifier != ".."
        && identifier
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        Ok(())
    } else {
        Err(SubstrateError::InvalidIdentifier(identifier.to_string()))
    }
}

fn zero_hash() -> Hash256 {
    Hash256("0".repeat(64))
}

pub(crate) fn persist_immutable(
    path: &Path,
    payload: &[u8],
    conflict: impl FnOnce(PathBuf) -> SubstrateError,
) -> Result<(), SubstrateError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| SubstrateError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    if path.is_file() {
        let existing = read_required(path)?;
        return if existing == payload {
            Ok(())
        } else {
            Err(conflict(path.to_path_buf()))
        };
    }

    let mut temporary =
        tempfile::NamedTempFile::new_in(parent).map_err(|source| SubstrateError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    temporary
        .write_all(payload)
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(|source| SubstrateError::Io {
            path: temporary.path().to_path_buf(),
            source,
        })?;
    match temporary.persist_noclobber(path) {
        Ok(_) => Ok(()),
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing = read_required(path)?;
            if existing == payload {
                Ok(())
            } else {
                Err(conflict(path.to_path_buf()))
            }
        }
        Err(error) => Err(SubstrateError::Io {
            path: path.to_path_buf(),
            source: error.error,
        }),
    }
}
