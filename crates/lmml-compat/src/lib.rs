//! llama.cpp server compatibility layer for lmml.
//!
//! llama.cpp changes command-line flags over time. This crate owns all
//! knowledge of those spellings and exposes a stable [`ServerConfig`] plus
//! [`build_argv`] translator for the rest of lmml.

use std::collections::BTreeSet;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use thiserror::Error;

/// Result of running a command through a [`CommandRunner`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    /// Whether the command exited successfully.
    pub success: bool,
    /// UTF-8 decoded stdout, using lossy replacement for invalid bytes.
    pub stdout: String,
    /// UTF-8 decoded stderr, using lossy replacement for invalid bytes.
    pub stderr: String,
}

/// Process runner abstraction used by tests to probe fake llama-server binaries.
pub trait CommandRunner {
    /// Run `program` with `args` and capture stdout/stderr.
    fn run(&self, program: &Path, args: &[&str]) -> impl Future<Output = CommandOutput> + Send;
}

/// Command runner backed by [`tokio::process::Command`].
#[derive(Debug, Clone, Copy, Default)]
pub struct RealCommandRunner;

impl CommandRunner for RealCommandRunner {
    async fn run(&self, program: &Path, args: &[&str]) -> CommandOutput {
        match tokio::process::Command::new(program)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
        {
            Ok(output) => CommandOutput {
                success: output.status.success(),
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            },
            Err(error) => CommandOutput {
                success: false,
                stdout: String::new(),
                stderr: error.to_string(),
            },
        }
    }
}

/// Capabilities detected from a `llama-server` binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlamaBinaryCapabilities {
    /// Version string reported by `llama-server --version`, if available.
    pub version: Option<String>,
    /// Whether flash attention can be enabled.
    pub flash_attn: bool,
    /// Whether memory locking can be enabled.
    pub mlock: bool,
    /// Whether API key authentication can be configured.
    pub api_key: bool,
    /// Whether micro-batch size can be configured.
    pub ubatch_size: bool,
    /// Whether a chat template can be passed directly.
    pub chat_template: bool,
    /// Whether Jinja chat templates are supported.
    pub jinja: bool,
    /// Whether reranking mode is supported.
    pub reranking: bool,
    /// All CLI flags parsed from `llama-server --help`.
    ///
    /// This keeps flag spelling decisions centralized in this crate while still
    /// allowing [`build_argv`] to adapt to upstream aliases such as `-ngl` and
    /// `--n-gpu-layers`.
    pub flags: Vec<String>,
}

impl LlamaBinaryCapabilities {
    /// Run `llama-server --version` and `llama-server --help` to detect flags.
    #[tracing::instrument(fields(binary = %binary.display()))]
    pub async fn probe(binary: &Path) -> Result<Self, CompatError> {
        probe_with_runner(&RealCommandRunner, binary).await
    }

    fn has_any(&self, names: &[&str]) -> bool {
        names
            .iter()
            .any(|name| self.flags.iter().any(|flag| flag == name))
    }
}

/// Probe a llama-server binary with an injected command runner.
#[tracing::instrument(skip(runner), fields(binary = %binary.display()))]
pub async fn probe_with_runner<R>(
    runner: &R,
    binary: &Path,
) -> Result<LlamaBinaryCapabilities, CompatError>
where
    R: CommandRunner + Sync,
{
    let (version, help) = tokio::join!(
        runner.run(binary, &["--version"]),
        runner.run(binary, &["--help"])
    );

    if !help.success {
        return Err(CompatError::HelpFailed {
            binary: binary.to_path_buf(),
            stderr: non_empty_error(&help),
        });
    }

    let flags = parse_help_flags(&format!("{}\n{}", help.stdout, help.stderr));
    let version = if version.success {
        parse_version_line(&version.stdout, &version.stderr)
    } else {
        None
    };

    let caps = capabilities_from_flags(version, flags);
    tracing::info!(
        version = ?caps.version,
        flags = caps.flags.len(),
        "llama-server capabilities probed"
    );
    Ok(caps)
}

