//! AI-oriented GPU catalog for interpreting detected accelerator names.
//!
//! The hardware probes collect raw device names from tools such as
//! `nvidia-smi` and `vulkaninfo`. This module maps those names to a stable
//! support catalog so the TUI can show backend, VRAM, and local-AI guidance
//! without hard-coding product strings in the UI layer.

use std::collections::BTreeSet;
use std::fmt;

use crate::SystemProfile;

/// GPU or accelerator vendor family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuVendor {
    /// NVIDIA CUDA-capable GPUs.
    Nvidia,
    /// AMD ROCm/HIP-capable GPUs.
    Amd,
    /// Intel oneAPI/OpenVINO/XPU accelerators.
    Intel,
}

impl fmt::Display for GpuVendor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GpuVendor::Nvidia => formatter.write_str("NVIDIA"),
            GpuVendor::Amd => formatter.write_str("AMD"),
            GpuVendor::Intel => formatter.write_str("Intel"),
        }
    }
}

/// Broad deployment class for an AI accelerator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuUseCase {
    /// Cluster or server-class training/inference hardware.
    Datacenter,
    /// Workstation GPUs intended for professional local AI.
    Workstation,
    /// High-end consumer or prosumer local-AI cards.
    ConsumerHighEnd,
    /// Mid-range consumer cards with enough VRAM for 8B-14B models.
    ConsumerMidRange,
    /// Entry local-AI cards for 7B-class quantized models.
    ConsumerEntry,
    /// Headless appliance or repurposed board used as a LAN inference node.
    HeadlessAppliance,
    /// Integrated NPUs or iGPUs for very light local AI.
    Integrated,
}

impl fmt::Display for GpuUseCase {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GpuUseCase::Datacenter => formatter.write_str("datacenter"),
            GpuUseCase::Workstation => formatter.write_str("workstation"),
            GpuUseCase::ConsumerHighEnd => formatter.write_str("prosumer"),
            GpuUseCase::ConsumerMidRange => formatter.write_str("mid-range"),
            GpuUseCase::ConsumerEntry => formatter.write_str("entry"),
            GpuUseCase::HeadlessAppliance => formatter.write_str("headless appliance"),
            GpuUseCase::Integrated => formatter.write_str("integrated"),
        }
    }
}

/// Preferred AI software ecosystem for a known accelerator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiBackend {
    /// NVIDIA CUDA, the primary llama.cpp GPU path for NVIDIA cards.
    Cuda,
    /// AMD ROCm/HIP, when the host driver and platform support it.
    Rocm,
    /// Vulkan backend, useful for non-ROCm AMD or broad GPU fallback.
    Vulkan,
    /// Intel oneAPI/OpenVINO/XPU software stack.
    OneApiOpenVino,
    /// Intel Gaudi/Habana software stack.
    Gaudi,
    /// CPU/NPU vendor runtimes for integrated low-power inference.
    Npu,
}

impl fmt::Display for AiBackend {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AiBackend::Cuda => formatter.write_str("CUDA"),
            AiBackend::Rocm => formatter.write_str("ROCm/HIP"),
            AiBackend::Vulkan => formatter.write_str("Vulkan"),
            AiBackend::OneApiOpenVino => formatter.write_str("oneAPI/OpenVINO"),
            AiBackend::Gaudi => formatter.write_str("Gaudi"),
            AiBackend::Npu => formatter.write_str("NPU"),
        }
    }
}

/// Static catalog entry for a GPU or AI accelerator relevant to local LLM use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnownGpu {
    /// Canonical display name.
    pub canonical_name: &'static str,
    /// Vendor family.
    pub vendor: GpuVendor,
    /// Deployment/use-case class.
    pub use_case: GpuUseCase,
    /// Nominal accelerator memory capacity, when meaningful for the device.
    pub memory_gb: Option<u16>,
    /// Memory technology label.
    pub memory_kind: &'static str,
    /// Preferred software ecosystem for AI workloads.
    pub backend: AiBackend,
    /// Short local-AI guidance string.
    pub local_ai_tier: &'static str,
    aliases: &'static [&'static str],
}

