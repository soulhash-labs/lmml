//! Hardware and prerequisite detection for lmml.
//!
//! This crate probes the local machine for the compiler, build tools, CUDA,
//! Vulkan, GPU architecture, CPU features, RAM, and disk space needed to build
//! and run llama.cpp. The main entry point is [`SystemProfile::detect`], which
//! runs the probes concurrently and returns a complete [`SystemProfile`].

use std::collections::BTreeSet;
use std::ffi::CString;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use sysinfo::System;
use thiserror::Error;
use tokio::io::AsyncWriteExt;

const MIN_DISK_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const CPP17_PROBE: &str = "#include <filesystem>\nint main() { return 0; }\n";

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

/// Abstraction over process execution so probes can be tested without invoking host tools.
pub trait CommandRunner {
    /// Run `program` with `args`, optionally piping `stdin` into the child.
    fn run(
        &self,
        program: &str,
        args: &[&str],
        stdin: Option<&str>,
    ) -> impl Future<Output = CommandOutput> + Send;
}

/// Command runner backed by [`tokio::process::Command`].
#[derive(Debug, Clone, Copy, Default)]
pub struct RealCommandRunner;

impl CommandRunner for RealCommandRunner {
    async fn run(&self, program: &str, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut command = tokio::process::Command::new(program);
        command.args(args);
        if stdin.is_some() {
            command.stdin(Stdio::piped());
        }
        command.stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                return CommandOutput {
                    success: false,
                    stdout: String::new(),
                    stderr: error.to_string(),
                };
            }
        };

        if let Some(input) = stdin {
            match child.stdin.take() {
                Some(mut pipe) => {
                    if let Err(error) = pipe.write_all(input.as_bytes()).await {
                        return CommandOutput {
                            success: false,
                            stdout: String::new(),
                            stderr: error.to_string(),
                        };
                    }
                }
                None => {
                    return CommandOutput {
                        success: false,
                        stdout: String::new(),
                        stderr: "failed to open child stdin".to_string(),
                    };
                }
            }
        }

        match child.wait_with_output().await {
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

/// Complete picture of hardware and toolchain capabilities on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemProfile {
    /// C++ compiler capable of building llama.cpp, if detected.
    pub compiler: Option<CompilerInfo>,
    /// CMake installation and version, if detected.
    pub cmake: Option<CmakeInfo>,
    /// Git installation and version, if detected.
    pub git: Option<GitInfo>,
    /// CUDA toolkit/GPU compatibility state.
    pub cuda: CudaCompatibility,
    /// CUDA-capable GPUs reported by `nvidia-smi`.
    pub gpus: Vec<GpuInfo>,
    /// Error returned by `nvidia-smi` when GPU enumeration failed.
    pub gpu_probe_error: Option<String>,
    /// `sccache` executable path, if available.
    pub sccache: Option<PathBuf>,
    /// Metal support on macOS.
    pub metal: MetalSupport,
    /// Vulkan loader/device support.
    pub vulkan: VulkanSupport,
    /// CPU model, thread count, and instruction features.
    pub cpu: CpuFeatures,
    /// Available system memory.
    pub memory: MemInfo,
    /// Available disk space at the build location.
    pub disk: DiskInfo,
}

impl SystemProfile {
    /// Run all probes concurrently and return the combined profile.
    #[tracing::instrument]
    pub async fn detect() -> SystemProfile {
        let runner = RealCommandRunner;
        let disk_path = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        detect_with_runner(&runner, disk_path).await
    }

    /// The recommended llama.cpp build backend for this machine.
    pub fn recommended_backend(&self) -> BuildBackend {
        match &self.cuda {
            CudaCompatibility::Compatible { archs } if !archs.is_empty() => BuildBackend::Cuda {
                archs: archs.clone(),
            },
            CudaCompatibility::Compatible { .. }
            | CudaCompatibility::ToolkitTooOld { .. }
            | CudaCompatibility::NoGpu
            | CudaCompatibility::NvccMissing => {
                if self.metal.available {
                    BuildBackend::Metal
                } else if self.vulkan.available {
                    BuildBackend::Vulkan
                } else if self.cpu.avx2 {
                    BuildBackend::CpuAvx2
                } else if self.cpu.avx {
                    BuildBackend::CpuAvx
                } else {
                    BuildBackend::CpuFallback
                }
            }
        }
    }

    /// All unmet hard prerequisites for building llama.cpp.
    pub fn missing_prerequisites(&self) -> Vec<MissingPrerequisite> {
        let mut missing = Vec::new();
        match &self.compiler {
            Some(compiler) if compiler.cpp17_ok => {}
            Some(_) => missing.push(MissingPrerequisite {
                name: "C++17 compiler",
                install: "install gcc/g++ or clang with C++17 support",
            }),
            None => missing.push(MissingPrerequisite {
                name: "C++ compiler",
                install: "sudo apt install build-essential",
            }),
        }

        match &self.cmake {
            Some(cmake) if cmake.meets_minimum => {}
            Some(_) | None => missing.push(MissingPrerequisite {
                name: "cmake >= 3.21",
                install: "sudo apt install cmake",
            }),
        }

        match &self.git {
            Some(git) if git.meets_minimum => {}
            Some(_) | None => missing.push(MissingPrerequisite {
                name: "git >= 2.28",
                install: "sudo apt install git",
            }),
        }

        if self.disk.require(MIN_DISK_BYTES).is_err() {
            missing.push(MissingPrerequisite {
                name: "4 GB free disk",
                install: "free disk space in the lmml build directory",
            });
        }

        missing
    }

