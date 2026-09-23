//! Runtime-flavor verification after a llama.cpp build.
//!
//! The checks in this module bind the resulting executable to the selected
//! trusted source flavor and reject mixed shared-library installations before
//! the build is recorded as usable. Prism ROCm builds also prove that the
//! private tensor kernels were compiled for every selected HIP target.

use std::path::{Path, PathBuf};

use lmml_detect::BuildBackend;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;

use crate::{
    AcceleratorLibraryKind, BuildConfig, BuildError, RuntimeLibraryIdentity, RuntimeVerification,
    RUNTIME_VERIFICATION_VERSION,
};

mod cuda;

const PRISM_TENSOR_TYPES: [u32; 2] = [142, 143];

pub(super) async fn preflight_build(config: &BuildConfig) -> Result<(), BuildError> {
    if config.flavor != lmml_compat::LlamaRuntimeFlavor::Prism {
        return Ok(());
    }

    verify_prism_source(config).await?;
    match &config.backend {
        BuildBackend::Cuda { archs } => cuda::preflight(config, archs).await?,
        BuildBackend::Rocm { targets } => {
            validate_rocm_targets(targets)?;
            verify_prism_hip_source(config).await?;
        }
        BuildBackend::Metal
        | BuildBackend::Vulkan
        | BuildBackend::CpuAvx2
        | BuildBackend::CpuAvx
        | BuildBackend::CpuFallback => {}
    }
    Ok(())
}

pub(super) async fn verify_runtime(
    config: &BuildConfig,
    server: &Path,
) -> Result<RuntimeVerification, BuildError> {
    preflight_build(config).await?;
    let linked_libraries = verify_linux_dependencies(config, server).await?;
    let mut report = RuntimeVerification {
        version: RUNTIME_VERIFICATION_VERSION,
        prism_tensor_types: if config.flavor == lmml_compat::LlamaRuntimeFlavor::Prism {
            PRISM_TENSOR_TYPES.to_vec()
        } else {
            Vec::new()
        },
        rocm_targets: Vec::new(),
        cuda_targets: Vec::new(),
        server_sha256: String::new(),
        hip_library: None,
        hip_library_sha256: None,
        cuda_library: None,
        cuda_library_sha256: None,
    };

    if config.flavor == lmml_compat::LlamaRuntimeFlavor::Prism {
        match &config.backend {
            BuildBackend::Cuda { archs } => {
                let cuda_library = linked_libraries.cuda.ok_or_else(|| {
                    BuildError::Verification(format!(
                        "Prism CUDA runtime {} is not linked to an isolated libggml-cuda",
                        server.display()
                    ))
                })?;
                cuda::verify_compiled_runtime(config, &cuda_library, archs).await?;
                report.cuda_targets = archs.iter().map(|arch| (*arch).to_string()).collect();
                report.cuda_library_sha256 = Some(sha256_file(&cuda_library).await?);
                report.cuda_library = Some(cuda_library);
            }
            BuildBackend::Rocm { targets } => {
                verify_rocm_compile_commands(config, targets).await?;
                let hip_library = linked_libraries.hip.ok_or_else(|| {
                    BuildError::Verification(format!(
                        "Prism ROCm runtime {} is not linked to an isolated libggml-hip",
                        server.display()
                    ))
                })?;
                verify_rocm_binary_targets(&hip_library, targets).await?;
                report.rocm_targets = targets.clone();
                report.hip_library_sha256 = Some(sha256_file(&hip_library).await?);
                report.hip_library = Some(hip_library);
            }
            BuildBackend::Metal
            | BuildBackend::Vulkan
            | BuildBackend::CpuAvx2
            | BuildBackend::CpuAvx
            | BuildBackend::CpuFallback => {}
        }
    }
    report.server_sha256 = sha256_file(server).await?;
    Ok(report)
}