/// Stable internal server configuration, independent of llama.cpp flag spelling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    /// GGUF model path.
    pub model: PathBuf,
    /// HTTP listen port.
    pub port: u16,
    /// HTTP listen host.
    pub host: String,
    /// Context size in tokens.
    pub ctx_size: u32,
    /// GPU layers to offload; `-1` means caller-selected auto behavior.
    pub n_gpu_layers: i32,
    /// Prompt processing batch size.
    pub batch_size: u32,
    /// Physical micro-batch size.
    pub ubatch_size: u32,
    /// Worker thread count.
    pub threads: usize,
    /// Enable flash attention when supported by the binary.
    pub flash_attn: bool,
    /// Lock model memory with mlock when supported by the binary.
    pub mlock: bool,
    /// Optional API key for server authentication.
    pub api_key: Option<String>,
    /// Optional chat template content or name.
    pub chat_template: Option<String>,
    /// Enable Jinja chat template processing when supported.
    pub jinja: bool,
    /// Extra caller-supplied arguments appended last.
    pub extra_args: Vec<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            model: PathBuf::new(),
            port: 8080,
            host: "127.0.0.1".to_string(),
            ctx_size: 4096,
            n_gpu_layers: -1,
            batch_size: 512,
            ubatch_size: 512,
            threads: 0,
            flash_attn: true,
            mlock: false,
            api_key: None,
            chat_template: None,
            jinja: false,
            extra_args: Vec::new(),
        }
    }
}

/// Translate a [`ServerConfig`] into argv for the detected llama-server binary.
pub fn build_argv(config: &ServerConfig, caps: &LlamaBinaryCapabilities) -> Vec<String> {
    let mut argv = Vec::new();

    push_value(
        &mut argv,
        caps,
        &[FlagName::Long("--model"), FlagName::Short("-m")],
        config.model.to_string_lossy().into_owned(),
    );
    push_value(
        &mut argv,
        caps,
        &[FlagName::Long("--host")],
        config.host.clone(),
    );
    push_value(
        &mut argv,
        caps,
        &[FlagName::Long("--port")],
        config.port.to_string(),
    );
    push_value(
        &mut argv,
        caps,
        &[
            FlagName::Long("--ctx-size"),
            FlagName::Long("--context-size"),
            FlagName::Short("-c"),
        ],
        config.ctx_size.to_string(),
    );
    push_value(
        &mut argv,
        caps,
        &[FlagName::Short("-ngl"), FlagName::Long("--n-gpu-layers")],
        config.n_gpu_layers.to_string(),
    );
    push_value(
        &mut argv,
        caps,
        &[FlagName::Long("--batch-size"), FlagName::Short("-b")],
        config.batch_size.to_string(),
    );
    if config.ubatch_size > 0 && caps.ubatch_size {
        push_value(
            &mut argv,
            caps,
            &[
                FlagName::Long("--ubatch-size"),
                FlagName::Long("--micro-batch-size"),
                FlagName::Short("-ub"),
            ],
            config.ubatch_size.to_string(),
        );
    }
    if config.threads > 0 {
        push_value(
            &mut argv,
            caps,
            &[FlagName::Long("--threads"), FlagName::Short("-t")],
            config.threads.to_string(),
        );
    }
    if config.flash_attn && caps.flash_attn {
        push_switch(
            &mut argv,
            caps,
            &[FlagName::Long("--flash-attn"), FlagName::Long("-fa")],
        );
    }
    if config.mlock && caps.mlock {
        push_switch(&mut argv, caps, &[FlagName::Long("--mlock")]);
    }
    if let Some(api_key) = config.api_key.as_ref().filter(|key| !key.is_empty()) {
        if caps.api_key {
            push_value(
                &mut argv,
                caps,
                &[FlagName::Long("--api-key")],
                api_key.clone(),
            );
        }
    }
    // llama-server accepts arbitrary template files only when --jinja appears
    // before --chat-template-file on the command line.
    if config.jinja && caps.jinja {
        push_switch(&mut argv, caps, &[FlagName::Long("--jinja")]);
    }
    if let Some(chat_template) = config
        .chat_template
        .as_ref()
        .filter(|template| !template.is_empty())
    {
        if caps.chat_template {
            push_value(
                &mut argv,
                caps,
                &[
                    FlagName::Long("--chat-template"),
                    FlagName::Long("--chat-template-file"),
                ],
                chat_template.clone(),
            );
        }
    }

    argv.extend(config.extra_args.iter().cloned());
    argv
}

