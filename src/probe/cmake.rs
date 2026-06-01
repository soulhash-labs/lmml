//! Map ProbeResult to cmake flags and ngl recommendations.
//!
//! Based on the detected hardware, generates optimal build flags
//! and runtime parameters for llama.cpp.

use super::{CudaProbe, ProbeResult, RocmProbe};

/// Generate cmake flags based on detected hardware.
pub fn generate_flags(result: &ProbeResult) -> Vec<String> {
    let mut flags = Vec::new();
    flags.push("-DCMAKE_BUILD_TYPE=Release".to_string());

    // CUDA
    if matches!(result.cuda, CudaProbe::Found { .. }) {
        flags.push("-DGGML_CUDA=ON".to_string());
        if let Some(archs) = cuda_architectures(result) {
            flags.push(format!("-DCMAKE_CUDA_ARCHITECTURES={archs}"));
        }

        // Check if we should force MMQ (older GPUs benefit)
        if let CudaProbe::Found {
            ref compute_cap, ..
        } = result.cuda
        {
            if compute_cap == "7.5" || compute_cap == "7.0" {
                // Turing/Volta — MMQ is fine
            }
        }
    } else {
        flags.push("-DGGML_CUDA=OFF".to_string());
    }

    // ROCm
    if matches!(result.rocm, RocmProbe::Found { .. }) {
        flags.push("-DGGML_HIP=ON".to_string());
        if let RocmProbe::Found { ref gpu_target, .. } = result.rocm {
            if gpu_target != "unknown" {
                flags.push(format!("-DGPU_TARGETS={gpu_target}"));
            }
        }
    }

    // Vulkan
    if result.vulkan {
        flags.push("-DGGML_VULKAN=ON".to_string());
    }

    // Metal (macOS only)
    if result.metal {
        flags.push("-DGGML_METAL=ON".to_string());
    }

    // CPU native optimizations
    let has_avx2 = result.cpu.features.iter().any(|f| f == "AVX2");
    let has_avx512 = result.cpu.features.iter().any(|f| f == "AVX-512");
    if has_avx2 || has_avx512 {
        flags.push("-DGGML_NATIVE=ON".to_string());
    }

    flags
}

fn cuda_architectures(result: &ProbeResult) -> Option<String> {
    let CudaProbe::Found {
        compute_cap, gpus, ..
    } = &result.cuda
    else {
        return None;
    };

    let mut caps: Vec<String> = gpus
        .iter()
        .filter_map(|gpu| normalize_compute_cap(&gpu.compute_cap))
        .collect();
    if caps.is_empty() {
        if let Some(cap) = normalize_compute_cap(compute_cap) {
            caps.push(cap);
        }
    }
    caps.sort();
    caps.dedup();
    if caps.is_empty() {
        None
    } else {
        Some(caps.join(";"))
    }
}