    /// Soft warnings for available but suboptimal tooling or hardware combinations.
    pub fn warnings(&self) -> Vec<DetectionWarning> {
        let mut warnings = Vec::new();
        if let Some(cmake) = &self.cmake {
            if !cmake.meets_minimum {
                warnings.push(DetectionWarning {
                    message: format!("cmake {} detected; 3.21+ required", cmake.version),
                });
            }
        }
        if let Some(git) = &self.git {
            if !git.meets_minimum {
                warnings.push(DetectionWarning {
                    message: format!("git {} detected; 2.28+ recommended", git.version),
                });
            }
        }
        if let CudaCompatibility::ToolkitTooOld {
            gpu_arch,
            minimum_toolkit,
            found_toolkit,
        } = &self.cuda
        {
            warnings.push(DetectionWarning {
                message: format!(
                    "{gpu_arch} requires CUDA >= {minimum_toolkit}; found {found_toolkit}"
                ),
            });
        }
        if self.sccache.is_none() {
            warnings.push(DetectionWarning {
                message: "sccache not found; repeat builds will be slower".to_string(),
            });
        }
        warnings
    }
}

/// Detect a full system profile with an injected command runner.
#[tracing::instrument(skip(runner), fields(disk_path = %disk_path.display()))]
pub async fn detect_with_runner<R>(runner: &R, disk_path: PathBuf) -> SystemProfile
where
    R: CommandRunner + Sync,
{
    let compiler = detect_compiler(runner);
    let cmake = detect_cmake(runner);
    let git = detect_git(runner);
    let nvcc = detect_nvcc(runner);
    let gpus = detect_gpus(runner);
    let sccache = detect_sccache(runner);
    let metal = detect_metal(runner);
    let vulkan = detect_vulkan(runner);
    let cpu = detect_cpu_features(runner);
    let memory = detect_memory();
    let disk = detect_disk(disk_path);

    let (compiler, cmake, git, nvcc, gpus, sccache, metal, vulkan, cpu, memory, disk) =
        tokio::join!(compiler, cmake, git, nvcc, gpus, sccache, metal, vulkan, cpu, memory, disk);

    let gpu_probe_error = gpus.error;
    let gpus = gpus.devices;
    let cuda = cuda_compatibility(nvcc.as_ref().map(|info| &info.version), &gpus);

    let profile = SystemProfile {
        compiler,
        cmake,
        git,
        cuda,
        gpus,
        gpu_probe_error,
        sccache,
        metal,
        vulkan,
        cpu,
        memory,
        disk,
    };
    tracing::info!(backend = ?profile.recommended_backend(), "system detection completed");
    profile
}

/// C++ compiler information and C++17 probe result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompilerInfo {
    /// Executable path returned by `which`.
    pub path: PathBuf,
    /// Raw version string from `--version`.
    pub version: String,
    /// Whether the compiler accepted a C++17 `<filesystem>` compile probe.
    pub cpp17_ok: bool,
    /// Failure message from the C++17 probe, if any.
    pub cpp17_error: Option<String>,
}

/// CMake version information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CmakeInfo {
    /// Executable path returned by `which`.
    pub path: PathBuf,
    /// Parsed CMake version.
    pub version: String,
    /// Whether the version satisfies the llama.cpp minimum.
    pub meets_minimum: bool,
}

/// Git version information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitInfo {
    /// Executable path returned by `which`.
    pub path: PathBuf,
    /// Parsed Git version.
    pub version: String,
    /// Whether the version satisfies lmml's recommended minimum.
    pub meets_minimum: bool,
}

/// CUDA compiler information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NvccInfo {
    /// Executable path returned by `which`.
    pub path: PathBuf,
    /// Parsed CUDA toolkit version.
    pub version: CudaVersion,
}

/// CUDA toolkit semantic version parsed from `nvcc --version`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CudaVersion {
    /// Original parsed version string.
    pub raw: String,
    /// Major version number.
    pub major: u32,
    /// Minor version number.
    pub minor: u32,
}

impl CudaVersion {
    /// Create a CUDA version from numeric major and minor components.
    pub fn new(major: u32, minor: u32) -> Self {
        Self {
            raw: format!("{major}.{minor}"),
            major,
            minor,
        }
    }
}

/// Single CUDA-capable GPU detected by `nvidia-smi`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuInfo {
    /// GPU product name.
    pub name: String,
    /// Total GPU memory in MiB.
    pub memory_total_mb: u64,
    /// Raw compute capability string, such as `8.6`.
    pub compute_cap: String,
    /// Canonical CUDA architecture, such as `sm_86`.
    pub arch: Option<&'static str>,
}

/// Compatibility between detected CUDA toolkit and detected GPUs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CudaCompatibility {
    /// nvcc version supports all detected GPU architectures.
    Compatible {
        /// Unique canonical CUDA architectures to compile for.
        archs: Vec<&'static str>,
    },
    /// nvcc is too old for one or more GPUs.
    ToolkitTooOld {
        /// GPU architecture that requires a newer toolkit.
        gpu_arch: &'static str,
        /// Minimum CUDA toolkit version for that architecture.
        minimum_toolkit: &'static str,
        /// Detected CUDA toolkit version.
        found_toolkit: String,
    },
    /// nvcc was found but no CUDA-capable GPUs were detected.
    NoGpu,
    /// nvcc was not found, so CUDA backend is unavailable.
    NvccMissing,
}