impl KnownGpu {
    /// Return a concise display summary suitable for TUI rows and diagnostics.
    pub fn summary(&self) -> String {
        let memory = self
            .memory_gb
            .map(|gb| format!("{gb}GB {}", self.memory_kind))
            .unwrap_or_else(|| self.memory_kind.to_string());
        format!(
            "{} · {} · {} · {} · {}",
            self.canonical_name, memory, self.backend, self.use_case, self.local_ai_tier
        )
    }

    /// Return the aliases used for name matching.
    pub fn aliases(&self) -> &'static [&'static str] {
        self.aliases
    }
}

/// Source that produced a catalog match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GpuCatalogSource {
    /// CUDA device enumeration from `nvidia-smi`.
    Cuda,
    /// Vulkan summary device lines.
    Vulkan,
    /// CPU model string for integrated accelerators.
    Cpu,
}

/// A known accelerator matched from a detected host profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuCatalogMatch {
    /// Raw detected device line or name.
    pub detected_name: String,
    /// Static catalog descriptor.
    pub gpu: &'static KnownGpu,
    /// Probe source that supplied the raw name.
    pub source: GpuCatalogSource,
}

/// Return the known GPU catalog used by lmml.
pub fn known_gpus() -> &'static [KnownGpu] {
    KNOWN_GPUS
}

/// Match a raw GPU or accelerator name against the known local-AI catalog.
pub fn known_gpu(name: &str) -> Option<&'static KnownGpu> {
    let normalized = normalize_name(name);
    KNOWN_GPUS.iter().find(|gpu| {
        gpu.aliases
            .iter()
            .map(|alias| normalize_name(alias))
            .any(|alias| normalized.contains(&alias))
    })
}

/// Collect known accelerators from a complete system profile.
pub fn matches_from_system_profile(profile: &SystemProfile) -> Vec<GpuCatalogMatch> {
    let mut seen = BTreeSet::new();
    let mut matches = Vec::new();

    for gpu in &profile.gpus {
        push_match(&mut matches, &mut seen, &gpu.name, GpuCatalogSource::Cuda);
    }

    for device in &profile.vulkan.devices {
        push_match(
            &mut matches,
            &mut seen,
            &vulkan_device_name(device),
            GpuCatalogSource::Vulkan,
        );
    }

    push_match(
        &mut matches,
        &mut seen,
        &profile.cpu.model,
        GpuCatalogSource::Cpu,
    );

    matches
}

fn push_match(
    matches: &mut Vec<GpuCatalogMatch>,
    seen: &mut BTreeSet<&'static str>,
    detected_name: &str,
    source: GpuCatalogSource,
) {
    let Some(gpu) = known_gpu(detected_name) else {
        return;
    };
    if !seen.insert(gpu.canonical_name) {
        return;
    }
    matches.push(GpuCatalogMatch {
        detected_name: detected_name.to_string(),
        gpu,
        source,
    });
}

fn vulkan_device_name(line: &str) -> String {
    line.split_once('=')
        .map(|(_, value)| value.trim().to_string())
        .unwrap_or_else(|| line.trim().to_string())
}

fn normalize_name(name: &str) -> String {
    let mut normalized = String::with_capacity(name.len());
    let mut previous_space = true;
    for character in name.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            normalized.push(character);
            previous_space = false;
        } else if !previous_space {
            normalized.push(' ');
            previous_space = true;
        }
    }
    normalized.trim().to_string()
}