fn normalize_compute_cap(cap: &str) -> Option<String> {
    let trimmed = cap.trim();
    if trimmed.is_empty() || trimmed == "N/A" {
        return None;
    }
    let normalized: String = trimmed.chars().filter(|c| c.is_ascii_digit()).collect();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

/// Suggest the number of GPU layers (-ngl) based on VRAM.
pub fn suggest_ngl(result: &ProbeResult) -> u32 {
    let vram_gb = match result.cuda {
        CudaProbe::Found { vram_gb, .. } => vram_gb,
        _ => 0,
    };

    if vram_gb >= 12 {
        99 // 12GB+ can handle most models fully
    } else if vram_gb >= 8 {
        50 // partial offload for 8GB cards
    } else if vram_gb >= 4 {
        20 // modest offload
    } else {
        0 // CPU-only if no GPU or very low VRAM
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::{BlasProbe, CpuInfo, CudaGpu, CudaProbe, OsInfo, ProbeResult, RocmProbe};

    fn minimal_result() -> ProbeResult {
        ProbeResult {
            os: OsInfo {
                os_type: "Linux".into(),
                arch: "x86_64".into(),
                kernel: "6.8.0".into(),
            },
            cuda: CudaProbe::NotFound,
            blas: BlasProbe::NotFound,
            rocm: RocmProbe::NotFound,
            vulkan: false,
            metal: false,
            cpu: CpuInfo {
                model: "Test CPU".into(),
                features: vec!["AVX2".into()],
                cores: 8,
                threads: 16,
            },
            ram_gb: 32,
            suggested_cmake_flags: vec![],
            suggested_ngl: 0,
            warnings: vec![],
            log_lines: vec![],
        }
    }

    #[test]
    fn test_cpu_only_flags() {
        let result = minimal_result();
        let flags = generate_flags(&result);
        assert!(flags.contains(&"-DCMAKE_BUILD_TYPE=Release".to_string()));
        assert!(flags.contains(&"-DGGML_CUDA=OFF".to_string()));
        assert!(flags.contains(&"-DGGML_NATIVE=ON".to_string()));
    }

    #[test]
    fn test_cuda_flags() {
        let mut result = minimal_result();
        result.cuda = CudaProbe::Found {
            version: "12.4".into(),
            gpu_name: "GTX 1080 Ti".into(),
            vram_gb: 11,
            compute_cap: "6.1".into(),
            gpus: vec![CudaGpu {
                name: "GTX 1080 Ti".into(),
                vram_gb: 11,
                compute_cap: "6.1".into(),
            }],
        };
        let flags = generate_flags(&result);
        assert!(flags.contains(&"-DGGML_CUDA=ON".to_string()));
        assert!(flags.contains(&"-DCMAKE_CUDA_ARCHITECTURES=61".to_string()));
        assert!(!flags.contains(&"-DGGML_CUDA=OFF".to_string()));
    }

    #[test]
    fn test_multi_gpu_cuda_arch_flags() {
        let mut result = minimal_result();
        result.cuda = CudaProbe::Found {
            version: "12.4".into(),
            gpu_name: "RTX 4090".into(),
            vram_gb: 24,
            compute_cap: "8.9".into(),
            gpus: vec![
                CudaGpu {
                    name: "RTX 4090".into(),
                    vram_gb: 24,
                    compute_cap: "8.9".into(),
                },
                CudaGpu {
                    name: "RTX 3070".into(),
                    vram_gb: 8,
                    compute_cap: "8.6".into(),
                },
            ],
        };
        let flags = generate_flags(&result);
        assert!(flags.contains(&"-DCMAKE_CUDA_ARCHITECTURES=86;89".to_string()));
    }

    #[test]
    fn test_ngl_cpu() {
        let result = minimal_result();
        assert_eq!(suggest_ngl(&result), 0);
    }

    #[test]
    fn test_ngl_cuda_24gb() {
        let mut result = minimal_result();
        result.cuda = CudaProbe::Found {
            version: "12.4".into(),
            gpu_name: "RTX 4090".into(),
            vram_gb: 24,
            compute_cap: "8.9".into(),
            gpus: vec![],
        };
        assert_eq!(suggest_ngl(&result), 99);
    }

    #[test]
    fn test_ngl_cuda_8gb() {
        let mut result = minimal_result();
        result.cuda = CudaProbe::Found {
            version: "12.4".into(),
            gpu_name: "RTX 3070".into(),
            vram_gb: 8,
            compute_cap: "8.6".into(),
            gpus: vec![],
        };
        assert_eq!(suggest_ngl(&result), 50);
    }

    #[test]
    fn test_vulkan_flag() {
        let mut result = minimal_result();
        result.vulkan = true;
        let flags = generate_flags(&result);
        assert!(flags.contains(&"-DGGML_VULKAN=ON".to_string()));
    }

    #[test]
    fn test_rocm_flag() {
        let mut result = minimal_result();
        result.rocm = RocmProbe::Found {
            version: "6.0".into(),
            gpu_target: "gfx1100".into(),
        };
        let flags = generate_flags(&result);
        assert!(flags.contains(&"-DGGML_HIP=ON".to_string()));
        assert!(flags.contains(&"-DGPU_TARGETS=gfx1100".to_string()));
    }

    #[test]
    fn test_no_avx_no_native() {
        let mut result = minimal_result();
        result.cpu = CpuInfo {
            model: "Test CPU".into(),
            features: vec![],
            cores: 4,
            threads: 8,
        };
        let flags = generate_flags(&result);
        assert!(!flags.contains(&"-DGGML_NATIVE=ON".to_string()));
    }
}
