//! Headless OCR command support built around llama.cpp `llama-mtmd-cli`.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;

/// Request parsed from `lmml ocr`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OcrRequest {
    /// Image path to process.
    pub image: PathBuf,
    /// Optional language model GGUF. Defaults to the selected LMML model.
    pub model: Option<PathBuf>,
    /// Optional multimodal projector GGUF. Defaults beside the model.
    pub mmproj: Option<PathBuf>,
    /// OCR prompt passed to llama.cpp. Defaults to the active OCR profile.
    pub prompt: Option<String>,
    /// Context size in tokens. Defaults to the active OCR profile.
    pub ctx_size: Option<u32>,
    /// Maximum generated tokens. Defaults to the active OCR profile.
    pub predict: Option<u32>,
    /// Enable llama.cpp warmup before inference.
    pub warmup: bool,
    /// Extra `llama-mtmd-cli` arguments appended last.
    pub extra_args: Vec<String>,
}

/// Run `llama-mtmd-cli` with Unlimited-OCR defaults.
pub(crate) async fn run_ocr(request: OcrRequest) -> i32 {
    let mut state = match lmml_state::AppState::load_existing_or_default() {
        Ok(state) => state,
        Err(error) => {
            eprintln!("state load failed: {error}");
            return 1;
        }
    };
    state.model.ensure_builtin_profiles();

    let model = request
        .model
        .unwrap_or_else(|| state.model.last_used.clone());
    if model.as_os_str().is_empty() {
        eprintln!("ocr failed: pass --model or select an Unlimited-OCR GGUF in lmml first");
        return 2;
    }
    let profile = state
        .model
        .ocr_profile_for_path(&model)
        .cloned()
        .unwrap_or_else(|| fallback_unlimited_ocr_profile(&model));
    let mmproj = request
        .mmproj
        .unwrap_or_else(|| resolve_profile_path_beside_model(&model, &profile.mmproj));
    let binary = mtmd_binary_path(&state.build.binary);
    let prompt = request.prompt.unwrap_or_else(|| profile.prompt.clone());
    let ctx_size = request.ctx_size.unwrap_or(profile.ctx_size);
    let predict = request.predict.unwrap_or(profile.predict);
    let no_warmup = profile.no_warmup && !request.warmup;
    let mut extra_args = profile.extra_args.clone();
    extra_args.extend(request.extra_args);

    if let Err(error) = validate_ocr_paths(&binary, &model, &mmproj, &request.image) {
        eprintln!("ocr failed: {error}");
        return 2;
    }

    let capabilities = match detect_mtmd_capabilities(&binary).await {
        Ok(capabilities) => capabilities,
        Err(error) => {
            eprintln!("failed to inspect {}: {error}", binary.display());
            return 1;
        }
    };
    let argv = match build_ocr_argv(
        OcrArgInputs {
            model: &model,
            mmproj: &mmproj,
            image: &request.image,
            prompt: &prompt,
            chat_template: &profile.chat_template,
            temp: &profile.temp,
            repeat_penalty: &profile.repeat_penalty,
            flash_attn: &profile.flash_attn,
            ctx_size,
            predict,
            no_warmup,
            extra_args: &extra_args,
        },
        &capabilities,
    ) {
        Ok(argv) => argv,
        Err(error) => {
            eprintln!("ocr failed: {error}");
            return 2;
        }
    };

    match tokio::process::Command::new(&binary)
        .args(&argv)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .await
    {
        Ok(status) => status.code().unwrap_or(1),
        Err(error) => {
            eprintln!("failed to start {}: {error}", binary.display());
            1
        }
    }
}

fn validate_ocr_paths(
    binary: &Path,
    model: &Path,
    mmproj: &Path,
    image: &Path,
) -> Result<(), String> {
    if !binary.is_file() {
        return Err(format!(
            "llama-mtmd-cli does not exist at {}; rebuild llama.cpp from a post-PR #24969 checkout",
            binary.display()
        ));
    }
    if !model.is_file() {
        return Err(format!("model does not exist: {}", model.display()));
    }
    if !mmproj.is_file() {
        return Err(format!(
            "multimodal projector does not exist: {}; place mmproj-unlimited-ocr-F16.gguf beside the model or pass --mmproj",
            mmproj.display()
        ));
    }
    if !image.is_file() {
        return Err(format!("image does not exist: {}", image.display()));
    }
    Ok(())
}

fn default_unlimited_ocr_mmproj_path(model: &Path) -> PathBuf {
    model
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join("mmproj-unlimited-ocr-F16.gguf")
}

fn fallback_unlimited_ocr_profile(model: &Path) -> lmml_state::OcrRuntimeProfile {
    lmml_state::OcrRuntimeProfile {
        name: "unlimited-ocr-default-mtmd".to_string(),
        model: model.to_path_buf(),
        mmproj: PathBuf::from("mmproj-unlimited-ocr-F16.gguf"),
        prompt: "document parsing.".to_string(),
        chat_template: "deepseek-ocr".to_string(),
        temp: "0".to_string(),
        repeat_penalty: "1.0".to_string(),
        flash_attn: "off".to_string(),
        ctx_size: 16_384,
        predict: 2_600,
        no_warmup: true,
        extra_args: Vec::new(),
    }
}