pub(super) async fn verify_artifact_attestation(
    server: &Path,
    expected_server_sha256: &str,
    expected_library: Option<RuntimeLibraryIdentity<'_>>,
) -> Result<(), BuildError> {
    verify_hash(server, expected_server_sha256, "llama-server").await?;
    if let Some(expected) = expected_library {
        let actual_path = linked_accelerator_library(server, expected.kind).await?;
        verify_resolved_library_identity(
            server,
            expected.kind,
            expected.path,
            expected.sha256,
            actual_path.as_deref(),
        )
        .await
    } else {
        Ok(())
    }
}

async fn verify_resolved_library_identity(
    server: &Path,
    kind: AcceleratorLibraryKind,
    expected_path: &Path,
    expected_hash: &str,
    resolved_path: Option<&Path>,
) -> Result<(), BuildError> {
    let label = accelerator_library_name(kind);
    let actual_path = resolved_path.ok_or_else(|| {
        BuildError::Verification(format!(
            "{} no longer resolves a linked {label}",
            server.display(),
        ))
    })?;
    let expected_path = canonical_path(expected_path)?;
    let actual_path = canonical_path(actual_path)?;
    if actual_path != expected_path {
        return Err(BuildError::Verification(format!(
            "{} now resolves {label} at {}, expected {}",
            server.display(),
            actual_path.display(),
            expected_path.display()
        )));
    }
    verify_hash(&actual_path, expected_hash, label).await
}

fn accelerator_library_name(kind: AcceleratorLibraryKind) -> &'static str {
    match kind {
        AcceleratorLibraryKind::Hip => "libggml-hip",
        AcceleratorLibraryKind::Cuda => "libggml-cuda",
    }
}

async fn verify_hash(path: &Path, expected: &str, label: &str) -> Result<(), BuildError> {
    let actual = sha256_file(path).await?;
    if actual == expected {
        Ok(())
    } else {
        Err(BuildError::Verification(format!(
            "{label} digest mismatch for {}: expected {expected}, found {actual}",
            path.display()
        )))
    }
}

pub(super) async fn sha256_file(path: &Path) -> Result<String, BuildError> {
    let mut file = tokio::fs::File::open(path).await.map_err(|error| {
        BuildError::Verification(format!("failed to hash {}: {error}", path.display()))
    })?;
    let mut hasher = Sha256::new();
    let mut chunk = vec![0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut chunk).await.map_err(|error| {
            BuildError::Verification(format!("failed to hash {}: {error}", path.display()))
        })?;
        if read == 0 {
            return Ok(hasher
                .finalize()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect());
        }
        hasher.update(&chunk[..read]);
    }
}

async fn verify_prism_source(config: &BuildConfig) -> Result<(), BuildError> {
    require_markers(
        &config.source_dir.join("ggml/include/ggml.h"),
        &[
            "GGML_TYPE_PQ2_0",
            "GGML_TYPE_PTQ1_0",
            "GGML_HINT_SRC0_IS_HADAMARD",
        ],
        "Prism tensor registry",
    )
    .await?;

    if !source_tree_contains(&config.source_dir.join("src"), "prism.hadamard.").await? {
        return Err(BuildError::Verification(format!(
            "Prism source at {} lacks the prism.hadamard metadata loader; tensor IDs alone are insufficient",
            config.source_dir.display()
        )));
    }
    Ok(())
}

