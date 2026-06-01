//! Build management for llama.cpp.
//!
//! This crate clones or updates llama.cpp, assembles CMake flags from detected
//! hardware, runs CMake with streaming output, verifies the resulting binaries,
//! and exposes rebuild fingerprint helpers for persistent state.

use std::future::Future;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use lmml_detect::BuildBackend;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::io::AsyncBufReadExt;
use tokio::sync::mpsc;

const LLAMA_CPP_URL: &str = "https://github.com/ggml-org/llama.cpp.git";
const DEFAULT_LOG_TAIL_LINES: usize = 500;

/// Build configuration for a llama.cpp source tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildConfig {
    /// Directory where llama.cpp source should exist.
    pub source_dir: PathBuf,
    /// Optional ref passed to `git checkout` after clone/update.
    pub git_ref: Option<String>,
    /// Recommended hardware backend.
    pub backend: BuildBackend,
    /// Optional sccache executable path.
    pub sccache: Option<PathBuf>,
    /// Extra CMake flags appended after generated flags.
    pub extra_cmake_flags: Vec<String>,
    /// Parallel build jobs. `0` means use available parallelism.
    pub jobs: usize,
    /// Remove the build directory before configuring.
    pub clean: bool,
    /// Number of failure log lines retained in [`BuildEvent::Failed`].
    pub log_tail_lines: usize,
}

impl BuildConfig {
    /// Create a build config with conservative defaults for a source directory.
    pub fn new(source_dir: PathBuf, backend: BuildBackend) -> Self {
        Self {
            source_dir,
            git_ref: None,
            backend,
            sccache: None,
            extra_cmake_flags: Vec::new(),
            jobs: 0,
            clean: false,
            log_tail_lines: DEFAULT_LOG_TAIL_LINES,
        }
    }
}

/// Build progress event emitted by [`BuildRunner`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildEvent {
    /// llama.cpp is being cloned from the given URL.
    Cloning {
        /// Repository URL.
        url: String,
    },
    /// CMake configuration has started.
    CmakeConfiguring,
    /// Compiler output line.
    Compiling {
        /// Raw stdout/stderr line.
        line: String,
    },
    /// Link phase was observed.
    Linking,
    /// Build completed and produced a server binary.
    Completed {
        /// Built `llama-server` binary.
        binary: PathBuf,
        /// Total elapsed build time.
        elapsed: Duration,
    },
    /// Build failed with a human-readable error and recent log lines.
    Failed {
        /// Primary error message.
        last_error: String,
        /// Recent build output.
        log_tail: Vec<String>,
    },
}

/// Runner abstraction for executing a build and streaming events.
pub trait BuildRunner {
    /// Start the build and return a receiver for progress events.
    fn run(&self, config: BuildConfig) -> impl Future<Output = mpsc::Receiver<BuildEvent>> + Send;
}

/// Real build runner backed by `git` and `cmake` subprocesses.
#[derive(Debug, Clone, Copy, Default)]
pub struct RealBuildRunner;

impl BuildRunner for RealBuildRunner {
    async fn run(&self, config: BuildConfig) -> mpsc::Receiver<BuildEvent> {
        let (tx, rx) = mpsc::channel(256);
        tokio::spawn(async move {
            run_build(config, tx).await;
        });
        rx
    }
}

/// Fingerprint used to decide if a source tree needs rebuilding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildFingerprint {
    /// Resolved source commit.
    pub commit: String,
    /// SHA-256 hash of the full CMake argv.
    pub cmake_hash: [u8; 32],
    /// Expected built binary.
    pub binary: PathBuf,
}

impl BuildFingerprint {
    /// Return true when the expected binary is missing or not executable.
    pub fn needs_rebuild(&self) -> bool {
        !is_executable(&self.binary)
    }
}

/// Update status for an existing llama.cpp checkout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateCheck {
    /// Local checkout already matches upstream.
    UpToDate {
        /// Current commit.
        current: String,
    },
    /// A newer upstream commit is available.
    Available {
        /// Current local commit.
        current: String,
        /// Latest upstream commit.
        latest: String,
        /// Number of commits local is behind upstream.
        commits_behind: usize,
    },
    /// Update check could not be completed.
    Unreachable {
        /// Human-readable reason.
        reason: String,
    },
}

