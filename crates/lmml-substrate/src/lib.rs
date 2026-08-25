//! Canonical model-substrate and artifact-lineage contracts.
//!
//! This crate keeps model identity separate from deployment artifacts. It
//! imports a Safetensors checkpoint by reading its manifests and tensor
//! headers, without loading tensor payloads into memory. GGUF conversion and
//! runtime crates consume these records but do not redefine their identity.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
mod adapter;
mod lifecycle;
mod storage;
mod types;

pub use adapter::*;
pub use lifecycle::*;
pub use storage::{
    append_artifact_manifest, append_gguf_candidate_manifest, manifest_json,
    parse_artifact_manifest_json, parse_gguf_candidate_manifest_json, parse_manifest_json,
    store_substrate_manifest, validate_identifier,
};
pub use types::*;

/// Hash a completed deployment artifact with SHA-256.
pub fn sha256_file(path: impl AsRef<Path>) -> Result<Hash256, SubstrateError> {
    hash_file(path.as_ref())
}

/// Hash an in-memory lifecycle payload with SHA-256.
pub fn sha256_data(bytes: &[u8]) -> Hash256 {
    sha256_bytes(bytes)
}

/// Verify that a file has the GGUF container magic used by llama.cpp.
pub fn validate_gguf(path: impl AsRef<Path>) -> Result<(), SubstrateError> {
    let path = path.as_ref();
    let mut file = File::open(path).map_err(|source| SubstrateError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut header = [0_u8; 24];
    file.read_exact(&mut header)
        .map_err(|source| SubstrateError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    let version = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
    if &header[..4] == b"GGUF" && matches!(version, 2 | 3) {
        Ok(())
    } else {
        Err(SubstrateError::InvalidGguf(path.to_path_buf()))
    }
}

/// Import a canonical Safetensors directory into a deterministic manifest.
pub fn import_safetensors(
    root: impl AsRef<Path>,
    lineage_id: &str,
) -> Result<SubstrateManifest, SubstrateError> {
    import_safetensors_with_parent(root, lineage_id, None, &mut |_| {})
}

/// Import a canonical Safetensors directory while reporting completed shards.
pub fn import_safetensors_with_progress(
    root: impl AsRef<Path>,
    lineage_id: &str,
    mut progress: impl FnMut(&ImportProgress),
) -> Result<SubstrateManifest, SubstrateError> {
    import_safetensors_with_parent(root, lineage_id, None, &mut progress)
}

/// Import a successor Safetensors directory with an explicit parent lineage.
pub fn import_successor_safetensors(
    root: impl AsRef<Path>,
    lineage_id: &str,
    parent: &str,
) -> Result<SubstrateManifest, SubstrateError> {
    storage::validate_identifier(parent)?;
    import_safetensors_with_parent(root, lineage_id, Some(parent), &mut |_| {})
}

fn import_safetensors_with_parent(
    root: impl AsRef<Path>,
    lineage_id: &str,
    parent: Option<&str>,
    progress: &mut dyn FnMut(&ImportProgress),
) -> Result<SubstrateManifest, SubstrateError> {
    storage::validate_identifier(lineage_id)?;
    let root = root.as_ref();
    if !root.is_dir() {
        return Err(SubstrateError::MissingRoot(root.to_path_buf()));
    }
    let config_path = root.join("config.json");
    let config_bytes = read_required(&config_path)?;
    let config: Value =
        serde_json::from_slice(&config_bytes).map_err(|source| SubstrateError::InvalidJson {
            path: config_path.clone(),
            source,
        })?;
    if !config.is_object() {
        return Err(SubstrateError::InvalidConfig(config_path));
    }
    let architecture = architecture_from_config(&config);
    let index_path = root.join("model.safetensors.index.json");
    let index_bytes = if index_path.is_file() {
        Some(read_required(&index_path)?)
    } else {
        None
    };
    let index: Option<SafetensorsIndex> = index_bytes
        .as_deref()
        .map(serde_json::from_slice)
        .transpose()
        .map_err(|source| SubstrateError::InvalidJson {
            path: index_path.clone(),
            source,
        })?;
    let referenced_shards = referenced_shards(root, index.as_ref())?;
    let actual_shards = actual_shards(root)?;
    if index.is_none() && actual_shards.len() != 1 {
        return Err(SubstrateError::InvalidIndex(index_path));
    }
    let missing_shards: Vec<String> = referenced_shards
        .difference(&actual_shards)
        .cloned()
        .collect();
    if !missing_shards.is_empty() {
        return Err(SubstrateError::MissingShards(missing_shards));
    }
    if actual_shards != referenced_shards {
        return Err(SubstrateError::MixedShardSet {
            referenced: referenced_shards.into_iter().collect(),
            actual: actual_shards.into_iter().collect(),
        });
    }

    let mut shards = Vec::new();
    let mut tensors = Vec::new();
    let mut discovered_tensor_names = BTreeSet::new();
    let total_shard_bytes =
        actual_shards
            .iter()
            .try_fold(0_u64, |total, shard| -> Result<u64, SubstrateError> {
                let path = root.join(shard);
                let metadata =
                    fs::metadata(&path).map_err(|source| SubstrateError::Io { path, source })?;
                total
                    .checked_add(metadata.len())
                    .ok_or_else(|| SubstrateError::ParameterOverflow("<shard bytes>".to_string()))
            })?;
    let mut bytes_hashed = 0_u64;
    for shard in &actual_shards {
        let path = root.join(shard);
        let headers = read_safetensors_headers(&path)?;
        let tensor_names: Vec<String> = headers.keys().cloned().collect();
        if let Some(weight_map) = index.as_ref().and_then(|value| value.weight_map.as_ref()) {
            for name in &tensor_names {
                if weight_map.get(name).map(String::as_str) != Some(shard.as_str()) {
                    return Err(SubstrateError::IndexMismatch {
                        tensor: name.clone(),
                        shard: shard.clone(),
                    });
                }
            }
        }
        for (name, header) in headers {
            if !discovered_tensor_names.insert(name.clone()) {
                return Err(SubstrateError::DuplicateTensor(name));
            }
            let parameter_count = header
                .shape
                .iter()
                .try_fold(1_u64, |total, dimension| total.checked_mul(*dimension))
                .ok_or_else(|| SubstrateError::ParameterOverflow(name.clone()))?;
            tensors.push(TensorDescriptor {
                name,
                shard: shard.clone(),
                dtype: header.dtype,
                shape: header.shape,
                parameter_count,
            });
        }
        let metadata = fs::metadata(&path).map_err(|source| SubstrateError::Io {
            path: path.clone(),
            source,
        })?;
        let sha256 = hash_file(&path)?;
        shards.push(ShardManifest {
            path: shard.clone(),
            size_bytes: metadata.len(),
            sha256,
            tensor_names,
        });
        bytes_hashed = bytes_hashed
            .checked_add(metadata.len())
            .ok_or_else(|| SubstrateError::ParameterOverflow("<shard bytes>".to_string()))?;
        progress(&ImportProgress {
            current_shard: shard.clone(),
            shards_completed: shards.len(),
            shards_total: actual_shards.len(),
            bytes_hashed,
            total_bytes: total_shard_bytes,
        });
    }
    if let Some(weight_map) = index.as_ref().and_then(|value| value.weight_map.as_ref()) {
        let indexed_tensor_names: BTreeSet<String> = weight_map.keys().cloned().collect();
        if indexed_tensor_names != discovered_tensor_names {
            return Err(SubstrateError::IndexTensorSetMismatch {
                indexed: indexed_tensor_names.into_iter().collect(),
                discovered: discovered_tensor_names.into_iter().collect(),
            });
        }
    }
    tensors.sort_by(|left, right| left.name.cmp(&right.name));
    let parameter_count = tensors
        .iter()
        .try_fold(0_u64, |total, tensor| {
            total.checked_add(tensor.parameter_count)
        })
        .ok_or(SubstrateError::ParameterOverflow("<total>".to_string()))?;
    let auxiliary_files = auxiliary_files(root)?;
    let tokenizer_hash = tokenizer_hash(&auxiliary_files, root)?;
    let model_name = model_name_from_config(&config, lineage_id);
    let mut manifest = SubstrateManifest {
        schema_version: SCHEMA_VERSION,
        model: ModelIdentity {
            lineage_id: lineage_id.to_string(),
            model_name,
            parent: parent.map(str::to_string),
            canonical_manifest_hash: Hash256::parse("0".repeat(64))?,
        },
        architecture,
        config_hash: sha256_bytes(&config_bytes),
        tokenizer_hash,
        index_hash: index_bytes.as_deref().map(sha256_bytes),
        auxiliary_files,
        tensor_count: tensors.len() as u64,
        parameter_count,
        shards,
        tensors,
    };
    storage::seal_manifest(&mut manifest)?;
    Ok(manifest)
}

/// Verify the files and hashes referenced by a substrate manifest.
pub fn verify_safetensors(
    root: impl AsRef<Path>,
    manifest: &SubstrateManifest,
) -> Result<(), SubstrateError> {
    storage::validate_substrate_manifest(manifest)?;
    let imported = import_safetensors_with_parent(
        root,
        &manifest.model.lineage_id,
        manifest.model.parent.as_deref(),
        &mut |_| {},
    )?;
    if imported != *manifest {
        return Err(SubstrateError::ManifestMismatch);
    }
    Ok(())
}

fn read_required(path: &Path) -> Result<Vec<u8>, SubstrateError> {
    fs::read(path).map_err(|source| SubstrateError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn actual_shards(root: &Path) -> Result<BTreeSet<String>, SubstrateError> {
    let mut paths = BTreeSet::new();
    for entry in fs::read_dir(root).map_err(|source| SubstrateError::Io {
        path: root.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| SubstrateError::Io {
            path: root.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "safetensors")
        {
            paths.insert(
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            );
        }
    }
    Ok(paths)
}

fn referenced_shards(
    root: &Path,
    index: Option<&SafetensorsIndex>,
) -> Result<BTreeSet<String>, SubstrateError> {
    if let Some(index) = index {
        let Some(weight_map) = &index.weight_map else {
            return Err(SubstrateError::InvalidIndex(
                root.join("model.safetensors.index.json"),
            ));
        };
        return Ok(weight_map.values().cloned().collect());
    }
    actual_shards(root)
}

fn architecture_from_config(config: &Value) -> ArchitectureFingerprint {
    let fields = config
        .as_object()
        .map(|object| {
            object
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()
        })
        .unwrap_or_default();
    ArchitectureFingerprint {
        model_type: config
            .get("model_type")
            .and_then(Value::as_str)
            .map(str::to_string),
        architectures: config
            .get("architectures")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        fields,
    }
}

fn model_name_from_config(config: &Value, lineage_id: &str) -> String {
    config
        .get("_name_or_path")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or(lineage_id)
        .to_string()
}

fn auxiliary_files(root: &Path) -> Result<Vec<AuxiliaryFileManifest>, SubstrateError> {
    let mut files = Vec::new();
    for entry in fs::read_dir(root).map_err(|source| SubstrateError::Io {
        path: root.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| SubstrateError::Io {
            path: root.to_path_buf(),
            source,
        })?;
        let file_type = entry.file_type().map_err(|source| SubstrateError::Io {
            path: entry.path(),
            source,
        })?;
        if !file_type.is_file() {
            continue;
        }
        let filename = entry.file_name().to_string_lossy().into_owned();
        if filename == "config.json"
            || filename == "model.safetensors.index.json"
            || filename.ends_with(".safetensors")
            || is_documentation_file(&filename)
        {
            continue;
        }
        let path = entry.path();
        let metadata = entry.metadata().map_err(|source| SubstrateError::Io {
            path: path.clone(),
            source,
        })?;
        files.push(AuxiliaryFileManifest {
            path: filename,
            size_bytes: metadata.len(),
            sha256: hash_file(&path)?,
        });
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

fn tokenizer_hash(
    auxiliary_files: &[AuxiliaryFileManifest],
    root: &Path,
) -> Result<Hash256, SubstrateError> {
    tokenizer_hash_from_records(auxiliary_files)
        .ok_or_else(|| SubstrateError::MissingTokenizer(root.to_path_buf()))
}

fn tokenizer_hash_from_records(auxiliary_files: &[AuxiliaryFileManifest]) -> Option<Hash256> {
    let tokenizer_files: Vec<&AuxiliaryFileManifest> = auxiliary_files
        .iter()
        .filter(|file| is_tokenizer_file(&file.path))
        .collect();
    if tokenizer_files.is_empty() {
        return None;
    }
    let mut hasher = Sha256::new();
    for file in tokenizer_files {
        hasher.update((file.path.len() as u64).to_le_bytes());
        hasher.update(file.path.as_bytes());
        hasher.update(file.size_bytes.to_le_bytes());
        hasher.update(file.sha256.as_str().as_bytes());
    }
    Some(format_digest(hasher.finalize()))
}

fn is_tokenizer_file(filename: &str) -> bool {
    matches!(
        filename,
        "tokenizer.json"
            | "tokenizer_config.json"
            | "tokenizer.model"
            | "sentencepiece.bpe.model"
            | "spiece.model"
            | "vocab.json"
            | "merges.txt"
            | "chat_template.jinja"
            | "special_tokens_map.json"
            | "added_tokens.json"
    )
}

fn is_documentation_file(filename: &str) -> bool {
    let lowercase = filename.to_ascii_lowercase();
    filename.starts_with('.')
        || lowercase.starts_with("readme")
        || lowercase.starts_with("license")
        || lowercase.starts_with("notice")
}

fn hash_file(path: &Path) -> Result<Hash256, SubstrateError> {
    let mut file = File::open(path).map_err(|source| SubstrateError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 16 * 1024 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|source| SubstrateError::Io {
                path: path.to_path_buf(),
                source,
            })?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format_digest(hasher.finalize()))
}

fn sha256_bytes(bytes: &[u8]) -> Hash256 {
    format_digest(Sha256::digest(bytes))
}

fn format_digest(digest: impl AsRef<[u8]>) -> Hash256 {
    let value: String = digest
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Hash256(value)
}

#[derive(Debug, Deserialize)]
struct SafetensorsIndex {
    weight_map: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SafetensorsHeader {
    pub(crate) dtype: String,
    pub(crate) shape: Vec<u64>,
    pub(crate) data_offsets: [u64; 2],
}

pub(crate) fn read_safetensors_headers(
    path: &Path,
) -> Result<BTreeMap<String, SafetensorsHeader>, SubstrateError> {
    let mut file = File::open(path).map_err(|source| SubstrateError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut length = [0_u8; 8];
    file.read_exact(&mut length)
        .map_err(|source| SubstrateError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    let header_length = u64::from_le_bytes(length);
    let header_length = usize::try_from(header_length)
        .map_err(|_| SubstrateError::HeaderTooLarge(path.to_path_buf()))?;
    if header_length > 64 * 1024 * 1024 {
        return Err(SubstrateError::HeaderTooLarge(path.to_path_buf()));
    }
    file.seek(SeekFrom::Start(8))
        .map_err(|source| SubstrateError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    let mut header = vec![0_u8; header_length];
    file.read_exact(&mut header)
        .map_err(|source| SubstrateError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    let values: BTreeMap<String, Value> =
        serde_json::from_slice(&header).map_err(|source| SubstrateError::InvalidJson {
            path: path.to_path_buf(),
            source,
        })?;
    let mut tensors = BTreeMap::new();
    for (name, value) in values {
        if name == "__metadata__" {
            continue;
        }
        let Some(object) = value.as_object() else {
            return Err(SubstrateError::InvalidTensorMetadata {
                path: path.to_path_buf(),
                tensor: name,
            });
        };
        let Some(dtype) = object.get("dtype").and_then(Value::as_str) else {
            return Err(SubstrateError::InvalidTensorMetadata {
                path: path.to_path_buf(),
                tensor: name,
            });
        };
        let Some(shape) = object.get("shape").and_then(Value::as_array) else {
            return Err(SubstrateError::InvalidTensorMetadata {
                path: path.to_path_buf(),
                tensor: name,
            });
        };
        let Some(offsets) = object.get("data_offsets").and_then(Value::as_array) else {
            return Err(SubstrateError::InvalidTensorMetadata {
                path: path.to_path_buf(),
                tensor: name,
            });
        };
        let [start, end] = offsets.as_slice() else {
            return Err(SubstrateError::InvalidTensorMetadata {
                path: path.to_path_buf(),
                tensor: name,
            });
        };
        let (Some(start), Some(end)) = (start.as_u64(), end.as_u64()) else {
            return Err(SubstrateError::InvalidTensorMetadata {
                path: path.to_path_buf(),
                tensor: name,
            });
        };
        let mut dimensions = Vec::with_capacity(shape.len());
        for dimension in shape {
            let Some(dimension) = dimension.as_u64() else {
                return Err(SubstrateError::InvalidTensorMetadata {
                    path: path.to_path_buf(),
                    tensor: name,
                });
            };
            dimensions.push(dimension);
        }
        let tensor = SafetensorsHeader {
            dtype: dtype.to_string(),
            shape: dimensions,
            data_offsets: [start, end],
        };
        tensors.insert(name, tensor);
    }
    if tensors.is_empty() {
        return Err(SubstrateError::NoTensors(path.to_path_buf()));
    }
    validate_tensor_layout(path, header_length, &tensors)?;
    Ok(tensors)
}

fn validate_tensor_layout(
    path: &Path,
    header_length: usize,
    tensors: &BTreeMap<String, SafetensorsHeader>,
) -> Result<(), SubstrateError> {
    let file_size = fs::metadata(path)
        .map_err(|source| SubstrateError::Io {
            path: path.to_path_buf(),
            source,
        })?
        .len();
    let data_start = 8_u64
        .checked_add(
            u64::try_from(header_length)
                .map_err(|_| SubstrateError::HeaderTooLarge(path.to_path_buf()))?,
        )
        .ok_or_else(|| SubstrateError::HeaderTooLarge(path.to_path_buf()))?;
    let payload_size = file_size
        .checked_sub(data_start)
        .ok_or_else(|| SubstrateError::InvalidTensorLayout(path.to_path_buf()))?;
    let mut ranges: Vec<(&str, &SafetensorsHeader)> = tensors
        .iter()
        .map(|(name, tensor)| (name.as_str(), tensor))
        .collect();
    ranges.sort_by_key(|(_, tensor)| tensor.data_offsets[0]);

    let mut cursor = 0_u64;
    for (name, tensor) in ranges {
        let [start, end] = tensor.data_offsets;
        if start != cursor || end < start {
            return Err(SubstrateError::InvalidTensorLayout(path.to_path_buf()));
        }
        let parameter_count = tensor
            .shape
            .iter()
            .try_fold(1_u64, |total, dimension| total.checked_mul(*dimension))
            .ok_or_else(|| SubstrateError::ParameterOverflow(name.to_string()))?;
        let expected_bytes = parameter_count
            .checked_mul(dtype_size_bytes(&tensor.dtype).ok_or_else(|| {
                SubstrateError::UnsupportedSafetensorsDtype {
                    tensor: name.to_string(),
                    dtype: tensor.dtype.clone(),
                }
            })?)
            .ok_or_else(|| SubstrateError::ParameterOverflow(name.to_string()))?;
        if end - start != expected_bytes {
            return Err(SubstrateError::InvalidTensorLayout(path.to_path_buf()));
        }
        cursor = end;
    }
    if cursor != payload_size {
        return Err(SubstrateError::InvalidTensorLayout(path.to_path_buf()));
    }
    Ok(())
}

pub(crate) fn dtype_size_bytes(dtype: &str) -> Option<u64> {
    match dtype {
        "F64" | "I64" | "U64" => Some(8),
        "F32" | "I32" | "U32" => Some(4),
        "F16" | "BF16" | "I16" | "U16" => Some(2),
        "BOOL" | "I8" | "U8" | "F8_E4M3" | "F8_E4M3FN" | "F8_E5M2" | "F8_E5M2FN" | "F8_E8M0" => {
            Some(1)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn write_shard(path: &Path, tensor: &str, value: &[u8]) {
        let header = format!(
            "{{\"{tensor}\":{{\"dtype\":\"U8\",\"shape\":[{}],\"data_offsets\":[0,{}]}}}}",
            value.len(),
            value.len()
        );
        let mut bytes = (header.len() as u64).to_le_bytes().to_vec();
        bytes.extend(header.as_bytes());
        bytes.extend(value);
        fs::write(path, bytes).expect("write shard");
    }

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(
            dir.path().join("config.json"),
            r#"{"model_type":"qwen3_5","architectures":["Qwen3_5ForConditionalGeneration"]}"#,
        )
        .expect("config");
        fs::write(dir.path().join("tokenizer.json"), "tokenizer").expect("tokenizer");
        fs::write(
            dir.path().join("model.safetensors.index.json"),
            r#"{"weight_map":{"layer.weight":"model-00001-of-00001.safetensors"}}"#,
        )
        .expect("index");
        write_shard(
            &dir.path().join("model-00001-of-00001.safetensors"),
            "layer.weight",
            &[1, 2, 3, 4],
        );
        dir
    }

    fn hash(character: char) -> Hash256 {
        Hash256::parse(character.to_string().repeat(64)).expect("valid test hash")
    }

    fn admission() -> ArtifactAdmission {
        ArtifactAdmission {
            backend: "llama.cpp".into(),
            backend_version: "test".into(),
            checked_at: "2026-08-25T00:00:00Z".into(),
        }
    }

    #[test]
    fn manifest_is_stable_and_verifiable() {
        let dir = fixture();
        let first = import_safetensors(dir.path(), "qwen38-27b").expect("import");
        let second = import_safetensors(dir.path(), "qwen38-27b").expect("import");
        assert_eq!(first, second);
        verify_safetensors(dir.path(), &first).expect("verify");
        assert_eq!(first.tensor_count, 1);
        assert_eq!(first.parameter_count, 4);
    }

    #[test]
    fn import_progress_reports_completed_shard_bytes() {
        let dir = fixture();
        let mut updates = Vec::new();
        import_safetensors_with_progress(dir.path(), "qwen38-27b", |progress| {
            updates.push(progress.clone());
        })
        .expect("import");
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].shards_completed, 1);
        assert_eq!(updates[0].shards_total, 1);
        assert_eq!(updates[0].bytes_hashed, updates[0].total_bytes);
        assert_eq!(updates[0].current_shard, "model-00001-of-00001.safetensors");
    }

    #[test]
    fn changed_shard_changes_manifest() {
        let dir = fixture();
        let first = import_safetensors(dir.path(), "qwen38-27b").expect("import");
        write_shard(
            &dir.path().join("model-00001-of-00001.safetensors"),
            "layer.weight",
            &[4, 3, 2, 1],
        );
        let second = import_safetensors(dir.path(), "qwen38-27b").expect("import");
        assert_ne!(
            first.model.canonical_manifest_hash,
            second.model.canonical_manifest_hash
        );
    }

    #[test]
    fn stale_extra_shard_is_rejected() {
        let dir = fixture();
        write_shard(&dir.path().join("stale.safetensors"), "old.weight", &[1]);
        let error = import_safetensors(dir.path(), "qwen38-27b").expect_err("mixed set");
        assert!(matches!(error, SubstrateError::MixedShardSet { .. }));
    }

    #[test]
    fn missing_referenced_shard_is_rejected() {
        let dir = fixture();
        fs::remove_file(dir.path().join("model-00001-of-00001.safetensors")).expect("remove shard");
        assert!(matches!(
            import_safetensors(dir.path(), "qwen38-27b"),
            Err(SubstrateError::MissingShards(_))
        ));
    }

    #[test]
    fn index_entries_must_all_exist_in_shard_headers() {
        let dir = fixture();
        fs::write(
            dir.path().join("model.safetensors.index.json"),
            r#"{"weight_map":{"layer.weight":"model-00001-of-00001.safetensors","ghost.weight":"model-00001-of-00001.safetensors"}}"#,
        )
        .expect("index");
        assert!(matches!(
            import_safetensors(dir.path(), "qwen38-27b"),
            Err(SubstrateError::IndexTensorSetMismatch { .. })
        ));
    }

    #[test]
    fn multiple_unindexed_shards_are_rejected() {
        let dir = fixture();
        fs::remove_file(dir.path().join("model.safetensors.index.json")).expect("remove index");
        write_shard(
            &dir.path().join("model-00002-of-00002.safetensors"),
            "second.weight",
            &[5, 6],
        );
        assert!(matches!(
            import_safetensors(dir.path(), "qwen38-27b"),
            Err(SubstrateError::InvalidIndex(_))
        ));
    }

    #[test]
    fn truncated_safetensors_payload_is_rejected() {
        let dir = fixture();
        let path = dir.path().join("model-00001-of-00001.safetensors");
        let file = File::options().write(true).open(&path).expect("open shard");
        let length = file.metadata().expect("metadata").len();
        file.set_len(length - 1).expect("truncate shard");
        assert!(matches!(
            import_safetensors(dir.path(), "qwen38-27b"),
            Err(SubstrateError::InvalidTensorLayout(_))
        ));
    }

    #[test]
    fn processor_assets_are_part_of_canonical_identity() {
        let dir = fixture();
        fs::write(dir.path().join("preprocessor_config.json"), "first").expect("processor");
        let first = import_safetensors(dir.path(), "qwen38-27b").expect("first import");
        fs::write(dir.path().join("preprocessor_config.json"), "second").expect("processor");
        let second = import_safetensors(dir.path(), "qwen38-27b").expect("second import");
        assert_ne!(
            first.model.canonical_manifest_hash,
            second.model.canonical_manifest_hash
        );
        assert_eq!(second.auxiliary_files.len(), 2);
    }

    #[test]
    fn successor_manifest_records_and_verifies_explicit_parent() {
        let dir = fixture();
        let manifest = import_successor_safetensors(dir.path(), "qwen38-successor-1", "qwen38-27b")
            .expect("successor import");
        assert_eq!(manifest.model.parent.as_deref(), Some("qwen38-27b"));
        verify_safetensors(dir.path(), &manifest).expect("successor verify");
    }

    #[test]
    fn model_and_artifact_identities_are_distinct() {
        let model = ModelIdentity {
            lineage_id: "qwen38-27b".into(),
            model_name: "Qwen3.8-27B".into(),
            parent: None,
            canonical_manifest_hash: hash('a'),
        };
        let artifact = ArtifactIdentity {
            artifact_id: "qwen38-27b-q6-k".into(),
            model_lineage_id: model.lineage_id.clone(),
            representation: ModelRepresentation::Gguf,
            quantization: Some(QuantizationKind::Q6_K),
            artifact_hash: hash('b'),
        };
        assert_ne!(model.lineage_id, artifact.artifact_id);
        assert_eq!(model.lineage_id, artifact.model_lineage_id);
    }

    #[test]
    fn llama_cpp_tap_provider_can_return_explicit_unsupported() {
        let provider = LlamaCppActivationProvider;
        assert_eq!(
            provider.capabilities(),
            &[RuntimeCapability::TextGeneration]
        );
        assert_eq!(
            provider
                .observe(ActivationObservationRequest {
                    input_ids: vec![1],
                    taps: vec!["tap-1".into()],
                })
                .expect_err("llama.cpp must not fabricate taps"),
            ActivationObservationError::Unsupported
        );
    }

    #[test]
    fn artifact_manifest_store_is_append_only() {
        let dir = tempfile::tempdir().expect("tempdir");
        let manifest = ArtifactManifest {
            schema_version: SCHEMA_VERSION,
            artifact: ArtifactIdentity {
                artifact_id: "qwen38-q8".into(),
                model_lineage_id: "qwen38-27b".into(),
                representation: ModelRepresentation::Gguf,
                quantization: Some(QuantizationKind::Q8_0),
                artifact_hash: hash('c'),
            },
            parent_artifact: None,
            canonical_model: "qwen38-27b".into(),
            tool: "test".into(),
            tool_version: "1".into(),
            command_or_parameters: vec!["q8_0".into()],
            source_hashes: vec![hash('d')],
            output_hash: hash('c'),
            artifact_path: dir.path().join("qwen38-q8.gguf"),
            admission: Some(admission()),
            created_at: "2026-08-24T00:00:00Z".into(),
        };
        let first = append_artifact_manifest(dir.path(), &manifest).expect("append");
        let second = append_artifact_manifest(dir.path(), &manifest).expect("idempotent append");
        assert_eq!(first, second);

        let mut changed = manifest;
        changed.tool_version = "2".into();
        assert!(matches!(
            append_artifact_manifest(dir.path(), &changed),
            Err(SubstrateError::ArtifactConflict(_))
        ));
    }

    #[test]
    fn artifact_manifest_parser_rejects_invalid_timestamp() {
        let manifest = ArtifactManifest {
            schema_version: SCHEMA_VERSION,
            artifact: ArtifactIdentity {
                artifact_id: "qwen38-q8".into(),
                model_lineage_id: "qwen38-27b".into(),
                representation: ModelRepresentation::Gguf,
                quantization: Some(QuantizationKind::Q8_0),
                artifact_hash: hash('c'),
            },
            parent_artifact: None,
            canonical_model: "qwen38-27b".into(),
            tool: "test".into(),
            tool_version: "1".into(),
            command_or_parameters: vec!["q8_0".into()],
            source_hashes: vec![hash('d')],
            output_hash: hash('c'),
            artifact_path: PathBuf::from("/tmp/qwen38-q8.gguf"),
            admission: Some(admission()),
            created_at: "not-a-timestamp".into(),
        };
        let payload = serde_json::to_string(&manifest).expect("serialize");
        assert!(matches!(
            parse_artifact_manifest_json(&payload),
            Err(SubstrateError::InvalidArtifactManifest(_))
        ));
    }

    #[test]
    fn pending_gguf_candidate_is_append_only_and_not_an_artifact() {
        let dir = tempfile::tempdir().expect("tempdir");
        let candidate = GgufCandidateManifest {
            schema_version: SCHEMA_VERSION,
            artifact: ArtifactIdentity {
                artifact_id: "qwen38-q8-pending".into(),
                model_lineage_id: "qwen38-27b".into(),
                representation: ModelRepresentation::Gguf,
                quantization: Some(QuantizationKind::Q8_0),
                artifact_hash: hash('c'),
            },
            parent_artifact: "qwen38-27b-safetensors".into(),
            canonical_model: "qwen38-27b".into(),
            tool: "llama.cpp".into(),
            tool_version: "test-revision".into(),
            command_or_parameters: vec!["Q8_0".into()],
            source_hashes: vec![hash('d')],
            output_hash: hash('c'),
            artifact_path: dir.path().join("qwen38-q8.gguf"),
            created_at: "2026-08-25T00:00:00Z".into(),
        };
        let first = append_gguf_candidate_manifest(dir.path(), &candidate).expect("append");
        let second = append_gguf_candidate_manifest(dir.path(), &candidate).expect("idempotent");
        assert_eq!(first, second);
        assert!(parse_artifact_manifest_json(
            &serde_json::to_string(&candidate).expect("serialize candidate")
        )
        .is_err());

        let mut changed = candidate;
        changed.tool_version = "different-revision".into();
        assert!(matches!(
            append_gguf_candidate_manifest(dir.path(), &changed),
            Err(SubstrateError::CandidateConflict(_))
        ));
    }

    #[test]
    fn canonical_manifest_store_rejects_lineage_replacement() {
        let source = fixture();
        let store = tempfile::tempdir().expect("store");
        let path = store.path().join("qwen38-27b.json");
        let first = import_safetensors(source.path(), "qwen38-27b").expect("first import");
        store_substrate_manifest(&path, &first).expect("first registration");
        store_substrate_manifest(&path, &first).expect("idempotent registration");

        write_shard(
            &source.path().join("model-00001-of-00001.safetensors"),
            "layer.weight",
            &[4, 3, 2, 1],
        );
        let changed = import_safetensors(source.path(), "qwen38-27b").expect("changed import");
        assert!(matches!(
            store_substrate_manifest(&path, &changed),
            Err(SubstrateError::ManifestConflict(_))
        ));
    }

    #[test]
    fn stale_checkpoint_cannot_replace_registered_qwen38_lineage() {
        let source = fixture();
        let store = tempfile::tempdir().expect("store");
        let path = store.path().join("qwen38-27b.json");
        let canonical = import_safetensors(source.path(), "qwen38-27b").expect("canonical");
        store_substrate_manifest(&path, &canonical).expect("register canonical");

        fs::write(
            source.path().join("config.json"),
            r#"{"model_type":"qwen3","hidden_size":4096,"num_hidden_layers":36}"#,
        )
        .expect("stale config");
        let stale = import_safetensors(source.path(), "qwen38-27b").expect("stale import");
        assert!(matches!(
            store_substrate_manifest(&path, &stale),
            Err(SubstrateError::ManifestConflict(_))
        ));
    }

    #[test]
    fn derivative_and_runtime_lease_retain_exact_lineage_hashes() {
        let canonical_hash = hash('a');
        let artifact_hash = hash('b');
        let artifact = ArtifactManifest {
            schema_version: SCHEMA_VERSION,
            artifact: ArtifactIdentity {
                artifact_id: "qwen38-q6-k".into(),
                model_lineage_id: "qwen38-27b".into(),
                representation: ModelRepresentation::Gguf,
                quantization: Some(QuantizationKind::Q6_K),
                artifact_hash: artifact_hash.clone(),
            },
            parent_artifact: Some("qwen38-bf16".into()),
            canonical_model: "qwen38-27b".into(),
            tool: "llama-quantize".into(),
            tool_version: "test".into(),
            command_or_parameters: vec!["Q6_K".into()],
            source_hashes: vec![canonical_hash.clone()],
            output_hash: artifact_hash.clone(),
            artifact_path: PathBuf::from("/tmp/qwen38-q6-k.gguf"),
            admission: Some(admission()),
            created_at: "2026-08-25T00:00:00Z".into(),
        };
        storage::validate_artifact_manifest(&artifact).expect("valid derivative");

        let lease = ModelLease {
            lease_id: "lease-1".into(),
            model_lineage_id: artifact.canonical_model.clone(),
            artifact_id: artifact.artifact.artifact_id.clone(),
            runtime_id: "llama-server-1".into(),
            endpoint: "http://127.0.0.1:1200".into(),
            manifest_hash: canonical_hash.clone(),
            artifact_hash: artifact_hash.clone(),
            capabilities: vec![RuntimeCapability::TextGeneration],
        };
        assert_eq!(lease.manifest_hash, canonical_hash);
        assert_eq!(lease.artifact_hash, artifact_hash);
        assert_eq!(lease.model_lineage_id, "qwen38-27b");
    }

    #[test]
    fn artifact_manifest_rejects_contradictory_hashes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let manifest = ArtifactManifest {
            schema_version: SCHEMA_VERSION,
            artifact: ArtifactIdentity {
                artifact_id: "qwen38-q8".into(),
                model_lineage_id: "qwen38-27b".into(),
                representation: ModelRepresentation::Gguf,
                quantization: Some(QuantizationKind::Q8_0),
                artifact_hash: hash('a'),
            },
            parent_artifact: Some("qwen38-bf16".into()),
            canonical_model: "qwen38-27b".into(),
            tool: "test".into(),
            tool_version: "1".into(),
            command_or_parameters: vec!["q8_0".into()],
            source_hashes: vec![hash('b')],
            output_hash: hash('c'),
            artifact_path: dir.path().join("qwen38-q8.gguf"),
            admission: Some(admission()),
            created_at: "2026-08-24T00:00:00Z".into(),
        };
        assert!(matches!(
            append_artifact_manifest(dir.path(), &manifest),
            Err(SubstrateError::InvalidArtifactManifest(_))
        ));
    }

    #[test]
    fn manifest_parser_rejects_unknown_schema_and_invalid_hashes() {
        let dir = fixture();
        let manifest = import_safetensors(dir.path(), "qwen38-27b").expect("manifest");
        let mut value = serde_json::to_value(&manifest).expect("value");
        value["schema_version"] = serde_json::json!(SCHEMA_VERSION + 1);
        assert!(matches!(
            parse_manifest_json(&value.to_string()),
            Err(SubstrateError::UnsupportedSchemaVersion { .. })
        ));

        value["schema_version"] = serde_json::json!(SCHEMA_VERSION);
        value["model"]["canonical_manifest_hash"] = serde_json::json!("not-a-hash");
        assert!(matches!(
            parse_manifest_json(&value.to_string()),
            Err(SubstrateError::ManifestJson(_))
        ));
    }

    #[test]
    fn manifest_parser_rejects_inconsistent_inventory() {
        let dir = fixture();
        let manifest = import_safetensors(dir.path(), "qwen38-27b").expect("manifest");
        let mut value = serde_json::to_value(&manifest).expect("value");
        value["tensor_count"] = serde_json::json!(2);
        assert!(matches!(
            parse_manifest_json(&value.to_string()),
            Err(SubstrateError::InvalidSubstrateManifest(_))
        ));
    }

    #[test]
    fn gguf_validation_rejects_short_and_malformed_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let short = dir.path().join("short.gguf");
        fs::write(&short, b"GGUF").expect("short file");
        assert!(matches!(
            validate_gguf(&short),
            Err(SubstrateError::Io { .. })
        ));

        let malformed = dir.path().join("malformed.gguf");
        fs::write(
            &malformed,
            [
                b'N', b'O', b'P', b'E', 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            ],
        )
        .expect("malformed file");
        assert!(matches!(
            validate_gguf(&malformed),
            Err(SubstrateError::InvalidGguf(_))
        ));
    }
}