fn resolve_profile_path_beside_model(model: &Path, profile_path: &Path) -> PathBuf {
    if profile_path.as_os_str().is_empty() {
        return default_unlimited_ocr_mmproj_path(model);
    }
    if profile_path.is_absolute() {
        return profile_path.to_path_buf();
    }
    model
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join(profile_path)
}

fn mtmd_binary_path(server_binary: &Path) -> PathBuf {
    sibling_llama_binary_path(server_binary, "llama-mtmd-cli")
}

fn sibling_llama_binary_path(server_binary: &Path, binary: &str) -> PathBuf {
    server_binary
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join(binary_name(binary))
}

fn binary_name(base: &str) -> String {
    if cfg!(windows) {
        format!("{base}.exe")
    } else {
        base.to_string()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct MtmdCapabilities {
    model: bool,
    mmproj: bool,
    image: bool,
    prompt: bool,
    chat_template: bool,
    temp: bool,
    repeat_penalty: bool,
    flash_attn: bool,
    predict: bool,
    ctx_size: bool,
    no_warmup: bool,
}

impl MtmdCapabilities {
    fn from_help(help: &str) -> Self {
        Self {
            model: has_any(help, &["-m", "--model"]),
            mmproj: help.contains("--mmproj"),
            image: help.contains("--image"),
            prompt: has_any(help, &["-p", "--prompt"]),
            chat_template: help.contains("--chat-template"),
            temp: has_any(help, &["--temp", "--temperature"]),
            repeat_penalty: help.contains("--repeat-penalty"),
            flash_attn: has_any(help, &["--flash-attn", "-fa"]),
            predict: has_any(help, &["-n", "--predict"]),
            ctx_size: has_any(help, &["-c", "--ctx-size", "--context-size"]),
            no_warmup: help.contains("--no-warmup"),
        }
    }
}

fn has_any(haystack: &str, needles: &[&str]) -> bool {
    let tokens = haystack.split(|character: char| {
        character.is_whitespace()
            || matches!(
                character,
                ',' | '=' | '[' | ']' | '<' | '>' | '(' | ')' | '{' | '}' | ':'
            )
    });
    tokens
        .filter(|token| !token.is_empty())
        .any(|token| needles.contains(&token))
}

async fn detect_mtmd_capabilities(binary: &Path) -> io::Result<MtmdCapabilities> {
    let output = tokio::process::Command::new(binary)
        .arg("--help")
        .output()
        .await?;
    let mut help = String::from_utf8_lossy(&output.stdout).into_owned();
    help.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok(MtmdCapabilities::from_help(&help))
}

struct OcrArgInputs<'a> {
    model: &'a Path,
    mmproj: &'a Path,
    image: &'a Path,
    prompt: &'a str,
    chat_template: &'a str,
    temp: &'a str,
    repeat_penalty: &'a str,
    flash_attn: &'a str,
    ctx_size: u32,
    predict: u32,
    no_warmup: bool,
    extra_args: &'a [String],
}

fn build_ocr_argv(
    inputs: OcrArgInputs<'_>,
    capabilities: &MtmdCapabilities,
) -> Result<Vec<String>, String> {
    let missing = required_missing(capabilities);
    if !missing.is_empty() {
        return Err(format!(
            "installed llama-mtmd-cli is missing required flags: {}; update and rebuild llama.cpp",
            missing.join(", ")
        ));
    }

    let mut argv = vec![
        "-m".to_string(),
        inputs.model.to_string_lossy().into_owned(),
        "--mmproj".to_string(),
        inputs.mmproj.to_string_lossy().into_owned(),
        "--image".to_string(),
        inputs.image.to_string_lossy().into_owned(),
        "-p".to_string(),
        inputs.prompt.to_string(),
        "--chat-template".to_string(),
        inputs.chat_template.to_string(),
        "--temp".to_string(),
        inputs.temp.to_string(),
        "--repeat-penalty".to_string(),
        inputs.repeat_penalty.to_string(),
        "--flash-attn".to_string(),
        inputs.flash_attn.to_string(),
        "-n".to_string(),
        inputs.predict.to_string(),
        "-c".to_string(),
        inputs.ctx_size.to_string(),
    ];
    if inputs.no_warmup && capabilities.no_warmup {
        argv.push("--no-warmup".to_string());
    }
    argv.extend(inputs.extra_args.iter().cloned());
    Ok(argv)
}

fn required_missing(capabilities: &MtmdCapabilities) -> Vec<&'static str> {
    [
        (capabilities.model, "-m/--model"),
        (capabilities.mmproj, "--mmproj"),
        (capabilities.image, "--image"),
        (capabilities.prompt, "-p/--prompt"),
        (capabilities.chat_template, "--chat-template"),
        (capabilities.temp, "--temp/--temperature"),
        (capabilities.repeat_penalty, "--repeat-penalty"),
        (capabilities.flash_attn, "--flash-attn"),
        (capabilities.predict, "-n/--predict"),
        (capabilities.ctx_size, "-c/--ctx-size"),
    ]
    .into_iter()
    .filter_map(|(available, label)| (!available).then_some(label))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn full_capabilities() -> MtmdCapabilities {
        MtmdCapabilities {
            model: true,
            mmproj: true,
            image: true,
            prompt: true,
            chat_template: true,
            temp: true,
            repeat_penalty: true,
            flash_attn: true,
            predict: true,
            ctx_size: true,
            no_warmup: true,
        }
    }

    #[test]
    fn default_projector_lives_beside_model() {
        assert_eq!(
            default_unlimited_ocr_mmproj_path(Path::new("/models/unlimited-ocr-Q8_0.gguf")),
            PathBuf::from("/models/mmproj-unlimited-ocr-F16.gguf")
        );
    }

    #[test]
    fn relative_profile_projector_lives_beside_model() {
        assert_eq!(
            resolve_profile_path_beside_model(
                Path::new("/models/unlimited-ocr-Q8_0.gguf"),
                Path::new("mmproj-unlimited-ocr-F16.gguf")
            ),
            PathBuf::from("/models/mmproj-unlimited-ocr-F16.gguf")
        );
    }

    #[test]
    fn mtmd_binary_lives_next_to_server_binary() {
        assert_eq!(
            mtmd_binary_path(Path::new("/lmml/llama.cpp/build/bin/llama-server")),
            PathBuf::from("/lmml/llama.cpp/build/bin").join(binary_name("llama-mtmd-cli"))
        );
    }

    #[test]
    fn builds_unlimited_ocr_q8_reference_argv() {
        let argv = build_ocr_argv(
            OcrArgInputs {
                model: Path::new("/models/unlimited-ocr-Q8_0.gguf"),
                mmproj: Path::new("/models/mmproj-unlimited-ocr-F16.gguf"),
                image: Path::new("/tmp/page.png"),
                prompt: "document parsing.",
                chat_template: "deepseek-ocr",
                temp: "0",
                repeat_penalty: "1.0",
                flash_attn: "off",
                ctx_size: 16_384,
                predict: 2_600,
                no_warmup: true,
                extra_args: &["--verbose".to_string()],
            },
            &full_capabilities(),
        )
        .expect("ocr argv");

        assert_eq!(
            argv,
            vec![
                "-m",
                "/models/unlimited-ocr-Q8_0.gguf",
                "--mmproj",
                "/models/mmproj-unlimited-ocr-F16.gguf",
                "--image",
                "/tmp/page.png",
                "-p",
                "document parsing.",
                "--chat-template",
                "deepseek-ocr",
                "--temp",
                "0",
                "--repeat-penalty",
                "1.0",
                "--flash-attn",
                "off",
                "-n",
                "2600",
                "-c",
                "16384",
                "--no-warmup",
                "--verbose",
            ]
        );
    }

    #[test]
    fn omits_no_warmup_when_binary_does_not_advertise_it() {
        let mut capabilities = full_capabilities();
        capabilities.no_warmup = false;

        let argv = build_ocr_argv(
            OcrArgInputs {
                model: Path::new("/models/unlimited-ocr-Q8_0.gguf"),
                mmproj: Path::new("/models/mmproj-unlimited-ocr-F16.gguf"),
                image: Path::new("/tmp/page.png"),
                prompt: "document parsing.",
                chat_template: "deepseek-ocr",
                temp: "0",
                repeat_penalty: "1.0",
                flash_attn: "off",
                ctx_size: 16_384,
                predict: 2_600,
                no_warmup: true,
                extra_args: &[],
            },
            &capabilities,
        )
        .expect("ocr argv");

        assert!(!argv.iter().any(|arg| arg == "--no-warmup"));
    }

    #[test]
    fn rejects_old_mtmd_binary_without_required_flags() {
        let mut capabilities = full_capabilities();
        capabilities.chat_template = false;
        capabilities.flash_attn = false;

        let error = build_ocr_argv(
            OcrArgInputs {
                model: Path::new("/models/unlimited-ocr-Q8_0.gguf"),
                mmproj: Path::new("/models/mmproj-unlimited-ocr-F16.gguf"),
                image: Path::new("/tmp/page.png"),
                prompt: "document parsing.",
                chat_template: "deepseek-ocr",
                temp: "0",
                repeat_penalty: "1.0",
                flash_attn: "off",
                ctx_size: 16_384,
                predict: 2_600,
                no_warmup: true,
                extra_args: &[],
            },
            &capabilities,
        )
        .expect_err("missing flags");

        assert!(error.contains("--chat-template"));
        assert!(error.contains("--flash-attn"));
    }
}
