//! ROCm container command construction, preflight, and provenance.

use std::path::{Path, PathBuf};

use tokio::process::Command;

use super::QuantizationArg;

pub(super) const CONVERTER_PREFLIGHT: &str = concat!(
    "import sys, torch, numpy, safetensors, transformers, gguf; ",
    "print('python=' + sys.version.replace('\\n', ' ') + ",
    "';torch=' + str(torch.__version__) + ",
    "';rocm=' + str(torch.version.hip) + ",
    "';numpy=' + str(numpy.__version__) + ",
    "';safetensors=' + str(safetensors.__version__) + ",
    "';transformers=' + str(transformers.__version__) + ';gguf=ok')"
);

pub(super) struct ContainerProvenance {
    pub(super) runtime_path: PathBuf,
    pub(super) runtime_version: String,
    pub(super) image: String,
    pub(super) image_identity: String,
    pub(super) environment: String,
}

pub(super) fn build_converter_command(
    python: &str,
    converter: &Path,
    source: &Path,
    output: &Path,
    quant: QuantizationArg,
    rocm_container: Option<&Path>,
    rocm_image: Option<&str>,
) -> Result<(PathBuf, Vec<String>), String> {
    let local_args = || {
        vec![
            converter.to_string_lossy().into_owned(),
            source.to_string_lossy().into_owned(),
            "--outfile".into(),
            output.to_string_lossy().into_owned(),
            "--outtype".into(),
            quant.converter_dtype().into(),
        ]
    };
    match (rocm_container, rocm_image) {
        (None, None) => Ok((PathBuf::from(python), local_args())),
        (Some(runtime), Some(image)) => {
            let converter_root = converter
                .parent()
                .ok_or_else(|| "converter path has no parent directory".to_string())?;
            let converter_name = converter
                .file_name()
                .ok_or_else(|| "converter path has no file name".to_string())?;
            let source = source
                .canonicalize()
                .map_err(|error| format!("could not resolve canonical source: {error}"))?;
            let output_parent = output
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .canonicalize()
                .map_err(|error| format!("could not resolve output directory: {error}"))?;
            let output_name = output
                .file_name()
                .ok_or_else(|| "temporary output path has no file name".to_string())?;
            let container_converter =
                format!("/lmml/llama.cpp/{}", converter_name.to_string_lossy());
            let container_output = format!("/lmml/output/{}", output_name.to_string_lossy());
            let args = vec![
                "run".into(),
                "--rm".into(),
                "-v".into(),
                format!("{}:/lmml/source:ro", source.display()),
                "-v".into(),
                format!("{}:/lmml/output", output_parent.display()),
                "-v".into(),
                format!("{}:/lmml/llama.cpp:ro", converter_root.display()),
                image.to_string(),
                python.to_string(),
                container_converter,
                "/lmml/source".into(),
                "--outfile".into(),
                container_output,
                "--outtype".into(),
                quant.converter_dtype().into(),
            ];
            Ok((runtime.to_path_buf(), args))
        }
        _ => Err("--rocm-container and --rocm-image must be supplied together".into()),
    }
}

pub(super) async fn inspect_rocm_container(
    runtime: &Path,
    image: &str,
    python: &str,
    converter: &Path,
) -> Result<ContainerProvenance, String> {
    let converter_root = converter
        .parent()
        .ok_or_else(|| "converter path has no parent directory".to_string())?
        .canonicalize()
        .map_err(|error| format!("could not resolve converter checkout: {error}"))?;
    let runtime_version = capture_command(runtime, &["--version".into()]).await?;
    let image_identity = capture_command(
        runtime,
        &[
            "image".into(),
            "inspect".into(),
            "--format".into(),
            "{{.Id}}|{{json .RepoDigests}}".into(),
            image.into(),
        ],
    )
    .await?;
    let environment = capture_command(
        runtime,
        &[
            "run".into(),
            "--rm".into(),
            "-v".into(),
            format!("{}:/lmml/llama.cpp:ro", converter_root.display()),
            "-e".into(),
            "PYTHONPATH=/lmml/llama.cpp:/lmml/llama.cpp/gguf-py".into(),
            image.into(),
            python.into(),
            "-c".into(),
            CONVERTER_PREFLIGHT.into(),
        ],
    )
    .await
    .map_err(|error| {
        format!(
            "image must import torch, numpy, safetensors, transformers, and llama.cpp gguf modules: {error}"
        )
    })?;
    Ok(ContainerProvenance {
        runtime_path: runtime.to_path_buf(),
        runtime_version,
        image: image.to_string(),
        image_identity,
        environment,
    })
}

async fn capture_command(program: &Path, args: &[String]) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .await
        .map_err(|error| format!("{}: {error}", program.display()))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !output.status.success() {
        return Err(if stderr.is_empty() {
            format!("{} exited with {}", program.display(), output.status)
        } else {
            format!(
                "{} exited with {}: {stderr}",
                program.display(),
                output.status
            )
        });
    }
    if stdout.is_empty() {
        Err(format!(
            "{} returned no version information",
            program.display()
        ))
    } else {
        Ok(stdout)
    }
}