/// macOS Metal capability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetalSupport {
    /// Whether Metal appears available.
    pub available: bool,
    /// Display/GPU lines captured from `system_profiler`, if any.
    pub displays: Vec<String>,
}

/// Vulkan loader and device capability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VulkanSupport {
    /// Whether `vulkaninfo` reported at least one Vulkan-capable device.
    pub available: bool,
    /// Summary or device lines captured from `vulkaninfo`.
    pub devices: Vec<String>,
}

/// CPU model, topology, and instruction features relevant to llama.cpp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CpuFeatures {
    /// Human-readable CPU model.
    pub model: String,
    /// Physical core count where known.
    pub cores: u32,
    /// Logical thread count.
    pub threads: u32,
    /// CPU supports AVX.
    pub avx: bool,
    /// CPU supports AVX2.
    pub avx2: bool,
    /// CPU supports AVX-512 foundation.
    pub avx512: bool,
    /// CPU supports ARM NEON.
    pub neon: bool,
    /// Additional normalized feature names.
    pub features: Vec<String>,
}

/// System memory information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemInfo {
    /// Total physical memory in MiB.
    pub total_mb: u64,
    /// Available physical memory in MiB.
    pub available_mb: u64,
}

/// Available disk space at a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskInfo {
    /// Available bytes at `path`.
    pub available_bytes: u64,
    /// Path checked with `statvfs`.
    pub path: PathBuf,
}

impl DiskInfo {
    /// Returns an error if less than `min_bytes` are available.
    pub fn require(&self, min_bytes: u64) -> Result<(), InsufficientDiskError> {
        if self.available_bytes >= min_bytes {
            Ok(())
        } else {
            Err(InsufficientDiskError {
                path: self.path.clone(),
                required_bytes: min_bytes,
                available_bytes: self.available_bytes,
            })
        }
    }
}

/// Error returned when a disk path has insufficient free space.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
#[error(
    "insufficient disk space at {path}: need {required_bytes} bytes, have {available_bytes} bytes"
)]
pub struct InsufficientDiskError {
    /// Path that was checked.
    pub path: PathBuf,
    /// Required free bytes.
    pub required_bytes: u64,
    /// Actual free bytes.
    pub available_bytes: u64,
}

/// Error returned by the C++17 compile probe.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum CompilerProbeError {
    /// The compiler process exited unsuccessfully.
    #[error("compiler rejected C++17 probe: {0}")]
    Failed(String),
}

/// Recommended build backend for llama.cpp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildBackend {
    /// CUDA backend with one or more target architectures.
    Cuda {
        /// Canonical CUDA architectures, such as `sm_86`.
        archs: Vec<&'static str>,
    },
    /// Apple Metal backend.
    Metal,
    /// Vulkan backend.
    Vulkan,
    /// CPU backend with AVX2 acceleration.
    CpuAvx2,
    /// CPU backend with AVX acceleration.
    CpuAvx,
    /// Portable CPU fallback backend.
    CpuFallback,
}

/// Hard prerequisite missing from the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingPrerequisite {
    /// Prerequisite name.
    pub name: &'static str,
    /// Human-readable installation hint.
    pub install: &'static str,
}

/// Soft detection warning shown to users.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectionWarning {
    /// Human-readable warning text.
    pub message: String,
}

/// Maps a raw compute capability string, such as `8.6`, to a canonical `sm_` arch.
pub fn compute_cap_to_arch(cap: &str) -> Option<&'static str> {
    match cap.trim() {
        "3.7" => Some("sm_37"),
        "5.0" => Some("sm_50"),
        "5.2" => Some("sm_52"),
        "5.3" => Some("sm_53"),
        "6.0" => Some("sm_60"),
        "6.1" => Some("sm_61"),
        "6.2" => Some("sm_62"),
        "7.0" => Some("sm_70"),
        "7.2" => Some("sm_72"),
        "7.5" => Some("sm_75"),
        "8.0" => Some("sm_80"),
        "8.6" => Some("sm_86"),
        "8.7" => Some("sm_87"),
        "8.9" => Some("sm_89"),
        "9.0" => Some("sm_90"),
        "9.0a" => Some("sm_90a"),
        "10.0" => Some("sm_100"),
        "10.0a" => Some("sm_100a"),
        _ => None,
    }
}

/// Collect unique CUDA architecture strings from detected GPUs.
pub fn cuda_arches_for_gpus(gpus: &[GpuInfo]) -> Vec<&'static str> {
    gpus.iter()
        .filter_map(|gpu| gpu.arch)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Cross-check detected nvcc version against detected GPU architectures.
