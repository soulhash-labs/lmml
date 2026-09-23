//! Deterministic pristine baseline execution and persistence.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;

pub(super) struct BaselineOptions<'a> {
    pub artifact_id: &'a str,
    pub baseline_id: &'a str,
    pub prompts: &'a Path,
    pub cli: Option<&'a Path>,
    pub output: Option<&'a Path>,
    pub predict: u32,
    pub gpu_layers: i32,
    pub extra_args: &'a [String],
    pub json: bool,
}

pub(super) async fn run(options: BaselineOptions<'_>, data_root: &Path) -> i32 {
    if let Err(error) = validate_extra_args(options.extra_args) {
        eprintln!("model baseline refused: {error}");
        return 1;
    }
    let artifacts =
        match lmml_substrate::load_artifact_manifests(data_root.join("lmml/models/artifacts")) {
            Ok(artifacts) => artifacts,
            Err(error) => {
                eprintln!("model baseline failed: {error}");
                return 1;
            }
        };
    let Some(artifact) = artifacts
        .iter()
        .find(|artifact| artifact.artifact.artifact_id == options.artifact_id)
    else {
        eprintln!(
            "model baseline failed: unknown artifact {}",
            options.artifact_id
        );
        return 1;
    };
    if artifact.admission.is_none()
        || artifact.artifact.representation != lmml_substrate::ModelRepresentation::Gguf
    {
        eprintln!("model baseline refused: artifact is not an admitted GGUF");
        return 1;
    }
    let actual_hash = match lmml_substrate::sha256_file(&artifact.artifact_path) {
        Ok(hash) => hash,
        Err(error) => {
            eprintln!("model baseline failed to hash artifact: {error}");
            return 1;
        }
    };
    if actual_hash != artifact.artifact.artifact_hash {
        eprintln!("model baseline refused: artifact hash does not match registration");
        return 1;
    }
    let prompts = match read_prompts(options.prompts) {
        Ok(prompts) => prompts,
        Err(error) => {
            eprintln!("model baseline failed: {error}");
            return 1;
        }
    };
    let cli = options
        .cli
        .map(PathBuf::from)
        .unwrap_or_else(default_cli_path);
    if !cli.is_file() {
        eprintln!(
            "model baseline refused: llama-cli not found: {}",
            cli.display()
        );
        return 1;
    }
    let backend_version = command_version(&cli).await;
    let mut cases = Vec::with_capacity(prompts.len());
    for (case_id, prompt) in prompts {
        let args = baseline_args(
            &artifact.artifact_path,
            &prompt,
            options.predict,
            options.gpu_layers,
            options.extra_args,
        );
        let output = match execute_case(&cli, &args, &case_id).await {
            Ok(output) => output,
            Err(error) => {
                eprintln!("model baseline case {case_id} failed: {error}");
                return 1;
            }
        };
        cases.push(lmml_substrate::BaselineCase {
            case_id,
            prompt,
            output_hash: lmml_substrate::sha256_data(output.as_bytes()),
            output,
        });
    }
    let created_at = match OffsetDateTime::now_utc().format(&Rfc3339) {
        Ok(created_at) => created_at,
        Err(error) => {
            eprintln!("model baseline failed to create timestamp: {error}");
            return 1;
        }
    };
    let manifest = lmml_substrate::BaselineManifest {
        schema_version: lmml_substrate::SCHEMA_VERSION,
        baseline_id: options.baseline_id.to_string(),
        model_lineage_id: artifact.artifact.model_lineage_id.clone(),
        artifact_id: artifact.artifact.artifact_id.clone(),
        artifact_hash: artifact.artifact.artifact_hash.clone(),
        backend: "llama.cpp".to_string(),
        backend_version,
        parameters: baseline_args(
            Path::new("<artifact>"),
            "<prompt>",
            options.predict,
            options.gpu_layers,
            options.extra_args,
        ),
        cases,
        created_at,
    };
    let output = options.output.map(PathBuf::from).unwrap_or_else(|| {
        data_root
            .join("lmml/models/baselines")
            .join(&manifest.model_lineage_id)
            .join(&manifest.baseline_id)
            .join("manifest.json")
    });
    if let Err(error) = lmml_substrate::store_baseline_manifest(&output, &manifest) {
        eprintln!("model baseline registration failed: {error}");
        return 1;
    }
    if options.json {
        match serde_json::to_string_pretty(&manifest) {
            Ok(payload) => println!("{payload}"),
            Err(error) => {
                eprintln!("could not serialize baseline manifest: {error}");
                return 1;
            }
        }
    } else {
        println!(
            "baseline {} recorded for {}\nmanifest: {}\ncases: {}",
            manifest.baseline_id,
            manifest.artifact_id,
            output.display(),
            manifest.cases.len()
        );
    }
    0
}