async fn verify_prism_hip_source(config: &BuildConfig) -> Result<(), BuildError> {
    require_markers(
        &config.source_dir.join("ggml/src/ggml-cuda/mmvq.cu"),
        &[
            "case GGML_TYPE_PQ2_0",
            "case GGML_TYPE_PTQ1_0",
            "mul_mat_vec_q_switch_ncols_dst<GGML_TYPE_PQ2_0>",
            "mul_mat_vec_q_switch_ncols_dst<GGML_TYPE_PTQ1_0>",
        ],
        "Prism HIP matrix-vector dispatch",
    )
    .await?;
    require_markers(
        &config.source_dir.join("ggml/src/ggml-cuda/vecdotq.cuh"),
        &[
            "defined(GGML_USE_HIP)",
            "vec_dot_pq2_0_q8_1",
            "vec_dot_ptq1_0_q8_1",
            "__builtin_amdgcn",
        ],
        "Prism HIP tensor kernels",
    )
    .await?;
    require_markers(
        &config.source_dir.join("ggml/src/ggml-hip/CMakeLists.txt"),
        &["GGML_SOURCES_ROCM", "../ggml-cuda/*.cu", "LANGUAGE HIP"],
        "Prism HIP build graph",
    )
    .await
}

pub(super) async fn require_markers(
    path: &Path,
    markers: &[&str],
    label: &str,
) -> Result<(), BuildError> {
    let source = tokio::fs::read_to_string(path).await.map_err(|error| {
        BuildError::Verification(format!(
            "failed to inspect {label} {}: {error}",
            path.display()
        ))
    })?;
    let missing = markers
        .iter()
        .filter(|marker| !source.contains(**marker))
        .copied()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(BuildError::Verification(format!(
            "{label} {} lacks required markers: {}",
            path.display(),
            missing.join(", ")
        )))
    }
}

async fn source_tree_contains(root: &Path, marker: &str) -> Result<bool, BuildError> {
    let mut directories = vec![root.to_path_buf()];
    while let Some(directory) = directories.pop() {
        let mut entries = tokio::fs::read_dir(&directory).await.map_err(|error| {
            BuildError::Verification(format!(
                "failed to inspect Prism source directory {}: {error}",
                directory.display()
            ))
        })?;
        while let Some(entry) = entries.next_entry().await.map_err(|error| {
            BuildError::Verification(format!(
                "failed to inspect Prism source directory {}: {error}",
                directory.display()
            ))
        })? {
            let file_type = entry.file_type().await.map_err(|error| {
                BuildError::Verification(format!(
                    "failed to inspect Prism source entry {}: {error}",
                    entry.path().display()
                ))
            })?;
            if file_type.is_dir() {
                directories.push(entry.path());
                continue;
            }
            if !file_type.is_file() || !is_source_file(&entry.path()) {
                continue;
            }
            let bytes = tokio::fs::read(entry.path()).await.map_err(|error| {
                BuildError::Verification(format!(
                    "failed to read Prism source {}: {error}",
                    entry.path().display()
                ))
            })?;
            if bytes
                .windows(marker.len())
                .any(|window| window == marker.as_bytes())
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn is_source_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("c" | "cc" | "cpp" | "cxx" | "h" | "hh" | "hpp" | "inc")
    )
}

fn validate_rocm_targets(targets: &[String]) -> Result<(), BuildError> {
    if targets.is_empty() {
        return Err(BuildError::Verification(
            "Prism ROCm build requires at least one explicit detected gfx target".to_string(),
        ));
    }
    let invalid = targets
        .iter()
        .filter(|target| {
            lmml_detect::normalize_rocm_target(target).as_deref() != Some(target.as_str())
        })
        .cloned()
        .collect::<Vec<_>>();
    if invalid.is_empty() {
        Ok(())
    } else {
        Err(BuildError::Verification(format!(
            "Prism ROCm build has invalid or generic GPU targets: {}; use concrete rocminfo targets such as gfx1201",
            invalid.join(", ")
        )))
    }
}

async fn verify_rocm_compile_commands(
    config: &BuildConfig,
    targets: &[String],
) -> Result<(), BuildError> {
    let path = config.source_dir.join("build/compile_commands.json");
    let bytes = tokio::fs::read(&path).await.map_err(|error| {
        BuildError::Verification(format!(
            "failed to inspect Prism ROCm compile commands {}: {error}",
            path.display()
        ))
    })?;
    let entries: Vec<Value> = serde_json::from_slice(&bytes).map_err(|error| {
        BuildError::Verification(format!(
            "invalid Prism ROCm compile commands {}: {error}",
            path.display()
        ))
    })?;
    let commands = entries
        .iter()
        .filter(|entry| {
            entry
                .get("file")
                .and_then(Value::as_str)
                .is_some_and(|file| file.ends_with("/ggml-cuda/mmvq.cu") || file == "mmvq.cu")
        })
        .filter_map(compile_command_args)
        .collect::<Vec<_>>();
    if commands.is_empty() {
        return Err(BuildError::Verification(format!(
            "{} does not show the Prism PQ2/PTQ1 matrix-vector kernel being compiled",
            path.display()
        )));
    }
    if !commands
        .iter()
        .any(|arguments| arguments.iter().any(|argument| is_hip_define(argument)))
    {
        return Err(BuildError::Verification(
            "Prism matrix-vector kernel compile command lacks GGML_USE_HIP".to_string(),
        ));
    }
    let missing = targets
        .iter()
        .filter(|target| {
            !commands
                .iter()
                .any(|command| command_targets(command, target))
        })
        .cloned()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(BuildError::Verification(format!(
            "Prism HIP kernels were not compiled for requested targets: {}",
            missing.join(", ")
        )))
    }
}

