//! Hardware detection engine for lmml.
//!
//! Probes the system for OS, GPU, and CPU capabilities, then maps
//! results to optimal llama.cpp build flags.
//!
//! The main entry point is [`run_all`], which returns a [`ProbeResult`].
//!
//! ## Sub-modules
//! - [`cuda`] — NVIDIA GPU + CUDA toolkit detection
//! - [`rocm`] — AMD ROCm/HIP detection
//! - [`vulkan`] — Vulkan SDK detection
//! - [`metal`] — macOS Metal detection
//! - [`cpu`] — CPU feature detection (AVX2, NEON, etc.)
//! - [`cmake`] — Map ProbeResult to cmake flags

pub mod blas;
pub mod cmake;
pub mod cpu;
pub mod cuda;
pub mod metal;
pub mod os;
pub mod rocm;
pub mod vulkan;

/// Events sent from the probe engine to the TUI.
#[derive(Debug, Clone)]
pub enum ProbeEvent {
    Line(String),
    Complete(Box<Result<ProbeResult, String>>),
}

/// Result of a full hardware probe.
#[derive(Debug, Clone, Default)]
pub struct ProbeResult {
    pub os: OsInfo,
    pub cuda: CudaProbe,
    pub blas: BlasProbe,
    pub rocm: RocmProbe,
    pub vulkan: bool,
    pub metal: bool,
    pub cpu: CpuInfo,
    pub ram_gb: u32,
    pub suggested_cmake_flags: Vec<String>,
    pub suggested_ngl: u32,
    pub warnings: Vec<String>,
    pub log_lines: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct OsInfo {
    pub os_type: String,
    pub arch: String,
    pub kernel: String,
}

#[derive(Debug, Clone, Default)]
pub enum CudaProbe {
    Found {
        version: String,
        gpu_name: String,
        vram_gb: u32,
        compute_cap: String,
        gpus: Vec<CudaGpu>,
    },
    #[default]
    NotFound,
    Error(String),
}

/// Single NVIDIA GPU detected by CUDA probing.
#[derive(Debug, Clone, Default)]
pub struct CudaGpu {
    pub name: String,
    pub vram_gb: u32,
    pub compute_cap: String,
}

#[derive(Debug, Clone, Default)]
pub enum BlasProbe {
    Found {
        library: String,
        version: String,
    },
    #[default]
    NotFound,
}

#[derive(Debug, Clone, Default)]
pub enum RocmProbe {
    Found {
        version: String,
        gpu_target: String,
    },
    #[default]
    NotFound,
    Error(String),
}

#[derive(Debug, Clone, Default)]
pub struct CpuInfo {
    pub model: String,
    pub cores: u32,
    pub threads: u32,
    pub features: Vec<String>, // avx2, avx512, neon, etc.
}

/// Run all hardware probes in sequence. Sends progress events on `tx`.
pub async fn run_all(tx: tokio::sync::mpsc::Sender<ProbeEvent>) {
    let mut result = ProbeResult::default();

    // OS probe
    let os_info = os::detect();
    result.os = os_info.clone();
    let _ = tx
        .send(ProbeEvent::Line(format!(
            "✓ OS: {} {} ({})",
            os_info.os_type, os_info.arch, os_info.kernel
        )))
        .await;

    // RAM
    result.ram_gb = detect_ram();
    let _ = tx
        .send(ProbeEvent::Line(format!("✓ RAM: {} GB", result.ram_gb)))
        .await;

    // CUDA
    match cuda::detect().await {
        CudaProbe::Found {
            version,
            gpu_name,
            vram_gb,
            compute_cap,
            gpus,
        } => {
            result.cuda = CudaProbe::Found {
                version: version.clone(),
                gpu_name: gpu_name.clone(),
                vram_gb,
                compute_cap: compute_cap.clone(),
                gpus: gpus.clone(),
            };
            let gpu_count = gpus.len().max(1);
            let _ = tx
                .send(ProbeEvent::Line(format!(
                    "✓ NVIDIA CUDA {version} detected ({gpu_name}, {vram_gb} GB VRAM, {gpu_count} GPU(s))"
                )))
                .await;
        }
        CudaProbe::NotFound => {
            let suggestion = if cfg!(target_os = "linux") {
                " — install with: sudo apt install nvidia-cuda-toolkit"
            } else {
                ""
            };
            let _ = tx
                .send(ProbeEvent::Line(format!(
                    "○ CUDA: not detected{suggestion}"
                )))
                .await;
        }
        CudaProbe::Error(ref e) => {
            let _ = tx
                .send(ProbeEvent::Line(format!("⚠ CUDA probe error: {e}")))
                .await;
        }
    }

    // ROCm
    match rocm::detect().await {
        RocmProbe::Found {
            version,
            gpu_target,
        } => {
            result.rocm = RocmProbe::Found {
                version: version.clone(),
                gpu_target: gpu_target.clone(),
            };
            let _ = tx
                .send(ProbeEvent::Line(format!(
                    "✓ ROCm {version} detected (target: {gpu_target})"
                )))
                .await;
        }
        RocmProbe::NotFound => {
            let suggestion = if cfg!(target_os = "linux") {
                " — install with: sudo apt install rocm-dev"
            } else {
                ""
            };
            let _ = tx
                .send(ProbeEvent::Line(format!(
                    "○ ROCm: not detected{suggestion}"
                )))
                .await;
        }
        RocmProbe::Error(ref e) => {
            let _ = tx
                .send(ProbeEvent::Line(format!("⚠ ROCm probe error: {e}")))
                .await;
        }
    }

    // Vulkan
    result.vulkan = vulkan::detect().await;
    if result.vulkan {
        let _ = tx
            .send(ProbeEvent::Line("✓ Vulkan SDK detected".into()))
            .await;
    } else {
        let suggestion = if cfg!(target_os = "linux") {
            " — install with: sudo apt install libvulkan-dev"
        } else if cfg!(target_os = "macos") {
            " — install with: brew install molten-vk"
        } else {
            ""
        };
        let _ = tx
            .send(ProbeEvent::Line(format!(
                "○ Vulkan: not detected{suggestion}"
            )))
            .await;
    }

    // Metal
    result.metal = metal::detect().await;
    if result.metal {
        let _ = tx
            .send(ProbeEvent::Line("✓ Metal detected (macOS)".into()))
            .await;
    } else if cfg!(target_os = "macos") {
        let _ = tx
            .send(ProbeEvent::Line(
                "○ Metal: not detected — ensure you're on macOS 12+".into(),
            ))
            .await;
    } else {
        let _ = tx
            .send(ProbeEvent::Line(
                "○ Metal: not detected (macOS only)".into(),
            ))
            .await;
    }

    // CPU
    result.cpu = cpu::detect().await;
    let _ = tx
        .send(ProbeEvent::Line(format!(
            "✓ CPU: {} ({} features, {} cores / {} threads)",
            result.cpu.model,
            result.cpu.features.join(", "),
            result.cpu.cores,
            result.cpu.threads
        )))
        .await;

    // BLAS
    result.blas = blas::detect().await;
    match &result.blas {
        BlasProbe::Found { library, version } => {
            let _ = tx
                .send(ProbeEvent::Line(format!("✓ {library} {version} detected")))
                .await;
        }
        BlasProbe::NotFound => {
            let suggestion = if cfg!(target_os = "linux") {
                " — install with: sudo apt install libopenblas-dev"
            } else if cfg!(target_os = "macos") {
                " — install with: brew install openblas"
            } else {
                ""
            };
            let _ = tx
                .send(ProbeEvent::Line(format!(
                    "○ BLAS: not detected{suggestion}"
                )))
                .await;
        }
    }

    // ccache
    let ccache_found = std::process::Command::new("which")
        .arg("ccache")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if ccache_found {
        let _ = tx
            .send(ProbeEvent::Line(
                "✓ ccache found — rebuilds will be faster".into(),
            ))
            .await;
    }

    // Generate cmake flags
    result.suggested_cmake_flags = cmake::generate_flags(&result);
    result.suggested_ngl = cmake::suggest_ngl(&result);

    let _ = tx
        .send(ProbeEvent::Line(format!(
            "→ Suggested cmake flags: {}",
            result.suggested_cmake_flags.join(" ")
        )))
        .await;
    let _ = tx
        .send(ProbeEvent::Line(format!(
            "→ Suggested ngl: {}",
            result.suggested_ngl
        )))
        .await;

    let _ = tx.send(ProbeEvent::Complete(Box::new(Ok(result)))).await;
}

/// Detect total system RAM in GB.
fn detect_ram() -> u32 {
    // Fallback: try sysinfo
    use sysinfo::System;
    let mut sys = System::new();
    sys.refresh_memory();
    let total_mb = sys.total_memory() / 1024 / 1024;
    total_mb as u32
}
