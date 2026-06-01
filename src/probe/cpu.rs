//! CPU feature detection.
//!
//! Detects available ISA extensions (AVX2, AVX-512, NEON, AMX, etc.)
//! and the number of cores/threads.

use super::CpuInfo;

pub async fn detect() -> CpuInfo {
    let model = detect_cpu_model();
    let (cores, threads) = detect_core_count();
    let features = detect_cpu_features();

    CpuInfo {
        model,
        cores,
        threads,
        features,
    }
}

fn detect_cpu_model() -> String {
    // Try /proc/cpuinfo on Linux
    if cfg!(target_os = "linux") {
        if let Ok(content) = std::fs::read_to_string("/proc/cpuinfo") {
            for line in content.lines() {
                if line.starts_with("model name") {
                    if let Some(name) = line.split(':').nth(1) {
                        return name.trim().to_string();
                    }
                }
            }
        }
    }

    // macOS
    if cfg!(target_os = "macos") {
        if let Ok(output) = std::process::Command::new("sysctl")
            .args(["-n", "machdep.cpu.brand_string"])
            .output()
        {
            return String::from_utf8_lossy(&output.stdout).trim().to_string();
        }
    }

    "Unknown CPU".to_string()
}

fn detect_core_count() -> (u32, u32) {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(4);

    // On Linux, physical cores may differ from logical cores
    if cfg!(target_os = "linux") {
        if let Ok(content) = std::fs::read_to_string("/proc/cpuinfo") {
            let mut core_ids: Vec<&str> = content
                .lines()
                .filter(|l| l.starts_with("core id"))
                .collect();
            core_ids.sort();
            core_ids.dedup();
            let count = core_ids.len() as u32;
            if count > 0 {
                return (count, cores);
            }
        }
    };

    (cores / 2, cores)
}

fn detect_cpu_features() -> Vec<String> {
    let mut features = Vec::new();

    // Linux: parse /proc/cpuinfo flags
    if cfg!(target_os = "linux") {
        if let Ok(content) = std::fs::read_to_string("/proc/cpuinfo") {
            for line in content.lines() {
                if line.starts_with("flags") || line.starts_with("Features") {
                    let flags = line.to_lowercase();
                    let feature_map: [(&str, &str); 8] = [
                        ("avx2", "AVX2"),
                        ("avx512f", "AVX-512"),
                        ("neon", "NEON"),
                        ("sse4_1", "SSE4.1"),
                        ("sse4_2", "SSE4.2"),
                        ("amx", "AMX"),
                        ("sve", "SVE"),
                        ("zvfh", "ZVFH"),
                    ];
                    for (flag, name) in &feature_map {
                        if flags.contains(flag) {
                            features.push(name.to_string());
                        }
                    }
                    break; // only first CPU
                }
            }
        }
    }

    // macOS: sysctl for features
    if cfg!(target_os = "macos") {
        if let Ok(output) = std::process::Command::new("sysctl")
            .args(["-n", "hw.optional.arm.FEAT_Flag"])
            .output()
        {
            let out = String::from_utf8_lossy(&output.stdout);
            if out.contains("1") {
                features.push("NEON".to_string());
            }
        }
    }

    if features.is_empty() {
        features.push("generic".to_string());
    }

    features
}