const KNOWN_GPUS: &[KnownGpu] = &[
    KnownGpu {
        canonical_name: "NVIDIA Blackwell B200 Tensor Core GPU",
        vendor: GpuVendor::Nvidia,
        use_case: GpuUseCase::Datacenter,
        memory_gb: Some(180),
        memory_kind: "HBM3e",
        backend: AiBackend::Cuda,
        local_ai_tier: "frontier training / massive inference",
        aliases: &["blackwell b200", "nvidia b200", "b200 tensor core"],
    },
    KnownGpu {
        canonical_name: "NVIDIA H200 Tensor Core GPU",
        vendor: GpuVendor::Nvidia,
        use_case: GpuUseCase::Datacenter,
        memory_gb: Some(141),
        memory_kind: "HBM3e",
        backend: AiBackend::Cuda,
        local_ai_tier: "enterprise LLM workhorse",
        aliases: &["nvidia h200", "h200 tensor core"],
    },
    KnownGpu {
        canonical_name: "NVIDIA RTX PRO 6000 Blackwell",
        vendor: GpuVendor::Nvidia,
        use_case: GpuUseCase::Workstation,
        memory_gb: Some(96),
        memory_kind: "GDDR7 ECC",
        backend: AiBackend::Cuda,
        local_ai_tier: "best single-card workstation tier",
        aliases: &["rtx pro 6000 blackwell", "nvidia rtx pro 6000"],
    },
    KnownGpu {
        canonical_name: "NVIDIA GeForce RTX 5090",
        vendor: GpuVendor::Nvidia,
        use_case: GpuUseCase::ConsumerHighEnd,
        memory_gb: Some(32),
        memory_kind: "GDDR7",
        backend: AiBackend::Cuda,
        local_ai_tier: "30B-70B quantized local AI",
        aliases: &["geforce rtx 5090", "rtx 5090"],
    },
    KnownGpu {
        canonical_name: "NVIDIA GeForce RTX 5080",
        vendor: GpuVendor::Nvidia,
        use_case: GpuUseCase::ConsumerMidRange,
        memory_gb: Some(16),
        memory_kind: "GDDR7",
        backend: AiBackend::Cuda,
        local_ai_tier: "8B-14B high-throughput local AI",
        aliases: &["geforce rtx 5080", "rtx 5080"],
    },
    KnownGpu {
        canonical_name: "NVIDIA GeForce RTX 5070",
        vendor: GpuVendor::Nvidia,
        use_case: GpuUseCase::ConsumerMidRange,
        memory_gb: Some(12),
        memory_kind: "GDDR7",
        backend: AiBackend::Cuda,
        local_ai_tier: "7B-12B quantized local AI",
        aliases: &["geforce rtx 5070", "rtx 5070"],
    },
    KnownGpu {
        canonical_name: "NVIDIA GeForce RTX 5060 Ti 16GB",
        vendor: GpuVendor::Nvidia,
        use_case: GpuUseCase::ConsumerMidRange,
        memory_gb: Some(16),
        memory_kind: "GDDR7",
        backend: AiBackend::Cuda,
        local_ai_tier: "budget 16GB CUDA sweet spot",
        aliases: &["geforce rtx 5060 ti", "rtx 5060 ti"],
    },
    KnownGpu {
        canonical_name: "NVIDIA GeForce RTX 5060",
        vendor: GpuVendor::Nvidia,
        use_case: GpuUseCase::ConsumerEntry,
        memory_gb: Some(8),
        memory_kind: "GDDR7",
        backend: AiBackend::Cuda,
        local_ai_tier: "small 7B quantized models",
        aliases: &["geforce rtx 5060", "rtx 5060"],
    },
    KnownGpu {
        canonical_name: "NVIDIA GeForce RTX 4090",
        vendor: GpuVendor::Nvidia,
        use_case: GpuUseCase::ConsumerHighEnd,
        memory_gb: Some(24),
        memory_kind: "GDDR6X",
        backend: AiBackend::Cuda,
        local_ai_tier: "excellent used-market 24GB CUDA card",
        aliases: &["geforce rtx 4090", "rtx 4090"],
    },
    KnownGpu {
        canonical_name: "NVIDIA GeForce RTX 4060 Ti 16GB",
        vendor: GpuVendor::Nvidia,
        use_case: GpuUseCase::ConsumerMidRange,
        memory_gb: Some(16),
        memory_kind: "GDDR6",
        backend: AiBackend::Cuda,
        local_ai_tier: "efficient mid-range 8B-14B models",
        aliases: &["geforce rtx 4060 ti", "rtx 4060 ti"],
    },
    KnownGpu {
        canonical_name: "NVIDIA GeForce RTX 3090",
        vendor: GpuVendor::Nvidia,
        use_case: GpuUseCase::ConsumerHighEnd,
        memory_gb: Some(24),
        memory_kind: "GDDR6X",
        backend: AiBackend::Cuda,
        local_ai_tier: "great-value 24GB prosumer card",
        aliases: &["geforce rtx 3090", "rtx 3090"],
    },
    KnownGpu {
        canonical_name: "NVIDIA GeForce RTX 3060 12GB",
        vendor: GpuVendor::Nvidia,
        use_case: GpuUseCase::ConsumerEntry,
        memory_gb: Some(12),
        memory_kind: "GDDR6",
        backend: AiBackend::Cuda,
        local_ai_tier: "cheapest native CUDA entry tier",
        aliases: &["geforce rtx 3060", "rtx 3060"],
    },
    KnownGpu {
        canonical_name: "AMD Instinct MI355X",
        vendor: GpuVendor::Amd,
        use_case: GpuUseCase::Datacenter,
        memory_gb: Some(288),
        memory_kind: "HBM3E",
        backend: AiBackend::Rocm,
        local_ai_tier: "massive ROCm memory capacity",
        aliases: &["instinct mi355x", "amd mi355x", "mi355x"],
    },
    KnownGpu {
        canonical_name: "AMD Instinct MI300X",
        vendor: GpuVendor::Amd,
        use_case: GpuUseCase::Datacenter,
        memory_gb: Some(192),
        memory_kind: "HBM3",
        backend: AiBackend::Rocm,
        local_ai_tier: "large-memory ROCm datacenter tier",
        aliases: &["instinct mi300x", "amd mi300x", "mi300x"],
    },
    KnownGpu {
        canonical_name: "AMD Radeon AI PRO R9700",
        vendor: GpuVendor::Amd,
        use_case: GpuUseCase::Workstation,
        memory_gb: Some(32),
        memory_kind: "GDDR6",
        backend: AiBackend::Rocm,
        local_ai_tier: "32GB workstation ROCm card",
        aliases: &["radeon ai pro r9700", "amd radeon ai pro r9700", "r9700"],
    },
    KnownGpu {
        canonical_name: "AMD Radeon RX 7900 XTX",
        vendor: GpuVendor::Amd,
        use_case: GpuUseCase::ConsumerHighEnd,
        memory_gb: Some(24),
        memory_kind: "GDDR6",
        backend: AiBackend::Rocm,
        local_ai_tier: "24GB VRAM-per-dollar ROCm card",
        aliases: &["radeon rx 7900 xtx", "rx 7900 xtx", "7900 xtx"],
    },
    KnownGpu {
        canonical_name: "AMD Radeon RX 9070 XT",
        vendor: GpuVendor::Amd,
        use_case: GpuUseCase::ConsumerMidRange,
        memory_gb: Some(16),
        memory_kind: "GDDR6",
        backend: AiBackend::Rocm,
        local_ai_tier: "current-gen 16GB ROCm option",
        aliases: &["radeon rx 9070 xt", "rx 9070 xt", "9070 xt"],
    },
    KnownGpu {
        canonical_name: "AMD Radeon RX 9060 XT 16GB",
        vendor: GpuVendor::Amd,
        use_case: GpuUseCase::ConsumerMidRange,
        memory_gb: Some(16),
        memory_kind: "GDDR6",
        backend: AiBackend::Rocm,
        local_ai_tier: "affordable 16GB RDNA 4 option",
        aliases: &[
            "radeon rx 9060 xt",
            "rx 9060 xt",
            "9060 xt",
            "radeon rx 960 xt",
            "rx 960 xt",
        ],
    },
    KnownGpu {
        canonical_name: "AMD BC-250 / Cyan Skillfish",
        vendor: GpuVendor::Amd,
        use_case: GpuUseCase::HeadlessAppliance,
        memory_gb: Some(16),
        memory_kind: "unified GDDR6",
        backend: AiBackend::Vulkan,
        local_ai_tier: "headless Qwen 9B Q4 Vulkan appliance",
        aliases: &[
            "amd bc 250",
            "amd bc250",
            "bc 250",
            "bc250",
            "asrock bc 250",
            "asrock bc250",
            "cyan skillfish",
            "gfx1013",
            "amd radeon graphics radv gfx1013",
        ],
    },
    KnownGpu {
        canonical_name: "Intel Gaudi 3 AI Accelerator",
        vendor: GpuVendor::Intel,
        use_case: GpuUseCase::Datacenter,
        memory_gb: Some(128),
        memory_kind: "HBM2e",
        backend: AiBackend::Gaudi,
        local_ai_tier: "Ethernet-scale AI accelerator",
        aliases: &["intel gaudi 3", "gaudi 3 ai accelerator", "gaudi3"],
    },
    KnownGpu {
        canonical_name: "Intel Arc Pro B70",
        vendor: GpuVendor::Intel,
        use_case: GpuUseCase::Workstation,
        memory_gb: Some(32),
        memory_kind: "GDDR6 ECC",
        backend: AiBackend::OneApiOpenVino,
        local_ai_tier: "affordable 32GB workstation tier",
        aliases: &["intel arc pro b70", "arc pro b70", "b70 graphics"],
    },
    KnownGpu {
        canonical_name: "Intel Arc B580",
        vendor: GpuVendor::Intel,
        use_case: GpuUseCase::ConsumerEntry,
        memory_gb: Some(12),
        memory_kind: "GDDR6",
        backend: AiBackend::OneApiOpenVino,
        local_ai_tier: "budget 12GB entry tier",
        aliases: &["intel arc b580", "arc b580", "b580 graphics"],
    },
    KnownGpu {
        canonical_name: "Intel Arc A770",
        vendor: GpuVendor::Intel,
        use_case: GpuUseCase::ConsumerEntry,
        memory_gb: Some(16),
        memory_kind: "GDDR6",
        backend: AiBackend::OneApiOpenVino,
        local_ai_tier: "discount 16GB entry tier",
        aliases: &["intel arc a770", "arc a770", "a770 graphics"],
    },
    KnownGpu {
        canonical_name: "Intel Core Ultra NPU",
        vendor: GpuVendor::Intel,
        use_case: GpuUseCase::Integrated,
        memory_gb: None,
        memory_kind: "shared memory",
        backend: AiBackend::Npu,
        local_ai_tier: "very light local AI only",
        aliases: &["intel core ultra", "core ultra processor", "core ultra"],
    },
];

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::{
        CmakeInfo, CompilerInfo, CpuFeatures, CudaCompatibility, DiskInfo, GitInfo, GpuInfo,
        MemInfo, MetalSupport, NvidiaDeviceNodes, SystemProfile, VulkanSupport,
    };

    use super::*;

    #[test]
    fn matches_cuda_catalog_names_without_confusing_5060_ti_and_5060() {
        let gpu = known_gpu("NVIDIA GeForce RTX 5060 Ti 16GB").expect("known RTX 5060 Ti");

        assert_eq!(gpu.canonical_name, "NVIDIA GeForce RTX 5060 Ti 16GB");
        assert_eq!(gpu.memory_gb, Some(16));
        assert_eq!(gpu.backend, AiBackend::Cuda);

        let base = known_gpu("NVIDIA GeForce RTX 5060").expect("known RTX 5060");
        assert_eq!(base.canonical_name, "NVIDIA GeForce RTX 5060");
        assert_eq!(base.memory_gb, Some(8));
    }

    #[test]
    fn accepts_rx_960_xt_as_typo_alias_for_rx_9060_xt() {
        let gpu = known_gpu("AMD Radeon RX 960 XT").expect("known RX alias");

        assert_eq!(gpu.canonical_name, "AMD Radeon RX 9060 XT 16GB");
        assert_eq!(gpu.backend, AiBackend::Rocm);
    }

    #[test]
    fn matches_bc250_as_vulkan_headless_appliance() {
        let gpu = known_gpu("deviceName = AMD Radeon Graphics (RADV GFX1013)")
            .expect("known BC-250 Vulkan device");

        assert_eq!(gpu.canonical_name, "AMD BC-250 / Cyan Skillfish");
        assert_eq!(gpu.backend, AiBackend::Vulkan);
        assert_eq!(gpu.use_case, GpuUseCase::HeadlessAppliance);
        assert_eq!(gpu.memory_gb, Some(16));
    }

    #[test]
    fn summarizes_workstation_and_datacenter_cards() {
        assert!(known_gpu("Intel Arc Pro B70")
            .expect("known B70")
            .summary()
            .contains("32GB GDDR6 ECC"));
        assert_eq!(
            known_gpu("AMD Instinct MI355X")
                .expect("known MI355X")
                .memory_gb,
            Some(288)
        );
    }

    #[test]
    fn collects_matches_from_cuda_vulkan_and_cpu_sources_once() {
        let profile = SystemProfile {
            compiler: Some(CompilerInfo {
                path: PathBuf::from("/usr/bin/c++"),
                version: "g++".to_string(),
                cpp17_ok: true,
                cpp17_error: None,
            }),
            cmake: Some(CmakeInfo {
                path: PathBuf::from("/usr/bin/cmake"),
                version: "3.28".to_string(),
                meets_minimum: true,
            }),
            git: Some(GitInfo {
                path: PathBuf::from("/usr/bin/git"),
                version: "2.45".to_string(),
                meets_minimum: true,
            }),
            cuda: CudaCompatibility::Compatible {
                archs: vec!["sm_89"],
            },
            rocm: crate::RocmSupport::default(),
            gpus: vec![GpuInfo {
                name: "NVIDIA GeForce RTX 4090".to_string(),
                memory_total_mb: 24_576,
                compute_cap: "8.9".to_string(),
                arch: Some("sm_89"),
            }],
            gpu_probe_error: None,
            nvidia_devices: NvidiaDeviceNodes {
                control: true,
                uvm: true,
                gpu_count: 1,
                errors: Vec::new(),
            },
            sccache: None,
            metal: MetalSupport {
                available: false,
                displays: Vec::new(),
            },
            vulkan: VulkanSupport {
                available: true,
                devices: vec![
                    "deviceName = Intel Arc B580 Graphics".to_string(),
                    "deviceName = AMD Radeon Graphics (RADV GFX1013)".to_string(),
                    "deviceName = NVIDIA GeForce RTX 4090".to_string(),
                ],
            },
            cpu: CpuFeatures {
                model: "Intel Core Ultra 9".to_string(),
                cores: 8,
                threads: 16,
                avx: true,
                avx2: true,
                avx512: false,
                neon: false,
                features: Vec::new(),
            },
            memory: MemInfo {
                total_mb: 65_536,
                available_mb: 32_768,
            },
            disk: DiskInfo {
                available_bytes: 8 * 1024 * 1024 * 1024,
                path: PathBuf::from("/tmp"),
            },
        };

        let matches = matches_from_system_profile(&profile);
        let names = matches
            .iter()
            .map(|matched| matched.gpu.canonical_name)
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            vec![
                "NVIDIA GeForce RTX 4090",
                "Intel Arc B580",
                "AMD BC-250 / Cyan Skillfish",
                "Intel Core Ultra NPU"
            ]
        );
    }
}