/// Error returned by build helpers.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum BuildError {
    /// A subprocess failed to start or exited unsuccessfully.
    #[error("{0}")]
    Command(String),
    /// Filesystem operation failed.
    #[error("{0}")]
    Filesystem(String),
    /// Build verification failed.
    #[error("{0}")]
    Verification(String),
}

/// Assemble the full CMake configure argv for a build config.
pub fn cmake_configure_args(config: &BuildConfig) -> Vec<String> {
    let build_dir = config.source_dir.join("build");
    let mut args = vec![
        "-S".to_string(),
        config.source_dir.to_string_lossy().into_owned(),
        "-B".to_string(),
        build_dir.to_string_lossy().into_owned(),
        "-DCMAKE_BUILD_TYPE=Release".to_string(),
        "-DLLAMA_BUILD_SERVER=ON".to_string(),
    ];

    match &config.backend {
        BuildBackend::Cuda { archs } => {
            args.push("-DGGML_CUDA=ON".to_string());
            if !archs.is_empty() {
                args.push(format!("-DCMAKE_CUDA_ARCHITECTURES={}", archs.join(";")));
            }
        }
        BuildBackend::Metal => args.push("-DGGML_METAL=ON".to_string()),
        BuildBackend::CpuAvx2 => args.push("-DGGML_AVX2=ON".to_string()),
        BuildBackend::CpuAvx => args.push("-DGGML_AVX=ON".to_string()),
        BuildBackend::CpuFallback => {}
    }

    if let Some(sccache) = &config.sccache {
        let launcher = sccache.to_string_lossy();
        args.push(format!("-DCMAKE_C_COMPILER_LAUNCHER={launcher}"));
        args.push(format!("-DCMAKE_CXX_COMPILER_LAUNCHER={launcher}"));
    }

    args.extend(config.extra_cmake_flags.iter().cloned());
    args
}

/// Hash the exact CMake configure argv.
pub fn cmake_hash(args: &[String]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for arg in args {
        hasher.update(arg.as_bytes());
        hasher.update([0]);
    }
    hasher.finalize().into()
}

/// Build a fingerprint from commit, CMake args, and expected binary path.
pub fn build_fingerprint(
    commit: impl Into<String>,
    cmake_args: &[String],
    binary: PathBuf,
) -> BuildFingerprint {
    BuildFingerprint {
        commit: commit.into(),
        cmake_hash: cmake_hash(cmake_args),
        binary,
    }
}

/// Check whether an existing checkout is behind its upstream remote.
pub async fn check_for_update(source_dir: &Path) -> UpdateCheck {
    match check_for_update_with_git(source_dir).await {
        Ok(update) => update,
        Err(error) => UpdateCheck::Unreachable {
            reason: error.to_string(),
        },
    }
}

async fn check_for_update_with_git(source_dir: &Path) -> Result<UpdateCheck, BuildError> {
    let source_dir = path_arg(source_dir);
    run_git(vec!["-C", &source_dir, "fetch", "--quiet", "origin"]).await?;
    let current = run_git(vec!["-C", &source_dir, "rev-parse", "HEAD"])
        .await?
        .trim()
        .to_string();
    let latest = run_git(vec!["-C", &source_dir, "rev-parse", "origin/HEAD"])
        .await?
        .trim()
        .to_string();
    if current == latest {
        return Ok(UpdateCheck::UpToDate { current });
    }

    let behind = run_git(vec![
        "-C",
        &source_dir,
        "rev-list",
        "--count",
        "HEAD..origin/HEAD",
    ])
    .await?;
    let commits_behind = behind.trim().parse().unwrap_or(0);
    Ok(UpdateCheck::Available {
        current,
        latest,
        commits_behind,
    })
}

async fn run_build(config: BuildConfig, tx: mpsc::Sender<BuildEvent>) {
    let started = Instant::now();
    let mut log_tail = LogTail::new(config.log_tail_lines);
    let result = run_build_inner(&config, &tx, &mut log_tail, started).await;
    if let Err(error) = result {
        send_event(
            &tx,
            BuildEvent::Failed {
                last_error: error.to_string(),
                log_tail: log_tail.lines,
            },
        )
        .await;
    }
}

