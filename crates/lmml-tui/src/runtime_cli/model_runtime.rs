//! GGUF-driven llama.cpp runtime selection.
//!
//! Selection happens before process management so deterministic format errors
//! cannot stop a healthy server or degrade into a readiness timeout.

use std::future::Future;
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

trait AcceleratorTargetProbe {
    fn rocm_targets(
        &self,
    ) -> impl Future<Output = Result<Vec<String>, lmml_detect::AcceleratorTargetProbeError>> + Send;

    fn cuda_archs(
        &self,
    ) -> impl Future<Output = Result<Vec<String>, lmml_detect::AcceleratorTargetProbeError>> + Send;
}

#[derive(Debug, Clone, Copy)]
struct LiveAcceleratorTargetProbe;

impl AcceleratorTargetProbe for LiveAcceleratorTargetProbe {
    async fn rocm_targets(&self) -> Result<Vec<String>, lmml_detect::AcceleratorTargetProbeError> {
        lmml_detect::probe_live_rocm_targets().await
    }

    async fn cuda_archs(&self) -> Result<Vec<String>, lmml_detect::AcceleratorTargetProbeError> {
        lmml_detect::probe_live_cuda_archs().await
    }
}

/// Inspect a GGUF and resolve the trusted compatible runtime before launch.
pub async fn resolve_model_runtime(
    build: &lmml_state::BuildState,
    model: &Path,
) -> Result<ResolvedModelRuntime, RuntimeCliError> {
    resolve_model_runtime_with_probe(build, model, &LiveAcceleratorTargetProbe).await
}

async fn resolve_model_runtime_with_probe<P>(
    build: &lmml_state::BuildState,
    model: &Path,
    probe: &P,
) -> Result<ResolvedModelRuntime, RuntimeCliError>
where
    P: AcceleratorTargetProbe + Sync,
{
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
    if selected == lmml_compat::LlamaRuntimeFlavor::Prism {
        verify_prism_attestation(build, probe).await?;
        let prism = &build.prism;
        let expected_library = match prism.backend.as_str() {
            "Rocm" => Some(lmml_build::RuntimeLibraryIdentity {
                kind: lmml_build::AcceleratorLibraryKind::Hip,
                path: &prism.verified_hip_library,
                sha256: &prism.verified_hip_library_sha256,
            }),
            "Cuda" => Some(lmml_build::RuntimeLibraryIdentity {
                kind: lmml_build::AcceleratorLibraryKind::Cuda,
                path: &prism.verified_cuda_library,
                sha256: &prism.verified_cuda_library_sha256,
            }),
            "Auto" | "Metal" | "Vulkan" | "CpuAvx2" | "CpuAvx" | "CpuFallback" => None,
            _ => None,
        };
        lmml_build::verify_runtime_artifacts(
            &binary,
            &prism.verified_server_sha256,
            expected_library,
        )
        .await
        .map_err(|source| RuntimeCliError::PrismArtifactVerification {
            path: binary.clone(),
            source,
        })?;
    }
    Ok(ResolvedModelRuntime {
        flavor: selected,
        binary,
    })
}

async fn verify_prism_attestation<P>(
    build: &lmml_state::BuildState,
    probe: &P,
) -> Result<(), RuntimeCliError>
where
    P: AcceleratorTargetProbe + Sync,
{
    let prism = &build.prism;
    let gaps = prism_attestation_gaps(prism);
    if !gaps.is_empty() {
        return Err(RuntimeCliError::PrismVerificationRequired {
            expected_version: lmml_build::RUNTIME_VERIFICATION_VERSION,
            found_version: prism.verification_version,
            missing_evidence: gaps,
        });
    }

    match prism.backend.as_str() {
        "Rocm" => {
            let detected = probe.rocm_targets().await.map_err(|source| {
                RuntimeCliError::PrismAcceleratorProbe {
                    backend: "ROCm",
                    source,
                }
            })?;
            verify_target_match(&detected, &prism.verified_rocm_targets, "ROCm")
        }
        "Cuda" => {
            let detected = probe.cuda_archs().await.map_err(|source| {
                RuntimeCliError::PrismAcceleratorProbe {
                    backend: "CUDA",
                    source,
                }
            })?;
            verify_target_match(&detected, &prism.verified_cuda_targets, "CUDA")
        }
        "Auto" | "Metal" | "Vulkan" | "CpuAvx2" | "CpuAvx" | "CpuFallback" => Ok(()),
        _ => Ok(()),
    }
}

