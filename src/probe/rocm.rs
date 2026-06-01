//! AMD ROCm/HIP detection.
//!
//! Runs `hipconfig --full` to detect ROCm installation and GPU targets.

use super::RocmProbe;

pub async fn detect() -> RocmProbe {
    // Check if hipconfig exists
    let hipconfig = match tokio::process::Command::new("hipconfig")
        .arg("--full")
        .output()
        .await
    {
        Ok(output) if output.status.success() => output,
        _ => {
            // Also try hipconfig --version as a lighter check
            match tokio::process::Command::new("hipconfig")
                .arg("--version")
                .output()
                .await
            {
                Ok(output) if output.status.success() => {
                    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    return RocmProbe::Found {
                        version,
                        gpu_target: "unknown".to_string(),
                    };
                }
                _ => return RocmProbe::NotFound,
            }
        }
    };

    let stdout = String::from_utf8_lossy(&hipconfig.stdout);
    let version = extract_rocm_version(&stdout);
    let gpu_target = extract_gpu_target(&stdout);

    RocmProbe::Found {
        version,
        gpu_target,
    }
}

fn extract_rocm_version(output: &str) -> String {
    for line in output.lines() {
        if line.contains("HIP_VERSION") || line.contains("ROCM_VERSION") {
            if let Some(value) = line.split('=').nth(1) {
                return value.trim().to_string();
            }
        }
    }
    "detected".to_string()
}

fn extract_gpu_target(output: &str) -> String {
    for line in output.lines() {
        if line.contains("gfx") {
            return line.trim().to_string();
        }
    }
    "unknown".to_string()
}