/// Return warnings for requested config settings unsupported by the binary.
pub fn unsupported_warnings(
    config: &ServerConfig,
    caps: &LlamaBinaryCapabilities,
) -> Vec<DetectionWarning> {
    let mut warnings = Vec::new();
    if config.flash_attn && !caps.flash_attn {
        warnings.push(DetectionWarning::unsupported("--flash-attn"));
    }
    if config.mlock && !caps.mlock {
        warnings.push(DetectionWarning::unsupported("--mlock"));
    }
    if config.api_key.as_ref().is_some_and(|key| !key.is_empty()) && !caps.api_key {
        warnings.push(DetectionWarning::unsupported("--api-key"));
    }
    if config.ubatch_size > 0 && !caps.ubatch_size {
        warnings.push(DetectionWarning::unsupported("--ubatch-size"));
    }
    if config
        .chat_template
        .as_ref()
        .is_some_and(|template| !template.is_empty())
        && !caps.chat_template
    {
        warnings.push(DetectionWarning::unsupported("--chat-template"));
    }
    if config.jinja && !caps.jinja {
        warnings.push(DetectionWarning::unsupported("--jinja"));
    }
    warnings
}

/// Warning surfaced when a requested setting is not available in the binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectionWarning {
    /// Human-readable warning message.
    pub message: String,
}

impl DetectionWarning {
    fn unsupported(flag: &'static str) -> Self {
        Self {
            message: format!("{flag} not available in this llama-server build"),
        }
    }
}

/// Error returned while probing a llama-server binary.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum CompatError {
    /// `llama-server --help` failed, so capabilities could not be detected.
    #[error("failed to run {binary} --help: {stderr}")]
    HelpFailed {
        /// Binary path that was probed.
        binary: PathBuf,
        /// Captured stderr or fallback error text.
        stderr: String,
    },
}

#[derive(Debug, Clone, Copy)]
enum FlagName {
    Short(&'static str),
    Long(&'static str),
}

impl FlagName {
    fn as_str(self) -> &'static str {
        match self {
            FlagName::Short(name) | FlagName::Long(name) => name,
        }
    }
}

fn capabilities_from_flags(version: Option<String>, flags: Vec<String>) -> LlamaBinaryCapabilities {
    let mut caps = LlamaBinaryCapabilities {
        version,
        flash_attn: false,
        mlock: false,
        api_key: false,
        ubatch_size: false,
        chat_template: false,
        jinja: false,
        reranking: false,
        flags,
    };
    caps.flash_attn = caps.has_any(&["--flash-attn", "-fa"]);
    caps.mlock = caps.has_any(&["--mlock"]);
    caps.api_key = caps.has_any(&["--api-key"]);
    caps.ubatch_size = caps.has_any(&["--ubatch-size", "--micro-batch-size", "-ub"]);
    caps.chat_template = caps.has_any(&["--chat-template", "--chat-template-file"]);
    caps.jinja = caps.has_any(&["--jinja"]);
    caps.reranking = caps.has_any(&["--reranking", "--rerank"]);
    caps
}

fn push_value(
    argv: &mut Vec<String>,
    caps: &LlamaBinaryCapabilities,
    candidates: &[FlagName],
    value: String,
) {
    if let Some(flag) = select_flag(caps, candidates) {
        argv.push(flag.to_string());
        argv.push(value);
    }
}

fn push_switch(argv: &mut Vec<String>, caps: &LlamaBinaryCapabilities, candidates: &[FlagName]) {
    if let Some(flag) = select_flag(caps, candidates) {
        argv.push(flag.to_string());
    }
}

fn select_flag(caps: &LlamaBinaryCapabilities, candidates: &[FlagName]) -> Option<&'static str> {
    candidates
        .iter()
        .map(|candidate| candidate.as_str())
        .find(|flag| caps.flags.iter().any(|available| available == flag))
}