fn verify_target_match(
    detected: &[String],
    verified: &[String],
    backend: &'static str,
) -> Result<(), RuntimeCliError> {
    if detected.iter().all(|target| verified.contains(target)) {
        Ok(())
    } else {
        Err(RuntimeCliError::PrismAcceleratorTargetMismatch {
            backend,
            detected_targets: detected.to_vec(),
            verified_targets: verified.to_vec(),
        })
    }
}

pub(crate) fn prism_attestation_gaps(prism: &lmml_state::RuntimeFlavorBuildState) -> Vec<String> {
    let mut gaps = Vec::new();
    if prism.verification_version != lmml_build::RUNTIME_VERIFICATION_VERSION {
        gaps.push("verification version".to_string());
    }
    for tensor_type in [142_u32, 143_u32] {
        if !prism.verified_prism_tensor_types.contains(&tensor_type) {
            gaps.push(format!("tensor type {tensor_type}"));
        }
    }
    if !is_sha256(&prism.verified_server_sha256) {
        gaps.push("llama-server SHA-256".to_string());
    }
    if prism.backend == "Rocm" {
        if prism.verified_rocm_targets.is_empty() {
            gaps.push("ROCm targets".to_string());
        } else if prism.archs != prism.verified_rocm_targets {
            gaps.push("ROCm build target binding".to_string());
        }
        if prism.verified_hip_library.as_os_str().is_empty() {
            gaps.push("libggml-hip path".to_string());
        }
        if !is_sha256(&prism.verified_hip_library_sha256) {
            gaps.push("libggml-hip SHA-256".to_string());
        }
    }
    if prism.backend == "Cuda" {
        if prism.verified_cuda_targets.is_empty() {
            gaps.push("CUDA targets".to_string());
        } else if prism.archs != prism.verified_cuda_targets {
            gaps.push("CUDA build target binding".to_string());
        }
        if prism.verified_cuda_library.as_os_str().is_empty() {
            gaps.push("libggml-cuda path".to_string());
        }
        if !is_sha256(&prism.verified_cuda_library_sha256) {
            gaps.push("libggml-cuda SHA-256".to_string());
        }
    }
    if !matches!(
        prism.backend.as_str(),
        "Cuda" | "Metal" | "Rocm" | "Vulkan" | "CpuAvx2" | "CpuAvx" | "CpuFallback"
    ) {
        gaps.push("recognized build backend".to_string());
    }
    gaps
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
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
        build.prism.backend = "CpuAvx2".to_string();
        build.prism.verification_version = lmml_build::RUNTIME_VERIFICATION_VERSION;
        build.prism.verified_prism_tensor_types = vec![142, 143];
        build.prism.verified_server_sha256 = file_sha256(&prism_binary);

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
        let mut build = lmml_state::BuildState::default();
        build.prism.binary = tempdir.path().join("missing/llama-server");

        assert!(matches!(
            resolve_model_runtime(&build, &prism_model).await,
            Err(RuntimeCliError::RuntimeUnavailable {
                flavor: lmml_compat::LlamaRuntimeFlavor::Prism,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn requires_verified_prism_rocm_target_before_spawn() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let model = tempdir.path().join("ternary.gguf");
        let binary = tempdir.path().join("prism/llama-server");
        fs::create_dir_all(binary.parent().expect("binary parent")).expect("binary directory");
        fs::write(&binary, b"binary").expect("Prism binary");
        fs::write(&model, fixture_gguf(142)).expect("Prism model");
        let mut build = lmml_state::BuildState::default();
        build.prism.binary = binary.clone();
        build.prism.backend = "Rocm".to_string();
        build.prism.archs = vec!["gfx1201".to_string()];
        build.prism.verification_version = lmml_build::RUNTIME_VERIFICATION_VERSION;
        build.prism.verified_prism_tensor_types = vec![142, 143];
        build.prism.verified_rocm_targets = vec!["gfx1201".to_string()];
        build.prism.verified_server_sha256 = file_sha256(&binary);
        build.prism.verified_hip_library = tempdir.path().join("libggml-hip.so");
        build.prism.verified_hip_library_sha256 = "1".repeat(64);

        assert!(
            verify_prism_attestation(&build, &FakeProbe::success("gfx1201", "sm_120"))
                .await
                .is_ok()
        );
        assert!(matches!(
            verify_prism_attestation(&build, &FakeProbe::failure()).await,
            Err(RuntimeCliError::PrismAcceleratorProbe {
                backend: "ROCm",
                ..
            })
        ));
        assert!(matches!(
            verify_prism_attestation(&build, &FakeProbe::success("gfx1100", "sm_120")).await,
            Err(RuntimeCliError::PrismAcceleratorTargetMismatch {
                backend: "ROCm",
                ..
            })
        ));
    }

    #[tokio::test]
    async fn cuda_prism_uses_cuda_probe_on_mixed_host() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let binary = tempdir.path().join("prism/llama-server");
        fs::create_dir_all(binary.parent().expect("binary parent")).expect("binary directory");
        fs::write(&binary, b"binary").expect("Prism binary");
        let mut build = lmml_state::BuildState::default();
        build.prism.binary = binary.clone();
        build.prism.backend = "Cuda".to_string();
        build.prism.archs = vec!["sm_120".to_string()];
        build.prism.verification_version = lmml_build::RUNTIME_VERIFICATION_VERSION;
        build.prism.verified_prism_tensor_types = vec![142, 143];
        build.prism.verified_cuda_targets = vec!["sm_120".to_string()];
        build.prism.verified_server_sha256 = file_sha256(&binary);
        build.prism.verified_cuda_library = tempdir.path().join("libggml-cuda.so");
        build.prism.verified_cuda_library_sha256 = "1".repeat(64);
        let probe = FakeProbe {
            rocm: Ok(vec!["gfx1201".to_string()]),
            cuda: Ok(vec!["sm_120".to_string()]),
        };

        assert!(verify_prism_attestation(&build, &probe).await.is_ok());
        assert!(matches!(
            verify_prism_attestation(&build, &FakeProbe::success("gfx1201", "sm_89")).await,
            Err(RuntimeCliError::PrismAcceleratorTargetMismatch {
                backend: "CUDA",
                ..
            })
        ));
    }

    #[tokio::test]
    async fn rejects_prism_binary_changed_after_admission() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let model = tempdir.path().join("ternary.gguf");
        let binary = tempdir.path().join("prism/llama-server");
        fs::create_dir_all(binary.parent().expect("binary parent")).expect("binary directory");
        fs::write(&binary, b"admitted binary").expect("Prism binary");
        fs::write(&model, fixture_gguf(142)).expect("Prism model");
        let mut build = lmml_state::BuildState::default();
        build.prism.binary = binary.clone();
        build.prism.backend = "CpuAvx2".to_string();
        build.prism.verification_version = lmml_build::RUNTIME_VERIFICATION_VERSION;
        build.prism.verified_prism_tensor_types = vec![142, 143];
        build.prism.verified_server_sha256 = file_sha256(&binary);
        fs::write(&binary, b"replaced binary").expect("replace Prism binary");

        assert!(matches!(
            resolve_model_runtime_with_probe(&build, &model, &FakeProbe::failure()).await,
            Err(RuntimeCliError::PrismArtifactVerification { .. })
        ));
    }

    #[tokio::test]
    async fn rejects_legacy_prism_build_without_attestation() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let model = tempdir.path().join("ternary.gguf");
        let binary = tempdir.path().join("prism/llama-server");
        fs::create_dir_all(binary.parent().expect("binary parent")).expect("binary directory");
        fs::write(&binary, b"binary").expect("Prism binary");
        fs::write(&model, fixture_gguf(143)).expect("Prism model");
        let mut build = lmml_state::BuildState::default();
        build.prism.binary = binary;

        assert!(matches!(
            resolve_model_runtime(&build, &model).await,
            Err(RuntimeCliError::PrismVerificationRequired {
                expected_version: lmml_build::RUNTIME_VERIFICATION_VERSION,
                found_version: 0,
                ref missing_evidence,
            }) if missing_evidence.contains(&"tensor type 143".to_string())
        ));
    }

    #[derive(Clone)]
    struct FakeProbe {
        rocm: Result<Vec<String>, lmml_detect::AcceleratorTargetProbeError>,
        cuda: Result<Vec<String>, lmml_detect::AcceleratorTargetProbeError>,
    }

    impl FakeProbe {
        fn success(rocm: &str, cuda: &str) -> Self {
            Self {
                rocm: Ok(vec![rocm.to_string()]),
                cuda: Ok(vec![cuda.to_string()]),
            }
        }

        fn failure() -> Self {
            let error = lmml_detect::AcceleratorTargetProbeError::NoSupportedTarget {
                program: "test-probe",
            };
            Self {
                rocm: Err(error.clone()),
                cuda: Err(error),
            }
        }
    }

    impl AcceleratorTargetProbe for FakeProbe {
        async fn rocm_targets(
            &self,
        ) -> Result<Vec<String>, lmml_detect::AcceleratorTargetProbeError> {
            self.rocm.clone()
        }

        async fn cuda_archs(
            &self,
        ) -> Result<Vec<String>, lmml_detect::AcceleratorTargetProbeError> {
            self.cuda.clone()
        }
    }

    fn file_sha256(path: &Path) -> String {
        lmml_substrate::sha256_file(path)
            .expect("file digest")
            .to_string()
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
