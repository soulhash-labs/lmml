//! Hardware and prerequisite detection for lmml.
//!
//! This crate probes the local machine for the compiler, build tools, CUDA,
//! ROCm/HIP, Vulkan, GPU architecture, CPU features, RAM, and disk space needed
//! to build and run llama.cpp. The main entry point is [`SystemProfile::detect`],
//! which runs the probes concurrently and returns a complete [`SystemProfile`].

use std::collections::BTreeSet;
use std::ffi::CString;
use std::future::Future;
use std::os::unix::fs::FileTypeExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use sysinfo::System;
use thiserror::Error;
use tokio::io::AsyncWriteExt;

pub mod gpu_catalog;

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
    /// ROCm/HIP toolchain and GPU target capability.
    pub rocm: RocmSupport,
    /// CUDA-capable GPUs reported by `nvidia-smi`.
    pub gpus: Vec<GpuInfo>,
    /// Error returned by `nvidia-smi` when GPU enumeration failed.
    pub gpu_probe_error: Option<String>,
    /// NVIDIA device-node availability for the current process environment.
    pub nvidia_devices: NvidiaDeviceNodes,
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

    /// Return a conservative profile without running host hardware probes.
    ///
    /// This exists for deterministic headless API tests and service startup
    /// paths where the process should bind before slow or brittle probe tools
    /// can block API availability.
    pub fn skipped_probe() -> SystemProfile {
        let disk_path = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        SystemProfile {
            compiler: None,
            cmake: None,
            git: None,
            cuda: CudaCompatibility::NvccMissing,
            rocm: RocmSupport::default(),
            gpus: Vec::new(),
            gpu_probe_error: Some("system probe skipped".to_string()),
            nvidia_devices: NvidiaDeviceNodes {
                control: false,
                uvm: false,
                gpu_count: 0,
                errors: Vec::new(),
            },
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
                model: "probe skipped".to_string(),
                cores: 1,
                threads: 1,
                avx: false,
                avx2: false,
                avx512: false,
                neon: false,
                features: Vec::new(),
            },
            memory: MemInfo {
                total_mb: 0,
                available_mb: 0,
            },
            disk: DiskInfo {
                available_bytes: 0,
                path: disk_path,
            },
        }
    }

    /// The recommended llama.cpp build backend for this machine.
    pub fn recommended_backend(&self) -> BuildBackend {
        match &self.cuda {
            CudaCompatibility::Compatible { archs } if !archs.is_empty() => BuildBackend::Cuda {
                archs: archs.clone(),
            },
            CudaCompatibility::Compatible { .. }
            | CudaCompatibility::ToolkitTooOld { .. }
            | CudaCompatibility::ToolkitTooNew { .. }
            | CudaCompatibility::NoGpu
            | CudaCompatibility::NvccMissing => {
                if self.metal.available {
                    BuildBackend::Metal
                } else if self.rocm.available {
                    BuildBackend::Rocm {
                        targets: self.rocm.targets.clone(),
                    }
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
        match &self.cuda {
            CudaCompatibility::ToolkitTooOld {
                gpu_arch,
                minimum_toolkit,
                found_toolkit,
            } => warnings.push(DetectionWarning {
                message: format!(
                    "{gpu_arch} requires CUDA >= {minimum_toolkit}; found {found_toolkit}"
                ),
            }),
            CudaCompatibility::ToolkitTooNew {
                gpu_arch,
                maximum_toolkit,
                found_toolkit,
            } => warnings.push(DetectionWarning {
                message: format!(
                    "{gpu_arch} is not supported by CUDA {found_toolkit}; use CUDA {maximum_toolkit}"
                ),
            }),
            CudaCompatibility::Compatible { .. }
            | CudaCompatibility::NoGpu
            | CudaCompatibility::NvccMissing => {}
        }
        if self.gpus.iter().any(|gpu| gpu.arch.is_none()) {
            let unknown = self
                .gpus
                .iter()
                .filter(|gpu| gpu.arch.is_none())
                .map(|gpu| format!("{} compute {}", gpu.name, gpu.compute_cap))
                .collect::<Vec<_>>()
                .join(", ");
            warnings.push(DetectionWarning {
                message: format!("unknown CUDA compute capability: {unknown}"),
            });
        }
        if !self.gpus.is_empty() {
            warnings.extend(
                self.nvidia_devices
                    .warnings()
                    .into_iter()
                    .map(|message| DetectionWarning { message }),
            );
        }
        if self.rocm.hipconfig_path.is_some() && !self.rocm.available {
            let detail = self
                .rocm
                .rocminfo_error
                .as_deref()
                .filter(|message| !message.is_empty())
                .unwrap_or("rocminfo did not report a supported gfx target");
            warnings.push(DetectionWarning {
                message: format!(
                    "ROCm/HIP tooling found, but HIP backend is not auto-selected: {detail}"
                ),
            });
        }
        if self.rocm.available {
            if let Some(error) = &self.rocm.rocm_smi_error {
                warnings.push(DetectionWarning {
                    message: format!("ROCm VRAM telemetry unavailable: {error}"),
                });
            }
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
    let rocm = detect_rocm(runner);
    let gpus = detect_gpus(runner);
    let nvidia_devices = detect_nvidia_device_nodes();
    let sccache = detect_sccache(runner);
    let metal = detect_metal(runner);
    let vulkan = detect_vulkan(runner);
    let cpu = detect_cpu_features(runner);
    let memory = detect_memory();
    let disk = detect_disk(disk_path);

    let (
        compiler,
        cmake,
        git,
        nvcc,
        rocm,
        gpus,
        nvidia_devices,
        sccache,
        metal,
        vulkan,
        cpu,
        memory,
        disk,
    ) = tokio::join!(
        compiler,
        cmake,
        git,
        nvcc,
        rocm,
        gpus,
        nvidia_devices,
        sccache,
        metal,
        vulkan,
        cpu,
        memory,
        disk
    );

    let gpu_probe_error = gpus.error;
    let gpus = gpus.devices;
    let cuda = cuda_compatibility(nvcc.as_ref().map(|info| &info.version), &gpus);

    let profile = SystemProfile {
        compiler,
        cmake,
        git,
        cuda,
        rocm,
        gpus,
        gpu_probe_error,
        nvidia_devices,
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

/// Live or probe-time GPU memory counters in MiB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpuVramInfo {
    /// Total VRAM in MiB.
    pub total_mb: u64,
    /// Used VRAM in MiB.
    pub used_mb: u64,
    /// Free VRAM in MiB.
    pub free_mb: u64,
}

/// ROCm/HIP GPU detected through `rocminfo`, optionally enriched by ROCm SMI tools.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RocmGpuInfo {
    /// Human-readable GPU name.
    pub name: String,
    /// Normalized HIP target, such as `gfx1100`.
    pub target: Option<String>,
    /// VRAM counters when `rocm-smi` or `amd-smi` can report them.
    pub vram: Option<GpuVramInfo>,
}

/// NVIDIA character device nodes visible to the lmml process.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NvidiaDeviceNodes {
    /// Whether `/dev/nvidiactl` exists and is a character device.
    pub control: bool,
    /// Whether `/dev/nvidia-uvm` exists and is a character device.
    pub uvm: bool,
    /// Number of `/dev/nvidiaN` GPU character devices visible.
    pub gpu_count: usize,
    /// Filesystem errors encountered while checking `/dev`.
    pub errors: Vec<String>,
}

impl NvidiaDeviceNodes {
    /// Return true when the minimal CUDA runtime device nodes are visible.
    pub fn usable_for_cuda(&self) -> bool {
        self.control && self.uvm && self.gpu_count > 0 && self.errors.is_empty()
    }

    /// Human-readable warnings for missing or inaccessible NVIDIA device nodes.
    pub fn warnings(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        if !self.control {
            warnings.push("/dev/nvidiactl is not visible to lmml".to_string());
        }
        if !self.uvm {
            warnings.push("/dev/nvidia-uvm is not visible to lmml".to_string());
        }
        if self.gpu_count == 0 {
            warnings.push("no /dev/nvidiaN GPU device nodes are visible to lmml".to_string());
        }
        warnings.extend(self.errors.iter().cloned());
        warnings
    }
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
    /// nvcc is too new and no longer supports one or more detected GPUs.
    ToolkitTooNew {
        /// GPU architecture no longer supported by this toolkit.
        gpu_arch: &'static str,
        /// Last known CUDA toolkit major version that supports this architecture.
        maximum_toolkit: &'static str,
        /// Detected CUDA toolkit version.
        found_toolkit: String,
    },
    /// nvcc was found but no CUDA-capable GPUs were detected.
    NoGpu,
    /// nvcc was not found, so CUDA backend is unavailable.
    NvccMissing,
}

/// ROCm/HIP toolchain and target information used for AMD GPU builds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RocmSupport {
    /// Whether lmml can safely auto-select the HIP backend.
    pub available: bool,
    /// `hipconfig` executable path, if detected.
    pub hipconfig_path: Option<PathBuf>,
    /// ROCm/HIP version string, when `hipconfig --version` reports one.
    pub version: Option<String>,
    /// ROCm root returned by `hipconfig -R`, exported as `HIP_PATH` for llama.cpp builds.
    pub hip_path: Option<PathBuf>,
    /// HIP clang executable inferred from `hipconfig -l`, if available.
    pub hip_clang_path: Option<PathBuf>,
    /// Normalized AMD GPU targets, such as `gfx1100`.
    pub targets: Vec<String>,
    /// ROCm/HIP GPUs matched from `rocminfo`, enriched with VRAM where available.
    pub devices: Vec<RocmGpuInfo>,
    /// `rocm-smi` executable path, if detected.
    pub rocm_smi_path: Option<PathBuf>,
    /// `amd-smi` executable path, if detected.
    pub amd_smi_path: Option<PathBuf>,
    /// Error returned by `rocminfo` when target detection failed.
    pub rocminfo_error: Option<String>,
    /// Error returned by ROCm VRAM telemetry when `rocm-smi` and `amd-smi` fail.
    pub rocm_smi_error: Option<String>,
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
    /// AMD ROCm/HIP backend with optional `gfx*` GPU targets.
    Rocm {
        /// Normalized AMD GPU targets, such as `gfx1100`.
        targets: Vec<String>,
    },
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
        "10.1" => Some("sm_101"),
        "10.1a" => Some("sm_101a"),
        "10.2" => Some("sm_102"),
        "10.2a" => Some("sm_102a"),
        "12.0" => Some("sm_120"),
        "12.0a" => Some("sm_120a"),
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
        if let Some((maximum, major)) = maximum_toolkit_for_arch(arch) {
            if version.major > major {
                return CudaCompatibility::ToolkitTooNew {
                    gpu_arch: arch,
                    maximum_toolkit: maximum,
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

async fn detect_rocm<R>(runner: &R) -> RocmSupport
where
    R: CommandRunner + Sync,
{
    let Some(hipconfig_path) = which(runner, "hipconfig").await else {
        return RocmSupport::default();
    };
    let program = hipconfig_path.to_string_lossy();
    let version_output = runner.run(&program, &["--version"], None).await;
    let version = version_output.success.then(|| {
        parse_version(&version_output.stdout)
            .or_else(|| parse_version(&version_output.stderr))
            .unwrap_or_else(|| first_line(&version_output.stdout, &version_output.stderr))
    });

    let hip_path = rocm_hip_path_from_hipconfig(runner, &program).await;
    let hip_clang_path = rocm_hip_clang_from_hipconfig(runner, &program).await;
    let rocminfo = runner.run("rocminfo", &[], None).await;
    let mut devices = if rocminfo.success {
        parse_rocm_devices(&rocminfo.stdout)
    } else {
        Vec::new()
    };
    let targets = if !devices.is_empty() {
        devices
            .iter()
            .filter_map(|device| device.target.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    } else if rocminfo.success {
        parse_rocm_targets(&rocminfo.stdout)
    } else {
        Vec::new()
    };
    let vram_probe = detect_rocm_vram_with_runner(runner).await;
    merge_rocm_vram(&mut devices, &targets, &vram_probe.devices);
    let rocminfo_error = (!rocminfo.success)
        .then(|| first_line(&rocminfo.stderr, &rocminfo.stdout))
        .filter(|message| !message.is_empty());

    RocmSupport {
        available: !targets.is_empty(),
        hipconfig_path: Some(hipconfig_path),
        version: version.filter(|value| !value.is_empty()),
        hip_path,
        hip_clang_path,
        targets,
        devices,
        rocm_smi_path: vram_probe.rocm_smi_path,
        amd_smi_path: vram_probe.amd_smi_path,
        rocminfo_error,
        rocm_smi_error: vram_probe.error,
    }
}

/// Result of probing ROCm VRAM counters with ROCm SMI tooling.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RocmVramProbe {
    /// `rocm-smi` executable path, if detected.
    pub rocm_smi_path: Option<PathBuf>,
    /// `amd-smi` executable path, if detected.
    pub amd_smi_path: Option<PathBuf>,
    /// One VRAM counter set per ROCm GPU, in SMI device order.
    pub devices: Vec<GpuVramInfo>,
    /// Probe failure reason when `rocm-smi` and `amd-smi` were unavailable or unusable.
    pub error: Option<String>,
}

/// Probe current ROCm VRAM counters using host `rocm-smi` or `amd-smi` commands.
pub async fn detect_rocm_vram() -> RocmVramProbe {
    let runner = RealCommandRunner;
    detect_rocm_vram_with_runner(&runner).await
}

/// Probe current ROCm VRAM counters with an injected command runner.
pub async fn detect_rocm_vram_with_runner<R>(runner: &R) -> RocmVramProbe
where
    R: CommandRunner + Sync,
{
    let rocm_smi_path = which(runner, "rocm-smi").await;
    if let Some(path) = &rocm_smi_path {
        let program = path.to_string_lossy();
        let output = runner
            .run(&program, &["--showmeminfo", "vram", "--csv"], None)
            .await;
        if output.success {
            let devices = parse_rocm_smi_vram_csv(&output.stdout);
            if !devices.is_empty() {
                return RocmVramProbe {
                    rocm_smi_path,
                    devices,
                    ..RocmVramProbe::default()
                };
            }
        }
    }

    let amd_smi_path = which(runner, "amd-smi").await;
    if let Some(path) = &amd_smi_path {
        let program = path.to_string_lossy();
        let output = runner
            .run(&program, &["monitor", "--vram-usage", "--csv"], None)
            .await;
        if output.success {
            let devices = parse_amd_smi_monitor_vram(&output.stdout);
            if !devices.is_empty() {
                return RocmVramProbe {
                    rocm_smi_path,
                    amd_smi_path,
                    devices,
                    error: None,
                };
            }
        }
    }

    RocmVramProbe {
        rocm_smi_path,
        amd_smi_path,
        devices: Vec::new(),
        error: Some("rocm-smi and amd-smi did not report VRAM totals".to_string()),
    }
}

async fn rocm_hip_path_from_hipconfig<R>(runner: &R, program: &str) -> Option<PathBuf>
where
    R: CommandRunner + Sync,
{
    let output = runner.run(program, &["-R"], None).await;
    output
        .success
        .then(|| {
            output
                .stdout
                .lines()
                .next()
                .map(str::trim)
                .unwrap_or_default()
        })
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

async fn rocm_hip_clang_from_hipconfig<R>(runner: &R, program: &str) -> Option<PathBuf>
where
    R: CommandRunner + Sync,
{
    let output = runner.run(program, &["-l"], None).await;
    output
        .success
        .then(|| {
            output
                .stdout
                .lines()
                .next()
                .map(str::trim)
                .unwrap_or_default()
        })
        .filter(|path| !path.is_empty())
        .map(|path| PathBuf::from(path).join("clang"))
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

async fn detect_nvidia_device_nodes() -> NvidiaDeviceNodes {
    detect_nvidia_device_nodes_in(Path::new("/dev"))
}

fn detect_nvidia_device_nodes_in(dev: &Path) -> NvidiaDeviceNodes {
    let mut nodes = NvidiaDeviceNodes {
        control: is_char_device(&dev.join("nvidiactl")),
        uvm: is_char_device(&dev.join("nvidia-uvm")),
        gpu_count: 0,
        errors: Vec::new(),
    };

    match std::fs::read_dir(dev) {
        Ok(entries) => {
            for entry in entries {
                match entry {
                    Ok(entry) => {
                        let name = entry.file_name();
                        let Some(name) = name.to_str() else {
                            continue;
                        };
                        let Some(suffix) = name.strip_prefix("nvidia") else {
                            continue;
                        };
                        if suffix.chars().all(|ch| ch.is_ascii_digit())
                            && is_char_device(&entry.path())
                        {
                            nodes.gpu_count += 1;
                        }
                    }
                    Err(error) => nodes
                        .errors
                        .push(format!("failed to read NVIDIA device node: {error}")),
                }
            }
        }
        Err(error) => nodes
            .errors
            .push(format!("failed to read {}: {error}", dev.display())),
    }

    nodes
}

fn is_char_device(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|metadata| metadata.file_type().is_char_device())
        .unwrap_or(false)
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

/// Parse and normalize AMD ROCm GPU targets from `rocminfo` output.
pub fn parse_rocm_targets(output: &str) -> Vec<String> {
    output
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter_map(normalize_rocm_target)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn normalize_rocm_target(token: &str) -> Option<String> {
    let token = token.trim();
    let suffix = token.strip_prefix("gfx")?;
    if suffix.len() < 3 || !suffix.chars().all(|ch| ch.is_ascii_alphanumeric()) {
        return None;
    }
    if !suffix.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        return None;
    }
    match token {
        "gfx000" => None,
        // llama.cpp upstream documents gfx1035 hosts as compiling for gfx1030.
        "gfx1035" => Some("gfx1030".to_string()),
        _ => Some(token.to_string()),
    }
}

/// Parse ROCm GPU names and targets from `rocminfo` output.
pub fn parse_rocm_devices(output: &str) -> Vec<RocmGpuInfo> {
    #[derive(Default)]
    struct PendingDevice {
        name: Option<String>,
        target: Option<String>,
    }

    fn flush(pending: &mut PendingDevice, devices: &mut Vec<RocmGpuInfo>) {
        let Some(target) = pending.target.take() else {
            pending.name = None;
            return;
        };
        let name = pending
            .name
            .take()
            .filter(|name| !name.eq_ignore_ascii_case("cpu"))
            .unwrap_or_else(|| format!("AMD ROCm GPU {target}"));
        devices.push(RocmGpuInfo {
            name,
            target: Some(target),
            vram: None,
        });
    }

    let mut devices = Vec::new();
    let mut pending = PendingDevice::default();
    for line in output.lines().map(str::trim) {
        if line.starts_with("Agent ") {
            flush(&mut pending, &mut devices);
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if key == "Name" {
            if let Some(target) = normalize_rocm_target(value) {
                pending.target = Some(target);
            } else if pending.name.is_none() && !value.is_empty() {
                pending.name = Some(value.to_string());
            }
        } else if key == "Marketing Name" && !value.is_empty() {
            pending.name = Some(value.to_string());
        }
    }
    flush(&mut pending, &mut devices);
    devices
}

/// Parse `rocm-smi --showmeminfo vram --csv` output into MiB counters.
pub fn parse_rocm_smi_vram_csv(output: &str) -> Vec<GpuVramInfo> {
    let rows = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('='))
        .map(split_csv_row)
        .collect::<Vec<_>>();
    let Some(header) = rows.first() else {
        return Vec::new();
    };
    let total_index = header
        .iter()
        .position(|column| column_contains_all(column, &["total", "memory"]));
    let used_index = header
        .iter()
        .position(|column| column_contains_all(column, &["used", "memory"]));
    let Some(total_index) = total_index else {
        return Vec::new();
    };
    let Some(used_index) = used_index else {
        return Vec::new();
    };

    rows.iter()
        .skip(1)
        .filter_map(|row| {
            let total_cell = row.get(total_index)?;
            let used_cell = row.get(used_index)?;
            let total_mb = parse_memory_mb(total_cell, &header[total_index])?;
            let used_mb = parse_memory_mb(used_cell, &header[used_index])?;
            Some(GpuVramInfo {
                total_mb,
                used_mb,
                free_mb: total_mb.saturating_sub(used_mb),
            })
        })
        .collect()
}

/// Parse `amd-smi monitor --vram-usage --csv` or table output into MiB counters.
pub fn parse_amd_smi_monitor_vram(output: &str) -> Vec<GpuVramInfo> {
    let csv = parse_amd_smi_monitor_vram_csv(output);
    if !csv.is_empty() {
        return csv;
    }
    parse_amd_smi_monitor_vram_table(output)
}

fn parse_amd_smi_monitor_vram_csv(output: &str) -> Vec<GpuVramInfo> {
    let rows = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('='))
        .map(split_csv_row)
        .collect::<Vec<_>>();
    let Some(header) = rows.first() else {
        return Vec::new();
    };
    let used_index = header.iter().position(|column| {
        let column = column.to_ascii_lowercase();
        column.contains("vram") && column.contains("used")
    });
    let total_index = header.iter().position(|column| {
        let column = column.to_ascii_lowercase();
        column.contains("vram") && column.contains("total")
    });
    let Some(used_index) = used_index else {
        return Vec::new();
    };
    let Some(total_index) = total_index else {
        return Vec::new();
    };
    if used_index == total_index {
        return Vec::new();
    }

    rows.iter()
        .skip(1)
        .filter_map(|row| {
            let used_cell = row.get(used_index)?;
            let total_cell = row.get(total_index)?;
            let used_mb = parse_memory_mb(used_cell, &header[used_index])?;
            let total_mb = parse_memory_mb(total_cell, &header[total_index])?;
            Some(GpuVramInfo {
                total_mb,
                used_mb,
                free_mb: total_mb.saturating_sub(used_mb),
            })
        })
        .collect()
}

fn parse_amd_smi_monitor_vram_table(output: &str) -> Vec<GpuVramInfo> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| line.chars().next().is_some_and(|ch| ch.is_ascii_digit()))
        .filter_map(|line| {
            let tokens = line.split_whitespace().collect::<Vec<_>>();
            let [.., used_value, used_unit, total_value, total_unit] = tokens.as_slice() else {
                return None;
            };
            let used_mb = parse_memory_mb(used_value, used_unit)?;
            let total_mb = parse_memory_mb(total_value, total_unit)?;
            Some(GpuVramInfo {
                total_mb,
                used_mb,
                free_mb: total_mb.saturating_sub(used_mb),
            })
        })
        .collect()
}

fn merge_rocm_vram(devices: &mut Vec<RocmGpuInfo>, targets: &[String], vram: &[GpuVramInfo]) {
    for (index, memory) in vram.iter().copied().enumerate() {
        if let Some(device) = devices.get_mut(index) {
            device.vram = Some(memory);
        } else {
            let target = targets.get(index).cloned();
            let name = target
                .as_ref()
                .map(|target| format!("AMD ROCm GPU {target}"))
                .unwrap_or_else(|| format!("AMD ROCm GPU {index}"));
            devices.push(RocmGpuInfo {
                name,
                target,
                vram: Some(memory),
            });
        }
    }
}

fn split_csv_row(row: &str) -> Vec<String> {
    let mut columns = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for character in row.chars() {
        match character {
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                columns.push(current.trim().trim_matches('"').to_string());
                current.clear();
            }
            _ => current.push(character),
        }
    }
    columns.push(current.trim().trim_matches('"').to_string());
    columns
}

fn column_contains_all(column: &str, needles: &[&str]) -> bool {
    let column = column.to_ascii_lowercase();
    needles.iter().all(|needle| column.contains(needle))
}

fn parse_memory_mb(value: &str, unit_hint: &str) -> Option<u64> {
    let number = parse_first_u64(value)?;
    let lower = format!("{unit_hint} {value}").to_ascii_lowercase();
    if lower.contains("(b)") || lower.contains(" bytes") {
        Some(number / (1024 * 1024))
    } else if lower.contains("(kb)") || lower.contains(" kib") || lower.contains(" kb") {
        Some(number / 1024)
    } else if lower.contains("(gb)") || lower.contains(" gib") || lower.contains(" gb") {
        Some(number * 1024)
    } else {
        Some(number)
    }
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
        "sm_100" | "sm_100a" | "sm_101" | "sm_101a" | "sm_102" | "sm_102a" => Some(("12.4", 12, 4)),
        "sm_120" | "sm_120a" => Some(("13.0", 13, 0)),
        _ => None,
    }
}

fn maximum_toolkit_for_arch(arch: &str) -> Option<(&'static str, u32)> {
    match arch {
        "sm_37" | "sm_50" | "sm_52" | "sm_53" | "sm_60" | "sm_61" | "sm_62" | "sm_70" | "sm_72" => {
            Some(("12.x", 12))
        }
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
            ("10.1", "sm_101"),
            ("10.1a", "sm_101a"),
            ("10.2", "sm_102"),
            ("10.2a", "sm_102a"),
            ("12.0", "sm_120"),
            ("12.0a", "sm_120a"),
        ];
        for (cap, arch) in cases {
            assert_eq!(compute_cap_to_arch(cap), Some(arch));
        }
        assert_eq!(compute_cap_to_arch("13.0"), None);
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
        let cuda13 = CudaVersion::new(13, 0);

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
            cuda_compatibility(Some(&cuda13), &[gpu("RTX 50", 16, "12.0")]),
            CudaCompatibility::Compatible {
                archs: vec!["sm_120"],
            }
        );
        assert_eq!(
            cuda_compatibility(Some(&cuda13), &[gpu("GTX 1080 Ti", 11, "6.1")]),
            CudaCompatibility::ToolkitTooNew {
                gpu_arch: "sm_61",
                maximum_toolkit: "12.x",
                found_toolkit: "13.0".to_string(),
            }
        );
        assert_eq!(
            cuda_compatibility(Some(&current), &[gpu("RTX 50", 16, "12.0")]),
            CudaCompatibility::ToolkitTooOld {
                gpu_arch: "sm_120",
                minimum_toolkit: "13.0",
                found_toolkit: "12.4".to_string(),
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
    fn parses_rocm_targets_from_rocminfo() {
        let output = "\
Agent 2
  Name:                    gfx1100
  Marketing Name:          AMD Radeon RX 7900 XTX
Agent 3
  Name:                    gfx1201
  Marketing Name:          AMD Radeon AI PRO R9700
  ISA Info:
    ISA 2
      Name:                    amdgcn-amd-amdhsa--gfx12-generic
Agent 4
  Name:                    gfx1035
Agent 5
  Name:                    gfx000
";
        assert_eq!(
            parse_rocm_targets(output),
            vec![
                "gfx1030".to_string(),
                "gfx1100".to_string(),
                "gfx1201".to_string()
            ]
        );
    }

    #[test]
    fn parses_rocm_devices_from_rocminfo() {
        let output = "\
Agent 1
  Name:                    AMD Ryzen CPU
Agent 2
  Name:                    gfx1100
  Marketing Name:          AMD Radeon RX 7900 XTX
Agent 3
  Name:                    gfx1201
  Marketing Name:          AMD Radeon AI PRO R9700
Agent 4
  Name:                    gfx1035
";
        assert_eq!(
            parse_rocm_devices(output),
            vec![
                RocmGpuInfo {
                    name: "AMD Radeon RX 7900 XTX".to_string(),
                    target: Some("gfx1100".to_string()),
                    vram: None,
                },
                RocmGpuInfo {
                    name: "AMD Radeon AI PRO R9700".to_string(),
                    target: Some("gfx1201".to_string()),
                    vram: None,
                },
                RocmGpuInfo {
                    name: "AMD ROCm GPU gfx1030".to_string(),
                    target: Some("gfx1030".to_string()),
                    vram: None,
                },
            ]
        );
    }

    #[test]
    fn parses_rocm_smi_vram_csv_as_mib() {
        let output = "\
device,vram Total Memory (B),vram Total Used Memory (B)
card0,25769803776,1073741824
card1,34359738368,2147483648
";
        assert_eq!(
            parse_rocm_smi_vram_csv(output),
            vec![
                GpuVramInfo {
                    total_mb: 24_576,
                    used_mb: 1_024,
                    free_mb: 23_552,
                },
                GpuVramInfo {
                    total_mb: 32_768,
                    used_mb: 2_048,
                    free_mb: 30_720,
                },
            ]
        );
    }

    #[test]
    fn parses_amd_smi_monitor_vram_output() {
        let csv = "\
GPU,VRAM_USED,VRAM_TOTAL
0,14 MB,33406976 KB
";
        assert_eq!(
            parse_amd_smi_monitor_vram(csv),
            vec![GpuVramInfo {
                total_mb: 32_624,
                used_mb: 14,
                free_mb: 32_610,
            }]
        );

        let table = "\
GPU  XCP    POWER    GPU_T    MEM_T   GFX_CLK    GFX%    MEM%   ENC%   DEC%   VRAM_USED   VRAM_TOTAL
  0    0    110 W    47 C     39 C    210 MHz    0 %     0 %    N/A    0 %      14 MB      32624 MB
";
        assert_eq!(
            parse_amd_smi_monitor_vram(table),
            vec![GpuVramInfo {
                total_mb: 32_624,
                used_mb: 14,
                free_mb: 32_610,
            }]
        );
    }

    #[tokio::test]
    async fn rocm_probe_detects_hipconfig_and_gfx_targets() {
        let runner = FakeRunner::default()
            .with(
                "which",
                &["hipconfig"],
                FakeRunner::success("/opt/rocm/bin/hipconfig\n"),
            )
            .with(
                "/opt/rocm/bin/hipconfig",
                &["--version"],
                FakeRunner::success("HIP version: 7.0.1\n"),
            )
            .with(
                "/opt/rocm/bin/hipconfig",
                &["-R"],
                FakeRunner::success("/opt/rocm\n"),
            )
            .with(
                "/opt/rocm/bin/hipconfig",
                &["-l"],
                FakeRunner::success("/opt/rocm/llvm/bin\n"),
            )
            .with(
                "rocminfo",
                &[],
                FakeRunner::success(
                    "Agent 2\n  Name: gfx942\n  Marketing Name: AMD Instinct MI300X\n",
                ),
            )
            .with(
                "which",
                &["rocm-smi"],
                FakeRunner::success("/opt/rocm/bin/rocm-smi\n"),
            )
            .with(
                "/opt/rocm/bin/rocm-smi",
                &["--showmeminfo", "vram", "--csv"],
                FakeRunner::success(
                    "device,vram Total Memory (B),vram Total Used Memory (B)\ncard0,206158430208,1073741824\n",
                ),
            );

        assert_eq!(
            detect_rocm(&runner).await,
            RocmSupport {
                available: true,
                hipconfig_path: Some(PathBuf::from("/opt/rocm/bin/hipconfig")),
                version: Some("7.0.1".to_string()),
                hip_path: Some(PathBuf::from("/opt/rocm")),
                hip_clang_path: Some(PathBuf::from("/opt/rocm/llvm/bin/clang")),
                targets: vec!["gfx942".to_string()],
                devices: vec![RocmGpuInfo {
                    name: "AMD Instinct MI300X".to_string(),
                    target: Some("gfx942".to_string()),
                    vram: Some(GpuVramInfo {
                        total_mb: 196_608,
                        used_mb: 1_024,
                        free_mb: 195_584,
                    }),
                }],
                rocm_smi_path: Some(PathBuf::from("/opt/rocm/bin/rocm-smi")),
                amd_smi_path: None,
                rocminfo_error: None,
                rocm_smi_error: None,
            }
        );
    }

    #[tokio::test]
    async fn rocm_vram_probe_falls_back_to_amd_smi() {
        let runner = FakeRunner::default()
            .with(
                "which",
                &["amd-smi"],
                FakeRunner::success("/opt/rocm/bin/amd-smi\n"),
            )
            .with(
                "/opt/rocm/bin/amd-smi",
                &["monitor", "--vram-usage", "--csv"],
                FakeRunner::success("GPU,VRAM_USED,VRAM_TOTAL\n0,14 MB,32624 MB\n"),
            );

        assert_eq!(
            detect_rocm_vram_with_runner(&runner).await,
            RocmVramProbe {
                rocm_smi_path: None,
                amd_smi_path: Some(PathBuf::from("/opt/rocm/bin/amd-smi")),
                devices: vec![GpuVramInfo {
                    total_mb: 32_624,
                    used_mb: 14,
                    free_mb: 32_610,
                }],
                error: None,
            }
        );
    }

    #[tokio::test]
    async fn rocm_vram_probe_reports_missing_smi_tools() {
        let runner = FakeRunner::default();

        assert_eq!(
            detect_rocm_vram_with_runner(&runner).await,
            RocmVramProbe {
                rocm_smi_path: None,
                amd_smi_path: None,
                devices: Vec::new(),
                error: Some("rocm-smi and amd-smi did not report VRAM totals".to_string()),
            }
        );
    }

    #[test]
    fn recommended_backend_prefers_cuda_then_metal_then_rocm_then_vulkan_then_cpu() {
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

        profile.cuda = CudaCompatibility::ToolkitTooNew {
            gpu_arch: "sm_61",
            maximum_toolkit: "12.x",
            found_toolkit: "13.0".to_string(),
        };
        profile.metal.available = true;
        assert_eq!(profile.recommended_backend(), BuildBackend::Metal);

        profile.cuda = CudaCompatibility::NvccMissing;
        assert_eq!(profile.recommended_backend(), BuildBackend::Metal);

        profile.metal.available = false;
        profile.rocm = RocmSupport {
            available: true,
            targets: vec!["gfx1100".to_string()],
            ..RocmSupport::default()
        };
        assert_eq!(
            profile.recommended_backend(),
            BuildBackend::Rocm {
                targets: vec!["gfx1100".to_string()],
            }
        );

        profile.rocm = RocmSupport::default();
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
            rocm: RocmSupport::default(),
            gpus: Vec::new(),
            gpu_probe_error: None,
            nvidia_devices: NvidiaDeviceNodes {
                control: false,
                uvm: false,
                gpu_count: 0,
                errors: Vec::new(),
            },
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

    #[test]
    fn nvidia_device_node_warnings_report_missing_runtime_nodes() {
        let nodes = NvidiaDeviceNodes {
            control: true,
            uvm: false,
            gpu_count: 1,
            errors: Vec::new(),
        };

        assert_eq!(
            nodes.warnings(),
            vec!["/dev/nvidia-uvm is not visible to lmml".to_string()]
        );
        assert!(!nodes.usable_for_cuda());
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