fn read_prompts(path: &Path) -> Result<Vec<(String, String)>, String> {
    let payload = std::fs::read_to_string(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    let values: Vec<serde_json::Value> = serde_json::from_str(&payload)
        .map_err(|error| format!("invalid prompt JSON {}: {error}", path.display()))?;
    let mut prompts = Vec::with_capacity(values.len());
    let mut case_ids = std::collections::BTreeSet::new();
    for value in values {
        let case_id = value
            .get("case_id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| "each prompt requires a string case_id".to_string())?;
        let prompt = value
            .get("prompt")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| "each prompt requires a string prompt".to_string())?;
        lmml_substrate::validate_identifier(case_id)
            .map_err(|error| format!("invalid baseline case_id {case_id}: {error}"))?;
        if prompt.is_empty() || !case_ids.insert(case_id.to_string()) {
            return Err("baseline case IDs must be unique and prompts non-empty".to_string());
        }
        prompts.push((case_id.to_string(), prompt.to_string()));
    }
    if prompts.is_empty() {
        return Err("baseline prompt set is empty".to_string());
    }
    Ok(prompts)
}

fn validate_extra_args(extra_args: &[String]) -> Result<(), String> {
    const RESERVED: &[&str] = &[
        "-m",
        "--model",
        "-p",
        "--prompt",
        "-f",
        "--file",
        "--prompt-file",
        "--random-prompt",
        "--temp",
        "--temperature",
        "--seed",
        "-n",
        "--predict",
        "--n-predict",
        "-ngl",
        "--gpu-layers",
        "--display-prompt",
        "--no-display-prompt",
        "--conversation",
        "--no-conversation",
        "--show-timings",
        "--no-show-timings",
        "--simple-io",
        "--no-warmup",
        "--log-disable",
    ];
    for argument in extra_args {
        let flag = argument
            .split_once('=')
            .map_or(argument.as_str(), |(flag, _)| flag);
        let reserved = RESERVED.contains(&flag)
            || ["-m", "-p", "-f", "-n"]
                .iter()
                .any(|short| flag.starts_with(short) && flag.len() > short.len());
        if reserved {
            return Err(format!(
                "extra argument {argument} overrides a fixed baseline parameter"
            ));
        }
    }
    Ok(())
}

fn baseline_args(
    artifact: &Path,
    prompt: &str,
    predict: u32,
    gpu_layers: i32,
    extra_args: &[String],
) -> Vec<String> {
    let mut args = vec![
        "-m".to_string(),
        artifact.to_string_lossy().into_owned(),
        "-p".to_string(),
        prompt.to_string(),
        "--temp".to_string(),
        "0".to_string(),
        "--seed".to_string(),
        "42".to_string(),
        "-n".to_string(),
        predict.to_string(),
        "-ngl".to_string(),
        gpu_layers.to_string(),
        "--no-display-prompt".to_string(),
        "--no-conversation".to_string(),
        "--no-show-timings".to_string(),
        "--simple-io".to_string(),
        "--no-warmup".to_string(),
        "--log-disable".to_string(),
    ];
    args.extend_from_slice(extra_args);
    args
}

async fn execute_case(program: &Path, args: &[String], case_id: &str) -> Result<String, String> {
    let mut child = Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("{}: {error}", program.display()))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| "llama-cli stdout pipe unavailable".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "llama-cli stderr pipe unavailable".to_string())?;
    let stderr_case = case_id.to_string();
    let stderr_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            eprintln!("[baseline:{stderr_case}] {line}");
        }
    });
    let mut output = Vec::new();
    stdout
        .read_to_end(&mut output)
        .await
        .map_err(|error| format!("could not read llama-cli output: {error}"))?;
    let status = child
        .wait()
        .await
        .map_err(|error| format!("could not wait for llama-cli: {error}"))?;
    stderr_task
        .await
        .map_err(|error| format!("stderr task failed: {error}"))?;
    if !status.success() {
        return Err(format!("llama-cli exited with {status}"));
    }
    String::from_utf8(output).map_err(|error| format!("llama-cli output is not UTF-8: {error}"))
}

async fn command_version(program: &Path) -> String {
    match Command::new(program).arg("--version").output().await {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let version = if stdout.trim().is_empty() {
                stderr.trim()
            } else {
                stdout.trim()
            };
            if version.is_empty() {
                "unknown".to_string()
            } else {
                version.to_string()
            }
        }
        Err(error) => format!("unknown ({error})"),
    }
}

fn default_cli_path() -> PathBuf {
    super::managed_data_root().join("lmml/llama.cpp/build/bin/llama-cli")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_extra_args_cannot_override_identity_or_sampling() {
        for arguments in [
            vec!["--model=/tmp/other.gguf".to_string()],
            vec!["--temp".to_string(), "1".to_string()],
            vec!["-n64".to_string()],
            vec!["--display-prompt".to_string()],
        ] {
            assert!(validate_extra_args(&arguments).is_err(), "{arguments:?}");
        }
        assert!(validate_extra_args(&[
            "--ctx-size".to_string(),
            "16384".to_string(),
            "--threads".to_string(),
            "8".to_string(),
        ])
        .is_ok());
    }

    #[test]
    fn baseline_prompt_set_rejects_duplicate_case_ids_before_execution() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("prompts.json");
        std::fs::write(
            &path,
            r#"[{"case_id":"same","prompt":"one"},{"case_id":"same","prompt":"two"}]"#,
        )
        .expect("prompts");

        assert!(read_prompts(&path).is_err());
    }
}