fn parse_help_flags(help: &str) -> Vec<String> {
    let mut flags = BTreeSet::new();
    for raw in help.split_whitespace() {
        let token = raw.trim_matches(|ch: char| {
            matches!(
                ch,
                ',' | ';' | ':' | '[' | ']' | '(' | ')' | '{' | '}' | '<' | '>' | '='
            )
        });
        if is_flag_token(token) {
            let flag = token
                .split_once('=')
                .map_or(token, |(name, _)| name)
                .trim_end_matches(',');
            flags.insert(flag.to_string());
        }
    }
    flags.into_iter().collect()
}

fn is_flag_token(token: &str) -> bool {
    if token.len() < 2 || !token.starts_with('-') {
        return false;
    }
    token
        .chars()
        .find(|ch| *ch != '-')
        .is_some_and(|ch| ch.is_ascii_alphabetic())
}

fn parse_version_line(stdout: &str, stderr: &str) -> Option<String> {
    stdout
        .lines()
        .chain(stderr.lines())
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(ToOwned::to_owned)
}

fn non_empty_error(output: &CommandOutput) -> String {
    output
        .stderr
        .lines()
        .chain(output.stdout.lines())
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("unknown error")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct FakeRunner {
        outputs: Arc<Mutex<HashMap<String, CommandOutput>>>,
    }

    impl FakeRunner {
        fn with(self, program: &Path, args: &[&str], output: CommandOutput) -> Self {
            self.outputs
                .lock()
                .expect("lock outputs")
                .insert(Self::key(program, args), output);
            self
        }

        fn success(stdout: &str) -> CommandOutput {
            CommandOutput {
                success: true,
                stdout: stdout.to_string(),
                stderr: String::new(),
            }
        }

        fn failure(stderr: &str) -> CommandOutput {
            CommandOutput {
                success: false,
                stdout: String::new(),
                stderr: stderr.to_string(),
            }
        }

        fn key(program: &Path, args: &[&str]) -> String {
            format!("{}\0{}", program.display(), args.join("\0"))
        }
    }

    impl CommandRunner for FakeRunner {
        async fn run(&self, program: &Path, args: &[&str]) -> CommandOutput {
            self.outputs
                .lock()
                .expect("lock outputs")
                .get(&Self::key(program, args))
                .cloned()
                .unwrap_or_else(|| FakeRunner::failure("not found"))
        }
    }

    #[test]
    fn parses_help_flags_and_capabilities() {
        let flags = parse_help_flags(
            r#"
            -m, --model FNAME
            --host HOST
            --port PORT
            -c, --ctx-size N
            -ngl, --n-gpu-layers N
            -b, --batch-size N
            -ub, --ubatch-size N
            -fa, --flash-attn
            --mlock
            --api-key KEY
            --chat-template TEMPLATE
            --jinja
            --reranking
            "#,
        );
        let caps = capabilities_from_flags(Some("version".to_string()), flags.clone());
        assert!(flags.contains(&"-ngl".to_string()));
        assert!(flags.contains(&"--n-gpu-layers".to_string()));
        assert!(caps.flash_attn);
        assert!(caps.mlock);
        assert!(caps.api_key);
        assert!(caps.ubatch_size);
        assert!(caps.chat_template);
        assert!(caps.jinja);
        assert!(caps.reranking);
    }

    #[tokio::test]
    async fn probes_binary_with_runner() {
        let binary = PathBuf::from("/tmp/llama-server");
        let runner = FakeRunner::default()
            .with(
                &binary,
                &["--version"],
                FakeRunner::success("llama build 123\n"),
            )
            .with(
                &binary,
                &["--help"],
                FakeRunner::success("--model FNAME --port PORT -ngl N --flash-attn\n"),
            );

        let caps = probe_with_runner(&runner, &binary)
            .await
            .expect("probe should succeed");
        assert_eq!(caps.version, Some("llama build 123".to_string()));
        assert!(caps.flash_attn);
        assert!(caps.has_any(&["--model"]));
    }

    #[tokio::test]
    async fn probe_fails_when_help_fails() {
        let binary = PathBuf::from("/tmp/missing-server");
        let runner = FakeRunner::default()
            .with(&binary, &["--version"], FakeRunner::failure("missing"))
            .with(&binary, &["--help"], FakeRunner::failure("missing"));

        assert_eq!(
            probe_with_runner(&runner, &binary).await,
            Err(CompatError::HelpFailed {
                binary,
                stderr: "missing".to_string(),
            })
        );
    }

    #[test]
    fn builds_argv_with_new_flag_spellings() {
        let caps = capabilities_from_flags(
            Some("new".to_string()),
            parse_help_flags(
                "--model --host --port --ctx-size -ngl --batch-size --ubatch-size --threads --flash-attn --mlock --api-key --chat-template --jinja",
            ),
        );
        let config = full_config();
        assert_eq!(
            build_argv(&config, &caps),
            vec![
                "--model",
                "/models/mistral.gguf",
                "--host",
                "0.0.0.0",
                "--port",
                "8081",
                "--ctx-size",
                "8192",
                "-ngl",
                "35",
                "--batch-size",
                "256",
                "--ubatch-size",
                "128",
                "--threads",
                "12",
                "--flash-attn",
                "--mlock",
                "--api-key",
                "secret",
                "--jinja",
                "--chat-template",
                "chatml",
                "--verbose",
            ]
        );
    }

    #[test]
    fn builds_argv_with_legacy_aliases() {
        let caps = capabilities_from_flags(
            Some("old".to_string()),
            parse_help_flags("-m --host --port -c --n-gpu-layers -b -t"),
        );
        let mut config = full_config();
        config.flash_attn = false;
        config.mlock = false;
        config.api_key = None;
        config.chat_template = None;
        config.jinja = false;
        config.extra_args.clear();

        assert_eq!(
            build_argv(&config, &caps),
            vec![
                "-m",
                "/models/mistral.gguf",
                "--host",
                "0.0.0.0",
                "--port",
                "8081",
                "-c",
                "8192",
                "--n-gpu-layers",
                "35",
                "-b",
                "256",
                "-t",
                "12",
            ]
        );
    }

    #[test]
    fn omits_unsupported_optional_flags_and_reports_warnings() {
        let caps = capabilities_from_flags(
            Some("minimal".to_string()),
            parse_help_flags("-m --port -c -ngl -b"),
        );
        let config = full_config();

        let argv = build_argv(&config, &caps);
        assert!(!argv.contains(&"--flash-attn".to_string()));
        assert!(!argv.contains(&"--api-key".to_string()));

        assert_eq!(
            unsupported_warnings(&config, &caps),
            vec![
                DetectionWarning::unsupported("--flash-attn"),
                DetectionWarning::unsupported("--mlock"),
                DetectionWarning::unsupported("--api-key"),
                DetectionWarning::unsupported("--ubatch-size"),
                DetectionWarning::unsupported("--chat-template"),
                DetectionWarning::unsupported("--jinja"),
            ]
        );
    }

    fn full_config() -> ServerConfig {
        ServerConfig {
            model: PathBuf::from("/models/mistral.gguf"),
            port: 8081,
            host: "0.0.0.0".to_string(),
            ctx_size: 8192,
            n_gpu_layers: 35,
            batch_size: 256,
            ubatch_size: 128,
            threads: 12,
            flash_attn: true,
            mlock: true,
            api_key: Some("secret".to_string()),
            chat_template: Some("chatml".to_string()),
            jinja: true,
            extra_args: vec!["--verbose".to_string()],
        }
    }
}