pub fn cuda_compatibility(
    nvcc_version: Option<&CudaVersion>,
    gpus: &[GpuInfo],
) -> CudaCompatibility {
    let Some(version) = nvcc_version else {
        return CudaCompatibility::NvccMissing;
    };
    let archs = cuda_arches_for_gpus(gpus);
    if archs.is_empty() {
        return CudaCompatibility::NoGpu;
    }

    for arch in &archs {
        if let Some((minimum, major, minor)) = minimum_toolkit_for_arch(arch) {
            if version.major < major || version.major == major && version.minor < minor {
                return CudaCompatibility::ToolkitTooOld {
                    gpu_arch: arch,
                    minimum_toolkit: minimum,
                    found_toolkit: version.raw.clone(),
                };
            }
        }
    }

    CudaCompatibility::Compatible { archs }
}

/// Run a C++17 `<filesystem>` compile probe with the real command runner.
pub async fn probe_cpp17(compiler: &Path) -> Result<(), CompilerProbeError> {
    probe_cpp17_with_runner(&RealCommandRunner, compiler).await
}

/// Run a C++17 `<filesystem>` compile probe with an injected command runner.
pub async fn probe_cpp17_with_runner<R>(
    runner: &R,
    compiler: &Path,
) -> Result<(), CompilerProbeError>
where
    R: CommandRunner + Sync,
{
    let program = compiler.to_string_lossy();
    let output = runner
        .run(
            &program,
            &["-std=c++17", "-x", "c++", "-", "-fsyntax-only"],
            Some(CPP17_PROBE),
        )
        .await;
    if output.success {
        Ok(())
    } else {
        let message = if output.stderr.trim().is_empty() {
            output.stdout.trim().to_string()
        } else {
            output.stderr.trim().to_string()
        };
        Err(CompilerProbeError::Failed(message))
    }
}

async fn detect_compiler<R>(runner: &R) -> Option<CompilerInfo>
where
    R: CommandRunner + Sync,
{
    for candidate in ["c++", "g++", "clang++"] {
        let Some(path) = which(runner, candidate).await else {
            continue;
        };
        let program = path.to_string_lossy();
        let output = runner.run(&program, &["--version"], None).await;
        let probe = probe_cpp17_with_runner(runner, &path).await;
        return Some(CompilerInfo {
            path,
            version: first_line(&output.stdout, &output.stderr),
            cpp17_ok: probe.is_ok(),
            cpp17_error: probe.err().map(|error| error.to_string()),
        });
    }
    None
}

async fn detect_cmake<R>(runner: &R) -> Option<CmakeInfo>
where
    R: CommandRunner + Sync,
{
    let path = which(runner, "cmake").await?;
    let program = path.to_string_lossy();
    let output = runner.run(&program, &["--version"], None).await;
    if !output.success {
        return None;
    }
    let version = parse_version(&output.stdout).unwrap_or_else(|| "0.0".to_string());
    Some(CmakeInfo {
        path,
        meets_minimum: version_at_least(&version, 3, 21),
        version,
    })
}

async fn detect_git<R>(runner: &R) -> Option<GitInfo>
where
    R: CommandRunner + Sync,
{
    let path = which(runner, "git").await?;
    let program = path.to_string_lossy();
    let output = runner.run(&program, &["--version"], None).await;
    if !output.success {
        return None;
    }
    let version = parse_version(&output.stdout).unwrap_or_else(|| "0.0".to_string());
    Some(GitInfo {
        path,
        meets_minimum: version_at_least(&version, 2, 28),
        version,
    })
}