async fn run_build_inner(
    config: &BuildConfig,
    tx: &mpsc::Sender<BuildEvent>,
    log_tail: &mut LogTail,
    started: Instant,
) -> Result<(), BuildError> {
    ensure_repo(config, tx, log_tail).await?;
    if let Some(git_ref) = &config.git_ref {
        stream_command(
            "git",
            &[
                "-C".to_string(),
                path_arg(&config.source_dir),
                "checkout".to_string(),
                git_ref.clone(),
            ],
            tx,
            log_tail,
        )
        .await?;
    }

    if config.clean {
        let build_dir = config.source_dir.join("build");
        if build_dir.exists() {
            tokio::fs::remove_dir_all(&build_dir)
                .await
                .map_err(|error| {
                    BuildError::Filesystem(format!(
                        "failed to remove {}: {error}",
                        build_dir.display()
                    ))
                })?;
        }
    }

    send_event(tx, BuildEvent::CmakeConfiguring).await;
    let configure_args = cmake_configure_args(config);
    stream_command("cmake", &configure_args, tx, log_tail).await?;

    let build_dir = config.source_dir.join("build");
    let jobs = if config.jobs == 0 {
        std::thread::available_parallelism()
            .map(|count| count.get())
            .unwrap_or(4)
    } else {
        config.jobs
    };
    let build_args = vec![
        "--build".to_string(),
        build_dir.to_string_lossy().into_owned(),
        "--config".to_string(),
        "Release".to_string(),
        "-j".to_string(),
        jobs.to_string(),
    ];
    stream_command("cmake", &build_args, tx, log_tail).await?;

    let server = build_dir.join("bin").join(binary_name("llama-server"));
    let cli = build_dir.join("bin").join(binary_name("llama-cli"));
    verify_binary(&cli).await?;
    if !is_executable(&server) {
        return Err(BuildError::Verification(format!(
            "expected server binary missing or not executable: {}",
            server.display()
        )));
    }

    send_event(
        tx,
        BuildEvent::Completed {
            binary: server,
            elapsed: started.elapsed(),
        },
    )
    .await;
    Ok(())
}

async fn ensure_repo(
    config: &BuildConfig,
    tx: &mpsc::Sender<BuildEvent>,
    log_tail: &mut LogTail,
) -> Result<(), BuildError> {
    if config.source_dir.join("CMakeLists.txt").exists() {
        stream_command(
            "git",
            &[
                "-C".to_string(),
                path_arg(&config.source_dir),
                "pull".to_string(),
                "--ff-only".to_string(),
            ],
            tx,
            log_tail,
        )
        .await
    } else {
        if let Some(parent) = config.source_dir.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|error| {
                BuildError::Filesystem(format!("failed to create {}: {error}", parent.display()))
            })?;
        }
        send_event(
            tx,
            BuildEvent::Cloning {
                url: LLAMA_CPP_URL.to_string(),
            },
        )
        .await;
        stream_command(
            "git",
            &[
                "clone".to_string(),
                "--depth".to_string(),
                "1".to_string(),
                LLAMA_CPP_URL.to_string(),
                path_arg(&config.source_dir),
            ],
            tx,
            log_tail,
        )
        .await
    }
}

async fn verify_binary(binary: &Path) -> Result<(), BuildError> {
    if !is_executable(binary) {
        return Err(BuildError::Verification(format!(
            "expected verification binary missing or not executable: {}",
            binary.display()
        )));
    }
    let output = tokio::process::Command::new(binary)
        .arg("--version")
        .output()
        .await
        .map_err(|error| {
            BuildError::Verification(format!("failed to verify {}: {error}", binary.display()))
        })?;
    if output.status.success() {
        Ok(())
    } else {
        Err(BuildError::Verification(format!(
            "{} --version failed",
            binary.display()
        )))
    }
}

