//! GGUF-driven llama.cpp runtime selection.
//!
//! Selection happens before process management so deterministic format errors
//! cannot stop a healthy server or degrade into a readiness timeout.

use std::path::{Path, PathBuf};

use super::RuntimeCliError;

/// Runtime flavor and binary selected from a model's GGUF contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedModelRuntime {
    /// Runtime implementation required for the model.
    pub flavor: lmml_compat::LlamaRuntimeFlavor,
    /// Exact llama-server binary to launch.
    pub binary: PathBuf,
}

/// Inspect a GGUF and resolve the trusted compatible runtime before launch.
pub async fn resolve_model_runtime(
    build: &lmml_state::BuildState,
    model: &Path,
) -> Result<ResolvedModelRuntime, RuntimeCliError> {
    let metadata = lmml_models::parse_gguf_metadata(model)
        .await
        .map_err(|source| RuntimeCliError::GgufInspection {
            path: model.to_path_buf(),
            source,
        })?;
    let required = match metadata.runtime {
        lmml_models::GgufRuntimeRequirement::Upstream => lmml_compat::LlamaRuntimeFlavor::Upstream,
        lmml_models::GgufRuntimeRequirement::Prism => lmml_compat::LlamaRuntimeFlavor::Prism,
        lmml_models::GgufRuntimeRequirement::Unsupported { tensor_types } => {
            return Err(RuntimeCliError::UnsupportedGguf {
                path: model.to_path_buf(),
                tensor_types,
            });
        }
    };
    let selected = match build.runtime_selection {
        lmml_state::RuntimeSelectionMode::Auto => required,
        lmml_state::RuntimeSelectionMode::Upstream => lmml_compat::LlamaRuntimeFlavor::Upstream,
        lmml_state::RuntimeSelectionMode::Prism => lmml_compat::LlamaRuntimeFlavor::Prism,
    };
    if required == lmml_compat::LlamaRuntimeFlavor::Prism
        && selected != lmml_compat::LlamaRuntimeFlavor::Prism
    {
        return Err(RuntimeCliError::RuntimeOverrideIncompatible {
            path: model.to_path_buf(),
            required,
            selected,
        });
    }
    let binary = build.runtime_binary(selected).to_path_buf();
    if !binary.is_file() {
        return Err(RuntimeCliError::RuntimeUnavailable {
            flavor: selected,
            path: binary,
        });
    }
    Ok(ResolvedModelRuntime {
        flavor: selected,
        binary,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[tokio::test]
    async fn selects_binary_from_gguf_contents() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let upstream_model = tempdir.path().join("ordinary.gguf");
        let prism_model = tempdir.path().join("ternary.gguf");
        let upstream_binary = tempdir.path().join("upstream/llama-server");
        let prism_binary = tempdir.path().join("prism/llama-server");
        fs::create_dir_all(upstream_binary.parent().expect("upstream parent"))
            .expect("upstream directory");
        fs::create_dir_all(prism_binary.parent().expect("Prism parent")).expect("Prism directory");
        fs::write(&upstream_binary, b"binary").expect("upstream binary");
        fs::write(&prism_binary, b"binary").expect("Prism binary");
        fs::write(&upstream_model, fixture_gguf(12)).expect("upstream model");
        fs::write(&prism_model, fixture_gguf(142)).expect("Prism model");
        let mut build = lmml_state::BuildState::default();
        build.binary = upstream_binary.clone();
        build.prism.binary = prism_binary.clone();

        assert_eq!(
            resolve_model_runtime(&build, &upstream_model)
                .await
                .expect("upstream runtime"),
            ResolvedModelRuntime {
                flavor: lmml_compat::LlamaRuntimeFlavor::Upstream,
                binary: upstream_binary,
            }
        );
        assert_eq!(
            resolve_model_runtime(&build, &prism_model)
                .await
                .expect("Prism runtime"),
            ResolvedModelRuntime {
                flavor: lmml_compat::LlamaRuntimeFlavor::Prism,
                binary: prism_binary,
            }
        );
    }

    #[tokio::test]
    async fn rejects_incompatible_override_and_unknown_types() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let prism_model = tempdir.path().join("ternary.gguf");
        let unknown_model = tempdir.path().join("unknown.gguf");
        fs::write(&prism_model, fixture_gguf(142)).expect("Prism model");
        fs::write(&unknown_model, fixture_gguf(144)).expect("unknown model");
        let mut build = lmml_state::BuildState {
            runtime_selection: lmml_state::RuntimeSelectionMode::Upstream,
            ..lmml_state::BuildState::default()
        };

        assert!(matches!(
            resolve_model_runtime(&build, &prism_model).await,
            Err(RuntimeCliError::RuntimeOverrideIncompatible {
                required: lmml_compat::LlamaRuntimeFlavor::Prism,
                selected: lmml_compat::LlamaRuntimeFlavor::Upstream,
                ..
            })
        ));
        build.runtime_selection = lmml_state::RuntimeSelectionMode::Auto;
        assert!(matches!(
            resolve_model_runtime(&build, &unknown_model).await,
            Err(RuntimeCliError::UnsupportedGguf { ref tensor_types, .. })
                if tensor_types == &std::collections::BTreeSet::from([144])
        ));
    }

    #[tokio::test]
    async fn reports_missing_prism_binary_before_spawn() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let prism_model = tempdir.path().join("ternary.gguf");
        fs::write(&prism_model, fixture_gguf(142)).expect("Prism model");

        assert!(matches!(
            resolve_model_runtime(&lmml_state::BuildState::default(), &prism_model).await,
            Err(RuntimeCliError::RuntimeUnavailable {
                flavor: lmml_compat::LlamaRuntimeFlavor::Prism,
                ..
            })
        ));
    }

    fn fixture_gguf(tensor_type: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"GGUF");
        bytes.extend_from_slice(&3_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u64.to_le_bytes());
        bytes.extend_from_slice(&1_u64.to_le_bytes());
        write_string(&mut bytes, "general.architecture");
        bytes.extend_from_slice(&8_u32.to_le_bytes());
        write_string(&mut bytes, "llama");
        write_string(&mut bytes, "blk.0.weight");
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u64.to_le_bytes());
        bytes.extend_from_slice(&tensor_type.to_le_bytes());
        bytes.extend_from_slice(&0_u64.to_le_bytes());
        bytes
    }

    fn write_string(bytes: &mut Vec<u8>, value: &str) {
        bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
        bytes.extend_from_slice(value.as_bytes());
    }
}
