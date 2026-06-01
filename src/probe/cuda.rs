//! NVIDIA GPU + CUDA toolkit detection.
//!
//! Runs `nvidia-smi` to find GPU model, VRAM, and driver version.
//! Runs `nvcc --version` to find CUDA toolkit version.

use super::{CudaGpu, CudaProbe};

pub async fn detect() -> CudaProbe {
    let queried_gpus = query_gpu_list().await.unwrap_or_default();

    // First check if nvidia-smi exists
    let nvidia_smi = match tokio::process::Command::new("nvidia-smi").output().await {
        Ok(output) if output.status.success() => output,
        _ => return CudaProbe::NotFound,
    };

    let stdout = String::from_utf8_lossy(&nvidia_smi.stdout);
    let gpu_name = extract_gpu_name(&stdout);
    let vram_mb = extract_vram_mb(&stdout);
    let driver_version = extract_driver_version(&stdout);
    let fallback_vram_gb = (vram_mb + 500) / 1024;
    let gpus = if queried_gpus.is_empty() {
        vec![CudaGpu {
            name: gpu_name.clone(),
            vram_gb: fallback_vram_gb,
            compute_cap: String::new(),
        }]
    } else {
        queried_gpus
    };
    let primary = gpus.first().cloned().unwrap_or_default();

    // Check nvcc for CUDA version
    let cuda_version = match tokio::process::Command::new("nvcc")
        .arg("--version")
        .output()
        .await
    {
        Ok(output) if output.status.success() => {
            let out = String::from_utf8_lossy(&output.stdout);
            extract_cuda_version(&out)
        }
        _ => "unknown".to_string(),
    };

    CudaProbe::Found {
        version: format!("{cuda_version} (driver: {driver_version})"),
        gpu_name: primary.name,
        vram_gb: primary.vram_gb,
        compute_cap: primary.compute_cap,
        gpus,
    }
}

async fn query_gpu_list() -> Option<Vec<CudaGpu>> {
    let output = tokio::process::Command::new("nvidia-smi")
        .args([
            "--query-gpu=name,memory.total,compute_cap",
            "--format=csv,noheader,nounits",
        ])
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let gpus: Vec<_> = stdout
        .lines()
        .filter_map(|line| {
            let mut parts = line.split(',').map(str::trim);
            let name = parts.next()?.to_string();
            let vram_mb = parts.next()?.parse::<u32>().ok()?;
            let compute_cap = parts.next().unwrap_or_default().to_string();
            Some(CudaGpu {
                name,
                vram_gb: (vram_mb + 500) / 1024,
                compute_cap,
            })
        })
        .collect();

    if gpus.is_empty() {
        None
    } else {
        Some(gpus)
    }
}

fn extract_gpu_name(output: &str) -> String {
    for line in output.lines() {
        if line.contains("GeForce")
            || line.contains("Tesla")
            || line.contains("RTX")
            || line.contains("Quadro")
        {
            let parts: Vec<&str> = line.split('|').collect();
            if parts.len() >= 2 {
                return parts[1].trim().to_string();
            }
            return line.trim().to_string();
        }
    }
    "Unknown NVIDIA GPU".to_string()
}

fn extract_vram_mb(output: &str) -> u32 {
    for line in output.lines() {
        if line.contains("MiB") {
            // Look for pattern like "10240 MiB"
            if let Some(pos) = line.find("MiB") {
                let before = &line[..pos].trim();
                if let Some(last_space) = before.rfind(' ') {
                    if let Ok(mb) = before[last_space + 1..].parse::<u32>() {
                        return mb;
                    }
                }
            }
        }
    }
    0
}

fn extract_driver_version(output: &str) -> String {
    for line in output.lines() {
        if line.contains("Driver Version") {
            if let Some(ver) = line.split_whitespace().find(|s| s.contains('.')) {
                return ver.to_string();
            }
        }
    }
    "unknown".to_string()
}

fn extract_cuda_version(output: &str) -> String {
    for line in output.lines() {
        if line.contains("release") {
            if let Some(rel) = line
                .split_whitespace()
                .find(|s| s.chars().all(|c| c.is_ascii_digit() || c == '.'))
            {
                return rel.trim_end_matches(',').to_string();
            }
        }
    }
    "unknown".to_string()
}
