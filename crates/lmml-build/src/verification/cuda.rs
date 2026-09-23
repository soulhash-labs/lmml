//! Prism CUDA kernel and artifact verification.

use std::path::Path;

use serde_json::Value;

use crate::{BuildConfig, BuildError};

use super::{binary_contains, compile_command_args, require_markers};

const CUDA_KERNEL_UNITS: [&str; 2] = ["mmvq.cu", "mmq.cu"];
const CUDA_KERNEL_MARKERS: [&[u8]; 2] = [b"pq2_0", b"ptq1_0"];

pub(super) async fn preflight(
    config: &BuildConfig,
    archs: &[&'static str],
) -> Result<(), BuildError> {
    validate_targets(archs)?;
    verify_source(config).await
}

pub(super) async fn verify_compiled_runtime(
    config: &BuildConfig,
    cuda_library: &Path,
    archs: &[&'static str],
) -> Result<(), BuildError> {
    verify_compile_commands(config, archs).await?;
    verify_binary_evidence(cuda_library, archs).await
}

fn validate_targets(archs: &[&'static str]) -> Result<(), BuildError> {
    if archs.is_empty() {
        return Err(BuildError::Verification(
            "Prism CUDA build requires at least one explicit detected sm target".to_string(),
        ));
    }
    let invalid = archs
        .iter()
        .filter(|arch| lmml_detect::normalize_cuda_arch(arch) != Some(**arch))
        .copied()
        .collect::<Vec<_>>();
    if invalid.is_empty() {
        Ok(())
    } else {
        Err(BuildError::Verification(format!(
            "Prism CUDA build has invalid or generic GPU targets: {}; use concrete detected targets such as sm_86",
            invalid.join(", ")
        )))
    }
}

async fn verify_source(config: &BuildConfig) -> Result<(), BuildError> {
    let cuda = config.source_dir.join("ggml/src/ggml-cuda");
    require_markers(
        &cuda.join("mmvq.cu"),
        &[
            "case GGML_TYPE_PQ2_0",
            "case GGML_TYPE_PTQ1_0",
            "mul_mat_vec_q_switch_ncols_dst<GGML_TYPE_PQ2_0>",
            "mul_mat_vec_q_switch_ncols_dst<GGML_TYPE_PTQ1_0>",
        ],
        "Prism CUDA matrix-vector dispatch",
    )
    .await?;
    require_markers(
        &cuda.join("mmq.cu"),
        &[
            "case GGML_TYPE_PQ2_0",
            "case GGML_TYPE_PTQ1_0",
            "mul_mat_q_case<GGML_TYPE_PQ2_0>",
            "mul_mat_q_case<GGML_TYPE_PTQ1_0>",
        ],
        "Prism CUDA matrix-matrix dispatch",
    )
    .await?;
    require_markers(
        &cuda.join("vecdotq.cuh"),
        &["vec_dot_pq2_0_q8_1", "vec_dot_ptq1_0_q8_1"],
        "Prism CUDA tensor kernels",
    )
    .await?;
    require_markers(
        &cuda.join("CMakeLists.txt"),
        &["GGML_SOURCES_CUDA", "*.cu"],
        "Prism CUDA build graph",
    )
    .await
}

async fn verify_compile_commands(
    config: &BuildConfig,
    archs: &[&'static str],
) -> Result<(), BuildError> {
    let path = config.source_dir.join("build/compile_commands.json");
    let bytes = tokio::fs::read(&path).await.map_err(|error| {
        BuildError::Verification(format!(
            "failed to inspect Prism CUDA compile commands {}: {error}",
            path.display()
        ))
    })?;
    let entries: Vec<Value> = serde_json::from_slice(&bytes).map_err(|error| {
        BuildError::Verification(format!(
            "invalid Prism CUDA compile commands {}: {error}",
            path.display()
        ))
    })?;

    for unit in CUDA_KERNEL_UNITS {
        let commands = entries
            .iter()
            .filter(|entry| {
                entry
                    .get("file")
                    .and_then(Value::as_str)
                    .is_some_and(|file| source_file_matches(file, unit))
            })
            .filter_map(compile_command_args)
            .collect::<Vec<_>>();
        if commands.is_empty() {
            return Err(BuildError::Verification(format!(
                "{} does not show the Prism CUDA tensor kernel {unit} being compiled",
                path.display()
            )));
        }
        let missing = archs
            .iter()
            .filter(|arch| {
                !commands
                    .iter()
                    .any(|command| command_targets(command, arch))
            })
            .copied()
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(BuildError::Verification(format!(
                "Prism CUDA kernel {unit} was not compiled for requested targets: {}",
                missing.join(", ")
            )));
        }
    }
    Ok(())
}

fn source_file_matches(file: &str, expected: &str) -> bool {
    file.rsplit(['/', '\\']).next() == Some(expected)
}

fn command_targets(arguments: &[String], target: &str) -> bool {
    cuda_target_markers(target).iter().any(|marker| {
        arguments.iter().any(|argument| {
            argument
                .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
                .any(|token| token == marker)
        })
    })
}

async fn verify_binary_evidence(
    cuda_library: &Path,
    archs: &[&'static str],
) -> Result<(), BuildError> {
    let mut missing_targets = Vec::new();
    for arch in archs {
        let markers = cuda_target_markers(arch);
        let mut found = false;
        for marker in &markers {
            if binary_contains(cuda_library, marker.as_bytes()).await? {
                found = true;
                break;
            }
        }
        if !found {
            missing_targets.push(*arch);
        }
    }
    if !missing_targets.is_empty() {
        return Err(BuildError::Verification(format!(
            "linked CUDA library {} has no code-object marker for targets: {}",
            cuda_library.display(),
            missing_targets.join(", ")
        )));
    }

    let mut missing_kernels = Vec::new();
    for marker in CUDA_KERNEL_MARKERS {
        if !binary_contains(cuda_library, marker).await? {
            missing_kernels.push(String::from_utf8_lossy(marker).into_owned());
        }
    }
    if missing_kernels.is_empty() {
        Ok(())
    } else {
        Err(BuildError::Verification(format!(
            "linked CUDA library {} lacks Prism tensor-kernel markers: {}",
            cuda_library.display(),
            missing_kernels.join(", ")
        )))
    }
}

fn cuda_target_markers(target: &str) -> Vec<String> {
    let mut markers = vec![target.to_string(), target.replacen("sm_", "compute_", 1)];
    if target.starts_with("sm_12") && !target.ends_with('a') {
        let architecture_specific = format!("{target}a");
        markers.push(architecture_specific.clone());
        markers.push(architecture_specific.replacen("sm_", "compute_", 1));
    }
    markers
}

#[cfg(test)]
mod tests {
    use std::fs;

    use lmml_detect::BuildBackend;

    use super::*;

    #[test]
    fn rejects_empty_generic_and_unknown_cuda_targets() {
        assert!(validate_targets(&["sm_86"]).is_ok());
        assert!(validate_targets(&[]).is_err());
        assert!(validate_targets(&["native"]).is_err());
        assert!(validate_targets(&["sm_000"]).is_err());
    }

    #[tokio::test]
    async fn accepts_both_prism_kernel_units_for_exact_cuda_target() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let build = tempdir.path().join("build");
        fs::create_dir_all(&build).expect("build directory");
        fs::write(
            build.join("compile_commands.json"),
            concat!(
                "[",
                r#"{"file":"/src/mmvq.cu","arguments":["nvcc","--generate-code=arch=compute_86,code=[compute_86,sm_86]","-c","mmvq.cu"]}"#,
                ",",
                r#"{"file":"/src/mmq.cu","arguments":["nvcc","--generate-code=arch=compute_86,code=[compute_86,sm_86]","-c","mmq.cu"]}"#,
                "]"
            ),
        )
        .expect("compile commands");
        let config = prism_cuda_config(tempdir.path());

        verify_compile_commands(&config, &["sm_86"])
            .await
            .expect("compiled CUDA kernels");
    }

    #[tokio::test]
    async fn rejects_missing_kernel_unit_and_target_substring() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let build = tempdir.path().join("build");
        fs::create_dir_all(&build).expect("build directory");
        fs::write(
            build.join("compile_commands.json"),
            r#"[{"file":"/src/mmvq.cu","command":"nvcc -arch=sm_860 -c mmvq.cu"}]"#,
        )
        .expect("compile commands");

        assert!(
            verify_compile_commands(&prism_cuda_config(tempdir.path()), &["sm_86"])
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn binary_requires_cuda_target_and_both_tensor_markers() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let library = tempdir.path().join("libggml-cuda.so");
        fs::write(&library, b"sm_86 pq2_0 ptq1_0").expect("CUDA library");
        verify_binary_evidence(&library, &["sm_86"])
            .await
            .expect("complete CUDA evidence");

        fs::write(&library, b"sm_86 pq2_0").expect("incomplete CUDA library");
        assert!(verify_binary_evidence(&library, &["sm_86"]).await.is_err());
    }

    #[test]
    fn accepts_cmake_blackwell_architecture_specific_rewrite() {
        let args = vec![
            "nvcc".to_string(),
            "--generate-code=arch=compute_120a,code=[compute_120a,sm_120a]".to_string(),
        ];
        assert!(command_targets(&args, "sm_120"));
    }

    fn prism_cuda_config(source_dir: &Path) -> BuildConfig {
        BuildConfig::for_flavor(
            source_dir.to_path_buf(),
            BuildBackend::Cuda {
                archs: vec!["sm_86"],
            },
            lmml_compat::LlamaRuntimeFlavor::Prism,
        )
    }
}