async fn detect_nvcc<R>(runner: &R) -> Option<NvccInfo>
where
    R: CommandRunner + Sync,
{
    let path = which(runner, "nvcc").await?;
    let program = path.to_string_lossy();
    let output = runner.run(&program, &["--version"], None).await;
    if !output.success {
        return None;
    }
    Some(NvccInfo {
        path,
        version: parse_cuda_version(&output.stdout)?,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GpuProbe {
    devices: Vec<GpuInfo>,
    error: Option<String>,
}

async fn detect_gpus<R>(runner: &R) -> GpuProbe
where
    R: CommandRunner + Sync,
{
    let output = runner
        .run(
            "nvidia-smi",
            &[
                "--query-gpu=name,memory.total,compute_cap",
                "--format=csv,noheader",
            ],
            None,
        )
        .await;
    if !output.success {
        let reason = first_line(&output.stderr, &output.stdout);
        return GpuProbe {
            devices: Vec::new(),
            error: (!reason.is_empty()).then_some(reason),
        };
    }
    GpuProbe {
        devices: parse_gpu_csv(&output.stdout),
        error: None,
    }
}

async fn detect_sccache<R>(runner: &R) -> Option<PathBuf>
where
    R: CommandRunner + Sync,
{
    which(runner, "sccache").await
}

async fn detect_metal<R>(runner: &R) -> MetalSupport
where
    R: CommandRunner + Sync,
{
    if !cfg!(target_os = "macos") {
        return MetalSupport {
            available: false,
            displays: Vec::new(),
        };
    }
    let output = runner
        .run("system_profiler", &["SPDisplaysDataType"], None)
        .await;
    let displays = output
        .stdout
        .lines()
        .map(str::trim)
        .filter(|line| {
            line.contains("Chipset Model:")
                || line.contains("Metal Support:")
                || line.contains("Metal:")
        })
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    MetalSupport {
        available: output.success && output.stdout.to_lowercase().contains("metal"),
        displays,
    }
}

async fn detect_vulkan<R>(runner: &R) -> VulkanSupport
where
    R: CommandRunner + Sync,
{
    let output = runner.run("vulkaninfo", &["--summary"], None).await;
    if !output.success {
        return VulkanSupport {
            available: false,
            devices: Vec::new(),
        };
    }
    let devices = output
        .stdout
        .lines()
        .map(str::trim)
        .filter(|line| {
            line.starts_with("GPU")
                || line.starts_with("deviceName")
                || line.starts_with("driverName")
                || line.starts_with("apiVersion")
        })
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    VulkanSupport {
        available: output.stdout.to_lowercase().contains("vulkan")
            || devices.iter().any(|line| line.contains("GPU")),
        devices,
    }
}

async fn detect_cpu_features<R>(runner: &R) -> CpuFeatures
where
    R: CommandRunner + Sync,
{
    let threads = std::thread::available_parallelism()
        .map(|count| count.get() as u32)
        .unwrap_or(1);

    if cfg!(target_os = "linux") {
        if let Ok(content) = tokio::fs::read_to_string("/proc/cpuinfo").await {
            return parse_linux_cpuinfo(&content, threads);
        }
    }

    if cfg!(target_os = "macos") {
        let brand = runner
            .run("sysctl", &["-n", "machdep.cpu.brand_string"], None)
            .await;
        let features = runner.run("sysctl", &["-a"], None).await;
        let model = first_line(&brand.stdout, &brand.stderr);
        let lower = features.stdout.to_lowercase();
        return CpuFeatures {
            model: if model.is_empty() {
                "Unknown CPU".to_string()
            } else {
                model
            },
            cores: (threads / 2).max(1),
            threads,
            avx: lower.contains("avx1.0") || lower.contains(" avx"),
            avx2: lower.contains("avx2"),
            avx512: lower.contains("avx512"),
            neon: lower.contains("neon") || lower.contains("asimd"),
            features: normalized_cpu_features(&lower),
        };
    }

    CpuFeatures {
        model: "Unknown CPU".to_string(),
        cores: (threads / 2).max(1),
        threads,
        avx: false,
        avx2: false,
        avx512: false,
        neon: false,
        features: vec!["generic".to_string()],
    }
}

async fn detect_memory() -> MemInfo {
    let mut system = System::new();
    system.refresh_memory();
    MemInfo {
        total_mb: system.total_memory() / (1024 * 1024),
        available_mb: system.available_memory() / (1024 * 1024),
    }
}

async fn detect_disk(path: PathBuf) -> DiskInfo {
    DiskInfo {
        available_bytes: available_disk_bytes(&path),
        path,
    }
}

async fn which<R>(runner: &R, program: &str) -> Option<PathBuf>
where
    R: CommandRunner + Sync,
{
    let output = runner.run("which", &[program], None).await;
    if !output.success {
        return None;
    }
    output
        .stdout
        .lines()
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
}

fn parse_gpu_csv(output: &str) -> Vec<GpuInfo> {
    output
        .lines()
        .filter_map(|line| {
            let mut parts = line.split(',').map(str::trim);
            let name = parts.next()?.to_string();
            let memory_total_mb = parse_first_u64(parts.next()?)?;
            let compute_cap = parts.next().unwrap_or_default().to_string();
            let arch = compute_cap_to_arch(&compute_cap);
            Some(GpuInfo {
                name,
                memory_total_mb,
                compute_cap,
                arch,
            })
        })
        .collect()
}

fn parse_cuda_version(output: &str) -> Option<CudaVersion> {
    if let Some(release_pos) = output.find("release") {
        let after_release = &output[release_pos + "release".len()..];
        if let Some(version) = parse_version(after_release) {
            let (major, minor) = parse_major_minor(&version)?;
            return Some(CudaVersion {
                raw: version,
                major,
                minor,
            });
        }
    }

    parse_version(output).and_then(|version| {
        let (major, minor) = parse_major_minor(&version)?;
        Some(CudaVersion {
            raw: version,
            major,
            minor,
        })
    })
}

fn parse_version(output: &str) -> Option<String> {
    for token in output.split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '.') {
        if token.chars().any(|ch| ch == '.')
            && token
                .chars()
                .all(|ch| ch.is_ascii_digit() || ch == '.' || ch.is_ascii_alphabetic())
            && token.chars().next().is_some_and(|ch| ch.is_ascii_digit())
        {
            return Some(token.trim_end_matches('.').to_string());
        }
    }
    None
}

fn parse_major_minor(version: &str) -> Option<(u32, u32)> {
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor_raw = parts.next().unwrap_or("0");
    let minor_digits: String = minor_raw
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect();
    let minor = minor_digits.parse().ok()?;
    Some((major, minor))
}

fn version_at_least(version: &str, major: u32, minor: u32) -> bool {
    parse_major_minor(version).is_some_and(|(actual_major, actual_minor)| {
        actual_major > major || actual_major == major && actual_minor >= minor
    })
}

fn minimum_toolkit_for_arch(arch: &str) -> Option<(&'static str, u32, u32)> {
    match arch {
        "sm_37" | "sm_50" | "sm_52" | "sm_53" | "sm_60" | "sm_61" | "sm_62" | "sm_70" | "sm_72"
        | "sm_75" => Some(("9.0", 9, 0)),
        "sm_80" | "sm_86" | "sm_87" => Some(("11.1", 11, 1)),
        "sm_89" => Some(("11.8", 11, 8)),
        "sm_90" | "sm_90a" => Some(("12.0", 12, 0)),
        "sm_100" | "sm_100a" => Some(("12.4", 12, 4)),
        _ => None,
    }
}

fn first_line(stdout: &str, stderr: &str) -> String {
    stdout
        .lines()
        .chain(stderr.lines())
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .to_string()
}

fn parse_first_u64(input: &str) -> Option<u64> {
    let digits: String = input
        .chars()
        .skip_while(|ch| !ch.is_ascii_digit())
        .take_while(|ch| ch.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

fn parse_linux_cpuinfo(content: &str, threads: u32) -> CpuFeatures {
    let model = content
        .lines()
        .find_map(|line| {
            line.strip_prefix("model name")
                .or(line.strip_prefix("Hardware"))
        })
        .and_then(|line| {
            line.split_once(':')
                .map(|(_, value)| value.trim().to_string())
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "Unknown CPU".to_string());

    let mut core_ids = content
        .lines()
        .filter_map(|line| line.strip_prefix("core id"))
        .filter_map(|line| {
            line.split_once(':')
                .map(|(_, value)| value.trim().to_string())
        })
        .collect::<Vec<_>>();
    core_ids.sort();
    core_ids.dedup();
    let cores = if core_ids.is_empty() {
        (threads / 2).max(1)
    } else {
        core_ids.len() as u32
    };

    let flags = content
        .lines()
        .find_map(|line| line.strip_prefix("flags").or(line.strip_prefix("Features")))
        .and_then(|line| line.split_once(':').map(|(_, value)| value.to_lowercase()))
        .unwrap_or_default();
    let features = normalized_cpu_features(&flags);

    CpuFeatures {
        model,
        cores,
        threads,
        avx: flags.split_whitespace().any(|flag| flag == "avx"),
        avx2: flags.split_whitespace().any(|flag| flag == "avx2"),
        avx512: flags.split_whitespace().any(|flag| flag == "avx512f"),
        neon: flags
            .split_whitespace()
            .any(|flag| flag == "neon" || flag == "asimd"),
        features,
    }
}

fn normalized_cpu_features(flags: &str) -> Vec<String> {
    let map = [
        ("avx", "AVX"),
        ("avx2", "AVX2"),
        ("avx512f", "AVX-512"),
        ("neon", "NEON"),
        ("asimd", "NEON"),
        ("sse4_1", "SSE4.1"),
        ("sse4_2", "SSE4.2"),
        ("amx", "AMX"),
        ("sve", "SVE"),
        ("zvfh", "ZVFH"),
    ];
    let mut features = map
        .iter()
        .filter(|(needle, _)| flags.split_whitespace().any(|flag| flag == *needle))
        .map(|(_, name)| (*name).to_string())
        .collect::<Vec<_>>();
    features.sort();
    features.dedup();
    if features.is_empty() {
        features.push("generic".to_string());
    }
    features
}

#[cfg(unix)]
fn available_disk_bytes(path: &Path) -> u64 {
    use std::os::unix::ffi::OsStrExt;

    let c_path = match CString::new(path.as_os_str().as_bytes()) {
        Ok(path) => path,
        Err(_) => return 0,
    };
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: `c_path` is a valid nul-terminated path and `stat` points to
    // writable memory for libc to initialize.
    let result = unsafe { libc::statvfs(c_path.as_ptr(), stat.as_mut_ptr()) };
    if result != 0 {
        return 0;
    }
    // SAFETY: statvfs returned success, so the struct has been initialized.
    let stat = unsafe { stat.assume_init() };
    stat.f_bavail.saturating_mul(stat.f_frsize)
}

#[cfg(not(unix))]
fn available_disk_bytes(_path: &Path) -> u64 {
    0
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
        fn with(self, program: &str, args: &[&str], output: CommandOutput) -> Self {
            let key = Self::key(program, args);
            self.outputs
                .lock()
                .expect("lock fake outputs")
                .insert(key, output);
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

        fn key(program: &str, args: &[&str]) -> String {
            format!("{program}\0{}", args.join("\0"))
        }
    }

    impl CommandRunner for FakeRunner {
        async fn run(&self, program: &str, args: &[&str], _stdin: Option<&str>) -> CommandOutput {
            self.outputs
                .lock()
                .expect("lock fake outputs")
                .get(&Self::key(program, args))
                .cloned()
                .unwrap_or_else(|| FakeRunner::failure("not found"))
        }
    }

    #[test]
    fn compute_cap_map_is_complete() {
        let cases = [
            ("3.7", "sm_37"),
            ("5.0", "sm_50"),
            ("5.2", "sm_52"),
            ("5.3", "sm_53"),
            ("6.0", "sm_60"),
            ("6.1", "sm_61"),
            ("6.2", "sm_62"),
            ("7.0", "sm_70"),
            ("7.2", "sm_72"),
            ("7.5", "sm_75"),
            ("8.0", "sm_80"),
            ("8.6", "sm_86"),
            ("8.7", "sm_87"),
            ("8.9", "sm_89"),
            ("9.0", "sm_90"),
            ("9.0a", "sm_90a"),
            ("10.0", "sm_100"),
            ("10.0a", "sm_100a"),
        ];
        for (cap, arch) in cases {
            assert_eq!(compute_cap_to_arch(cap), Some(arch));
        }
        assert_eq!(compute_cap_to_arch("11.0"), None);
    }

    #[test]
    fn cuda_arches_are_unique_and_sorted() {
        let gpus = vec![gpu("A", 24, "8.9"), gpu("B", 8, "8.6"), gpu("C", 8, "8.6")];
        assert_eq!(cuda_arches_for_gpus(&gpus), vec!["sm_86", "sm_89"]);
    }

    #[test]
    fn cuda_compatibility_handles_all_states() {
        let old = CudaVersion::new(11, 0);
        let current = CudaVersion::new(12, 4);

        assert_eq!(
            cuda_compatibility(Some(&old), &[gpu("RTX 4090", 24, "8.9")]),
            CudaCompatibility::ToolkitTooOld {
                gpu_arch: "sm_89",
                minimum_toolkit: "11.8",
                found_toolkit: "11.0".to_string(),
            }
        );
        assert_eq!(
            cuda_compatibility(Some(&current), &[gpu("Blackwell", 32, "10.0a")]),
            CudaCompatibility::Compatible {
                archs: vec!["sm_100a"],
            }
        );
        assert_eq!(
            cuda_compatibility(Some(&current), &[]),
            CudaCompatibility::NoGpu
        );
        assert_eq!(
            cuda_compatibility(None, &[gpu("RTX 4090", 24, "8.9")]),
            CudaCompatibility::NvccMissing
        );
    }

    #[test]
    fn recommended_backend_prefers_cuda_then_metal_then_vulkan_then_cpu() {
        let mut profile = minimal_profile();
        profile.cuda = CudaCompatibility::Compatible {
            archs: vec!["sm_86"],
        };
        assert_eq!(
            profile.recommended_backend(),
            BuildBackend::Cuda {
                archs: vec!["sm_86"],
            }
        );

        profile.cuda = CudaCompatibility::NvccMissing;
        profile.metal.available = true;
        assert_eq!(profile.recommended_backend(), BuildBackend::Metal);

        profile.metal.available = false;
        profile.vulkan.available = true;
        assert_eq!(profile.recommended_backend(), BuildBackend::Vulkan);

        profile.vulkan.available = false;
        profile.cpu.avx2 = true;
        assert_eq!(profile.recommended_backend(), BuildBackend::CpuAvx2);

        profile.cpu.avx2 = false;
        profile.cpu.avx = true;
        assert_eq!(profile.recommended_backend(), BuildBackend::CpuAvx);

        profile.cpu.avx = false;
        assert_eq!(profile.recommended_backend(), BuildBackend::CpuFallback);
    }

    #[tokio::test]
    async fn vulkan_probe_detects_summary_devices() {
        let runner = FakeRunner::default().with(
            "vulkaninfo",
            &["--summary"],
            FakeRunner::success(
                "Vulkan Instance Version: 1.3.280\nGPU0:\n\tdeviceName = Example GPU\n",
            ),
        );

        let support = detect_vulkan(&runner).await;

        assert!(support.available);
        assert_eq!(
            support.devices,
            vec!["GPU0:".to_string(), "deviceName = Example GPU".to_string()]
        );
    }

    #[tokio::test]
    async fn gpu_probe_preserves_nvidia_smi_failure_reason() {
        let runner = FakeRunner::default().with(
            "nvidia-smi",
            &[
                "--query-gpu=name,memory.total,compute_cap",
                "--format=csv,noheader",
            ],
            FakeRunner::failure("driver unavailable"),
        );

        let probe = detect_gpus(&runner).await;

        assert_eq!(
            probe,
            GpuProbe {
                devices: Vec::new(),
                error: Some("driver unavailable".to_string()),
            }
        );
    }

    #[test]
    fn disk_require_reports_shortfall() {
        let disk = DiskInfo {
            available_bytes: 10,
            path: PathBuf::from("/tmp/lmml-test"),
        };
        assert!(disk.require(5).is_ok());
        assert_eq!(
            disk.require(11),
            Err(InsufficientDiskError {
                path: PathBuf::from("/tmp/lmml-test"),
                required_bytes: 11,
                available_bytes: 10,
            })
        );
    }

    #[tokio::test]
    async fn cpp17_probe_uses_runner_success_and_failure() {
        let compiler = PathBuf::from("/usr/bin/c++");
        let runner = FakeRunner::default().with(
            "/usr/bin/c++",
            &["-std=c++17", "-x", "c++", "-", "-fsyntax-only"],
            FakeRunner::success(""),
        );
        assert!(probe_cpp17_with_runner(&runner, &compiler).await.is_ok());

        let runner = FakeRunner::default().with(
            "/usr/bin/c++",
            &["-std=c++17", "-x", "c++", "-", "-fsyntax-only"],
            FakeRunner::failure("filesystem unavailable"),
        );
        assert_eq!(
            probe_cpp17_with_runner(&runner, &compiler).await,
            Err(CompilerProbeError::Failed(
                "filesystem unavailable".to_string()
            ))
        );
    }

    #[test]
    fn parses_nvidia_smi_csv_with_units() {
        let output = "NVIDIA GeForce RTX 4090, 24564 MiB, 8.9\nTesla K80, 11441 MiB, 3.7\n";
        assert_eq!(
            parse_gpu_csv(output),
            vec![
                gpu("NVIDIA GeForce RTX 4090", 24564, "8.9"),
                gpu("Tesla K80", 11441, "3.7")
            ]
        );
    }

    #[test]
    fn parses_nvcc_release_version() {
        let output = "Cuda compilation tools, release 12.4, V12.4.131\n";
        assert_eq!(parse_cuda_version(output), Some(CudaVersion::new(12, 4)));
    }

    #[test]
    fn parses_linux_cpuinfo_features() {
        let cpuinfo = "\
processor\t: 0
model name\t: Example CPU
core id\t\t: 0
flags\t\t: fpu sse4_1 sse4_2 avx avx2 avx512f

processor\t: 1
model name\t: Example CPU
core id\t\t: 1
flags\t\t: fpu sse4_1 sse4_2 avx avx2 avx512f
";
        assert_eq!(
            parse_linux_cpuinfo(cpuinfo, 2),
            CpuFeatures {
                model: "Example CPU".to_string(),
                cores: 2,
                threads: 2,
                avx: true,
                avx2: true,
                avx512: true,
                neon: false,
                features: vec![
                    "AVX".to_string(),
                    "AVX-512".to_string(),
                    "AVX2".to_string(),
                    "SSE4.1".to_string(),
                    "SSE4.2".to_string(),
                ],
            }
        );
    }

    #[tokio::test]
    async fn detect_with_runner_combines_probe_results() {
        let runner = FakeRunner::default()
            .with("which", &["c++"], FakeRunner::success("/usr/bin/c++\n"))
            .with("which", &["cmake"], FakeRunner::success("/usr/bin/cmake\n"))
            .with("which", &["git"], FakeRunner::success("/usr/bin/git\n"))
            .with(
                "which",
                &["nvcc"],
                FakeRunner::success("/usr/local/cuda/bin/nvcc\n"),
            )
            .with(
                "which",
                &["sccache"],
                FakeRunner::success("/usr/bin/sccache\n"),
            )
            .with(
                "/usr/bin/c++",
                &["--version"],
                FakeRunner::success("g++ 13.2.0\n"),
            )
            .with(
                "/usr/bin/c++",
                &["-std=c++17", "-x", "c++", "-", "-fsyntax-only"],
                FakeRunner::success(""),
            )
            .with(
                "/usr/bin/cmake",
                &["--version"],
                FakeRunner::success("cmake version 3.28.1\n"),
            )
            .with(
                "/usr/bin/git",
                &["--version"],
                FakeRunner::success("git version 2.45.0\n"),
            )
            .with(
                "/usr/local/cuda/bin/nvcc",
                &["--version"],
                FakeRunner::success("Cuda compilation tools, release 12.4, V12.4.131\n"),
            )
            .with(
                "nvidia-smi",
                &[
                    "--query-gpu=name,memory.total,compute_cap",
                    "--format=csv,noheader",
                ],
                FakeRunner::success("RTX 3090, 24576 MiB, 8.6\n"),
            );

        let tempdir = tempfile::tempdir().expect("create tempdir");
        let profile = detect_with_runner(&runner, tempdir.path().to_path_buf()).await;

        assert!(profile.compiler.as_ref().is_some_and(|info| info.cpp17_ok));
        assert!(profile
            .cmake
            .as_ref()
            .is_some_and(|info| info.meets_minimum));
        assert!(profile.git.as_ref().is_some_and(|info| info.meets_minimum));
        assert_eq!(
            profile.cuda,
            CudaCompatibility::Compatible {
                archs: vec!["sm_86"],
            }
        );
        assert_eq!(profile.sccache, Some(PathBuf::from("/usr/bin/sccache")));
        assert!(profile.missing_prerequisites().is_empty());
    }

    fn minimal_profile() -> SystemProfile {
        SystemProfile {
            compiler: Some(CompilerInfo {
                path: PathBuf::from("/usr/bin/c++"),
                version: "g++ 13".to_string(),
                cpp17_ok: true,
                cpp17_error: None,
            }),
            cmake: Some(CmakeInfo {
                path: PathBuf::from("/usr/bin/cmake"),
                version: "3.28.0".to_string(),
                meets_minimum: true,
            }),
            git: Some(GitInfo {
                path: PathBuf::from("/usr/bin/git"),
                version: "2.45.0".to_string(),
                meets_minimum: true,
            }),
            cuda: CudaCompatibility::NvccMissing,
            gpus: Vec::new(),
            gpu_probe_error: None,
            sccache: None,
            metal: MetalSupport {
                available: false,
                displays: Vec::new(),
            },
            vulkan: VulkanSupport {
                available: false,
                devices: Vec::new(),
            },
            cpu: CpuFeatures {
                model: "CPU".to_string(),
                cores: 8,
                threads: 16,
                avx: false,
                avx2: false,
                avx512: false,
                neon: false,
                features: vec!["generic".to_string()],
            },
            memory: MemInfo {
                total_mb: 64 * 1024,
                available_mb: 32 * 1024,
            },
            disk: DiskInfo {
                available_bytes: MIN_DISK_BYTES,
                path: PathBuf::from("/tmp"),
            },
        }
    }

    fn gpu(name: &str, memory_total_mb: u64, compute_cap: &str) -> GpuInfo {
        GpuInfo {
            name: name.to_string(),
            memory_total_mb,
            compute_cap: compute_cap.to_string(),
            arch: compute_cap_to_arch(compute_cap),
        }
    }
}