async fn stream_command(
    program: &str,
    args: &[String],
    tx: &mpsc::Sender<BuildEvent>,
    log_tail: &mut LogTail,
) -> Result<(), BuildError> {
    let mut child = tokio::process::Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| BuildError::Command(format!("failed to start {program}: {error}")))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| BuildError::Command(format!("failed to capture {program} stdout")))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| BuildError::Command(format!("failed to capture {program} stderr")))?;

    let mut stdout = tokio::io::BufReader::new(stdout).lines();
    let mut stderr = tokio::io::BufReader::new(stderr).lines();
    let mut stdout_done = false;
    let mut stderr_done = false;

    while !stdout_done || !stderr_done {
        tokio::select! {
            line = stdout.next_line(), if !stdout_done => {
                match line {
                    Ok(Some(line)) => handle_line(tx, log_tail, line).await,
                    Ok(None) => stdout_done = true,
                    Err(error) => return Err(BuildError::Command(format!("{program} stdout read failed: {error}"))),
                }
            }
            line = stderr.next_line(), if !stderr_done => {
                match line {
                    Ok(Some(line)) => handle_line(tx, log_tail, line).await,
                    Ok(None) => stderr_done = true,
                    Err(error) => return Err(BuildError::Command(format!("{program} stderr read failed: {error}"))),
                }
            }
        }
    }

    let status = child
        .wait()
        .await
        .map_err(|error| BuildError::Command(format!("{program} process failed: {error}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(BuildError::Command(format!(
            "{program} exited with {status}"
        )))
    }
}

async fn handle_line(tx: &mpsc::Sender<BuildEvent>, log_tail: &mut LogTail, line: String) {
    let line = truncate_line(line);
    if line.to_lowercase().contains("linking") {
        send_event(tx, BuildEvent::Linking).await;
    }
    log_tail.push(line.clone());
    send_event(tx, BuildEvent::Compiling { line }).await;
}

async fn run_git(args: Vec<&str>) -> Result<String, BuildError> {
    let output = tokio::process::Command::new("git")
        .args(args)
        .output()
        .await
        .map_err(|error| BuildError::Command(format!("failed to run git: {error}")))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(BuildError::Command(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ))
    }
}

async fn send_event(tx: &mpsc::Sender<BuildEvent>, event: BuildEvent) {
    let _ignored = tx.send(event).await;
}

