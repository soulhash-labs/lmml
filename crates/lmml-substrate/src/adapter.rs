//! LoRA adapter tensor inventory and finite-value validation.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::{dtype_size_bytes, read_safetensors_headers, SubstrateError, TensorDescriptor};

/// Validate a Safetensors payload and reject NaN or infinity in its tensors.
///
/// Integer tensors are structurally checked and treated as finite. Floating
/// formats supported by successor adapters are scanned without loading the
/// complete file into memory.
pub fn validate_finite_safetensors(path: impl AsRef<Path>) -> Result<(), SubstrateError> {
    let path = path.as_ref();
    let tensors = read_safetensors_headers(path)?;
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
    let data_start = 8_u64
        .checked_add(u64::from_le_bytes(length))
        .ok_or_else(|| SubstrateError::HeaderTooLarge(path.to_path_buf()))?;
    let mut buffer = vec![0_u8; 64 * 1024];
    for (name, tensor) in tensors {
        let element_size = dtype_size_bytes(&tensor.dtype).ok_or_else(|| {
            SubstrateError::UnsupportedSafetensorsDtype {
                tensor: name.clone(),
                dtype: tensor.dtype.clone(),
            }
        })? as usize;
        file.seek(SeekFrom::Start(data_start + tensor.data_offsets[0]))
            .map_err(|source| SubstrateError::Io {
                path: path.to_path_buf(),
                source,
            })?;
        let mut remaining = tensor.data_offsets[1] - tensor.data_offsets[0];
        while remaining > 0 {
            let count = usize::try_from(remaining.min(buffer.len() as u64))
                .map_err(|_| SubstrateError::InvalidTensorLayout(path.to_path_buf()))?;
            file.read_exact(&mut buffer[..count])
                .map_err(|source| SubstrateError::Io {
                    path: path.to_path_buf(),
                    source,
                })?;
            if count % element_size != 0 || contains_non_finite(&tensor.dtype, &buffer[..count])? {
                return Err(SubstrateError::NonFiniteTrainingState(format!(
                    "tensor {name}"
                )));
            }
            remaining -= count as u64;
        }
    }
    Ok(())
}

/// Validate every weight shard in a canonical manifest for finite values.
///
/// Successor admission uses this after structural verification so a merged
/// checkpoint containing NaN or infinity cannot become a canonical lineage.
pub fn validate_finite_checkpoint(
    root: impl AsRef<Path>,
    manifest: &crate::SubstrateManifest,
) -> Result<(), SubstrateError> {
    let root = root.as_ref();
    for shard in &manifest.shards {
        validate_finite_safetensors(root.join(&shard.path))?;
    }
    Ok(())
}

/// Inspect one Safetensors file and return its deterministic tensor inventory.
pub fn inspect_safetensors_file(
    path: impl AsRef<Path>,
) -> Result<Vec<TensorDescriptor>, SubstrateError> {
    let path = path.as_ref();
    let shard = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("adapter_model.safetensors")
        .to_string();
    read_safetensors_headers(path)?
        .into_iter()
        .map(|(name, header)| {
            let parameter_count = header
                .shape
                .iter()
                .try_fold(1_u64, |count, dimension| count.checked_mul(*dimension));
            Ok(TensorDescriptor {
                name,
                shard: shard.clone(),
                dtype: header.dtype,
                shape: header.shape,
                parameter_count: parameter_count.ok_or_else(|| {
                    SubstrateError::ParameterOverflow("<adapter tensor>".to_string())
                })?,
            })
        })
        .collect()
}

/// Validate that an adapter contains only the exact expected LoRA tensors.
pub fn validate_adapter_safetensors(
    path: impl AsRef<Path>,
    expected: &[TensorDescriptor],
) -> Result<(), SubstrateError> {
    let path = path.as_ref();
    let discovered = inspect_safetensors_file(path)?;
    if discovered != expected {
        return Err(SubstrateError::InvalidLifecycleManifest(
            "adapter tensor names, dtypes, or shapes do not match the training report".to_string(),
        ));
    }
    if discovered.is_empty()
        || discovered.iter().any(|tensor| {
            !matches!(tensor.dtype.as_str(), "F16" | "BF16" | "F32")
                || !is_lora_tensor_name(&tensor.name)
        })
    {
        return Err(SubstrateError::InvalidLifecycleManifest(
            "adapter contains a non-LoRA or unsupported tensor".to_string(),
        ));
    }
    validate_finite_safetensors(path)
}

fn is_lora_tensor_name(name: &str) -> bool {
    [
        ".lora_A.",
        ".lora_B.",
        ".lora_embedding_A.",
        ".lora_embedding_B.",
    ]
    .iter()
    .any(|marker| name.contains(marker))
}

fn contains_non_finite(dtype: &str, bytes: &[u8]) -> Result<bool, SubstrateError> {
    let non_finite = match dtype {
        "F16" => bytes.chunks_exact(2).any(|value| {
            let bits = u16::from_le_bytes([value[0], value[1]]);
            bits & 0x7c00 == 0x7c00
        }),
        "BF16" => bytes.chunks_exact(2).any(|value| {
            let bits = u16::from_le_bytes([value[0], value[1]]);
            bits & 0x7f80 == 0x7f80
        }),
        "F32" => bytes
            .chunks_exact(4)
            .any(|value| !f32::from_le_bytes([value[0], value[1], value[2], value[3]]).is_finite()),
        "F64" => bytes.chunks_exact(8).any(|value| {
            !f64::from_le_bytes([
                value[0], value[1], value[2], value[3], value[4], value[5], value[6], value[7],
            ])
            .is_finite()
        }),
        "BOOL" | "I8" | "U8" | "I16" | "U16" | "I32" | "U32" | "I64" | "U64" => false,
        unsupported => {
            return Err(SubstrateError::UnsupportedSafetensorsDtype {
                tensor: "<finite scan>".to_string(),
                dtype: unsupported.to_string(),
            });
        }
    };
    Ok(non_finite)
}