pub(super) fn compile_command_args(entry: &Value) -> Option<Vec<String>> {
    if let Some(arguments) = entry.get("arguments").and_then(Value::as_array) {
        return Some(
            arguments
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect(),
        );
    }
    shlex::split(entry.get("command")?.as_str()?)
}

fn is_hip_define(argument: &str) -> bool {
    argument == "-DGGML_USE_HIP" || argument.starts_with("-DGGML_USE_HIP=")
}

fn command_targets(arguments: &[String], target: &str) -> bool {
    let flags = [
        format!("--offload-arch={target}"),
        format!("--amdgpu-target={target}"),
        format!("--cuda-gpu-arch={target}"),
        format!("-mcpu={target}"),
    ];
    arguments.iter().any(|argument| flags.contains(argument))
}

async fn verify_rocm_binary_targets(
    hip_library: &Path,
    targets: &[String],
) -> Result<(), BuildError> {
    let mut missing = Vec::new();
    for target in targets {
        if !binary_contains(hip_library, target.as_bytes()).await? {
            missing.push(target.clone());
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(BuildError::Verification(format!(
            "linked HIP library {} has no code-object marker for targets: {}",
            hip_library.display(),
            missing.join(", ")
        )))
    }
}

pub(super) async fn binary_contains(path: &Path, needle: &[u8]) -> Result<bool, BuildError> {
    let mut file = tokio::fs::File::open(path).await.map_err(|error| {
        BuildError::Verification(format!("failed to inspect {}: {error}", path.display()))
    })?;
    let mut chunk = vec![0_u8; 64 * 1024];
    let mut overlap = Vec::new();
    loop {
        let read = file.read(&mut chunk).await.map_err(|error| {
            BuildError::Verification(format!("failed to inspect {}: {error}", path.display()))
        })?;
        if read == 0 {
            return Ok(false);
        }
        overlap.extend_from_slice(&chunk[..read]);
        if overlap.windows(needle.len()).any(|window| window == needle) {
            return Ok(true);
        }
        let keep = needle.len().saturating_sub(1).min(overlap.len());
        overlap.drain(..overlap.len() - keep);
    }
}

#[cfg(target_os = "linux")]
async fn linked_dependencies(server: &Path) -> Result<String, BuildError> {
    let output = tokio::process::Command::new("ldd")
        .arg(server)
        .output()
        .await
        .map_err(|error| {
            BuildError::Verification(format!(
                "failed to inspect shared libraries for {}: {error}",
                server.display()
            ))
        })?;
    if !output.status.success() {
        return Err(BuildError::Verification(format!(
            "ldd failed for {}",
            server.display()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(not(target_os = "linux"))]
async fn linked_dependencies(_server: &Path) -> Result<String, BuildError> {
    Ok(String::new())
}

#[cfg(target_os = "linux")]
async fn linked_accelerator_library(
    server: &Path,
    kind: AcceleratorLibraryKind,
) -> Result<Option<PathBuf>, BuildError> {
    let dependencies = linked_dependencies(server).await?;
    let library_name = accelerator_library_name(kind);
    for line in dependencies
        .lines()
        .filter(|line| line.contains(library_name))
    {
        if line.contains("not found") {
            return Err(BuildError::Verification(format!(
                "runtime dependency is unresolved: {}",
                line.trim()
            )));
        }
        if let Some(path) = dependency_path(line) {
            return canonical_path(&path).map(Some);
        }
    }
    Ok(None)
}

#[cfg(not(target_os = "linux"))]
async fn linked_accelerator_library(
    _server: &Path,
    _kind: AcceleratorLibraryKind,
) -> Result<Option<PathBuf>, BuildError> {
    Ok(None)
}

#[derive(Debug, Default)]
struct LinkedAcceleratorLibraries {
    hip: Option<PathBuf>,
    cuda: Option<PathBuf>,
}

#[cfg(target_os = "linux")]
async fn verify_linux_dependencies(
    config: &BuildConfig,
    server: &Path,
) -> Result<LinkedAcceleratorLibraries, BuildError> {
    let build_root = canonical_path(&config.source_dir.join("build"))?;
    let dependencies = linked_dependencies(server).await?;
    let mut libraries = LinkedAcceleratorLibraries::default();
    for line in dependencies.lines().filter(|line| {
        ["libggml", "libllama", "libmtmd"]
            .iter()
            .any(|library| line.contains(library))
    }) {
        if line.contains("not found") {
            return Err(BuildError::Verification(format!(
                "runtime dependency is unresolved: {}",
                line.trim()
            )));
        }
        let Some(path) = dependency_path(line) else {
            continue;
        };
        let resolved = canonical_path(&path)?;
        if !resolved.starts_with(&build_root) {
            return Err(BuildError::Verification(format!(
                "{} runtime would load {} outside its isolated build {}",
                config.flavor,
                resolved.display(),
                build_root.display()
            )));
        }
        if line.contains("libggml-hip") {
            libraries.hip = Some(resolved);
        } else if line.contains("libggml-cuda") {
            libraries.cuda = Some(resolved);
        }
    }
    Ok(libraries)
}

#[cfg(not(target_os = "linux"))]
async fn verify_linux_dependencies(
    _config: &BuildConfig,
    _server: &Path,
) -> Result<LinkedAcceleratorLibraries, BuildError> {
    Ok(LinkedAcceleratorLibraries::default())
}

#[cfg(target_os = "linux")]
fn dependency_path(line: &str) -> Option<PathBuf> {
    let candidate = line
        .split_once("=>")
        .map(|(_, right)| right.trim())
        .unwrap_or_else(|| line.trim())
        .split_whitespace()
        .next()?;
    candidate.starts_with('/').then(|| PathBuf::from(candidate))
}

fn canonical_path(path: &Path) -> Result<PathBuf, BuildError> {
    path.canonicalize().map_err(|error| {
        BuildError::Verification(format!(
            "failed to resolve runtime path {}: {error}",
            path.display()
        ))
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn validates_only_specific_rocm_targets() {
        assert!(validate_rocm_targets(&["gfx1201".to_string()]).is_ok());
        for targets in [
            Vec::new(),
            vec!["gfx12-generic".to_string()],
            vec!["native".to_string()],
            vec!["gfx".to_string()],
            vec!["gfx000".to_string()],
            vec!["gfx1035".to_string()],
        ] {
            assert!(validate_rocm_targets(&targets).is_err());
        }
    }

    #[tokio::test]
    async fn accepts_prism_hip_source_and_targeted_compile_command() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        write_prism_source_fixture(tempdir.path());
        let build = tempdir.path().join("build");
        fs::create_dir_all(&build).expect("build directory");
        fs::write(
            build.join("compile_commands.json"),
            r#"[{"file":"/src/ggml-cuda/mmvq.cu","command":"clang -DGGML_USE_HIP --offload-arch=gfx1201 -c mmvq.cu"}]"#,
        )
        .expect("compile commands");
        let config = prism_rocm_config(tempdir.path());

        preflight_build(&config).await.expect("source preflight");
        verify_rocm_compile_commands(&config, &["gfx1201".to_string()])
            .await
            .expect("compiled target");
    }

    #[tokio::test]
    async fn rejects_compile_commands_without_selected_target() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let build = tempdir.path().join("build");
        fs::create_dir_all(&build).expect("build directory");
        fs::write(
            build.join("compile_commands.json"),
            r#"[{"file":"/src/ggml-cuda/mmvq.cu","command":"clang -DGGML_USE_HIP --offload-arch=gfx12010 -c mmvq.cu"}]"#,
        )
        .expect("compile commands");

        assert!(verify_rocm_compile_commands(
            &prism_rocm_config(tempdir.path()),
            &["gfx1201".to_string()]
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn scans_target_markers_across_binary_chunks() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let library = tempdir.path().join("libggml-hip.so");
        let mut bytes = vec![0_u8; 64 * 1024 - 3];
        bytes.extend_from_slice(b"gfx1201");
        fs::write(&library, bytes).expect("library fixture");

        assert!(binary_contains(&library, b"gfx1201")
            .await
            .expect("binary scan"));
        assert!(!binary_contains(&library, b"gfx1100")
            .await
            .expect("binary scan"));
    }

    #[tokio::test]
    async fn artifact_hash_rejects_replaced_server_bytes() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let server = tempdir.path().join("llama-server");
        fs::write(&server, b"admitted").expect("server fixture");
        let digest = sha256_file(&server).await.expect("server digest");

        verify_artifact_attestation(&server, &digest, None)
            .await
            .expect("matching artifact");
        fs::write(&server, b"replaced").expect("replace server fixture");
        assert!(verify_artifact_attestation(&server, &digest, None)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn hip_identity_rejects_changed_library_bytes() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let server = tempdir.path().join("llama-server");
        let hip_library = tempdir.path().join("libggml-hip.so");
        fs::write(&server, b"server").expect("server fixture");
        fs::write(&hip_library, b"admitted HIP library").expect("HIP fixture");
        let digest = sha256_file(&hip_library).await.expect("HIP digest");
        fs::write(&hip_library, b"replaced HIP library").expect("replace HIP fixture");

        assert!(verify_resolved_library_identity(
            &server,
            AcceleratorLibraryKind::Hip,
            &hip_library,
            &digest,
            Some(&hip_library)
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn hip_identity_rejects_different_resolved_path() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let server = tempdir.path().join("llama-server");
        let expected = tempdir.path().join("admitted/libggml-hip.so");
        let resolved = tempdir.path().join("replacement/libggml-hip.so");
        fs::create_dir_all(expected.parent().expect("expected parent")).expect("expected dir");
        fs::create_dir_all(resolved.parent().expect("resolved parent")).expect("resolved dir");
        fs::write(&server, b"server").expect("server fixture");
        fs::write(&expected, b"same bytes").expect("expected HIP fixture");
        fs::write(&resolved, b"same bytes").expect("resolved HIP fixture");
        let digest = sha256_file(&expected).await.expect("HIP digest");

        assert!(verify_resolved_library_identity(
            &server,
            AcceleratorLibraryKind::Hip,
            &expected,
            &digest,
            Some(&resolved)
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn hip_identity_rejects_missing_resolution() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let server = tempdir.path().join("llama-server");
        let hip_library = tempdir.path().join("libggml-hip.so");
        fs::write(&server, b"server").expect("server fixture");
        fs::write(&hip_library, b"HIP library").expect("HIP fixture");
        let hip_digest = sha256_file(&hip_library).await.expect("HIP digest");

        assert!(verify_resolved_library_identity(
            &server,
            AcceleratorLibraryKind::Hip,
            &hip_library,
            &hip_digest,
            None
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn cuda_identity_rejects_changed_bytes_and_resolved_path() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let server = tempdir.path().join("llama-server");
        let expected = tempdir.path().join("admitted/libggml-cuda.so");
        let replacement = tempdir.path().join("replacement/libggml-cuda.so");
        fs::create_dir_all(expected.parent().expect("expected parent")).expect("expected dir");
        fs::create_dir_all(replacement.parent().expect("replacement parent"))
            .expect("replacement dir");
        fs::write(&server, b"server").expect("server fixture");
        fs::write(&expected, b"admitted CUDA library").expect("expected CUDA fixture");
        fs::write(&replacement, b"admitted CUDA library").expect("replacement CUDA fixture");
        let digest = sha256_file(&expected).await.expect("CUDA digest");

        assert!(verify_resolved_library_identity(
            &server,
            AcceleratorLibraryKind::Cuda,
            &expected,
            &digest,
            Some(&replacement)
        )
        .await
        .is_err());

        fs::write(&expected, b"replaced CUDA library").expect("replace CUDA fixture");
        assert!(verify_resolved_library_identity(
            &server,
            AcceleratorLibraryKind::Cuda,
            &expected,
            &digest,
            Some(&expected)
        )
        .await
        .is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parses_linked_dependency_paths() {
        assert_eq!(
            dependency_path("libllama.so => /tmp/prism/build/lib/libllama.so (0x01)"),
            Some(PathBuf::from("/tmp/prism/build/lib/libllama.so"))
        );
        assert_eq!(dependency_path("libggml.so => not found"), None);
    }

    fn prism_rocm_config(source_dir: &Path) -> BuildConfig {
        BuildConfig::for_flavor(
            source_dir.to_path_buf(),
            BuildBackend::Rocm {
                targets: vec!["gfx1201".to_string()],
            },
            lmml_compat::LlamaRuntimeFlavor::Prism,
        )
    }

    fn write_prism_source_fixture(root: &Path) {
        let header = root.join("ggml/include");
        let cuda = root.join("ggml/src/ggml-cuda");
        let hip = root.join("ggml/src/ggml-hip");
        let src = root.join("src");
        for directory in [&header, &cuda, &hip, &src] {
            fs::create_dir_all(directory).expect("source directory");
        }
        fs::write(
            header.join("ggml.h"),
            "GGML_TYPE_PQ2_0 GGML_TYPE_PTQ1_0 GGML_HINT_SRC0_IS_HADAMARD",
        )
        .expect("header");
        fs::write(src.join("loader.cpp"), "prism.hadamard.block_size").expect("metadata loader");
        fs::write(
            cuda.join("mmvq.cu"),
            concat!(
                "case GGML_TYPE_PQ2_0 case GGML_TYPE_PTQ1_0 ",
                "mul_mat_vec_q_switch_ncols_dst<GGML_TYPE_PQ2_0> ",
                "mul_mat_vec_q_switch_ncols_dst<GGML_TYPE_PTQ1_0>"
            ),
        )
        .expect("mmvq");
        fs::write(
            cuda.join("vecdotq.cuh"),
            concat!(
                "defined(GGML_USE_HIP) vec_dot_pq2_0_q8_1 ",
                "vec_dot_ptq1_0_q8_1 __builtin_amdgcn"
            ),
        )
        .expect("vecdot");
        fs::write(
            hip.join("CMakeLists.txt"),
            "GGML_SOURCES_ROCM ../ggml-cuda/*.cu LANGUAGE HIP",
        )
        .expect("HIP CMake");
    }
}