fn path_arg(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn truncate_line(line: String) -> String {
    if line.len() > 500 {
        format!("{}...", &line[..497])
    } else {
        line
    }
}

fn binary_name(base: &str) -> String {
    if cfg!(windows) {
        format!("{base}.exe")
    } else {
        base.to_string()
    }
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    path.metadata()
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.metadata()
        .map(|metadata| metadata.is_file())
        .unwrap_or(false)
}

#[derive(Debug)]
struct LogTail {
    max_lines: usize,
    lines: Vec<String>,
}

impl LogTail {
    fn new(max_lines: usize) -> Self {
        Self {
            max_lines,
            lines: Vec::new(),
        }
    }

    fn push(&mut self, line: String) {
        self.lines.push(line);
        if self.lines.len() > self.max_lines {
            let overflow = self.lines.len() - self.max_lines;
            self.lines.drain(0..overflow);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;

    #[test]
    fn assembles_cuda_cmake_flags_with_sccache() {
        let mut config = BuildConfig::new(
            PathBuf::from("/tmp/llama.cpp"),
            BuildBackend::Cuda {
                archs: vec!["sm_75", "sm_86"],
            },
        );
        config.sccache = Some(PathBuf::from("/usr/bin/sccache"));
        config.extra_cmake_flags = vec!["-DGGML_NATIVE=ON".to_string()];

        assert_eq!(
            cmake_configure_args(&config),
            vec![
                "-S",
                "/tmp/llama.cpp",
                "-B",
                "/tmp/llama.cpp/build",
                "-DCMAKE_BUILD_TYPE=Release",
                "-DLLAMA_BUILD_SERVER=ON",
                "-DGGML_CUDA=ON",
                "-DCMAKE_CUDA_ARCHITECTURES=sm_75;sm_86",
                "-DCMAKE_C_COMPILER_LAUNCHER=/usr/bin/sccache",
                "-DCMAKE_CXX_COMPILER_LAUNCHER=/usr/bin/sccache",
                "-DGGML_NATIVE=ON",
            ]
        );
    }

    #[test]
    fn assembles_backend_specific_flags() {
        let cases = [
            (BuildBackend::Metal, Some("-DGGML_METAL=ON")),
            (BuildBackend::CpuAvx2, Some("-DGGML_AVX2=ON")),
            (BuildBackend::CpuAvx, Some("-DGGML_AVX=ON")),
            (BuildBackend::CpuFallback, None),
        ];

        for (backend, expected) in cases {
            let config = BuildConfig::new(PathBuf::from("/tmp/llama.cpp"), backend);
            let args = cmake_configure_args(&config);
            if let Some(expected) = expected {
                assert!(args.contains(&expected.to_string()));
            } else {
                assert!(!args.iter().any(|arg| arg.starts_with("-DGGML_AVX")));
                assert!(!args.iter().any(|arg| arg == "-DGGML_METAL=ON"));
            }
        }
    }

    #[test]
    fn cmake_hash_changes_when_args_change() {
        let config = BuildConfig::new(PathBuf::from("/tmp/a"), BuildBackend::CpuFallback);
        let mut changed = config.clone();
        changed.extra_cmake_flags.push("-DTEST=ON".to_string());
        assert_ne!(
            cmake_hash(&cmake_configure_args(&config)),
            cmake_hash(&cmake_configure_args(&changed))
        );
    }

    #[test]
    fn fingerprint_detects_missing_and_executable_binary() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let binary = tempdir.path().join("llama-server");
        let fingerprint = build_fingerprint("abc", &["cmake".to_string()], binary.clone());
        assert!(fingerprint.needs_rebuild());

        fs::write(&binary, b"#!/bin/sh\n").expect("write binary");
        #[cfg(unix)]
        {
            let mut perms = fs::metadata(&binary).expect("metadata").permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&binary, perms).expect("chmod");
        }
        assert!(!fingerprint.needs_rebuild());
    }

    #[test]
    fn log_tail_retains_recent_lines() {
        let mut tail = LogTail::new(2);
        tail.push("one".to_string());
        tail.push("two".to_string());
        tail.push("three".to_string());
        assert_eq!(tail.lines, vec!["two".to_string(), "three".to_string()]);
    }

    #[tokio::test]
    async fn update_check_reports_up_to_date_and_available() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let remote = tempdir.path().join("remote.git");
        let seed = tempdir.path().join("seed");
        let checkout = tempdir.path().join("checkout");

        git(tempdir.path(), &["init", "--bare", path_str(&remote)]).expect("init bare remote");
        git(
            tempdir.path(),
            &["clone", path_str(&remote), path_str(&seed)],
        )
        .expect("clone seed");
        git(&seed, &["config", "user.name", "lmml test"]).expect("configure name");
        git(&seed, &["config", "user.email", "lmml@example.test"]).expect("configure email");
        fs::write(seed.join("README.md"), "one\n").expect("write seed file");
        git(&seed, &["add", "README.md"]).expect("add seed");
        git(&seed, &["commit", "-m", "initial"]).expect("commit seed");
        git(&seed, &["push", "origin", "master"]).expect("push seed");

        git(
            tempdir.path(),
            &["clone", path_str(&remote), path_str(&checkout)],
        )
        .expect("clone checkout");
        assert!(matches!(
            check_for_update(&checkout).await,
            UpdateCheck::UpToDate { .. }
        ));

        fs::write(seed.join("README.md"), "two\n").expect("update seed file");
        git(&seed, &["add", "README.md"]).expect("add update");
        git(&seed, &["commit", "-m", "update"]).expect("commit update");
        git(&seed, &["push", "origin", "master"]).expect("push update");

        assert!(matches!(
            check_for_update(&checkout).await,
            UpdateCheck::Available {
                commits_behind: 1,
                ..
            }
        ));
    }

    fn git(cwd: &Path, args: &[&str]) -> Result<(), String> {
        let output = Command::new("git")
            .current_dir(cwd)
            .args(args)
            .output()
            .map_err(|error| error.to_string())?;
        if output.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&output.stderr).to_string())
        }
    }

    fn path_str(path: &Path) -> &str {
        path.to_str().expect("test paths are utf-8")
    }
}
